//! Real HTTP integration tests. Every test uses a separate PostgreSQL schema.
//! Run explicitly with TEST_DATABASE_URL=... cargo test --test api -- --ignored.
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

fn product_input(value: &Value) -> Value {
    let mut value = value.clone();
    value.as_object_mut().unwrap().remove("id");
    value
}

impl TestApp {
    async fn new() -> Self {
        Self::with_delivery_area(true).await
    }
    async fn with_delivery_area(allow_delivery: bool) -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("Set TEST_DATABASE_URL to a disposable local PostgreSQL database before running integration tests");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("TEST_DATABASE_URL must point to a reachable PostgreSQL database");
        let schema = format!("brunch_test_{}", Uuid::new_v4().simple());
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
        if allow_delivery {
            sqlx::query("INSERT INTO delivery_areas (postal_code,label) VALUES ('11000','Testovací oblast')")
                .execute(&pool)
                .await
                .unwrap();
        }
        auth::bootstrap_admin(&pool, "admin@example.test")
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let config = Config {
            payment_timeout_minutes: 60,
            teya: None,
            database_url,
            bind: address,
            public_url: url.clone(),
            order_email: "orders@example.test".into(),
            admin_email: "admin@example.test".into(),
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
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
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
        serde_json::from_str(&text)
            .unwrap_or_else(|_| panic!("Non-JSON response on {path}: {text}"))
    }
    async fn get(&self, client: &Client, path: &str) -> Value {
        self.call(client, Method::GET, path, None, StatusCode::OK)
            .await
    }
    async fn post(&self, client: &Client, path: &str, body: &Value) -> Value {
        self.call(client, Method::POST, path, Some(body), StatusCode::OK)
            .await
    }
    async fn token(&self, client: &Client, email: &str) -> String {
        self.post(client, "/api/auth/request", &json!({"email": email}))
            .await;
        let body: String = sqlx::query_scalar("SELECT body FROM outbox WHERE to_email=$1 AND subject='Přihlášení do Brunch' ORDER BY created_at DESC LIMIT 1")
            .bind(email.to_lowercase()).fetch_one(&self.pool).await.unwrap();
        body.split("login_token=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .into()
    }
    async fn login(&self, email: &str) -> Client {
        let client = self.client();
        let token = self.token(&client, email).await;
        self.post(&client, "/api/auth/verify", &json!({"token":token}))
            .await;
        client
    }
    async fn order(&self, client: &Client, email: &str, product: &Value, quantity: i64) -> Value {
        let options = self.get(client, "/api/delivery-options").await;
        let settings = self.get(client, "/api/settings").await;
        let subtotal = product["price_cents"].as_i64().unwrap() * quantity;
        let fee = if settings["free_delivery_from_cents"]
            .as_i64()
            .is_some_and(|limit| subtotal >= limit)
        {
            0
        } else {
            settings["delivery_fee_cents"].as_i64().unwrap()
        };
        json!({
            "idempotency_key":Uuid::new_v4(),"email":email,"street":"Testovací 12",
            "city":"Praha","postal_code":"110 00","notes":"3. patro, výtah vlevo. Zvoňte Novák.",
            "delivery_date":options["dates"][0]["value"],"delivery_slot":options["slots"][0],
            "payment_method":"card_on_delivery","items":[{"product_id":product["id"],"quantity":quantity}],
            "expected_total_cents":subtotal + fee
        })
    }
    async fn count(&self, table: &str) -> i64 {
        assert!(["orders", "order_items", "outbox", "users", "product_prices"].contains(&table));
        sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
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
async fn guest_checkout_totals_transactional_mail_idempotency_and_price_snapshot() {
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let product = &products[0];
    let input = app.order(&guest, "guest@example.test", product, 2).await;
    let created = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(created["total_cents"], input["expected_total_cents"]);
    assert_eq!(created["payment_method"], "card_on_delivery");
    assert_eq!(created["payment_status"], "unpaid");
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("outbox").await, 2);
    assert_eq!(
        app.count("users").await,
        1,
        "guest checkout does not create an account"
    );
    let recipients: Vec<String> =
        sqlx::query_scalar("SELECT to_email FROM outbox ORDER BY to_email")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(recipients, ["guest@example.test", "orders@example.test"]);
    let bodies: Vec<String> = sqlx::query_scalar("SELECT body FROM outbox")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert!(
        bodies
            .iter()
            .all(|body| body.contains(created["number"].as_str().unwrap()))
    );
    assert!(bodies.iter().any(|body| body.contains("3. patro")));
    let replay = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(replay["id"], created["id"]);
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(
        app.count("outbox").await,
        2,
        "retry must not enqueue duplicate mail"
    );
    let mut altered = input.clone();
    altered["notes"] = json!("jiné místo předání");
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&altered),
        StatusCode::CONFLICT,
    )
    .await;
    let mut stale = input.clone();
    stale["idempotency_key"] = json!(Uuid::new_v4());
    stale["expected_total_cents"] = json!(1);
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&stale),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("outbox").await, 2);

    let admin = app.login("admin@example.test").await;
    let mut updated = product.clone();
    updated["name"] = json!("Nový název produktu");
    updated["price_cents"] = json!(product["price_cents"].as_i64().unwrap() + 1000);
    app.call(
        &admin,
        Method::PUT,
        &format!("/api/admin/products/{}", product["id"]),
        Some(&product_input(&updated)),
        StatusCode::OK,
    )
    .await;
    let detail = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", created["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(detail["items"][0]["name"], product["name"]);
    assert_eq!(
        detail["items"][0]["unit_price_cents"],
        product["price_cents"]
    );
    assert_eq!(detail["total_cents"], created["total_cents"]);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn stock_contention_cancellation_and_concurrent_idempotency() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let products = app.get(&admin, "/api/admin/products").await;
    let mut product = products[0].clone();
    product["stock"] = json!(1);
    let path = format!("/api/admin/products/{}", product["id"]);
    let product = app
        .call(
            &admin,
            Method::PUT,
            &path,
            Some(&product_input(&product)),
            StatusCode::OK,
        )
        .await;
    let input = app.order(&guest, "stock@example.test", &product, 1).await;
    let mut competing = input.clone();
    competing["idempotency_key"] = json!(Uuid::new_v4());
    let (first, second) = tokio::join!(
        guest
            .post(format!("{}/api/orders", app.url))
            .json(&input)
            .send(),
        guest
            .post(format!("{}/api/orders", app.url))
            .json(&competing)
            .send()
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(
        [first.status(), second.status()]
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [first.status(), second.status()]
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let order: Value = if first.status() == StatusCode::OK {
        first.json().await.unwrap()
    } else {
        second.json().await.unwrap()
    };
    assert_eq!(app.count("orders").await, 1);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 0);
    let order_path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
    let cancel = json!({"status":"cancelled","payment_status":"unpaid","version":order["version"]});
    let cancelled = app
        .call(
            &admin,
            Method::PATCH,
            &order_path,
            Some(&cancel),
            StatusCode::OK,
        )
        .await;
    app.call(
        &admin,
        Method::PATCH,
        &order_path,
        Some(&cancel),
        StatusCode::CONFLICT,
    )
    .await;
    let invalid =
        json!({"status":"confirmed","payment_status":"unpaid","version":cancelled["version"]});
    assert!(
        app.call(
            &admin,
            Method::PATCH,
            &order_path,
            Some(&invalid),
            StatusCode::CONFLICT
        )
        .await["error"]
            .is_string()
    );
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 1, "cancellation returns stock exactly once");

    let input = app.order(&guest, "repeat@example.test", &product, 1).await;
    let (first, second) = tokio::join!(
        app.post(&guest, "/api/orders", &input),
        app.post(&guest, "/api/orders", &input)
    );
    assert_eq!(first["id"], second["id"]);
    assert_eq!(app.count("orders").await, 2);
    assert_eq!(app.count("order_items").await, 2);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn passwordless_registration_one_use_expiry_logout_and_csrf() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = app.token(&client, "new@example.test").await;
    assert_eq!(
        app.count("users").await,
        1,
        "unverified email does not create a user"
    );
    let request = client
        .post(format!("{}/api/auth/verify", app.url))
        .json(&json!({"token":token}));
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"));
    assert_eq!(app.count("users").await, 2);
    app.call(
        &app.client(),
        Method::POST,
        "/api/auth/verify",
        Some(&json!({"token":token})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let user = app.get(&client, "/api/auth/me").await;
    assert_eq!(user["user"]["email"], "new@example.test");
    assert_eq!(user["user"]["is_admin"], false);
    let raw = Client::new();
    let response = raw
        .post(format!("{}/api/auth/logout", app.url))
        .header("cookie", cookie.split(';').next().unwrap())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "missing anti-CSRF header"
    );
    let response = client
        .post(format!("{}/api/auth/logout", app.url))
        .header("Origin", "https://attacker.example")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "cross-origin request"
    );
    app.post(&client, "/api/auth/logout", &json!({})).await;
    app.call(
        &client,
        Method::GET,
        "/api/auth/me",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let response = raw
        .get(format!("{}/api/auth/me", app.url))
        .header("cookie", cookie.split(';').next().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "logged-out session is revoked server-side"
    );
    let token = app.token(&client, "expired@example.test").await;
    sqlx::query("UPDATE login_tokens SET expires_at=now()-interval '1 second' WHERE email='expired@example.test'").execute(&app.pool).await.unwrap();
    app.call(
        &client,
        Method::POST,
        "/api/auth/verify",
        Some(&json!({"token":token})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(app.count("users").await, 2);
    let token = app.token(&client, "race@example.test").await;
    let other = app.client();
    let (first, second) = tokio::join!(
        client
            .post(format!("{}/api/auth/verify", app.url))
            .json(&json!({"token":token}))
            .send(),
        other
            .post(format!("{}/api/auth/verify", app.url))
            .json(&json!({"token":token}))
            .send()
    );
    let mut statuses = vec![
        first.unwrap().status().as_u16(),
        second.unwrap().status().as_u16(),
    ];
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [200, 400],
        "one token can be redeemed once under concurrency"
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn customer_order_isolation_and_all_admin_routes_require_role() {
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let guest_input = app
        .order(&guest, "alice@example.test", &products[0], 1)
        .await;
    app.post(&guest, "/api/orders", &guest_input).await;
    let alice = app.login("alice@example.test").await;
    let bob = app.login("bob@example.test").await;
    assert_eq!(
        app.get(&alice, "/api/orders/mine").await,
        json!([]),
        "email ownership must not reveal old guest orders"
    );
    let input = app
        .order(&alice, "alice@example.test", &products[0], 1)
        .await;
    let order = app.post(&alice, "/api/orders", &input).await;
    assert_eq!(
        app.get(&alice, "/api/orders/mine")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(app.get(&bob, "/api/orders/mine").await, json!([]));
    let other_email = app.order(&alice, "bob@example.test", &products[0], 1).await;
    app.post(&alice, "/api/orders", &other_email).await;
    assert_eq!(
        app.get(&bob, "/api/orders/mine").await,
        json!([]),
        "an order using another email remains a guest order"
    );
    assert_eq!(
        app.get(&alice, "/api/orders/mine")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.call(
        &guest,
        Method::GET,
        "/api/orders/mine",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let id = order["id"].as_str().unwrap();
    for path in [
        "/api/admin/products".to_owned(),
        format!("/api/admin/products/{}/prices", products[0]["id"]),
        "/api/admin/orders".into(),
        format!("/api/admin/orders/{id}"),
        "/api/admin/settings".into(),
        "/api/admin/customers".into(),
        "/api/admin/mail".into(),
    ] {
        app.call(&guest, Method::GET, &path, None, StatusCode::UNAUTHORIZED)
            .await;
        app.call(&bob, Method::GET, &path, None, StatusCode::FORBIDDEN)
            .await;
    }
    let settings = json!({"delivery_fee_cents":0,"free_delivery_from_cents":null,"order_email":"stolen@example.test","version":1});
    let order_update =
        json!({"status":"confirmed","payment_status":"unpaid","version":order["version"]});
    let methods = [
        (
            Method::POST,
            "/api/admin/products".to_owned(),
            product_input(&products[0]),
        ),
        (
            Method::PUT,
            format!("/api/admin/products/{}", products[0]["id"]),
            product_input(&products[0]),
        ),
        (
            Method::PATCH,
            format!("/api/admin/orders/{id}"),
            order_update,
        ),
        (Method::PUT, "/api/admin/settings".to_owned(), settings),
        (
            Method::POST,
            format!("/api/admin/mail/{}/retry", Uuid::new_v4()),
            json!({}),
        ),
    ];
    for (method, path, body) in methods {
        app.call(
            &guest,
            method.clone(),
            &path,
            Some(&body),
            StatusCode::UNAUTHORIZED,
        )
        .await;
        app.call(&bob, method, &path, Some(&body), StatusCode::FORBIDDEN)
            .await;
    }
    let response = alice
        .get(format!("{}/api/orders/mine", app.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    sqlx::query("UPDATE sessions SET expires_at=now()-interval '1 second' WHERE user_id=(SELECT id FROM users WHERE email='alice@example.test')").execute(&app.pool).await.unwrap();
    app.call(
        &alice,
        Method::GET,
        "/api/orders/mine",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn catalog_price_history_delivery_settings_conflicts_and_mail_retry() {
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let guest = app.client();
    let input = json!({"name":"Testovací müsli","description":"Čerstvé müsli","category":"Sladké","tag":"Novinka","weight":"200 g","price_cents":12345,"vegan":true,"ingredients":"Oves","allergens":"Lepek","image":"","active":true,"stock":5});
    let product = app.post(&admin, "/api/admin/products", &input).await;
    let path = format!("/api/admin/products/{}", product["id"]);
    let prices_path = format!("{path}/prices");
    assert_eq!(
        app.get(&admin, &prices_path)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut change = product.clone();
    change["price_cents"] = json!(12900);
    let updated = app
        .call(
            &admin,
            Method::PUT,
            &path,
            Some(&product_input(&change)),
            StatusCode::OK,
        )
        .await;
    app.call(
        &admin,
        Method::PUT,
        &path,
        Some(&product_input(&change)),
        StatusCode::CONFLICT,
    )
    .await;
    let prices = app.get(&admin, &prices_path).await;
    assert_eq!(prices.as_array().unwrap().len(), 2);
    assert!(
        prices
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["price_cents"] == 12345)
    );
    assert!(
        prices
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["price_cents"] == 12900 && !p["changed_by"].is_null())
    );
    let mut archive = updated.clone();
    archive["active"] = json!(false);
    app.call(
        &admin,
        Method::PUT,
        &path,
        Some(&product_input(&archive)),
        StatusCode::OK,
    )
    .await;
    assert!(
        !app.get(&guest, "/api/products")
            .await
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == product["id"])
    );
    assert_eq!(
        app.get(&admin, &prices_path)
            .await
            .as_array()
            .unwrap()
            .len(),
        2,
        "metadata edit does not change price history"
    );

    let old = app.get(&admin, "/api/admin/settings").await;
    let settings = json!({"delivery_fee_cents":7700,"free_delivery_from_cents":null,"order_email":"kitchen@example.test","version":old["version"]});
    let updated = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&settings),
            StatusCode::OK,
        )
        .await;
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&settings),
        StatusCode::CONFLICT,
    )
    .await;
    let public = app.get(&guest, "/api/settings").await;
    assert_eq!(public["delivery_fee_cents"], 7700);
    assert!(
        public.get("order_email").is_none(),
        "operational email is private"
    );
    let products = app.get(&guest, "/api/products").await;
    let input = app
        .order(&guest, "customer@example.test", &products[0], 1)
        .await;
    let first = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(first["delivery_fee_cents"], 7700);
    let recipients: Vec<String> =
        sqlx::query_scalar("SELECT to_email FROM outbox WHERE subject<>'Přihlášení do Brunch'")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(recipients.contains(&"kitchen@example.test".into()));
    let free = json!({"delivery_fee_cents":7700,"free_delivery_from_cents":products[0]["price_cents"],"order_email":"kitchen@example.test","version":updated["version"]});
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&free),
        StatusCode::OK,
    )
    .await;
    let input = app
        .order(&guest, "customer@example.test", &products[0], 1)
        .await;
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["delivery_fee_cents"],
        0
    );
    let mail_id: Uuid = sqlx::query_scalar("UPDATE outbox SET status='failed',attempts=8,last_error='Test SMTP failure' WHERE id=(SELECT id FROM outbox WHERE subject<>'Přihlášení do Brunch' LIMIT 1) RETURNING id").fetch_one(&app.pool).await.unwrap();
    let mail = app.get(&admin, "/api/admin/mail").await;
    assert!(
        mail.as_array()
            .unwrap()
            .iter()
            .all(|message| message.get("body").is_none()),
        "mail list must never expose login tokens"
    );
    let retry_path = format!("/api/admin/mail/{mail_id}/retry");
    app.post(&admin, &retry_path, &json!({})).await;
    app.call(
        &admin,
        Method::POST,
        &retry_path,
        Some(&json!({})),
        StatusCode::CONFLICT,
    )
    .await;
    let row: (String, i32, Option<String>) =
        sqlx::query_as("SELECT status,attempts,last_error FROM outbox WHERE id=$1")
            .bind(mail_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(row, ("pending".into(), 0, None));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn order_state_machine_payment_and_audit_history() {
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let mut input = app
        .order(&guest, "cash@example.test", &products[0], 1)
        .await;
    input["payment_method"] = json!("cash_on_delivery");
    let mut order = app.post(&guest, "/api/orders", &input).await;
    let path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
    let skip = json!({"status":"delivered","payment_status":"paid","version":order["version"]});
    app.call(
        &admin,
        Method::PATCH,
        &path,
        Some(&skip),
        StatusCode::CONFLICT,
    )
    .await;
    for status in ["confirmed", "preparing", "out_for_delivery", "delivered"] {
        if status == "preparing" {
            let premature =
                json!({"status":status,"payment_status":"paid","version":order["version"]});
            app.call(
                &admin,
                Method::PATCH,
                &path,
                Some(&premature),
                StatusCode::CONFLICT,
            )
            .await;
        }
        let update = json!({"status":status,"payment_status":if status == "delivered" {"paid"} else {"unpaid"},"version":order["version"]});
        order = app
            .call(&admin, Method::PATCH, &path, Some(&update), StatusCode::OK)
            .await;
    }
    let cancel = json!({"status":"cancelled","payment_status":"unpaid","version":order["version"]});
    app.call(
        &admin,
        Method::PATCH,
        &path,
        Some(&cancel),
        StatusCode::CONFLICT,
    )
    .await;
    let detail = app.get(&admin, &path).await;
    assert_eq!(detail["payment_method"], "cash_on_delivery");
    assert_eq!(detail["history"].as_array().unwrap().len(), 5);
    assert_eq!(detail["payment_status"], "paid");
    let filtered = app
        .get(
            &admin,
            "/api/admin/orders?status=delivered&q=cash%40example.test&page=1",
        )
        .await;
    assert_eq!(filtered["total"], 1);
    assert_eq!(
        app.get(&admin, "/api/admin/orders?status=new&page=1").await["total"],
        0
    );
    let input = app
        .order(&guest, "paid@example.test", &products[0], 1)
        .await;
    let mut order = app.post(&guest, "/api/orders", &input).await;
    let path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
    for status in ["confirmed", "preparing", "out_for_delivery"] {
        let update = json!({"status":status,"payment_status":if status == "out_for_delivery" {"paid"} else {"unpaid"},"version":order["version"]});
        order = app
            .call(&admin, Method::PATCH, &path, Some(&update), StatusCode::OK)
            .await;
    }
    let cancel = json!({"status":"cancelled","payment_status":"unpaid","version":order["version"]});
    app.call(
        &admin,
        Method::PATCH,
        &path,
        Some(&cancel),
        StatusCode::CONFLICT,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn checkout_rejects_invalid_input_without_order_or_email_side_effects() {
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let base = app
        .order(&guest, "input@example.test", &products[0], 1)
        .await;
    let cases = [
        ("email", json!("invalid")),
        ("street", json!("  ")),
        ("city", json!("")),
        ("postal_code", json!("abc")),
        ("notes", json!("x".repeat(10001))),
        ("delivery_date", json!("2001-01-01")),
        ("delivery_slot", json!("půlnoc")),
        ("payment_method", json!("online_card")),
        ("items", json!([])),
        (
            "items",
            json!([{"product_id":products[0]["id"],"quantity":0}]),
        ),
        (
            "items",
            json!([{"product_id":products[0]["id"],"quantity":100}]),
        ),
    ];
    for (key, value) in cases {
        let mut input = base.clone();
        input["idempotency_key"] = json!(Uuid::new_v4());
        input[key] = value;
        let response = guest
            .post(format!("{}/api/orders", app.url))
            .json(&input)
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_client_error(),
            "invalid {key} was accepted: {}",
            response.text().await.unwrap()
        );
    }
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("order_items").await, 0);
    assert_eq!(app.count("outbox").await, 0);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn failed_outbox_insert_rolls_back_order_items_stock_and_first_email() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let products = app.get(&admin, "/api/admin/products").await;
    let mut product = products[0].clone();
    product["stock"] = json!(1);
    let path = format!("/api/admin/products/{}", product["id"]);
    let product = app
        .call(
            &admin,
            Method::PUT,
            &path,
            Some(&product_input(&product)),
            StatusCode::OK,
        )
        .await;
    let before_mail = app.count("outbox").await;
    // Fail the second enqueue after the business email and stock update took place.
    sqlx::query("CREATE FUNCTION fail_customer_mail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.to_email='rollback@example.test' THEN RAISE EXCEPTION 'Injected test outbox failure'; END IF; RETURN NEW; END $$").execute(&app.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_customer_mail BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION fail_customer_mail()").execute(&app.pool).await.unwrap();
    let input = app
        .order(&guest, "rollback@example.test", &product, 1)
        .await;
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("order_items").await, 0);
    assert_eq!(app.count("outbox").await, before_mail);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 1, "failed persistence must not consume inventory");
    sqlx::query("DROP TRIGGER fail_customer_mail ON outbox")
        .execute(&app.pool)
        .await
        .unwrap();
    app.post(&guest, "/api/orders", &input).await;
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("outbox").await, before_mail + 2);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn delivery_area_management_permissions_normalization_and_conflicts() {
    let app = TestApp::with_delivery_area(false).await;
    let guest = app.client();
    let customer = app.login("area-customer@example.test").await;
    let admin = app.login("admin@example.test").await;
    let input = json!({"postal_code":"\t1\u{00a0}10\u{2003}0\r\n0\u{0085}\u{feff}","label":" Centrum ","active":true});
    for (client, status) in [
        (&guest, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        app.call(
            client,
            Method::GET,
            "/api/admin/delivery-areas",
            None,
            status,
        )
        .await;
        app.call(
            client,
            Method::POST,
            "/api/admin/delivery-areas",
            Some(&input),
            status,
        )
        .await;
        let mut update = input.clone();
        update["version"] = json!(1);
        app.call(
            client,
            Method::PUT,
            "/api/admin/delivery-areas/1",
            Some(&update),
            status,
        )
        .await;
    }
    let created = app.post(&admin, "/api/admin/delivery-areas", &input).await;
    assert_eq!(created["postal_code"], "11000");
    assert_eq!(created["label"], "Centrum");
    assert_eq!(created["version"], 1);
    assert_eq!(
        app.get(&guest, "/api/settings").await["delivery_postal_codes"],
        json!(["11000"])
    );
    let duplicate = json!({"postal_code":"110 00"});
    app.call(
        &admin,
        Method::POST,
        "/api/admin/delivery-areas",
        Some(&duplicate),
        StatusCode::CONFLICT,
    )
    .await;
    for invalid in [
        "",
        "1100",
        "110000",
        "110-00",
        "11a00",
        "１１０００",
        "110\u{200b}00",
    ] {
        app.call(
            &admin,
            Method::POST,
            "/api/admin/delivery-areas",
            Some(&json!({"postal_code":invalid})),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    app.call(
        &admin,
        Method::POST,
        "/api/admin/delivery-areas",
        Some(&json!({"postal_code":"12000","label":"x".repeat(101)})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let path = format!("/api/admin/delivery-areas/{}", created["id"]);
    let stale = json!({"postal_code":"11000","label":"Původní verze","active":true,"version":1});
    let updated = app.call(&admin, Method::PUT, &path, Some(&json!({"postal_code":" 1\t2\u{202f}000 ","label":"Nová oblast","active":false,"version":1})), StatusCode::OK).await;
    assert_eq!(updated["postal_code"], "12000");
    assert_eq!(updated["version"], 2);
    assert_eq!(
        app.get(&guest, "/api/settings").await["delivery_postal_codes"],
        json!([])
    );
    app.call(
        &admin,
        Method::PUT,
        &path,
        Some(&stale),
        StatusCode::CONFLICT,
    )
    .await;
    let codes = app.get(&admin, "/api/admin/delivery-areas").await;
    assert_eq!(codes.as_array().unwrap().len(), 1);
    assert_eq!(codes[0]["active"], false);
    let mut reactivated = product_input(&updated);
    reactivated["active"] = json!(true);
    app.call(
        &admin,
        Method::PUT,
        &path,
        Some(&reactivated),
        StatusCode::OK,
    )
    .await;

    // Canonical uniqueness holds even when two administrators add different whitespace variants.
    let (first, second) = tokio::join!(
        admin
            .post(format!("{}/api/admin/delivery-areas", app.url))
            .json(&json!({"postal_code":"13000"}))
            .send(),
        admin
            .post(format!("{}/api/admin/delivery-areas", app.url))
            .json(&json!({"postal_code":"13\t0 00"}))
            .send()
    );
    let mut statuses = [
        first.unwrap().status().as_u16(),
        second.unwrap().status().as_u16(),
    ];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(
        app.get(&guest, "/api/settings").await["delivery_postal_codes"],
        json!(["12000", "13000"])
    );
    let duplicate_edit =
        json!({"postal_code":"130 00","label":"Duplicitní","active":true,"version":3});
    app.call(
        &admin,
        Method::PUT,
        &path,
        Some(&duplicate_edit),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(
        app.get(&guest, "/api/settings").await["delivery_postal_codes"],
        json!(["12000", "13000"])
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn delivery_allowlist_blocks_new_orders_preserves_replays_and_has_no_side_effects() {
    let app = TestApp::with_delivery_area(false).await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let products = app.get(&guest, "/api/products").await;
    let product = &products[0];
    sqlx::query("UPDATE products SET stock=5 WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .execute(&app.pool)
        .await
        .unwrap();
    let baseline_mail = app.count("outbox").await;
    let mut input = app.order(&guest, "postal@example.test", product, 1).await;
    input["postal_code"] = json!("\u{feff}\t11\u{00a0}0\r\n0\u{2003}0\u{0085}");
    assert_eq!(
        app.get(&guest, "/api/settings").await["delivery_postal_codes"],
        json!([])
    );
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let area = app
        .post(
            &admin,
            "/api/admin/delivery-areas",
            &json!({"postal_code":"110 00","label":"Test"}),
        )
        .await;
    let mut unsupported = input.clone();
    unsupported["postal_code"] = json!("120 00");
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&unsupported),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("order_items").await, 0);
    assert_eq!(app.count("outbox").await, baseline_mail);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 5);
    let created = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(created["postal_code"], "11000");
    let path = format!("/api/admin/delivery-areas/{}", area["id"]);
    let mut update = product_input(&area);
    update["active"] = json!(false);
    let deactivated = app
        .call(&admin, Method::PUT, &path, Some(&update), StatusCode::OK)
        .await;
    input["postal_code"] = json!("1 1 0\t0 0");
    let replayed = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(
        created["id"], replayed["id"],
        "changing whitespace preserves idempotency even after area deactivation"
    );
    input["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("order_items").await, 1);
    assert_eq!(app.count("outbox").await, baseline_mail + 2);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 4);
    let mut update = product_input(&deactivated);
    update["active"] = json!(true);
    app.call(&admin, Method::PUT, &path, Some(&update), StatusCode::OK)
        .await;
    let second = app.post(&guest, "/api/orders", &input).await;
    assert_ne!(second["id"], created["id"]);
    assert_eq!(app.count("orders").await, 2);
    app.close().await;
}
