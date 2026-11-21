//! PostgreSQL/HTTP checks for product and combined banquet minimums.
use brunch_server::{AppState, Config, app, auth, commerce, migrate};
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
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL must point to a local test database");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_banquet_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&database)
            .await
            .unwrap();
        let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .connect_with(options)
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        commerce::seed(&pool, "orders@example.test").await.unwrap();
        sqlx::query("INSERT INTO delivery_areas(postal_code,label) VALUES ('11000','Test')")
            .execute(&pool)
            .await
            .unwrap();
        auth::initialize_admin(&pool, "admin@example.test", "Test-only-admin-password-2026")
            .await
            .unwrap();
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
        Client::builder()
            .cookie_store(true)
            .default_headers(headers)
            .build()
            .unwrap()
    }
    async fn call(
        &self,
        client: &Client,
        method: Method,
        path: &str,
        body: Option<&Value>,
        expected: StatusCode,
    ) -> Value {
        let mut request = client.request(method.clone(), format!("{}{path}", self.url));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, expected, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap()
    }
    async fn get(&self, client: &Client, path: &str) -> Value {
        self.call(client, Method::GET, path, None, StatusCode::OK)
            .await
    }
    async fn post(&self, client: &Client, path: &str, body: &Value) -> Value {
        self.call(client, Method::POST, path, Some(body), StatusCode::OK)
            .await
    }
    async fn login(&self, email: &str) -> Client {
        let client = self.client();
        if email == "admin@example.test" {
            self.post(
                &client,
                "/api/admin/auth/login",
                &json!({"email":email,"password":"Test-only-admin-password-2026"}),
            )
            .await;
        } else {
            self.post(&client, "/api/auth/request", &json!({"email":email}))
                .await;
            let body:String=sqlx::query_scalar("SELECT body FROM outbox WHERE to_email=$1 AND subject='Přihlášení do Brunch Era' ORDER BY created_at DESC LIMIT 1").bind(email).fetch_one(&self.pool).await.unwrap();
            let token = body
                .split("login_token=")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            self.post(&client, "/api/auth/verify", &json!({"token":token}))
                .await;
        }
        client
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
    async fn fixture_products(&self) -> Vec<i64> {
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM products ORDER BY id LIMIT 4")
            .fetch_all(&self.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE products SET price_cents=100,stock=1000,minimum_quantity=10,banquet=true WHERE id=ANY($1)")
            .bind(&ids[..3]).execute(&self.pool).await.unwrap();
        sqlx::query("UPDATE products SET price_cents=100,stock=1000,minimum_quantity=0,banquet=false WHERE id=$1")
            .bind(ids[3]).execute(&self.pool).await.unwrap();
        ids
    }
    async fn order(&self, client: &Client, items: Value, pickup: bool) -> Value {
        let options = self.get(client, "/api/delivery-options").await;
        let subtotal: i64 = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["quantity"].as_i64().unwrap() * 100)
            .sum();
        let mut body = json!({"idempotency_key":Uuid::new_v4(),"terms_version":1,"email":"banquet@example.test","name":"Test Customer","phone":"+420777123456","street":"Testovací 12","city":"Praha","postal_code":"11000","delivery_date":options["dates"][0]["value"],"delivery_slot":options["slots"][0],"payment_method":"cash_on_delivery","items":items,"expected_total_cents":subtotal+if pickup || subtotal>=35000 {0} else {4900}});
        if pickup {
            body["fulfillment_method"] = json!("pickup");
            body["pickup_version"] = json!(1);
            body["delivery_slot"] = json!("08:00–17:00");
            for field in ["street", "city", "postal_code"] {
                body.as_object_mut().unwrap().remove(field);
            }
        }
        body
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
async fn banquet_catalog_and_settings_validate_and_preserve_older_clients() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let public = app.get(&guest, "/api/settings").await;
    assert_eq!(public["banquet_minimum_quantity"], 30);
    let normal = app
        .post(
            &admin,
            "/api/admin/products",
            &json!({"name":"Běžné zboží","price_cents":100}),
        )
        .await;
    assert_eq!(normal["banquet"], false);
    assert_eq!(normal["minimum_quantity"], 0);
    for minimum in [None, Some(0), Some(-1), Some(100)] {
        let mut body = json!({"name":"Banketní pečivo","price_cents":100,"banquet":true});
        if let Some(value) = minimum {
            body["minimum_quantity"] = json!(value);
        }
        app.call(
            &admin,
            Method::POST,
            "/api/admin/products",
            Some(&body),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let product = app.post(&admin,"/api/admin/products",&json!({"name":"Banketní houska","price_cents":100,"banquet":true,"minimum_quantity":10})).await;
    assert_eq!(product["banquet"], true);
    assert_eq!(product["minimum_quantity"], 10);
    let path = format!("/api/admin/products/{}", product["id"]);
    let updated = app.call(&admin,Method::PUT,&path,Some(&json!({"name":"Přejmenovaná houska","price_cents":100,"version":product["version"]})),StatusCode::OK).await;
    assert_eq!(updated["banquet"], true);
    assert_eq!(updated["minimum_quantity"], 10);
    app.call(&admin,Method::PUT,&path,Some(&json!({"name":"Houska","price_cents":100,"minimum_quantity":0,"version":updated["version"]})),StatusCode::BAD_REQUEST).await;
    let standard=app.call(&admin,Method::PUT,&path,Some(&json!({"name":"Běžná houska","price_cents":100,"minimum_quantity":0,"banquet":false,"version":updated["version"]})),StatusCode::OK).await;
    assert_eq!(standard["banquet"], false);
    assert_eq!(standard["minimum_quantity"], 0);
    app.call(&admin,Method::PUT,&path,Some(&json!({"name":"Houska","price_cents":100,"banquet":true,"version":standard["version"]})),StatusCode::BAD_REQUEST).await;
    let optional = app
        .post(
            &admin,
            "/api/admin/products",
            &json!({"name":"Běžné s minimem","price_cents":100,"minimum_quantity":5}),
        )
        .await;
    assert_eq!(optional["minimum_quantity"], 5);
    assert_eq!(optional["banquet"], false);

    let mut settings = app.get(&admin, "/api/admin/settings").await;
    for minimum in [0, -1, 4951] {
        settings["banquet_minimum_quantity"] = json!(minimum);
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&settings),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    settings["banquet_minimum_quantity"] = json!(40);
    app.call(
        &guest,
        Method::PUT,
        "/api/admin/settings",
        Some(&settings),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let saved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&settings),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["banquet_minimum_quantity"], 40);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&settings),
        StatusCode::CONFLICT,
    )
    .await;
    let mut older = saved.clone();
    older
        .as_object_mut()
        .unwrap()
        .remove("banquet_minimum_quantity");
    let preserved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&older),
            StatusCode::OK,
        )
        .await;
    assert_eq!(preserved["banquet_minimum_quantity"], 40);
    assert_eq!(
        app.get(&guest, "/api/settings").await["banquet_minimum_quantity"],
        40
    );
    assert!(
        sqlx::query("UPDATE products SET banquet=true,minimum_quantity=0 WHERE id=$1")
            .bind(normal["id"].as_i64().unwrap())
            .execute(&app.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE shop_settings SET banquet_minimum_quantity=0")
            .execute(&app.pool)
            .await
            .is_err()
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn mixed_banquet_totals_exclude_normal_goods_and_rejections_have_no_side_effects() {
    let app = TestApp::new().await;
    let guest = app.client();
    let ids = app.fixture_products().await;
    let item = |id: usize, quantity: i32| json!({"product_id":ids[id],"quantity":quantity});
    for items in [
        json!([item(0, 10), item(1, 10), item(3, 10)]),
        json!([item(0, 29)]),
        json!([item(0, 9), item(1, 11), item(2, 10)]),
    ] {
        let body = app.order(&guest, items, false).await;
        let error = app
            .call(
                &guest,
                Method::POST,
                "/api/orders",
                Some(&body),
                StatusCode::CONFLICT,
            )
            .await;
        assert!(error.to_string().contains("Minimální odběr"));
    }
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM orders),(SELECT count(*) FROM outbox),(SELECT count(*) FROM users WHERE email='banquet@example.test'),(SELECT sum(stock)::bigint FROM products WHERE id=ANY($1))")
        .bind(&ids).fetch_one(&app.pool).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 4000));
    // The exact requested 10 + 10 + 10 mix passes, independently of delivery fees.
    let body = app
        .order(
            &guest,
            json!([item(0, 10), item(1, 10), item(2, 10)]),
            false,
        )
        .await;
    let accepted = app.post(&guest, "/api/orders", &body).await;
    assert_eq!(accepted["subtotal_cents"], 3000);
    let stocks: Vec<i32> =
        sqlx::query_scalar("SELECT stock FROM products WHERE id=ANY($1) ORDER BY id")
            .bind(&ids)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(stocks, [990, 990, 990, 1000]);
    sqlx::query("UPDATE shop_settings SET banquet_minimum_quantity=40")
        .execute(&app.pool)
        .await
        .unwrap();
    let replay = app.post(&guest, "/api/orders", &body).await;
    assert_eq!(replay["id"], accepted["id"]);
    let mut new_attempt = body.clone();
    new_attempt["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&new_attempt),
        StatusCode::CONFLICT,
    )
    .await;
    let forty = app
        .order(
            &guest,
            json!([item(0, 20), item(1, 10), item(2, 10)]),
            false,
        )
        .await;
    app.post(&guest, "/api/orders", &forty).await;
    let ordinary = app.order(&guest, json!([item(3, 1)]), false).await;
    app.post(&guest, "/api/orders", &ordinary).await;
    sqlx::query("UPDATE products SET active=false,minimum_quantity=99 WHERE id=$1")
        .bind(ids[0])
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        app.post(&guest, "/api/orders", &body).await["id"],
        accepted["id"]
    );
    let totals: (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM orders),(SELECT count(*) FROM outbox)")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(totals, (3, 6));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_enforces_the_same_combined_and_individual_minimums() {
    let app = TestApp::new().await;
    let guest = app.client();
    let ids = app.fixture_products().await;
    sqlx::query("UPDATE pickup_settings SET enabled=true,name='Pekárna',street='Testovací 1',city='Praha',postal_code='11000',slots=ARRAY['08:00–17:00']").execute(&app.pool).await.unwrap();
    let short = app
        .order(&guest, json!([{"product_id":ids[0],"quantity":20}]), true)
        .await;
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&short),
        StatusCode::CONFLICT,
    )
    .await;
    let enough=app.order(&guest,json!([{"product_id":ids[0],"quantity":10},{"product_id":ids[1],"quantity":10},{"product_id":ids[2],"quantity":10}]),true).await;
    let accepted = app.post(&guest, "/api/orders", &enough).await;
    assert_eq!(accepted["fulfillment_method"], "pickup");
    assert_eq!(accepted["delivery_fee_cents"], 0);
    assert_eq!(accepted["total_cents"], 3000);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn team_can_collect_partial_choices_but_must_meet_combined_minimum_to_close_or_export() {
    let app = TestApp::new().await;
    let guest = app.client();
    let owner = app.user().await;
    let ids = app.fixture_products().await;
    let team=app.post(&owner,"/api/team-orders",&json!({"name":"Banket pro tým","deadline":(chrono::Utc::now()+chrono::Duration::hours(2)).with_timezone(&chrono_tz::Europe::Prague).format("%Y-%m-%dT%H:%M").to_string()})).await;
    let join = format!("/api/team-joins/{}", team["invite_token"].as_str().unwrap());
    let path = format!("/api/team-orders/{}", team["id"].as_str().unwrap());
    for (i, id) in [ids[0], ids[1], ids[3]].into_iter().enumerate() {
        app.call(
            &guest,
            Method::PUT,
            &join,
            Some(&json!({"name":format!("Kolega {i}"),"items":[{"product_id":id,"quantity":10}]})),
            StatusCode::OK,
        )
        .await;
    }
    app.call(
        &owner,
        Method::POST,
        &format!("{path}/close"),
        Some(&json!({})),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(
        app.get(&owner, &path).await["team"]["closed_at"],
        Value::Null
    );
    app.call(
        &guest,
        Method::PUT,
        &join,
        Some(&json!({"name":"Kolegyně","items":[{"product_id":ids[2],"quantity":10}]})),
        StatusCode::OK,
    )
    .await;
    app.post(&owner, &format!("{path}/close"), &json!({})).await;
    let export = app.get(&owner, &format!("{path}/export")).await;
    assert_eq!(export["items"].as_array().unwrap().len(), 4);
    sqlx::query("UPDATE shop_settings SET banquet_minimum_quantity=31")
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
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    app.close().await;
}
