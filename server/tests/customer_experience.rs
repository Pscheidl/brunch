//! Real PostgreSQL/HTTP verification of customer ownership and transactional requests.
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
        let schema = format!("brunch_customer_test_{}", Uuid::new_v4().simple());
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
    async fn create_order(&self, client: &Client, email: &str, billing: Option<Value>) -> Value {
        let body = self.order_input(client, email, billing).await;
        self.post(client, "/api/orders", &body).await
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
fn address() -> Value {
    json!({"label":" Kancelář ","name":"Test Customer","phone":"+420777123456","organization":"Firma","street":"Testovací 12","city":"Praha","postal_code":"110 00","notes":"Recepce"})
}
fn billing() -> Value {
    json!({"name":"Test firma","address":"Testovací 12, Praha","company_id":"12345678","tax_id":"CZ12345678"})
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn checkout_and_customer_requests_do_not_deadlock_with_admin_audit() {
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let admin_id: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email='admin@example.test'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let input = app.order_input(&admin, "admin@example.test", None).await;
    let product = input["items"][0]["product_id"].as_i64().unwrap();

    // Hold the same product lock as an administrator changing its price.
    // Checkout reaches that lock after resolving the customer account.
    let mut edit = app.pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *edit)
        .await
        .unwrap();
    let price: i64 = sqlx::query_scalar("SELECT price_cents FROM products WHERE id=$1 FOR UPDATE")
        .bind(product)
        .fetch_one(&mut *edit)
        .await
        .unwrap();
    let request = admin.post(format!("{}/api/orders", app.url)).json(&input);
    let checkout = tokio::spawn(async move { request.send().await.unwrap() });
    app.wait_for_blocked_request(blocker).await;
    // The audit FK takes KEY SHARE on users(admin). It must remain compatible
    // with checkout, otherwise product -> user and user -> product deadlock.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::query(
            "INSERT INTO product_prices(product_id,price_cents,changed_by) VALUES($1,$2,$3)",
        )
        .bind(product)
        .bind(price)
        .bind(admin_id)
        .execute(&mut *edit),
    )
    .await
    .expect("price audit must not wait for the checkout user lock")
    .expect("price audit must not deadlock");
    edit.commit().await.unwrap();
    let response = checkout.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let order: Value = response.json().await.unwrap();
    let replay = app.post(&admin, "/api/orders", &input).await;
    assert_eq!(replay["id"], order["id"]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // A customer who is also an administrator can request a change while
    // another tab updates that order and records the administrator's audit.
    let order_id = Uuid::parse_str(order["id"].as_str().unwrap()).unwrap();
    let mut edit = app.pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *edit)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM orders WHERE id=$1 FOR UPDATE")
        .bind(order_id)
        .execute(&mut *edit)
        .await
        .unwrap();
    let request = admin
        .post(format!("{}/api/customer/orders/{order_id}/requests", app.url))
        .json(&json!({"idempotency_key":Uuid::new_v4(),"kind":"change","message":"Předat na recepci."}));
    let change = tokio::spawn(async move { request.send().await.unwrap() });
    app.wait_for_blocked_request(blocker).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sqlx::query("INSERT INTO order_history(order_id,status,payment_status,changed_by) VALUES($1,'new','unpaid',$2)")
            .bind(order_id)
            .bind(admin_id)
            .execute(&mut *edit),
    )
    .await
    .expect("order audit must not wait for the customer request user lock")
    .expect("order audit must not deadlock");
    edit.commit().await.unwrap();
    assert_eq!(change.await.unwrap().status(), StatusCode::OK);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn addresses_and_billing_are_private_versioned_and_validated() {
    let app = TestApp::new().await;
    let guest = app.client();
    let alice = app.login("alice@example.test").await;
    let bob = app.login("bob@example.test").await;
    for endpoint in [
        "/api/customer/addresses",
        "/api/customer/billing",
        "/api/customer/orders",
    ] {
        app.call(
            &guest,
            Method::GET,
            endpoint,
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    }
    let saved = app
        .post(&alice, "/api/customer/addresses", &address())
        .await;
    assert_eq!(saved["postal_code"], "11000");
    assert_eq!(saved["label"], "Kancelář");
    assert_eq!(app.get(&bob, "/api/customer/addresses").await, json!([]));
    let path = format!("/api/customer/addresses/{}", saved["id"].as_str().unwrap());
    let mut update = address();
    update["version"] = json!(1);
    update["label"] = json!("Domů");
    app.call(
        &bob,
        Method::PUT,
        &path,
        Some(&update),
        StatusCode::CONFLICT,
    )
    .await;
    app.call(
        &bob,
        Method::DELETE,
        &path,
        Some(&json!({"version":1})),
        StatusCode::CONFLICT,
    )
    .await;
    let updated = app
        .call(&alice, Method::PUT, &path, Some(&update), StatusCode::OK)
        .await;
    assert_eq!(updated["version"], 2);
    app.call(
        &alice,
        Method::PUT,
        &path,
        Some(&update),
        StatusCode::CONFLICT,
    )
    .await;
    let mut invalid = address();
    invalid["postal_code"] = json!("abc");
    app.call(
        &alice,
        Method::POST,
        "/api/customer/addresses",
        Some(&invalid),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let mut duplicate = address();
    duplicate["label"] = json!("DOMŮ");
    app.call(
        &alice,
        Method::POST,
        "/api/customer/addresses",
        Some(&duplicate),
        StatusCode::CONFLICT,
    )
    .await;
    let forbidden = alice
        .post(format!("{}/api/customer/addresses", app.url))
        .header("origin", "https://evil.example.test")
        .json(&address())
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    app.call(
        &alice,
        Method::DELETE,
        &path,
        Some(&json!({"version":2})),
        StatusCode::OK,
    )
    .await;
    assert_eq!(app.get(&alice, "/api/customer/addresses").await, json!([]));
    let mut data = billing();
    data["version"] = json!(0);
    let saved = app
        .call(
            &alice,
            Method::PUT,
            "/api/customer/billing",
            Some(&data),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["version"], 1);
    assert_eq!(app.get(&bob, "/api/customer/billing").await, Value::Null);
    app.call(
        &alice,
        Method::PUT,
        "/api/customer/billing",
        Some(&data),
        StatusCode::CONFLICT,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn checkout_billing_and_issued_invoice_follow_order_owner_and_remain_immutable() {
    let app = TestApp::new().await;
    let guest = app.client();
    let order = app
        .create_order(&guest, "buyer@example.test", Some(billing()))
        .await;
    let buyer = app.login("buyer@example.test").await;
    let stranger = app.login("stranger@example.test").await;
    let admin = app.login("admin@example.test").await;
    let id = order["id"].as_str().unwrap();
    let base = format!("/api/customer/orders/{id}");
    for suffix in ["billing", "invoice", "requests"] {
        app.call(
            &guest,
            Method::GET,
            &format!("{base}/{suffix}"),
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
        app.call(
            &stranger,
            Method::GET,
            &format!("{base}/{suffix}"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    let customer_billing = app.get(&buyer, &format!("{base}/billing")).await;
    assert_eq!(customer_billing["name"], "Test firma");
    assert_eq!(customer_billing["version"], 1);
    let own_orders = app.get(&buyer, "/api/customer/orders").await;
    assert_eq!(own_orders["total"], 1);
    assert_eq!(
        own_orders["active"][0]["items"].as_array().unwrap().len(),
        1
    );
    assert_eq!(app.get(&stranger, "/api/customer/orders").await["total"], 0);
    app.call(
        &buyer,
        Method::GET,
        "/api/customer/orders?page=0",
        None,
        StatusCode::BAD_REQUEST,
    )
    .await;
    app.call(
        &stranger,
        Method::PUT,
        &format!("{base}/billing"),
        Some(&customer_billing),
        StatusCode::NOT_FOUND,
    )
    .await;
    let supplier = json!({"name":"Pekárna Test","address":"Ulice 1","company_id":"87654321","tax_id":"","registration":"","vat_payer":false,"version":1});
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/invoice-supplier",
        Some(&supplier),
        StatusCode::OK,
    )
    .await;
    let invoice=app.post(&admin,&format!("/api/admin/orders/{id}/invoice"),&json!({"supplier_version":2,"customer_name":"Test firma","customer_address":"Testovací 12, Praha","customer_company_id":"12345678","customer_tax_id":"CZ12345678","supply_date":order["delivery_date"],"due_date":order["delivery_date"],"vat_rates":{}})).await;
    assert_eq!(app.get(&buyer, &format!("{base}/invoice")).await, invoice);
    app.call(
        &buyer,
        Method::PUT,
        &format!("{base}/billing"),
        Some(&customer_billing),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.get(&buyer, &format!("{base}/invoice")).await, invoice);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn pickup_invoice_preserves_location_snapshot_and_separate_customer_address() {
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let buyer = app.login("pickup-buyer@example.test").await;
    let pickup = json!({"enabled":true,"name":"Pekárna při objednání","street":"Odběrná 88","city":"Liberec","postal_code":"46001","instructions":"Vchod ze dvora","parking_free":true,"slots":["10:00–12:00"],"version":1});
    let pickup = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&pickup),
            StatusCode::OK,
        )
        .await;
    let mut input = app
        .order_input(&buyer, "pickup-buyer@example.test", None)
        .await;
    input["fulfillment_method"] = json!("pickup");
    input["pickup_version"] = pickup["version"].clone();
    input["delivery_slot"] = pickup["slots"][0].clone();
    input["expected_total_cents"] =
        app.get(&buyer, "/api/products").await[0]["price_cents"].clone();
    for key in ["street", "city", "postal_code"] {
        input.as_object_mut().unwrap().remove(key);
    }
    let order = app.post(&buyer, "/api/orders", &input).await;
    assert_eq!(order["delivery_fee_cents"], 0);
    assert_eq!(order["street"], "Odběrná 88");
    let id = order["id"].as_str().unwrap();
    let customer_base = format!("/api/customer/orders/{id}");
    assert_eq!(
        app.get(&buyer, &format!("{customer_base}/billing")).await,
        Value::Null
    );
    let listed = app.get(&buyer, "/api/customer/orders").await;
    assert_eq!(
        listed["active"][0]["pickup_location"],
        order["pickup_location"]
    );
    assert_eq!(listed["items"][0]["fulfillment_method"], "pickup");
    let supplier = json!({"name":"Pekárna Test","address":"Sídlo 1","company_id":"87654321","tax_id":"","registration":"","vat_payer":false,"version":1});
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/invoice-supplier",
        Some(&supplier),
        StatusCode::OK,
    )
    .await;
    let invoice_path = format!("/api/admin/orders/{id}/invoice");
    let mut issue = json!({"supplier_version":2,"customer_name":"Kupující","customer_address":"","customer_company_id":"","customer_tax_id":"","supply_date":order["delivery_date"],"due_date":order["delivery_date"],"vat_rates":{}});
    app.call(
        &admin,
        Method::POST,
        &invoice_path,
        Some(&issue),
        StatusCode::BAD_REQUEST,
    )
    .await;
    issue["customer_address"] = json!("Vlastní adresa zákazníka 15, Praha");
    let mut moved = pickup.clone();
    moved["name"] = json!("Nové odběrné místo");
    moved["street"] = json!("Nová výdejna 200");
    moved["parking_free"] = json!(false);
    let moved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/pickup-settings",
            Some(&moved),
            StatusCode::OK,
        )
        .await;
    let invoice = app.post(&admin, &invoice_path, &issue).await;
    assert_eq!(invoice["fulfillment_method"], "pickup");
    assert_eq!(invoice["pickup_location"], order["pickup_location"]);
    assert_eq!(invoice["pickup_location"]["parking_free"], true);
    assert_eq!(invoice["customer"]["address"], issue["customer_address"]);
    assert!(
        invoice["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|line| line["name"] != "Doprava")
    );
    assert_eq!(invoice["total_cents"], order["subtotal_cents"]);
    assert_eq!(
        app.get(&buyer, &format!("{customer_base}/invoice")).await,
        invoice
    );
    let mut disabled = moved;
    disabled["enabled"] = json!(false);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/pickup-settings",
        Some(&disabled),
        StatusCode::OK,
    )
    .await;
    issue["customer_address"] = json!("Jiná adresa nesmí přepsat vystavený doklad");
    assert_eq!(app.post(&admin, &invoice_path, &issue).await, invoice);
    assert_eq!(
        app.get(&buyer, &format!("{customer_base}/invoice")).await,
        invoice
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL via TEST_DATABASE_URL"]
async fn customer_requests_are_atomic_idempotent_private_and_require_admin_resolution() {
    let app = TestApp::new().await;
    let buyer = app.login("buyer@example.test").await;
    let stranger = app.login("stranger@example.test").await;
    let admin = app.login("admin@example.test").await;
    let order = app.create_order(&buyer, "buyer@example.test", None).await;
    let id = order["id"].as_str().unwrap();
    let endpoint = format!("/api/customer/orders/{id}/requests");
    let request = json!({"idempotency_key":Uuid::new_v4(),"kind":"change","message":"Prosím předat na druhé recepci."});
    app.call(
        &stranger,
        Method::POST,
        &endpoint,
        Some(&request),
        StatusCode::NOT_FOUND,
    )
    .await;
    // Queue failure must not leave an invisible saved request.
    sqlx::raw_sql("CREATE FUNCTION fail_request_mail() RETURNS trigger AS $$ BEGIN IF NEW.subject LIKE 'Žádost o %' THEN RAISE EXCEPTION 'test queue failure'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; CREATE TRIGGER fail_request BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION fail_request_mail();").execute(&app.pool).await.unwrap();
    app.call(
        &buyer,
        Method::POST,
        &endpoint,
        Some(&request),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(app.get(&buyer, &endpoint).await, json!([]));
    sqlx::query("DROP TRIGGER fail_request ON outbox")
        .execute(&app.pool)
        .await
        .unwrap();
    let (one, two) = tokio::join!(
        app.post(&buyer, &endpoint, &request),
        app.post(&buyer, &endpoint, &request)
    );
    assert_eq!(one, two);
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE subject LIKE 'Žádost o %'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(queued, 1);
    let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "new");
    let mut changed = request.clone();
    changed["message"] = json!("Jiná změna");
    app.call(
        &buyer,
        Method::POST,
        &endpoint,
        Some(&changed),
        StatusCode::CONFLICT,
    )
    .await;
    changed["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &buyer,
        Method::POST,
        &endpoint,
        Some(&changed),
        StatusCode::CONFLICT,
    )
    .await;
    app.call(
        &buyer,
        Method::GET,
        "/api/admin/customer-requests",
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
    let admin_list = app.get(&admin, "/api/admin/customer-requests").await;
    assert_eq!(admin_list[0]["id"], one["id"]);
    let path = format!(
        "/api/admin/customer-requests/{}",
        one["id"].as_str().unwrap()
    );
    let response =
        json!({"status":"resolved","response":"Pokyny pro kurýra jsme upravili.","version":1});
    app.call(
        &buyer,
        Method::PATCH,
        &path,
        Some(&response),
        StatusCode::FORBIDDEN,
    )
    .await;
    let resolved = app
        .call(
            &admin,
            Method::PATCH,
            &path,
            Some(&response),
            StatusCode::OK,
        )
        .await;
    assert_eq!(resolved["version"], 2);
    app.call(
        &admin,
        Method::PATCH,
        &path,
        Some(&response),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.get(&buyer, &endpoint).await[0]["status"], "resolved");
    let replies:i64=sqlx::query_scalar("SELECT count(*) FROM outbox WHERE subject LIKE 'Odpověď k objednávce %' AND to_email='buyer@example.test'").fetch_one(&app.pool).await.unwrap();
    assert_eq!(replies, 1);
    // Retrying the original accepted request is safe even after resolution.
    assert_eq!(app.post(&buyer, &endpoint, &request).await["id"], one["id"]);
    sqlx::query("UPDATE orders SET status='cancelled' WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap())
        .execute(&app.pool)
        .await
        .unwrap();
    app.call(
        &buyer,
        Method::POST,
        &endpoint,
        Some(&changed),
        StatusCode::CONFLICT,
    )
    .await;
    app.close().await;
}
