//! Real HTTP payment regressions using isolated PostgreSQL schemas and a local provider.
//! Run with TEST_DATABASE_URL=... cargo test --test payments -- --ignored.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use brunch_server::{AppState, Config, app, auth, commerce, migrate, payments};
use rand::{SeedableRng, rngs::StdRng};
use reqwest::{Client, StatusCode};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs1v15::SigningKey,
    pkcs8::{DecodePublicKey, EncodePublicKey, LineEnding},
    signature::{SignatureEncoding, Signer},
};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, LazyLock, Mutex},
};
use tokio::task::JoinHandle;
use uuid::Uuid;

static TEST_KEY: LazyLock<RsaPrivateKey> =
    LazyLock::new(|| RsaPrivateKey::new(&mut StdRng::seed_from_u64(173), 2048).unwrap());

fn public_key() -> String {
    RsaPublicKey::from(&*TEST_KEY)
        .to_public_key_pem(LineEnding::LF)
        .unwrap()
}

fn signature(body: &[u8]) -> String {
    STANDARD.encode(
        SigningKey::<Sha256>::new(TEST_KEY.clone())
            .sign(body)
            .to_bytes(),
    )
}

#[derive(Clone)]
struct FakeTeya {
    url: String,
    store_id: String,
    state: Arc<Mutex<ProviderData>>,
    session_gate: Arc<Mutex<Option<SessionGate>>>,
    token_gate: Arc<Mutex<Option<SessionGate>>>,
}

struct SessionGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[derive(Default)]
struct ProviderData {
    sessions: HashMap<String, Value>,
    requests: Vec<(String, Value)>,
    paid: bool,
    fail_create_response_once: bool,
    response_patch: Option<Value>,
    session_http_status: Option<StatusCode>,
}

impl FakeTeya {
    async fn start() -> (Self, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            url: format!("http://{}", listener.local_addr().unwrap()),
            store_id: Uuid::new_v4().to_string(),
            state: Arc::new(Mutex::new(ProviderData::default())),
            session_gate: Arc::new(Mutex::new(None)),
            token_gate: Arc::new(Mutex::new(None)),
        };
        let router = Router::new()
            .route("/oauth-token", post(fake_token))
            .route("/v2/checkout/sessions", post(fake_create))
            .route("/v2/checkout/sessions/{id}", get(fake_session))
            .with_state(fake.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (fake, task)
    }

    fn config(&self) -> payments::TeyaConfig {
        payments::TeyaConfig {
            environment: "staging".into(),
            store_id: self.store_id.clone(),
            client_id: "test-client".into(),
            client_secret: "test-secret".into(),
            webhook_public_keys: vec![RsaPublicKey::from_public_key_pem(&public_key()).unwrap()],
            api_base: self.url.clone(),
            token_url: format!("{}/oauth-token", self.url),
            checkout_host: "checkout.teya.xyz".into(),
        }
    }

    fn session(&self) -> Value {
        self.state
            .lock()
            .unwrap()
            .sessions
            .values()
            .next()
            .unwrap()
            .clone()
    }

    fn webhook(&self, transaction_id: &str) -> Value {
        let session = self.session();
        json!({
            "event": "payment.succeeded.v1", "timestamp": chrono::Utc::now().to_rfc3339(),
            "data": {
                "transaction_id": transaction_id,
                "payment_method": {"type":"CARD","brand":"VISA","last4":"1111"},
                "amount": session["amount"], "status": "SUCCESS",
                "session_id": session["session_id"], "merchant_reference": session["merchant_reference"],
                "authentication_id": Uuid::new_v4(), "authorisation_code":"123456", "token_response":null
            }
        })
    }
}

async fn fake_token(State(fake): State<FakeTeya>, headers: HeaderMap, body: String) -> Json<Value> {
    assert!(headers.contains_key("user-agent"));
    assert_eq!(headers["content-type"], "application/x-www-form-urlencoded");
    let form: HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(
        form.get("client_id").map(String::as_str),
        Some("test-client")
    );
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some("test-secret")
    );
    let scopes = form["scope"].split_whitespace().collect::<Vec<_>>();
    assert!(scopes.contains(&"checkout/sessions/create"));
    assert!(scopes.contains(&"checkout/sessions/id/get"));
    assert!(!scopes.contains(&"refunds/create"));
    let gate = fake.token_gate.lock().unwrap().take();
    if let Some(gate) = gate {
        gate.entered.send(()).unwrap();
        gate.release.await.unwrap();
    }
    Json(json!({"access_token":"provider-token","token_type":"Bearer","expires_in":3600}))
}

async fn fake_create(
    State(fake): State<FakeTeya>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer provider-token");
    assert!(headers.contains_key("user-agent"));
    let key = headers["idempotency-key"].to_str().unwrap().to_string();
    assert_eq!(body["type"], "SALE");
    assert_eq!(body["amount"]["currency"], "CZK");
    let mut data = fake.state.lock().unwrap();
    data.requests.push((key.clone(), body.clone()));
    let session = data.sessions.entry(key).or_insert_with(|| {
        let id = Uuid::new_v4().to_string();
        json!({
            "session_id": id, "session_url": format!("https://checkout.teya.xyz/pay/{id}"),
            "type":"SALE", "amount": body["amount"], "merchant_reference": body["merchant_reference"],
            "business_data": {"id": fake.store_id}, "transaction_id": format!("tr_{id}"),
            "session_status":"ACTIVE", "payment_status":"NONE"
        })
    }).clone();
    if data.fail_create_response_once {
        data.fail_create_response_once = false;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"code":"TEMPORARY"})),
        )
            .into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({"session_id":session["session_id"],"session_url":session["session_url"]})),
    )
        .into_response()
}

async fn fake_session(
    State(fake): State<FakeTeya>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer provider-token");
    let response = {
        let data = fake.state.lock().unwrap();
        if let Some(status) = data.session_http_status {
            return (status, Json(json!({"code":"SIMULATED_PROVIDER_OUTAGE"}))).into_response();
        }
        let Some(mut session) = data
            .sessions
            .values()
            .find(|session| session["session_id"] == id)
            .cloned()
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if data.paid {
            session["session_status"] = json!("COMPLETED");
            session["payment_status"] = json!("SUCCESS");
        }
        if let Some(patch) = &data.response_patch {
            for (key, value) in patch.as_object().unwrap() {
                if value.is_null() {
                    session.as_object_mut().unwrap().remove(key);
                } else {
                    session[key] = value.clone();
                }
            }
        }
        Json(session).into_response()
    };
    let gate = fake.session_gate.lock().unwrap().take();
    if let Some(gate) = gate {
        gate.entered.send(()).unwrap();
        gate.release.await.unwrap();
    }
    response
}

struct TestApp {
    state: AppState,
    url: String,
    database: PgPool,
    schema: String,
    task: JoinHandle<()>,
}

impl TestApp {
    async fn new(teya: Option<payments::TeyaConfig>) -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("Set TEST_DATABASE_URL to a disposable local PostgreSQL database");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_payments_{}", Uuid::new_v4().simple());
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
        sqlx::query(
            "INSERT INTO delivery_areas(postal_code,label) VALUES ('11000','Testovací oblast')",
        )
        .execute(&pool)
        .await
        .unwrap();
        auth::bootstrap_admin(&pool, "admin@example.test")
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let config = Config {
            app_env: brunch_server::config::AppEnvironment::Development,
            payment_timeout_minutes: 60,
            teya,
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
        let state = AppState {
            pool,
            config: Arc::new(config),
        };
        let router = app(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            state,
            url,
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

    async fn get(&self, client: &Client, path: &str) -> Value {
        client
            .get(format!("{}{path}", self.url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn post(&self, client: &Client, path: &str, body: &Value, expected: StatusCode) -> Value {
        let response = client
            .post(format!("{}{path}", self.url))
            .json(body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, expected, "{path}: {text}");
        serde_json::from_str(&text).unwrap()
    }

    async fn order_input(&self, client: &Client, payment_method: &str) -> Value {
        let products = self.get(client, "/api/products").await;
        let options = self.get(client, "/api/delivery-options").await;
        let settings = self.get(client, "/api/settings").await;
        let subtotal = products[0]["price_cents"].as_i64().unwrap();
        let fee = if settings["free_delivery_from_cents"]
            .as_i64()
            .is_some_and(|limit| subtotal >= limit)
        {
            0
        } else {
            settings["delivery_fee_cents"].as_i64().unwrap()
        };
        json!({
            "idempotency_key": Uuid::new_v4(), "email": "customer@example.test",
            "street": "Testovací 12", "city": "Praha", "postal_code": "11000",
            "delivery_date": options["dates"][0]["value"], "delivery_slot": options["slots"][0],
            "payment_method": payment_method,
            "items": [{"product_id": products[0]["id"], "quantity": 1}],
            "expected_total_cents": subtotal + fee
        })
    }

    async fn login_admin(&self) -> Client {
        self.login("admin@example.test").await
    }

    async fn login(&self, email: &str) -> Client {
        let client = self.client();
        self.post(
            &client,
            "/api/auth/request",
            &json!({"email":email}),
            StatusCode::OK,
        )
        .await;
        let body: String = sqlx::query_scalar("SELECT body FROM outbox WHERE to_email=$1 AND subject='Přihlášení do Brunch Era' ORDER BY created_at DESC LIMIT 1")
            .bind(email).fetch_one(&self.state.pool).await.unwrap();
        let token = body
            .split("login_token=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        self.post(
            &client,
            "/api/auth/verify",
            &json!({"token":token}),
            StatusCode::OK,
        )
        .await;
        client
    }

    async fn restart(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
        self.state.pool.close().await;
        let options = sqlx::postgres::PgConnectOptions::from_str(&self.state.config.database_url)
            .unwrap()
            .options([("search_path", self.schema.as_str())]);
        self.state = AppState {
            pool: PgPoolOptions::new()
                .max_connections(12)
                .connect_with(options)
                .await
                .unwrap(),
            config: Arc::new((*self.state.config).clone()),
        };
        let listener = tokio::net::TcpListener::bind(self.state.config.bind)
            .await
            .unwrap();
        let router = app(self.state.clone());
        self.task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    }

    async fn run_due(&self) {
        sqlx::query("UPDATE payment_attempts SET next_check_at=now()")
            .execute(&self.state.pool)
            .await
            .unwrap();
        payments::process_batch(&self.state).await.unwrap();
    }

    async fn status(&self, client: &Client, order: &Value, input: &Value) -> Value {
        self.post(
            client,
            &format!("/api/orders/{}/payment", order["id"].as_str().unwrap()),
            &json!({"idempotency_key":input["idempotency_key"]}),
            StatusCode::OK,
        )
        .await
    }

    async fn webhook(&self, body: &[u8], signature: Option<&str>) -> StatusCode {
        let mut request = Client::new()
            .post(format!("{}/api/payments/teya/webhook", self.url))
            .header("content-type", "application/json")
            .body(body.to_vec());
        if let Some(signature) = signature {
            request = request.header("x-teya-signature", signature)
        }
        request.send().await.unwrap().status()
    }

    async fn payment_counts(&self) -> (i64, i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM payment_receipts), (SELECT count(*) FROM payment_notifications), (SELECT count(*) FROM order_history), (SELECT count(*) FROM outbox)")
            .fetch_one(&self.state.pool).await.unwrap()
    }

    async fn cancel(&self, admin: &Client, order: &Value, expected: StatusCode) {
        let path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
        let current = self.get(admin, &path).await;
        let response = admin.patch(format!("{}{path}", self.url))
            .json(&json!({"status":"cancelled","payment_status":"unpaid","version":current["version"]}))
            .send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, expected, "cancelling order: {body}");
    }

    async fn stock(&self, input: &Value) -> i32 {
        sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
            .bind(input["items"][0]["product_id"].as_i64().unwrap())
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
        self.state.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.database)
            .await
            .unwrap();
        self.database.close().await;
    }
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn on_delivery_orders_are_accepted_and_unconfigured_online_checkout_is_rejected() {
    let app = TestApp::new(None).await;
    let client = app.client();
    assert_eq!(
        app.get(&client, "/api/settings").await["online_payment_enabled"],
        false
    );
    for method in ["cash_on_delivery", "card_on_delivery"] {
        let input = app.order_input(&client, method).await;
        let order = app
            .post(&client, "/api/orders", &input, StatusCode::OK)
            .await;
        assert_eq!(order["status"], "new");
        assert_eq!(order["payment_status"], "unpaid");
    }
    let card = app.order_input(&client, "card").await;
    app.post(&client, "/api/orders", &card, StatusCode::CONFLICT)
        .await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn restart_recovers_lost_create_response_and_missed_webhook_without_duplicate_payment() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    assert_eq!(order["status"], "awaiting_payment");
    assert_eq!(order["payment_status"], "unpaid");
    assert_eq!(app.payment_counts().await, (0, 0, 1, 1));
    let pending_mail: (String, String, String) =
        sqlx::query_as("SELECT to_email,subject,body FROM outbox")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(pending_mail.0, "customer@example.test");
    assert!(pending_mail.1.contains("čeká na zaplacení"));
    assert!(
        pending_mail
            .2
            .contains("K vyřízení ji přijmeme až po úspěšné úhradě")
    );
    assert!(
        pending_mail
            .2
            .contains(input["delivery_date"].as_str().unwrap())
    );
    assert!(
        pending_mail
            .2
            .contains(input["delivery_slot"].as_str().unwrap())
    );
    provider.state.lock().unwrap().fail_create_response_once = true;
    app.run_due().await;
    let stored_session: Option<String> =
        sqlx::query_scalar("SELECT session_id FROM payment_attempts")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert!(
        stored_session.is_none(),
        "provider accepted the request but the response was lost"
    );
    // Model a process that died while holding work. Only the expired durable lease is reclaimable.
    sqlx::query("UPDATE payment_attempts SET lease_id=$1,lease_until=now()-interval '1 second'")
        .bind(Uuid::new_v4())
        .execute(&app.state.pool)
        .await
        .unwrap();
    app.restart().await;
    app.run_due().await;
    {
        let data = provider.state.lock().unwrap();
        assert_eq!(
            data.sessions.len(),
            1,
            "retry must not create a second payable session"
        );
        assert_eq!(data.requests.len(), 2);
        assert_eq!(
            data.requests[0], data.requests[1],
            "both key and body survive restart unchanged"
        );
    }
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["payment_state"], "ready");
    assert!(
        status["checkout_url"]
            .as_str()
            .unwrap()
            .starts_with("https://checkout.teya.xyz/")
    );
    provider.state.lock().unwrap().paid = true;
    app.restart().await;
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "new");
    assert_eq!(status["payment_status"], "paid");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.payment_counts().await, (1, 0, 2, 3));
    let confirmations: Vec<(String, String)> = sqlx::query_as("SELECT to_email,body FROM outbox WHERE subject LIKE 'Zaplacená objednávka%' OR subject LIKE 'Přijali jsme platbu%' ORDER BY to_email")
        .fetch_all(&app.state.pool).await.unwrap();
    assert_eq!(
        confirmations
            .iter()
            .map(|(email, _)| email.as_str())
            .collect::<Vec<_>>(),
        vec!["customer@example.test", "orders@example.test"]
    );
    for (_, body) in confirmations {
        for expected in [
            format!(
                "Doručení: {} {}",
                input["delivery_date"].as_str().unwrap(),
                input["delivery_slot"].as_str().unwrap()
            ),
            format!("1 × {}", order["items"][0]["name"].as_str().unwrap()),
            "Celkem uhrazeno: 168,00 Kč".into(),
            "Platba: Online kartou".into(),
            "Adresa: Testovací 12, 11000 Praha".into(),
        ] {
            assert!(
                body.contains(&expected),
                "payment confirmation is missing {expected}: {body}"
            );
        }
    }
    let session = provider.session();
    let payload = provider.webhook(session["transaction_id"].as_str().unwrap());
    let body = serde_json::to_vec(&payload).unwrap();
    let signed = signature(&body);
    for _ in 0..2 {
        assert_eq!(
            app.webhook(&body, Some(&signed)).await,
            StatusCode::NO_CONTENT
        )
    }
    app.run_due().await;
    assert_eq!(
        app.payment_counts().await,
        (1, 1, 2, 3),
        "late duplicate delivery must not duplicate receipts or mail"
    );
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn webhook_requires_exact_raw_signature_and_matching_order_then_waits_for_api_confirmation() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    let session = provider.session();
    let payload = provider.webhook(session["transaction_id"].as_str().unwrap());
    let body = serde_json::to_vec(&payload).unwrap();
    let signed = signature(&body);
    assert_eq!(app.webhook(&body, None).await, StatusCode::UNAUTHORIZED);
    let reformatted = serde_json::to_vec_pretty(&payload).unwrap();
    assert_eq!(
        app.webhook(&reformatted, Some(&signed)).await,
        StatusCode::UNAUTHORIZED
    );
    for (field, value, expected) in [
        (
            "amount",
            json!({"value":1,"currency":"CZK"}),
            StatusCode::CONFLICT,
        ),
        (
            "amount",
            json!({"value":input["expected_total_cents"],"currency":"EUR"}),
            StatusCode::CONFLICT,
        ),
        ("session_id", json!(Uuid::new_v4()), StatusCode::CONFLICT),
        (
            "merchant_reference",
            json!(Uuid::new_v4()),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let mut wrong = payload.clone();
        wrong["data"][field] = value;
        let wrong_body = serde_json::to_vec(&wrong).unwrap();
        assert_eq!(
            app.webhook(&wrong_body, Some(&signature(&wrong_body)))
                .await,
            expected,
            "{field}"
        );
    }
    assert_eq!(app.payment_counts().await, (0, 0, 1, 1));
    assert_eq!(
        app.webhook(&body, Some(&signed)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "unpaid",
        "webhook alone cannot mark paid"
    );
    assert_eq!(app.payment_counts().await, (0, 1, 1, 1));
    app.restart().await;
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "unpaid",
        "pending provider status is not successful payment"
    );
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    assert_eq!(app.payment_counts().await, (1, 1, 2, 3));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn authenticated_provider_mismatches_never_accept_order() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    provider.state.lock().unwrap().paid = true;
    for patch in [
        json!({"business_data":{"id":Uuid::new_v4()}}),
        json!({"merchant_reference":Uuid::new_v4()}),
        json!({"session_id":Uuid::new_v4()}),
        json!({"amount":{"value":1,"currency":"CZK"}}),
        json!({"amount":{"value":input["expected_total_cents"],"currency":"EUR"}}),
        json!({"type":"PRE_AUTHORISATION"}),
    ] {
        provider.state.lock().unwrap().response_patch = Some(patch.clone());
        sqlx::query("UPDATE payment_attempts SET state='pending'")
            .execute(&app.state.pool)
            .await
            .unwrap();
        app.run_due().await;
        let status = app.status(&client, &order, &input).await;
        assert_eq!(status["payment_state"], "review", "{patch}");
        assert_eq!(status["status"], "awaiting_payment", "{patch}");
        assert_eq!(status["payment_status"], "unpaid", "{patch}");
        assert!(status["checkout_url"].is_null());
        assert_eq!(app.payment_counts().await, (0, 0, 1, 1));
    }
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn checkout_capability_and_admin_cannot_forge_online_payment_or_release_live_stock() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    let path = format!("/api/orders/{}/payment", order["id"].as_str().unwrap());
    app.post(
        &client,
        &path,
        &json!({"idempotency_key":Uuid::new_v4()}),
        StatusCode::NOT_FOUND,
    )
    .await;
    let admin = app.login_admin().await;
    for (status, payment) in [("new", "paid"), ("new", "unpaid"), ("cancelled", "unpaid")] {
        let response = admin
            .patch(format!(
                "{}/api/admin/orders/{}",
                app.url,
                order["id"].as_str().unwrap()
            ))
            .json(&json!({"status":status,"payment_status":payment,"version":order["version"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "{status} {payment}: {}",
            response.text().await.unwrap()
        );
    }
    assert_eq!(
        app.status(&client, &order, &input).await["status"],
        "awaiting_payment"
    );
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn stale_worker_cannot_record_payment_or_overwrite_reassigned_lease() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    provider.state.lock().unwrap().paid = true;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *provider.session_gate.lock().unwrap() = Some(SessionGate {
        entered: entered_tx,
        release: release_rx,
    });
    sqlx::query("UPDATE payment_attempts SET next_check_at=now()")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let state = app.state.clone();
    let worker = tokio::spawn(async move { payments::process_batch(&state).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .unwrap()
        .unwrap();
    let new_lease = Uuid::new_v4();
    sqlx::query("UPDATE payment_attempts SET lease_id=$1,lease_until=now()+interval '2 minutes',failures=17")
        .bind(new_lease).execute(&app.state.pool).await.unwrap();
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    let (lease, failures): (Option<Uuid>, i32) =
        sqlx::query_as("SELECT lease_id,failures FROM payment_attempts")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(lease, Some(new_lease));
    assert_eq!(failures, 17);
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "unpaid"
    );
    assert_eq!(app.payment_counts().await, (0, 0, 1, 1));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn omitted_status_never_means_paid_and_terminal_failed_session_can_be_cancelled() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    for session_status in ["ACTIVE", "PROCESSING"] {
        provider.state.lock().unwrap().response_patch =
            Some(json!({"session_status":session_status,"payment_status":null}));
        app.run_due().await;
        let status = app.status(&client, &order, &input).await;
        assert_eq!(
            status["payment_state"],
            if session_status == "ACTIVE" {
                "ready"
            } else {
                "pending"
            }
        );
        if session_status == "PROCESSING" {
            assert!(status["checkout_url"].is_null());
        }
        assert_eq!(status["payment_status"], "unpaid");
    }
    provider.state.lock().unwrap().response_patch =
        Some(json!({"session_status":"COMPLETED","payment_status":null}));
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "unpaid"
    );
    assert_eq!(app.payment_counts().await, (0, 0, 1, 1));
    provider.state.lock().unwrap().response_patch =
        Some(json!({"session_status":"COMPLETED","payment_status":"FAILED"}));
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_state"],
        "failed"
    );
    let admin = app.login_admin().await;
    let response = admin
        .patch(format!(
            "{}/api/admin/orders/{}",
            app.url,
            order["id"].as_str().unwrap()
        ))
        .json(&json!({"status":"cancelled","payment_status":"unpaid","version":order["version"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    assert_eq!(
        app.status(&client, &order, &input).await["status"],
        "cancelled"
    );
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_receipts")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(receipts, 0);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn one_provider_transaction_cannot_pay_two_orders() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let first_input = app.order_input(&client, "card").await;
    let first = app
        .post(&client, "/api/orders", &first_input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    assert_eq!(
        app.status(&client, &first, &first_input).await["payment_status"],
        "paid"
    );
    let first_transaction = provider.session()["transaction_id"].clone();
    let second_input = app.order_input(&client, "card").await;
    let second = app
        .post(&client, "/api/orders", &second_input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().response_patch =
        Some(json!({"transaction_id":first_transaction}));
    app.run_due().await;
    let second_status = app.status(&client, &second, &second_input).await;
    assert_eq!(second_status["payment_status"], "unpaid");
    assert_eq!(second_status["payment_state"], "review");
    assert_eq!(app.payment_counts().await, (1, 0, 3, 4));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn shared_signing_key_notification_cannot_poison_a_lost_session_recovery() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().fail_create_response_once = true;
    app.run_due().await;
    let session = provider.session();
    let actual = provider.webhook(session["transaction_id"].as_str().unwrap());
    let mut nonexistent = actual.clone();
    nonexistent["data"]["session_id"] = json!(Uuid::new_v4());
    for (notification, patch) in [
        (nonexistent, None),
        (actual, Some(json!({"business_data":{"id":Uuid::new_v4()}}))),
    ] {
        provider.state.lock().unwrap().response_patch = patch;
        let body = serde_json::to_vec(&notification).unwrap();
        assert_eq!(
            app.webhook(&body, Some(&signature(&body))).await,
            StatusCode::NO_CONTENT
        );
        let canonical: Option<String> =
            sqlx::query_scalar("SELECT session_id FROM payment_attempts")
                .fetch_one(&app.state.pool)
                .await
                .unwrap();
        assert!(
            canonical.is_none(),
            "a signed notification supplies an untrusted candidate only"
        );
        app.run_due().await;
        let (canonical, candidate): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT session_id,candidate_session_id FROM payment_attempts")
                .fetch_one(&app.state.pool)
                .await
                .unwrap();
        assert!(canonical.is_none());
        assert!(
            candidate.is_none(),
            "inaccessible or foreign-store candidates must be discarded"
        );
        assert_eq!(
            app.status(&client, &order, &input).await["payment_status"],
            "unpaid"
        );
    }
    provider.state.lock().unwrap().response_patch = None;
    app.run_due().await;
    {
        let data = provider.state.lock().unwrap();
        assert_eq!(data.sessions.len(), 1);
        assert_eq!(data.requests.len(), 2);
        assert_eq!(data.requests[0], data.requests[1]);
    }
    assert_eq!(
        app.status(&client, &order, &input).await["payment_state"],
        "ready"
    );
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    assert_eq!(app.payment_counts().await, (1, 2, 2, 3));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn old_ambiguous_create_is_not_repeated_and_late_webhook_can_recover_payment() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().fail_create_response_once = true;
    app.run_due().await;
    sqlx::query("UPDATE payment_attempts SET created_at=now()-interval '2 hours'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    app.restart().await;
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_state"],
        "review"
    );
    assert_eq!(
        provider.state.lock().unwrap().requests.len(),
        1,
        "never create again after the bounded ambiguous recovery period"
    );
    provider.state.lock().unwrap().paid = true;
    let session = provider.session();
    let body =
        serde_json::to_vec(&provider.webhook(session["transaction_id"].as_str().unwrap())).unwrap();
    assert_eq!(
        app.webhook(&body, Some(&signature(&body))).await,
        StatusCode::NO_CONTENT
    );
    app.restart().await;
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "paid"
    );
    assert_eq!(provider.state.lock().unwrap().requests.len(), 1);
    assert_eq!(app.payment_counts().await, (1, 1, 2, 3));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn verified_receipt_survives_restart_and_is_only_exposed_in_admin_order_detail() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let customer = app.login("customer@example.test").await;
    let anonymous = app.client();
    let admin = app.login_admin().await;
    let input = app.order_input(&customer, "card").await;
    let order = app
        .post(&customer, "/api/orders", &input, StatusCode::OK)
        .await;
    let path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
    let pending = app.get(&admin, &path).await;
    assert!(pending["online_payment"]["receipt"].is_null());
    app.run_due().await;
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    let session = provider.session();
    let paid = app.get(&admin, &path).await;
    let receipt = &paid["online_payment"]["receipt"];
    assert_eq!(receipt["transaction_id"], session["transaction_id"]);
    assert_eq!(receipt["session_id"], session["session_id"]);
    assert_eq!(receipt["amount_cents"], order["total_cents"]);
    assert_eq!(receipt["currency"], "CZK");
    chrono::DateTime::parse_from_rfc3339(receipt["verified_at"].as_str().unwrap()).unwrap();
    let saved: Value = sqlx::query_scalar(
        "SELECT to_jsonb(r) - 'order_id' FROM payment_receipts r WHERE order_id=$1",
    )
    .bind(Uuid::parse_str(order["id"].as_str().unwrap()).unwrap())
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(saved["transaction_id"], receipt["transaction_id"]);
    assert_eq!(saved["session_id"], receipt["session_id"]);
    assert_eq!(saved["amount_cents"], receipt["amount_cents"]);
    assert_eq!(saved["currency"], receipt["currency"]);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(saved["verified_at"].as_str().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339(receipt["verified_at"].as_str().unwrap()).unwrap()
    );
    app.restart().await;
    assert_eq!(
        app.get(&admin, &path).await["online_payment"]["receipt"],
        *receipt
    );
    for (client, expected) in [
        (&anonymous, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        let response = client
            .get(format!("{}{path}", app.url))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let replay = app
        .post(&customer, "/api/orders", &input, StatusCode::OK)
        .await;
    let mine = app.get(&customer, "/api/orders/mine").await;
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], order["id"]);
    let status = app.status(&anonymous, &order, &input).await;
    for public in [&order, &replay, &mine[0], &status] {
        assert!(public.get("online_payment").is_none());
        let body = public.to_string();
        assert!(!body.contains(session["transaction_id"].as_str().unwrap()));
        assert!(!body.contains(session["session_id"].as_str().unwrap()));
    }
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn payment_confirmation_mail_failure_rolls_back_all_changes_and_retries_once() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    assert!(
        order.get("online_payment").is_none(),
        "public checkout must not expose operator diagnostics"
    );
    app.run_due().await;
    let admin = app.login_admin().await;
    let admin_path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
    let admin_order = app.get(&admin, &admin_path).await;
    assert_eq!(admin_order["online_payment"]["state"], "ready");
    assert_eq!(admin_order["online_payment"]["cancellable"], false);
    assert_eq!(app.payment_counts().await, (0, 0, 1, 2));
    // Fail the second confirmation message after receipt, order, history and first mail writes.
    // The entire payment-acceptance transaction must roll back, including that first mail.
    sqlx::query("CREATE FUNCTION fail_payment_confirmation_mail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.subject LIKE 'Přijali jsme platbu%' THEN RAISE EXCEPTION 'simulated temporary mail queue failure'; END IF; RETURN NEW; END $$")
        .execute(&app.state.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_payment_confirmation_mail BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION fail_payment_confirmation_mail()")
        .execute(&app.state.pool).await.unwrap();
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    assert_eq!(
        app.payment_counts().await,
        (0, 0, 1, 2),
        "receipt, history and both confirmation emails must roll back together"
    );
    let admin_order = app.get(&admin, &admin_path).await;
    assert_eq!(admin_order["status"], "awaiting_payment");
    assert_eq!(admin_order["payment_status"], "unpaid");
    assert_eq!(admin_order["version"], order["version"]);
    assert_eq!(
        admin_order["online_payment"]["last_error"],
        "database_error"
    );
    let (state, failures, lease, retries_later): (String, i32, Option<Uuid>, bool) =
        sqlx::query_as(
            "SELECT state,failures,lease_id,next_check_at > now() FROM payment_attempts",
        )
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert!(
        matches!(state.as_str(), "pending" | "ready"),
        "temporary DB failure must stay retryable, got {state}"
    );
    assert_eq!(failures, 1);
    assert!(lease.is_none());
    assert!(retries_later);
    let replay = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    assert_eq!(replay["id"], order["id"]);
    assert!(
        replay.get("online_payment").is_none(),
        "public replay must not expose the recorded database error"
    );
    sqlx::query("DROP TRIGGER fail_payment_confirmation_mail ON outbox")
        .execute(&app.state.pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION fail_payment_confirmation_mail()")
        .execute(&app.state.pool)
        .await
        .unwrap();
    app.run_due().await;
    app.run_due().await;
    assert_eq!(
        app.payment_counts().await,
        (1, 0, 2, 4),
        "successful retry must accept once and queue exactly two confirmation emails"
    );
    let admin_order = app.get(&admin, &admin_path).await;
    assert_eq!(admin_order["status"], "new");
    assert_eq!(admin_order["payment_status"], "paid");
    assert_eq!(admin_order["online_payment"]["state"], "paid");
    assert!(admin_order["online_payment"]["last_error"].is_null());
    let replay = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    assert_eq!(replay["payment_status"], "paid");
    assert!(replay.get("online_payment").is_none());
    assert_eq!(app.payment_counts().await, (1, 0, 2, 4));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn declined_attempt_in_active_session_can_later_succeed_without_new_payment() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let admin = app.login_admin().await;
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    let before = app.payment_counts().await;
    assert_eq!(app.stock(&input).await, 4);

    // A declined attempt does not necessarily close the customer's checkout session.
    provider.state.lock().unwrap().response_patch =
        Some(json!({"session_status":"ACTIVE","payment_status":"FAILED"}));
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "awaiting_payment");
    assert_eq!(status["payment_status"], "unpaid");
    assert_eq!(status["payment_state"], "ready");
    let original_url = status["checkout_url"].as_str().unwrap().to_owned();
    app.cancel(&admin, &order, StatusCode::CONFLICT).await;
    assert_eq!(app.payment_counts().await, before);
    assert_eq!(app.stock(&input).await, 4);

    provider.state.lock().unwrap().response_patch =
        Some(json!({"session_status":"PROCESSING","payment_status":"NONE"}));
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["payment_state"], "pending");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.payment_counts().await, before);

    {
        let mut data = provider.state.lock().unwrap();
        data.response_patch = None;
        data.paid = true;
        assert_eq!(data.sessions.len(), 1);
        assert_eq!(data.requests.len(), 1);
        assert_eq!(
            data.sessions.values().next().unwrap()["session_url"],
            original_url
        );
    }
    app.run_due().await;
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "new");
    assert_eq!(status["payment_status"], "paid");
    assert!(status["checkout_url"].is_null());
    assert_eq!(
        app.payment_counts().await,
        (1, 0, before.2 + 1, before.3 + 2)
    );
    assert_eq!(app.stock(&input).await, 4);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn expired_sessions_remain_unpaid_and_cancellation_restores_stock_exactly_once() {
    for payment_status in ["NONE", "FAILED"] {
        let (provider, provider_task) = FakeTeya::start().await;
        let mut app = TestApp::new(Some(provider.config())).await;
        let client = app.client();
        let admin = app.login_admin().await;
        sqlx::query("UPDATE products SET stock=5")
            .execute(&app.state.pool)
            .await
            .unwrap();
        let input = app.order_input(&client, "card").await;
        let order = app
            .post(&client, "/api/orders", &input, StatusCode::OK)
            .await;
        let before = app.payment_counts().await;
        provider.state.lock().unwrap().response_patch =
            Some(json!({"session_status":"EXPIRED","payment_status":payment_status}));
        app.run_due().await;
        let status = app.status(&client, &order, &input).await;
        assert_eq!(status["payment_state"], "expired");
        assert_eq!(status["status"], "awaiting_payment");
        assert_eq!(status["payment_status"], "unpaid");
        assert!(status["checkout_url"].is_null());
        assert_eq!(app.payment_counts().await, before);
        assert_eq!(app.stock(&input).await, 4);
        let path = format!("/api/admin/orders/{}", order["id"].as_str().unwrap());
        assert_eq!(
            app.get(&admin, &path).await["online_payment"]["cancellable"],
            true
        );

        app.cancel(&admin, &order, StatusCode::OK).await;
        assert_eq!(app.stock(&input).await, 5);
        assert_eq!(
            app.status(&client, &order, &input).await["status"],
            "cancelled"
        );
        let after_cancel = app.payment_counts().await;
        assert_eq!(after_cancel, (0, 0, before.2 + 1, before.3 + 1));
        app.restart().await;
        app.run_due().await;
        app.cancel(&admin, &order, StatusCode::CONFLICT).await;
        assert_eq!(app.stock(&input).await, 5);
        assert_eq!(app.payment_counts().await, after_cancel);
        app.close().await;
        provider_task.abort();
    }
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn late_success_after_cancellation_records_money_for_review_without_fulfilment() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let admin = app.login_admin().await;
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().response_patch =
        Some(json!({"session_status":"COMPLETED","payment_status":"FAILED"}));
    app.run_due().await;
    app.cancel(&admin, &order, StatusCode::OK).await;
    assert_eq!(app.stock(&input).await, 5);
    let before = app.payment_counts().await;

    // Defensive simulation of contradictory late provider information after a terminal failure.
    {
        let mut data = provider.state.lock().unwrap();
        data.response_patch = None;
        data.paid = true;
    }
    let session = provider.session();
    let body =
        serde_json::to_vec(&provider.webhook(session["transaction_id"].as_str().unwrap())).unwrap();
    assert_eq!(
        app.webhook(&body, Some(&signature(&body))).await,
        StatusCode::NO_CONTENT
    );
    app.restart().await;
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "cancelled");
    assert_eq!(status["payment_state"], "review");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.stock(&input).await, 5);
    assert_eq!(app.payment_counts().await, (1, 1, before.2, before.3 + 2));
    let detail = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(
        detail["online_payment"]["last_error"],
        "paid_cancelled_order"
    );
    assert_eq!(detail["online_payment"]["cancellable"], false);
    let receipt: (i64, String) =
        sqlx::query_as("SELECT amount_cents,currency FROM payment_receipts")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(
        receipt,
        (order["total_cents"].as_i64().unwrap(), "CZK".into())
    );
    assert_eq!(
        app.webhook(&body, Some(&signature(&body))).await,
        StatusCode::NO_CONTENT
    );
    app.run_due().await;
    assert_eq!(app.payment_counts().await, (1, 1, before.2, before.3 + 2));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn provider_outage_is_not_a_decline_and_confirmation_recovers_after_restart() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let admin = app.login_admin().await;
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    let before = app.payment_counts().await;
    {
        let mut data = provider.state.lock().unwrap();
        data.paid = true;
        data.session_http_status = Some(StatusCode::SERVICE_UNAVAILABLE);
    }
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["payment_status"], "unpaid");
    assert_eq!(status["status"], "awaiting_payment");
    assert_ne!(status["payment_state"], "failed");
    assert_ne!(status["payment_state"], "expired");
    app.cancel(&admin, &order, StatusCode::CONFLICT).await;
    assert_eq!(app.payment_counts().await, before);
    let (retry, failures, lease): (bool, i32, Option<Uuid>) =
        sqlx::query_as("SELECT next_check_at>now(),failures,lease_id FROM payment_attempts")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert!(retry);
    assert_eq!(failures, 1);
    assert!(lease.is_none());
    provider.state.lock().unwrap().session_http_status = None;
    app.restart().await;
    app.run_due().await;
    assert_eq!(
        app.status(&client, &order, &input).await["payment_status"],
        "paid"
    );
    assert_eq!(
        app.payment_counts().await,
        (1, 0, before.2 + 1, before.3 + 2)
    );
    assert_eq!(provider.state.lock().unwrap().requests.len(), 1);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn payment_deadline_uses_default_or_custom_duration_and_survives_config_change() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let default_input = app.order_input(&client, "card").await;
    let default_order = app
        .post(&client, "/api/orders", &default_input, StatusCode::OK)
        .await;
    let default_id = Uuid::parse_str(default_order["id"].as_str().unwrap()).unwrap();
    let default_deadline = default_order["payment_expires_at"].clone();
    let default_duration: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM (p.expires_at-o.created_at))::bigint FROM orders o JOIN payment_attempts p ON p.order_id=o.id WHERE o.id=$1")
        .bind(default_id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(default_duration, 60 * 60);

    Arc::make_mut(&mut app.state.config).payment_timeout_minutes = 17;
    app.restart().await;
    let custom_input = app.order_input(&client, "card").await;
    let custom_order = app
        .post(&client, "/api/orders", &custom_input, StatusCode::OK)
        .await;
    let custom_id = Uuid::parse_str(custom_order["id"].as_str().unwrap()).unwrap();
    let custom_deadline = custom_order["payment_expires_at"].clone();
    let custom_duration: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM (p.expires_at-o.created_at))::bigint FROM orders o JOIN payment_attempts p ON p.order_id=o.id WHERE o.id=$1")
        .bind(custom_id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(custom_duration, 17 * 60);
    Arc::make_mut(&mut app.state.config).payment_timeout_minutes = 90;
    app.restart().await;
    payments::expire_orders(&app.state).await.unwrap();
    for (input, order, deadline) in [
        (&default_input, &default_order, default_deadline),
        (&custom_input, &custom_order, custom_deadline),
    ] {
        assert!(deadline.is_string());
        let replay = app
            .post(&client, "/api/orders", input, StatusCode::OK)
            .await;
        assert_eq!(replay["id"], order["id"]);
        assert_eq!(replay["payment_expires_at"], deadline);
        let status = app.status(&client, order, input).await;
        assert_eq!(status["expires_at"], deadline);
        assert_eq!(status["status"], "awaiting_payment");
    }
    assert_eq!(app.payment_counts().await, (0, 0, 2, 2));
    assert!(provider.state.lock().unwrap().requests.is_empty());
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn due_online_orders_expire_once_with_provider_disabled_but_paid_and_delivery_orders_do_not()
{
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=10")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let paid_input = app.order_input(&client, "card").await;
    let paid_order = app
        .post(&client, "/api/orders", &paid_input, StatusCode::OK)
        .await;
    provider.state.lock().unwrap().paid = true;
    app.run_due().await;
    provider.state.lock().unwrap().paid = false;
    let unpaid_input = app.order_input(&client, "card").await;
    let unpaid_order = app
        .post(&client, "/api/orders", &unpaid_input, StatusCode::OK)
        .await;
    let delivery_input = app.order_input(&client, "cash_on_delivery").await;
    let delivery_order = app
        .post(&client, "/api/orders", &delivery_input, StatusCode::OK)
        .await;
    assert_eq!(app.stock(&unpaid_input).await, 7);
    let before = app.payment_counts().await;
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(
        app.payment_counts().await,
        before,
        "a future deadline cannot expire an order"
    );
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    // The maintenance path must run without credentials or access to Teya.
    Arc::make_mut(&mut app.state.config).teya = None;
    app.restart().await;
    let (first, second) = tokio::join!(
        payments::expire_orders(&app.state),
        payments::expire_orders(&app.state),
    );
    first.unwrap();
    second.unwrap();
    payments::expire_orders(&app.state).await.unwrap();
    let status = app.status(&client, &unpaid_order, &unpaid_input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_status"], "unpaid");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.stock(&unpaid_input).await, 8);
    assert_eq!(
        app.payment_counts().await,
        (before.0, before.1, before.2 + 1, before.3 + 1)
    );
    let expired_id = Uuid::parse_str(unpaid_order["id"].as_str().unwrap()).unwrap();
    let (restored, version): (bool, i64) =
        sqlx::query_as("SELECT stock_restored,version FROM orders WHERE id=$1")
            .bind(expired_id)
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert!(restored);
    assert_eq!(version, unpaid_order["version"].as_i64().unwrap() + 1);
    let paid = app.status(&client, &paid_order, &paid_input).await;
    assert_eq!(paid["status"], "new");
    assert_eq!(paid["payment_status"], "paid");
    let delivery = app
        .post(&client, "/api/orders", &delivery_input, StatusCode::OK)
        .await;
    assert_eq!(delivery["id"], delivery_order["id"]);
    assert_eq!(delivery["status"], "new");
    assert!(delivery.get("payment_expires_at").is_none());
    assert_eq!(provider.state.lock().unwrap().requests.len(), 1);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn deadline_hides_existing_checkout_and_prevents_new_session_before_expiration_sweep() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let ready_input = app.order_input(&client, "card").await;
    let ready_order = app
        .post(&client, "/api/orders", &ready_input, StatusCode::OK)
        .await;
    app.run_due().await;
    assert!(app.status(&client, &ready_order, &ready_input).await["checkout_url"].is_string());
    let pending_input = app.order_input(&client, "card").await;
    let pending_order = app
        .post(&client, "/api/orders", &pending_input, StatusCode::OK)
        .await;
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    for (order, input) in [
        (&ready_order, &ready_input),
        (&pending_order, &pending_input),
    ] {
        let status = app.status(&client, order, input).await;
        assert_eq!(status["status"], "awaiting_payment");
        assert!(status["checkout_url"].is_null());
    }
    app.run_due().await;
    let pending = app.status(&client, &pending_order, &pending_input).await;
    assert_eq!(pending["payment_state"], "review");
    let error: String =
        sqlx::query_scalar("SELECT last_error FROM payment_attempts WHERE order_id=$1")
            .bind(Uuid::parse_str(pending_order["id"].as_str().unwrap()).unwrap())
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(error, "order_deadline_passed");
    assert_eq!(
        provider.state.lock().unwrap().requests.len(),
        1,
        "no new payable session after the deadline"
    );
    payments::expire_orders(&app.state).await.unwrap();
    for (order, input) in [
        (&ready_order, &ready_input),
        (&pending_order, &pending_input),
    ] {
        assert_eq!(app.status(&client, order, input).await["status"], "expired");
    }
    assert_eq!(app.payment_counts().await, (0, 0, 4, 4));
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn late_success_after_expiration_survives_restart_and_alerts_once_without_reopening() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(app.stock(&input).await, 5);
    let before = app.payment_counts().await;
    provider.state.lock().unwrap().paid = true;
    let session = provider.session();
    let payload = provider.webhook(session["transaction_id"].as_str().unwrap());
    let body = serde_json::to_vec(&payload).unwrap();
    app.restart().await;
    assert_eq!(
        app.webhook(&body, Some(&signature(&body))).await,
        StatusCode::NO_CONTENT
    );
    app.run_due().await;
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_status"], "unpaid");
    assert_eq!(status["payment_state"], "review");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.stock(&input).await, 5);
    let error: String = sqlx::query_scalar("SELECT last_error FROM payment_attempts")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(error, "paid_expired_order");
    assert_eq!(app.payment_counts().await, (1, 1, before.2, before.3 + 2));
    let alerts: Vec<String> = sqlx::query_scalar("SELECT to_email FROM outbox WHERE subject LIKE 'Pozdní platba%' OR subject LIKE 'Přijatá platba k uzavřené%' ORDER BY to_email")
        .fetch_all(&app.state.pool).await.unwrap();
    assert_eq!(alerts, vec!["customer@example.test", "orders@example.test"]);
    let acceptance: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE subject LIKE 'Přijali jsme platbu%' OR subject LIKE 'Zaplacená objednávka%'")
        .fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(acceptance, 0);
    // A distinct signed delivery must recheck the receipt without sending the alerts again.
    let mut redelivery = payload;
    redelivery["timestamp"] = json!("later-delivery");
    let redelivery = serde_json::to_vec(&redelivery).unwrap();
    assert_eq!(
        app.webhook(&redelivery, Some(&signature(&redelivery)))
            .await,
        StatusCode::NO_CONTENT
    );
    app.run_due().await;
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(app.payment_counts().await, (1, 2, before.2, before.3 + 2));
    assert_eq!(app.stock(&input).await, 5);
    let admin = app.login_admin().await;
    let detail = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(detail["status"], "expired");
    assert_eq!(detail["online_payment"]["state"], "review");
    let receipt = &detail["online_payment"]["receipt"];
    assert_eq!(receipt["transaction_id"], session["transaction_id"]);
    assert_eq!(receipt["session_id"], session["session_id"]);
    assert_eq!(receipt["amount_cents"], order["total_cents"]);
    assert_eq!(receipt["currency"], "CZK");
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn expiration_wins_inflight_success_race_without_reusing_reallocated_stock() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=1")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    assert_eq!(app.stock(&input).await, 0);
    provider.state.lock().unwrap().paid = true;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *provider.session_gate.lock().unwrap() = Some(SessionGate {
        entered: entered_tx,
        release: release_rx,
    });
    sqlx::query("UPDATE payment_attempts SET next_check_at=now()")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let state = app.state.clone();
    let worker = tokio::spawn(async move { payments::process_batch(&state).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .unwrap()
        .unwrap();
    // The successful API response is captured, but has not reached the application yet.
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(app.stock(&input).await, 1);
    let replacement_input = app.order_input(&client, "cash_on_delivery").await;
    let replacement = app
        .post(&client, "/api/orders", &replacement_input, StatusCode::OK)
        .await;
    assert_eq!(replacement["status"], "new");
    assert_eq!(app.stock(&input).await, 0);
    let before = app.payment_counts().await;
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_state"], "review");
    assert_eq!(
        app.stock(&input).await,
        0,
        "late success cannot reserve the replacement order's stock"
    );
    assert_eq!(app.payment_counts().await, (1, 0, before.2, before.3 + 2));
    let retry = app.order_input(&client, "cash_on_delivery").await;
    app.post(&client, "/api/orders", &retry, StatusCode::CONFLICT)
        .await;
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(app.stock(&input).await, 0);
    let accepted: i64 = sqlx::query_scalar("SELECT count(*) FROM orders WHERE status='new'")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(accepted, 1);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn expiration_mail_failure_rolls_back_stock_history_and_order_then_retries_once() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let before = app.payment_counts().await;
    sqlx::query("CREATE FUNCTION fail_expiration_mail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.subject LIKE '%: Vypršela' THEN RAISE EXCEPTION 'simulated expiration queue failure'; END IF; RETURN NEW; END $$")
        .execute(&app.state.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_expiration_mail BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION fail_expiration_mail()")
        .execute(&app.state.pool).await.unwrap();
    assert!(payments::expire_orders(&app.state).await.is_err());
    assert_eq!(app.payment_counts().await, before);
    assert_eq!(app.stock(&input).await, 4);
    let (status, restored, version): (String, bool, i64) =
        sqlx::query_as("SELECT status,stock_restored,version FROM orders")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(status, "awaiting_payment");
    assert!(!restored);
    assert_eq!(version, order["version"].as_i64().unwrap());
    sqlx::query("DROP TRIGGER fail_expiration_mail ON outbox")
        .execute(&app.state.pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION fail_expiration_mail()")
        .execute(&app.state.pool)
        .await
        .unwrap();
    payments::expire_orders(&app.state).await.unwrap();
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(
        app.status(&client, &order, &input).await["status"],
        "expired"
    );
    assert_eq!(app.stock(&input).await, 5);
    assert_eq!(
        app.payment_counts().await,
        (0, 0, before.2 + 1, before.3 + 1)
    );
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn success_response_after_deadline_expires_order_even_before_maintenance_runs() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    app.run_due().await;
    let before = app.payment_counts().await;
    provider.state.lock().unwrap().paid = true;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *provider.session_gate.lock().unwrap() = Some(SessionGate {
        entered: entered_tx,
        release: release_rx,
    });
    sqlx::query("UPDATE payment_attempts SET next_check_at=now()")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let state = app.state.clone();
    let worker = tokio::spawn(async move { payments::process_batch(&state).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE payment_attempts SET expires_at=now()-interval '1 second'")
        .execute(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(
        app.status(&client, &order, &input).await["status"],
        "awaiting_payment"
    );
    // Release success without running expiration maintenance first.
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_status"], "unpaid");
    assert_eq!(status["payment_state"], "review");
    assert_eq!(app.stock(&input).await, 5);
    assert_eq!(
        app.payment_counts().await,
        (1, 0, before.2 + 1, before.3 + 3)
    );
    payments::expire_orders(&app.state).await.unwrap();
    assert_eq!(
        app.payment_counts().await,
        (1, 0, before.2 + 1, before.3 + 3)
    );
    let acceptance: i64 =
        sqlx::query_scalar("SELECT count(*) FROM order_history WHERE status='new'")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(acceptance, 0);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn payment_waiting_for_order_lock_rechecks_deadline_after_the_lock_is_acquired() {
    let (provider, provider_task) = FakeTeya::start().await;
    let app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.state.pool)
        .await
        .unwrap();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    let id = Uuid::parse_str(order["id"].as_str().unwrap()).unwrap();
    app.run_due().await;
    provider.state.lock().unwrap().paid = true;
    sqlx::query("UPDATE payment_attempts SET next_check_at=now()")
        .execute(&app.state.pool)
        .await
        .unwrap();

    let mut blocker = app.state.pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM orders WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let state = app.state.clone();
    let worker = tokio::spawn(async move { payments::process_batch(&state).await.unwrap() });
    let worker_started = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let started: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
                "SELECT xact_start FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)) AND query LIKE 'SELECT status FROM orders%' LIMIT 1",
            )
            .bind(blocker_pid)
            .fetch_optional(&app.state.pool)
            .await
            .unwrap();
            if let Some(started) = started {
                break started;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the payment worker must be waiting for this order's row lock");

    // Set the deadline after the worker began its transaction while it is still blocked.
    // PostgreSQL now() would retain that earlier transaction time after the lock is released.
    let deadline: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE payment_attempts SET expires_at=clock_timestamp() WHERE order_id=$1 RETURNING expires_at",
    )
    .bind(id)
    .fetch_one(&mut *blocker)
    .await
    .unwrap();
    assert!(
        deadline > worker_started,
        "this regression must cross the worker's transaction start"
    );
    blocker.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();

    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_status"], "unpaid");
    assert_eq!(status["payment_state"], "review");
    assert!(status["checkout_url"].is_null());
    assert_eq!(app.stock(&input).await, 5);
    assert_eq!(app.payment_counts().await, (1, 0, 2, 4));
    let error: String = sqlx::query_scalar("SELECT last_error FROM payment_attempts")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(error, "paid_expired_order");
    let accepted: i64 = sqlx::query_scalar("SELECT count(*) FROM order_history WHERE status='new'")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(accepted, 0);
    app.close().await;
    provider_task.abort();
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn acknowledged_webhook_survives_a_stale_worker_entering_deadline_review() {
    let (provider, provider_task) = FakeTeya::start().await;
    let mut app = TestApp::new(Some(provider.config())).await;
    let client = app.client();
    let input = app.order_input(&client, "card").await;
    let order = app
        .post(&client, "/api/orders", &input, StatusCode::OK)
        .await;
    // Teya created a session but our application did not receive its identifier.
    provider.state.lock().unwrap().fail_create_response_once = true;
    app.run_due().await;
    let session = provider.session();
    let stored: Option<String> = sqlx::query_scalar("SELECT session_id FROM payment_attempts")
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert!(stored.is_none());
    let deadline: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE payment_attempts SET expires_at=clock_timestamp()+interval '2 seconds',next_check_at=now() RETURNING expires_at",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    provider.state.lock().unwrap().paid = true;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *provider.token_gate.lock().unwrap() = Some(SessionGate {
        entered: entered_tx,
        release: release_rx,
    });
    let state = app.state.clone();
    let worker = tokio::spawn(async move { payments::process_batch(&state).await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered_rx)
        .await
        .unwrap()
        .unwrap();
    // The worker already read an attempt without a session and is waiting for OAuth.
    // A successful callback now supplies durable recovery work before the deadline check.
    let body =
        serde_json::to_vec(&provider.webhook(session["transaction_id"].as_str().unwrap())).unwrap();
    assert_eq!(
        app.webhook(&body, Some(&signature(&body))).await,
        StatusCode::NO_CONTENT
    );
    let (canonical, candidate): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT session_id,candidate_session_id FROM payment_attempts")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert!(canonical.is_none());
    assert_eq!(candidate.as_deref(), session["session_id"].as_str());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while chrono::Utc::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap();
    // The stale worker must preserve the callback's pending work, release its lease,
    // then verify that original session without creating a second payable session.
    let status = app.status(&client, &order, &input).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(status["payment_state"], "review");
    let (error, canonical, candidate, lease): (
        String,
        Option<String>,
        Option<String>,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT last_error,session_id,candidate_session_id,lease_id FROM payment_attempts",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(error, "paid_expired_order");
    assert_eq!(canonical.as_deref(), session["session_id"].as_str());
    assert!(candidate.is_none());
    assert!(lease.is_none());
    assert_eq!(provider.state.lock().unwrap().requests.len(), 1);
    assert_eq!(app.payment_counts().await, (1, 1, 2, 4));
    app.restart().await;
    app.run_due().await;
    assert_eq!(app.payment_counts().await, (1, 1, 2, 4));
    app.close().await;
    provider_task.abort();
}
