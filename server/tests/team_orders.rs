//! Real isolated PostgreSQL tests for collaborative carts, without creating orders.
use brunch_server::{AppState, Config, app, auth, commerce, migrate};
use chrono::{Duration, Utc};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{str::FromStr, sync::Arc};
use tokio::task::JoinHandle;
use uuid::Uuid;

struct TestApp {
    url: String,
    pool: PgPool,
    database: PgPool,
    schema: String,
    task: JoinHandle<()>,
}
impl TestApp {
    async fn new() -> Self {
        let database_url =
            std::env::var("TEST_DATABASE_URL").expect("isolated local TEST_DATABASE_URL required");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_team_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&database)
            .await
            .unwrap();
        let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        commerce::seed(&pool, "orders@example.test").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let config = Config {
            app_env: brunch_server::config::AppEnvironment::Development,
            trust_caddy_client_ip: false,
            payment_timeout_minutes: 60,
            teya: None,
            database_url,
            bind: address,
            public_url: url.clone(),
            order_email: "orders@example.test".into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port: 1025,
            smtp_username: None,
            smtp_password: None,
            smtp_from: "brunch@example.test".into(),
            smtp_tls: false,
            secure_cookies: false,
            dist_dir: "../public".into(),
        };
        let router = app(AppState {
            pool: pool.clone(),
            config: Arc::new(config),
        });
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap()
        });
        Self {
            url,
            pool,
            database,
            schema,
            task,
        }
    }
    fn client(&self) -> Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-brunch-request", "1".parse().unwrap());
        Client::builder().default_headers(headers).build().unwrap()
    }
    async fn user(&self) -> Client {
        let id = Uuid::new_v4();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sqlx::query("INSERT INTO users(id,email) VALUES($1,$2)")
            .bind(id)
            .bind(format!("{id}@example.test"))
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')").bind(auth::token_hash(&token)).bind(id).execute(&self.pool).await.unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-brunch-request", "1".parse().unwrap());
        headers.insert("cookie", format!("brunch_session={token}").parse().unwrap());
        Client::builder().default_headers(headers).build().unwrap()
    }
    async fn call(
        &self,
        client: &Client,
        method: Method,
        path: &str,
        body: Option<Value>,
        status: StatusCode,
    ) -> Value {
        let mut req = client.request(method, format!("{}{path}", self.url));
        if let Some(body) = body {
            req = req.json(&body)
        }
        let response = req.send().await.unwrap();
        let actual = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(actual, status, "{path}: {text}");
        serde_json::from_str(&text).unwrap()
    }
    async fn team(&self, client: &Client) -> Value {
        self.call(client,Method::POST,"/api/team-orders",Some(json!({"name":"Snídaně pro tým","deadline":(Utc::now()+Duration::hours(2)).with_timezone(&chrono_tz::Europe::Prague).format("%Y-%m-%dT%H:%M").to_string()})),StatusCode::OK).await
    }
    async fn product(&self) -> i64 {
        sqlx::query_scalar("SELECT id FROM products ORDER BY id LIMIT 1")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.database)
            .await
            .unwrap();
        self.database.close().await;
    }
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn team_access_is_private_and_contributor_tokens_are_isolated() {
    let app = TestApp::new().await;
    let guest = app.client();
    let owner = app.user().await;
    let other = app.user().await;
    app.call(
        &guest,
        Method::GET,
        "/api/team-orders",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let team = app.team(&owner).await;
    let id = team["id"].as_str().unwrap();
    let token = team["invite_token"].as_str().unwrap();
    let path = format!("/api/team-orders/{id}");
    let join = format!("/api/team-joins/{token}");
    for suffix in ["", "/export"] {
        app.call(
            &other,
            Method::GET,
            &format!("{path}{suffix}"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    app.call(
        &other,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::NOT_FOUND,
    )
    .await;
    app.call(
        &guest,
        Method::GET,
        "/api/team-joins/not-a-token",
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    let public = app
        .call(&guest, Method::GET, &join, None, StatusCode::OK)
        .await;
    assert!(public.get("owner_id").is_none());
    assert!(public.get("invite_token").is_none());
    assert!(public.get("contributions").is_none());
    let product = app.product().await;
    let input = json!({"name":"Jana","items":[{"product_id":product,"quantity":1}]});
    let saved = app
        .call(
            &guest,
            Method::PUT,
            &join,
            Some(input.clone()),
            StatusCode::OK,
        )
        .await;
    let contribution_token = saved["contribution_token"].as_str().unwrap();
    let stored: String =
        sqlx::query_scalar("SELECT token_hash FROM team_contributions WHERE id=$1")
            .bind(Uuid::parse_str(saved["id"].as_str().unwrap()).unwrap())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored, auth::token_hash(contribution_token));
    assert_ne!(stored, contribution_token);
    let public = app
        .call(&guest, Method::GET, &join, None, StatusCode::OK)
        .await;
    assert!(public["own"].is_null());
    assert!(!public.to_string().contains("Jana"));
    let response = guest
        .get(format!("{}{join}", app.url))
        .header("x-team-contribution", contribution_token)
        .send()
        .await
        .unwrap();
    let own: Value = response.json().await.unwrap();
    assert_eq!(own["own"]["name"], "Jana");
    let mut wrong = input.clone();
    wrong["contribution_token"] = json!("a".repeat(64));
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(wrong),
        StatusCode::FORBIDDEN,
    )
    .await;
    let response = Client::new()
        .put(format!("{}{join}", app.url))
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = guest
        .put(format!("{}{join}", app.url))
        .header("origin", "https://evil.example")
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let detail = app
        .call(&owner, Method::GET, &path, None, StatusCode::OK)
        .await;
    assert_eq!(detail["contributions"][0]["name"], "Jana");
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn team_minimum_variants_close_and_export_never_create_an_order() {
    let app = TestApp::new().await;
    let owner = app.user().await;
    let guest = app.client();
    let team = app.team(&owner).await;
    let id = team["id"].as_str().unwrap();
    let token = team["invite_token"].as_str().unwrap();
    let path = format!("/api/team-orders/{id}");
    let join = format!("/api/team-joins/{token}");
    let product = app.product().await;
    sqlx::query(
        "UPDATE products SET stock=10,minimum_quantity=3,variants=ARRAY['Sýr','Šunka'] WHERE id=$1",
    )
    .bind(product)
    .execute(&app.pool)
    .await
    .unwrap();
    for quantity in [0, 100] {
        app.call(&guest,Method::PUT,&join,Some(json!({"name":"Jana","items":[{"product_id":product,"quantity":quantity,"variant":"Sýr"}]})),StatusCode::BAD_REQUEST).await;
    }
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(json!({"name":"Jana","items":[{"product_id":product,"quantity":1}]})),
        StatusCode::CONFLICT,
    )
    .await;
    let a=app.call(&guest,Method::PUT,&join,Some(json!({"name":"Jana","items":[{"product_id":product,"quantity":1,"variant":"Sýr"}]})),StatusCode::OK).await;
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::CONFLICT,
    )
    .await;
    let b=app.call(&guest,Method::PUT,&join,Some(json!({"name":"Pavel","items":[{"product_id":product,"quantity":2,"variant":"Šunka"}]})),StatusCode::OK).await;
    let conflict = app
        .call(
            &owner,
            Method::POST,
            &format!("{path}/close"),
            Some(json!({})),
            StatusCode::CONFLICT,
        )
        .await;
    assert!(
        conflict["error"]
            .as_str()
            .unwrap()
            .contains("různé varianty")
    );
    app.call(
        &owner,
        Method::PUT,
        &format!("{path}/contributions/{}", b["id"].as_str().unwrap()),
        Some(json!({"name":"Pavel","items":[{"product_id":product,"quantity":2,"variant":"Sýr"}]})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(json!({"name":"Jana","items":[],"contribution_token":a["contribution_token"]})),
        StatusCode::CONFLICT,
    )
    .await;
    app.call(
        &owner,
        Method::PUT,
        &format!("{path}/contributions/{}", a["id"].as_str().unwrap()),
        Some(json!({"name":"Jana","items":[]})),
        StatusCode::CONFLICT,
    )
    .await;
    let exported = app
        .call(
            &owner,
            Method::GET,
            &format!("{path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        exported["items"],
        json!([{"product_id":product,"quantity":3,"variant":"Sýr"}])
    );
    assert_eq!(exported["team_id"], id);
    assert!(exported["notes"].as_str().unwrap().contains("Jana"));
    assert!(!exported.to_string().contains(token));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 10);
    sqlx::query("UPDATE products SET stock=2 WHERE id=$1")
        .bind(product)
        .execute(&app.pool)
        .await
        .unwrap();
    app.call(
        &owner,
        Method::GET,
        &format!("{path}/export"),
        None,
        StatusCode::CONFLICT,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn concurrent_contributions_cannot_exceed_stock_and_deadline_is_enforced() {
    let app = TestApp::new().await;
    let owner = app.user().await;
    let guest = app.client();
    let team = app.team(&owner).await;
    let id = team["id"].as_str().unwrap();
    let token = team["invite_token"].as_str().unwrap();
    let path = format!("/api/team-orders/{id}");
    let join = format!("/api/team-joins/{token}");
    let product = app.product().await;
    sqlx::query("UPDATE products SET stock=5 WHERE id=$1")
        .bind(product)
        .execute(&app.pool)
        .await
        .unwrap();
    let input = json!({"name":"Kolega","items":[{"product_id":product,"quantity":4}]});
    let (a, b) = tokio::join!(
        guest.put(format!("{}{join}", app.url)).json(&input).send(),
        guest.put(format!("{}{join}", app.url)).json(&input).send()
    );
    let statuses = [a.unwrap().status(), b.unwrap().status()];
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM team_contributions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query(
        "UPDATE team_orders SET deadline=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(Uuid::parse_str(id).unwrap())
    .execute(&app.pool)
    .await
    .unwrap();
    let view = app
        .call(&guest, Method::GET, &join, None, StatusCode::OK)
        .await;
    assert_eq!(view["accepting"], false);
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(input),
        StatusCode::CONFLICT,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    let exported = app
        .call(
            &owner,
            Method::GET,
            &format!("{path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(exported["items"][0]["quantity"], 4);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn first_contribution_retry_is_idempotent_and_reopening_preserves_selections() {
    let app = TestApp::new().await;
    let owner = app.user().await;
    let guest = app.client();
    let team = app.team(&owner).await;
    let id = team["id"].as_str().unwrap();
    let token = team["invite_token"].as_str().unwrap();
    let path = format!("/api/team-orders/{id}");
    let join = format!("/api/team-joins/{token}");
    let product = app.product().await;
    let key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let input =
        json!({"name":"Jana","client_key":key,"items":[{"product_id":product,"quantity":1}]});
    let first = app
        .call(
            &guest,
            Method::PUT,
            &join,
            Some(input.clone()),
            StatusCode::OK,
        )
        .await;
    let replay = app
        .call(&guest, Method::PUT, &join, Some(input), StatusCode::OK)
        .await;
    assert_eq!(first, replay);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM team_contributions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    let cutoff = (Utc::now() + Duration::hours(3))
        .with_timezone(&chrono_tz::Europe::Prague)
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/reopen"),
        Some(json!({"deadline":cutoff})),
        StatusCode::OK,
    )
    .await;
    let view = app
        .call(&guest, Method::GET, &join, None, StatusCode::OK)
        .await;
    assert_eq!(view["accepting"], true);
    assert_eq!(view["contributor_count"], 1);
    let second = app.team(&owner).await;
    app.call(
        &guest,
        Method::PUT,
        &format!(
            "/api/team-joins/{}",
            second["invite_token"].as_str().unwrap()
        ),
        Some(json!({"name":"Jana","client_key":key,"items":[]})),
        StatusCode::FORBIDDEN,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn team_checkout_cannot_be_claimed_twice_with_different_order_keys() {
    let app = TestApp::new().await;
    let owner = app.user().await;
    let other = app.user().await;
    let guest = app.client();
    let team = app.team(&owner).await;
    let id = team["id"].as_str().unwrap();
    let token = team["invite_token"].as_str().unwrap();
    let path = format!("/api/team-orders/{id}");
    let join = format!("/api/team-joins/{token}");
    let product = app.product().await;
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(json!({"name":"Jana","items":[{"product_id":product,"quantity":1}]})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    sqlx::query("INSERT INTO delivery_areas(postal_code,label) VALUES('11000','Test')")
        .execute(&app.pool)
        .await
        .unwrap();
    let who = app
        .call(&owner, Method::GET, "/api/auth/me", None, StatusCode::OK)
        .await;
    let options = app
        .call(
            &guest,
            Method::GET,
            "/api/delivery-options",
            None,
            StatusCode::OK,
        )
        .await;
    let settings = app
        .call(&guest, Method::GET, "/api/settings", None, StatusCode::OK)
        .await;
    let price: i64 = sqlx::query_scalar("SELECT price_cents FROM products WHERE id=$1")
        .bind(product)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let fee = if settings["free_delivery_from_cents"]
        .as_i64()
        .is_some_and(|n| price >= n)
    {
        0
    } else {
        settings["delivery_fee_cents"].as_i64().unwrap()
    };
    let mut input = json!({"idempotency_key":Uuid::new_v4(),"team_order_id":id,"terms_version":1,"email":who["user"]["email"],"name":"Jana Testová","phone":"+420777123456","street":"Testovací 1","city":"Praha","postal_code":"11000","delivery_date":options["dates"][0]["value"],"delivery_slot":options["slots"][0],"payment_method":"cash_on_delivery","items":[{"product_id":product,"quantity":1}],"expected_total_cents":price+fee});
    let exported = app
        .call(
            &owner,
            Method::GET,
            &format!("{path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    input["team_revision"] = exported["revision"].clone();
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(input.clone()),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let other_user = app
        .call(&other, Method::GET, "/api/auth/me", None, StatusCode::OK)
        .await;
    let mut other_input = input.clone();
    other_input["email"] = other_user["user"]["email"].clone();
    app.call(
        &other,
        Method::POST,
        "/api/orders",
        Some(other_input),
        StatusCode::NOT_FOUND,
    )
    .await;
    let exported = app
        .call(
            &owner,
            Method::GET,
            &format!("{path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    input["team_revision"] = exported["revision"].clone();
    let reopen_deadline = (Utc::now() + Duration::hours(3))
        .with_timezone(&chrono_tz::Europe::Prague)
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/reopen"),
        Some(json!({"deadline":reopen_deadline})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        "/api/orders",
        Some(input.clone()),
        StatusCode::CONFLICT,
    )
    .await;
    let latest = app
        .call(
            &owner,
            Method::GET,
            &format!("{path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_ne!(latest["revision"], input["team_revision"]);
    input["team_revision"] = latest["revision"].clone();
    let first = input.clone();
    input["idempotency_key"] = json!(Uuid::new_v4());
    let (a, b) = tokio::join!(
        owner
            .post(format!("{}/api/orders", app.url))
            .json(&first)
            .send(),
        owner
            .post(format!("{}/api/orders", app.url))
            .json(&input)
            .send()
    );
    let responses = [a.unwrap(), b.unwrap()];
    let statuses = [responses[0].status(), responses[1].status()];
    let mut success = None;
    for response in responses {
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        if status == StatusCode::OK {
            success = Some(body)
        } else {
            assert_eq!(status, StatusCode::CONFLICT, "{body}")
        }
    }
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    let success = success.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let team_detail = app
        .call(&owner, Method::GET, &path, None, StatusCode::OK)
        .await;
    assert_eq!(team_detail["team"]["order_id"], success["id"]);
    app.call(
        &owner,
        Method::GET,
        &format!("{path}/export"),
        None,
        StatusCode::CONFLICT,
    )
    .await;
    let cutoff = (Utc::now() + Duration::hours(3))
        .with_timezone(&chrono_tz::Europe::Prague)
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/reopen"),
        Some(json!({"deadline":cutoff})),
        StatusCode::CONFLICT,
    )
    .await;
    // Reopen and checkout take the same user -> team lock order. One wins,
    // the other observes the resulting revision/order and returns conflict.
    let racing_team = app.team(&owner).await;
    let racing_id = racing_team["id"].as_str().unwrap();
    let racing_path = format!("/api/team-orders/{racing_id}");
    app.call(
        &guest,
        Method::PUT,
        &format!(
            "/api/team-joins/{}",
            racing_team["invite_token"].as_str().unwrap()
        ),
        Some(json!({"name":"Jana","items":[{"product_id":product,"quantity":1}]})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &owner,
        Method::POST,
        &format!("{racing_path}/close"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    let racing_export = app
        .call(
            &owner,
            Method::GET,
            &format!("{racing_path}/export"),
            None,
            StatusCode::OK,
        )
        .await;
    let mut racing_input = input.clone();
    racing_input["idempotency_key"] = json!(Uuid::new_v4());
    racing_input["team_order_id"] = json!(racing_id);
    racing_input["team_revision"] = racing_export["revision"].clone();
    let race = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            owner
                .post(format!("{}/api/orders", app.url))
                .json(&racing_input)
                .send(),
            owner
                .post(format!("{}{racing_path}/reopen", app.url))
                .json(&json!({"deadline":reopen_deadline}))
                .send()
        )
    })
    .await
    .expect("reopen and checkout must not deadlock");
    let race_status = [race.0.unwrap().status(), race.1.unwrap().status()];
    assert_eq!(
        race_status.iter().filter(|s| **s == StatusCode::OK).count(),
        1
    );
    assert_eq!(
        race_status
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    app.close().await;
}
