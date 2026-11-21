//! Real PostgreSQL/HTTP verification of pickup settings, snapshots and checkout behavior.
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
        let schema = format!("brunch_pickup_test_{}", Uuid::new_v4().simple());
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
    async fn order_input(&self, client: &Client, email: &str, billing: Option<Value>) -> Value {
        let products = self.get(client, "/api/products").await;
        let options = self.get(client, "/api/delivery-options").await;
        let settings = self.get(client, "/api/settings").await;
        let product = &products[0];
        let subtotal = product["price_cents"].as_i64().unwrap();
        let fee = if settings["free_delivery_from_cents"]
            .as_i64()
            .is_some_and(|n| subtotal >= n)
        {
            0
        } else {
            settings["delivery_fee_cents"].as_i64().unwrap()
        };
        let mut body = json!({"idempotency_key":Uuid::new_v4(),"terms_version":1,"email":email,"name":"Test Customer","phone":"+420777123456","street":"Testovací 12","city":"Praha","postal_code":"11000","delivery_date":options["dates"][0]["value"],"delivery_slot":options["slots"][0],"payment_method":"cash_on_delivery","items":[{"product_id":product["id"],"quantity":1}],"expected_total_cents":subtotal+fee});
        if let Some(billing) = billing {
            body["billing"] = billing;
        }
        body
    }
    async fn wait_for_blocked_request(&self, blocker: i32) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
                )
                .bind(blocker)
                .fetch_one(&self.pool)
                .await
                .unwrap();
                if blocked {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("HTTP request must reach the deliberately held row lock");
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
fn enabled_settings(version: i64) -> Value {
    json!({"enabled":true,"name":"Pekárna","street":"Odběrná 5","city":"Liberec","postal_code":"460 01","instructions":"Vchod ze dvora.","parking_free":true,"slots":["08:00-12:00","14:00–16:00"],"version":version})
}
async fn enable(app: &TestApp, admin: &Client) -> Value {
    let settings = app.get(admin, "/api/admin/pickup-settings").await;
    app.call(
        admin,
        Method::PUT,
        "/api/admin/pickup-settings",
        Some(&enabled_settings(settings["version"].as_i64().unwrap())),
        StatusCode::OK,
    )
    .await
}
async fn pickup_input(app: &TestApp, client: &Client, version: &Value) -> Value {
    let mut input = app.order_input(client, "pickup@example.test", None).await;
    let price: i64 = sqlx::query_scalar("SELECT price_cents FROM products WHERE id=$1")
        .bind(input["items"][0]["product_id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    input["fulfillment_method"] = json!("pickup");
    input["pickup_version"] = version.clone();
    input["expected_total_cents"] = json!(price);
    input["delivery_slot"] = json!("08:00–12:00");
    for key in ["street", "city", "postal_code"] {
        input.as_object_mut().unwrap().remove(key);
    }
    input
}
async fn counts(app: &TestApp) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('orders',(SELECT count(*) FROM orders),'items',(SELECT count(*) FROM order_items),'outbox',(SELECT count(*) FROM outbox),'users',(SELECT count(*) FROM users),'stock',(SELECT jsonb_agg(jsonb_build_array(id,stock,version) ORDER BY id) FROM products))")
        .fetch_one(&app.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_settings_require_admin_valid_configuration_and_current_version() {
    let app = TestApp::new().await;
    let guest = app.client();
    let customer = app.login("customer@example.test").await;
    let admin = app.login("admin@example.test").await;
    let initial = app.get(&admin, "/api/admin/pickup-settings").await;
    assert_eq!(initial["enabled"], false);
    assert_eq!(initial["parking_free"], true);
    assert_eq!(initial["slots"], json!([]));
    assert_eq!(app.get(&guest, "/api/settings").await["pickup"], initial);
    for (client, status) in [
        (&guest, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        app.call(
            client,
            Method::GET,
            "/api/admin/pickup-settings",
            None,
            status,
        )
        .await;
        app.call(
            client,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&initial),
            status,
        )
        .await;
    }
    let response = admin
        .put(format!("{}/api/admin/pickup-settings", app.url))
        .header("x-brunch-request", "")
        .json(&initial)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = admin
        .put(format!("{}/api/admin/pickup-settings", app.url))
        .header("origin", "https://foreign.example.test")
        .json(&initial)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for (key, value) in [
        ("name", json!("")),
        ("street", json!("")),
        ("city", json!("")),
        ("postal_code", json!("1234")),
        ("name", json!("a".repeat(201))),
        ("instructions", json!("a".repeat(2001))),
        ("slots", json!([])),
        ("slots", json!(["08:00–12:00", "11:00–13:00"])),
        ("slots", json!(["25:00–26:00"])),
        ("slots", json!(["10:00–08:00"])),
        ("version", json!(0)),
    ] {
        let mut invalid = enabled_settings(1);
        invalid[key] = value;
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&invalid),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert_eq!(app.get(&admin, "/api/admin/pickup-settings").await, initial);
    let enabled = enable(&app, &admin).await;
    assert_eq!(enabled["postal_code"], "46001");
    assert_eq!(enabled["slots"], json!(["08:00–12:00", "14:00–16:00"]));
    let normal = app.get(&admin, "/api/admin/settings").await;
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&normal),
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        app.get(&admin, "/api/admin/pickup-settings").await,
        enabled,
        "Older settings clients cannot overwrite pickup configuration"
    );
    let update = enabled_settings(2);
    let (a, b) = tokio::join!(
        admin
            .put(format!("{}/api/admin/pickup-settings", app.url))
            .json(&update)
            .send(),
        admin
            .put(format!("{}/api/admin/pickup-settings", app.url))
            .json(&update)
            .send()
    );
    let mut statuses = [a.unwrap().status().as_u16(), b.unwrap().status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let mut disabled = initial;
    disabled["version"] = json!(3);
    let saved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&disabled),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["enabled"], false);
    assert_eq!(saved["street"], "");
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_is_free_without_delivery_coverage_and_snapshots_location_in_orders_and_mail() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let enabled = enable(&app, &admin).await;
    sqlx::query("DELETE FROM delivery_areas")
        .execute(&app.pool)
        .await
        .unwrap();
    let input = pickup_input(&app, &guest, &enabled["version"]).await;
    let order = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(order["fulfillment_method"], "pickup");
    assert_eq!(order["delivery_fee_cents"], 0);
    assert_eq!(order["total_cents"], order["subtotal_cents"]);
    assert_eq!(order["street"], "Odběrná 5");
    assert_eq!(order["city"], "Liberec");
    assert_eq!(order["postal_code"], "46001");
    assert_eq!(
        order["pickup_location"],
        json!({"name":"Pekárna","street":"Odběrná 5","city":"Liberec","postal_code":"46001","instructions":"Vchod ze dvora.","parking_free":true})
    );
    let original = counts(&app).await;
    let mut changed = enabled.clone();
    changed["street"] = json!("Nová 99");
    changed["parking_free"] = json!(false);
    let changed = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&changed),
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["id"],
        order["id"]
    );
    assert_eq!(counts(&app).await, original);
    let mut stale = input.clone();
    stale["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&stale),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(counts(&app).await, original);
    let mut disabled = changed;
    disabled["enabled"] = json!(false);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/pickup-settings",
        Some(&disabled),
        StatusCode::OK,
    )
    .await;
    let replay = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(replay["pickup_location"], order["pickup_location"]);
    let messages: Vec<String> =
        sqlx::query_scalar("SELECT body FROM outbox WHERE to_email='pickup@example.test'")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(messages.len(), 1);
    for text in [
        "Osobní odběr: 0,00 Kč",
        "Hotově při vyzvednutí",
        "Parkování zdarma.",
        "Odběrná 5",
        "Vchod ze dvora.",
    ] {
        assert!(messages[0].contains(text), "missing {text}");
    }
    let mut version = order["version"].clone();
    for (status, payment) in [
        ("confirmed", "unpaid"),
        ("out_for_delivery", "unpaid"),
        ("delivered", "paid"),
    ] {
        let updated = app
            .call(
                &admin,
                Method::PATCH,
                &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
                Some(&json!({"status":status,"payment_status":payment,"version":version})),
                StatusCode::OK,
            )
            .await;
        version = updated["version"].clone();
    }
    let ready:(String,String)=sqlx::query_as("SELECT subject,body FROM outbox WHERE to_email='pickup@example.test' AND subject LIKE '%Připravena k vyzvednutí%'").fetch_one(&app.pool).await.unwrap();
    assert!(ready.1.contains("Platíte až při vyzvednutí."));
    assert!(ready.1.contains("Odběrná 5"));
    assert!(!ready.1.contains("Nová 99"));
    assert!(ready.1.contains("Parkování zdarma."));
    let finished:String=sqlx::query_scalar("SELECT subject FROM outbox WHERE to_email='pickup@example.test' AND subject LIKE '%Vyzvednuta%'").fetch_one(&app.pool).await.unwrap();
    assert!(finished.contains("Vyzvednuta"));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_rejects_stale_disabled_invalid_and_forged_orders_without_side_effects() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let before = counts(&app).await;
    let disabled = pickup_input(&app, &guest, &json!(1)).await;
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&disabled),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(counts(&app).await, before);
    let enabled = enable(&app, &admin).await;
    let input = pickup_input(&app, &guest, &enabled["version"]).await;
    for (key, value, status) in [
        ("pickup_version", json!(1), StatusCode::CONFLICT),
        ("pickup_version", Value::Null, StatusCode::BAD_REQUEST),
        ("pickup_version", json!(0), StatusCode::BAD_REQUEST),
        (
            "fulfillment_method",
            json!("teleport"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "delivery_slot",
            json!("07:00–09:00"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "delivery_date",
            json!("2000-01-03"),
            StatusCode::BAD_REQUEST,
        ),
        ("name", json!(""), StatusCode::BAD_REQUEST),
        ("phone", json!(""), StatusCode::BAD_REQUEST),
        (
            "expected_total_cents",
            json!(input["expected_total_cents"].as_i64().unwrap() + 4900),
            StatusCode::CONFLICT,
        ),
    ] {
        let mut invalid = input.clone();
        invalid[key] = value;
        app.call(&guest, Method::POST, "/api/orders", Some(&invalid), status)
            .await;
        assert_eq!(counts(&app).await, before, "No effects for invalid {key}");
    }
    let mut forged = input.clone();
    forged["street"] = json!("Customer-controlled store");
    forged["city"] = json!("Wrong");
    forged["postal_code"] = json!("bad");
    let order = app.post(&guest, "/api/orders", &forged).await;
    assert_eq!(order["street"], "Odběrná 5");
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["id"],
        order["id"],
        "Ignored delivery address is canonical for pickup retries"
    );
    let mut conflicting = input.clone();
    conflicting["pickup_version"] = json!(3);
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&conflicting),
        StatusCode::CONFLICT,
    )
    .await;
    let mut legacy = app.order_input(&guest, "delivery@example.test", None).await;
    let delivered = app.post(&guest, "/api/orders", &legacy).await;
    assert_eq!(delivered["fulfillment_method"], "delivery");
    assert!(delivered["pickup_location"].is_null());
    legacy["fulfillment_method"] = json!("delivery");
    assert_eq!(
        app.post(&guest, "/api/orders", &legacy).await["id"],
        delivered["id"]
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_configuration_change_serializes_with_checkout_and_preserves_selected_address() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let enabled = enable(&app, &admin).await;
    let input = pickup_input(&app, &guest, &enabled["version"]).await;
    let mut held = app.pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM products WHERE id=$1 FOR UPDATE")
        .bind(input["items"][0]["product_id"].as_i64().unwrap())
        .fetch_one(&mut *held)
        .await
        .unwrap();
    let request = guest.post(format!("{}/api/orders", app.url)).json(&input);
    let checkout = tokio::spawn(async move { request.send().await.unwrap() });
    app.wait_for_blocked_request(blocker).await;
    let mut config = enabled.clone();
    config["street"] = json!("Nové místo 10");
    let request = admin
        .put(format!("{}/api/admin/pickup-settings", app.url))
        .json(&config);
    let update = tokio::spawn(async move { request.send().await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(5),async{
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE query LIKE 'UPDATE pickup_settings SET%' AND cardinality(pg_blocking_pids(pid))>0)").fetch_one(&app.pool).await.unwrap();
            if blocked {break}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    held.commit().await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), checkout)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let order: Value = result.json().await.unwrap();
    assert_eq!(order["street"], "Odběrná 5");
    let updated = tokio::time::timeout(std::time::Duration::from_secs(5), update)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(
        app.get(&guest, "/api/settings").await["pickup"]["street"],
        "Nové místo 10"
    );
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["pickup_location"],
        order["pickup_location"]
    );
    let mut stale = input;
    stale["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&stale),
        StatusCode::CONFLICT,
    )
    .await;
    app.close().await;
}
