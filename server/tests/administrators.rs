//! Administrator membership and restart safety through real HTTP and PostgreSQL.
//! TEST_DATABASE_URL must point to a disposable database. Every test isolates its schema.
use brunch_server::{AppState, Config, app, auth, migrate};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use uuid::Uuid;

const ENDPOINT: &str = "/api/admin/administrators";
const INITIAL_ADMIN: &str = "admin@example.test";
const PASSWORD: &str = "Test-only-admin-password-2026";

struct Fixture {
    url: String,
    pool: PgPool,
    database: PgPool,
    schema: String,
    task: JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        Self::with_secure_cookies(false).await
    }
    async fn with_secure_cookies(secure_cookies: bool) -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("Set TEST_DATABASE_URL to an isolated local PostgreSQL database");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_administrators_{}", Uuid::new_v4().simple());
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
        auth::initialize_admin(&pool, INITIAL_ADMIN, PASSWORD)
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind = listener.local_addr().unwrap();
        let url = format!("http://{bind}");
        let config = Config {
            app_env: brunch_server::config::AppEnvironment::Development,
            payment_timeout_minutes: 60,
            teya: None,
            database_url,
            bind,
            public_url: url.clone(),
            order_email: "orders@example.test".into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port: 9,
            smtp_username: None,
            smtp_password: None,
            smtp_from: "brunch@example.test".into(),
            smtp_tls: false,
            secure_cookies,
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
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let mut request = client.request(method.clone(), format!("{}{path}", self.url));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert_eq!(status, expected, "{method} {path}: {text}");
        serde_json::from_str(&text).unwrap()
    }
    async fn login(&self, email: &str) -> Client {
        let client = self.client();
        let is_admin: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1 AND is_admin)")
                .bind(email)
                .fetch_one(&self.pool)
                .await
                .unwrap();
        if is_admin {
            self.call(
                &client,
                Method::POST,
                "/api/admin/auth/login",
                Some(json!({"email":email,"password":PASSWORD})),
                StatusCode::OK,
            )
            .await;
            return client;
        }
        self.call(
            &client,
            Method::POST,
            "/api/auth/request",
            Some(json!({"email": email})),
            StatusCode::OK,
        )
        .await;
        let body: String = sqlx::query_scalar(
            "SELECT body FROM outbox WHERE to_email=$1 ORDER BY created_at DESC,id DESC LIMIT 1",
        )
        .bind(email)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let token = body
            .split("login_token=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        self.call(
            &client,
            Method::POST,
            "/api/auth/verify",
            Some(json!({"token":token})),
            StatusCode::OK,
        )
        .await;
        client
    }
    async fn customer_login(&self, email: &str) -> Client {
        let client = self.client();
        self.call(
            &client,
            Method::POST,
            "/api/auth/request",
            Some(json!({"email":email})),
            StatusCode::OK,
        )
        .await;
        let body: String = sqlx::query_scalar(
            "SELECT body FROM outbox WHERE to_email=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(email)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let token = body
            .split("login_token=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let result = self
            .call(
                &client,
                Method::POST,
                "/api/auth/verify",
                Some(json!({"token":token})),
                StatusCode::OK,
            )
            .await;
        assert_eq!(result["user"]["is_admin"], false);
        client
    }
    async fn membership(
        &self,
        client: &Client,
        method: Method,
        email: &str,
        expected: StatusCode,
    ) -> Value {
        self.call(
            client,
            method,
            ENDPOINT,
            Some(json!({"email": email, "password": PASSWORD})),
            expected,
        )
        .await
    }
    async fn admins(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT email FROM users WHERE is_admin ORDER BY email")
            .fetch_all(&self.pool)
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
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn membership_endpoints_require_admin_and_mutation_csrf_protection() {
    let fixture = Fixture::new().await;
    let anonymous = fixture.client();
    let customer = fixture.login("customer@example.test").await;
    for (client, expected) in [
        (&anonymous, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        fixture
            .call(client, Method::GET, ENDPOINT, None, expected)
            .await;
        for method in [Method::POST, Method::DELETE] {
            fixture
                .membership(client, method, INITIAL_ADMIN, expected)
                .await;
        }
    }
    let admin = fixture.login(INITIAL_ADMIN).await;
    for method in [Method::POST, Method::DELETE] {
        for (name, value) in [
            ("x-brunch-request", "0"),
            ("origin", "https://attacker.example"),
            ("sec-fetch-site", "cross-site"),
        ] {
            let response = admin
                .request(method.clone(), format!("{}{ENDPOINT}", fixture.url))
                .header(name, value)
                .json(&json!({"email": INITIAL_ADMIN}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
    assert_eq!(fixture.admins().await, [INITIAL_ADMIN]);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn administrator_add_normalizes_validates_and_deduplicates_email() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    for email in [
        "",
        "invalid",
        "Name <user@example.test>",
        "a b@example.test",
        "a@example.test\r\nBcc:other@example.test",
    ] {
        for method in [Method::POST, Method::DELETE] {
            fixture
                .membership(&admin, method, email, StatusCode::BAD_REQUEST)
                .await;
        }
    }
    let added = fixture
        .membership(
            &admin,
            Method::POST,
            "  Second@Example.Test  ",
            StatusCode::OK,
        )
        .await;
    assert_eq!(added["administrator"]["email"], "second@example.test");
    assert!(Uuid::parse_str(added["administrator"]["id"].as_str().unwrap()).is_ok());
    fixture
        .membership(
            &admin,
            Method::POST,
            "SECOND@example.test",
            StatusCode::CONFLICT,
        )
        .await;
    fixture
        .membership(
            &admin,
            Method::DELETE,
            "missing@example.test",
            StatusCode::NOT_FOUND,
        )
        .await;
    let list = fixture
        .call(&admin, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    assert_eq!(list["administrators"].as_array().unwrap().len(), 2);
    assert_eq!(list["administrators"][1], added["administrator"]);
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE to_email='second@example.test'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(
        queued, 0,
        "Granting access must not send an unsolicited message"
    );
    fixture
        .membership(
            &admin,
            Method::DELETE,
            " Second@Example.Test ",
            StatusCode::OK,
        )
        .await;
    assert_eq!(fixture.admins().await, [INITIAL_ADMIN]);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn promoting_customer_preserves_account_and_orders_but_requires_fresh_login() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    let customer = fixture.login("customer@example.test").await;
    let another_session = fixture.login("customer@example.test").await;
    let before: (Uuid, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT id,created_at FROM users WHERE email='customer@example.test'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let order_id = Uuid::new_v4();
    sqlx::query("INSERT INTO orders(id,idempotency_key,request_payload,user_id,email,street,city,postal_code,delivery_date,delivery_slot,payment_method,subtotal_cents,delivery_fee_cents,total_cents) VALUES ($1,$2,'{}',$3,'customer@example.test','Test 1','Praha','11000',current_date,'07:00–09:00','cash_on_delivery',0,0,0)")
        .bind(order_id).bind(Uuid::new_v4()).bind(before.0).execute(&fixture.pool).await.unwrap();
    fixture
        .membership(
            &admin,
            Method::POST,
            "customer@example.test",
            StatusCode::OK,
        )
        .await;
    for client in [&customer, &another_session] {
        fixture
            .call(
                client,
                Method::GET,
                ENDPOINT,
                None,
                StatusCode::UNAUTHORIZED,
            )
            .await;
    }
    let promoted = fixture.login("customer@example.test").await;
    fixture
        .call(&promoted, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    let after: (Uuid, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT id,created_at FROM users WHERE email='customer@example.test'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let order_owner: Uuid = sqlx::query_scalar("SELECT user_id FROM orders WHERE id=$1")
        .bind(order_id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(order_owner, before.0);
    let session_seconds: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (expires_at-created_at))::float8 FROM sessions WHERE user_id=$1",
    )
    .bind(before.0)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert!((session_seconds - 43_200.0).abs() < 1.0);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn removal_revokes_all_sessions_and_restart_does_not_restore_initial_admin() {
    let fixture = Fixture::new().await;
    let first = fixture.login(INITIAL_ADMIN).await;
    let first_other_session = fixture.login(INITIAL_ADMIN).await;
    fixture
        .membership(&first, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    let second = fixture.login("second@example.test").await;
    fixture
        .membership(&second, Method::DELETE, INITIAL_ADMIN, StatusCode::OK)
        .await;
    for client in [&first, &first_other_session] {
        fixture
            .call(
                client,
                Method::GET,
                ENDPOINT,
                None,
                StatusCode::UNAUTHORIZED,
            )
            .await;
        fixture
            .membership(
                client,
                Method::POST,
                "third@example.test",
                StatusCode::UNAUTHORIZED,
            )
            .await;
    }
    let error = auth::initialize_admin(&fixture.pool, INITIAL_ADMIN, PASSWORD)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    migrate(&fixture.pool).await.unwrap();
    assert_eq!(fixture.admins().await, ["second@example.test"]);
    let first_as_customer = fixture.login(INITIAL_ADMIN).await;
    fixture
        .call(
            &first_as_customer,
            Method::GET,
            ENDPOINT,
            None,
            StatusCode::FORBIDDEN,
        )
        .await;
    fixture
        .call(&second, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn last_admin_is_protected_and_self_removal_is_allowed_when_another_exists() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    let denied = fixture
        .membership(&admin, Method::DELETE, INITIAL_ADMIN, StatusCode::CONFLICT)
        .await;
    assert_eq!(denied["error"], "Posledního administrátora nelze odebrat.");
    fixture
        .membership(&admin, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    fixture
        .membership(&admin, Method::DELETE, INITIAL_ADMIN, StatusCode::OK)
        .await;
    fixture
        .call(
            &admin,
            Method::GET,
            "/api/auth/me",
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    assert_eq!(fixture.admins().await, ["second@example.test"]);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn concurrent_self_removals_cannot_remove_last_admin() {
    let fixture = Fixture::new().await;
    let first = fixture.login(INITIAL_ADMIN).await;
    fixture
        .membership(&first, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    let second = fixture.login("second@example.test").await;
    let first_request = first
        .delete(format!("{}{ENDPOINT}", fixture.url))
        .json(&json!({"email": INITIAL_ADMIN}));
    let second_request = second
        .delete(format!("{}{ENDPOINT}", fixture.url))
        .json(&json!({"email": "second@example.test"}));
    let (first_result, second_result) = tokio::join!(first_request.send(), second_request.send());
    let mut codes = [
        first_result.unwrap().status().as_u16(),
        second_result.unwrap().status().as_u16(),
    ];
    codes.sort_unstable();
    assert_eq!(codes, [200, 409]);
    assert_eq!(fixture.admins().await.len(), 1);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn queued_membership_change_rechecks_revoked_actor_after_lock() {
    let fixture = Fixture::new().await;
    let first = fixture.login(INITIAL_ADMIN).await;
    fixture
        .membership(&first, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    let second = fixture.login("second@example.test").await;
    let mut revocation = fixture.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(87654321)")
        .execute(&mut *revocation)
        .await
        .unwrap();
    let request = second
        .post(format!("{}{ENDPOINT}", fixture.url))
        .json(&json!({"email": "third@example.test", "password": PASSWORD}));
    let queued = tokio::spawn(async move { request.send().await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=87654321 AND NOT granted)")
                .fetch_one(&fixture.pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("membership request should wait behind the revocation lock");
    sqlx::query("UPDATE users SET is_admin=false WHERE email='second@example.test'")
        .execute(&mut *revocation)
        .await
        .unwrap();
    sqlx::query("DELETE FROM sessions WHERE user_id=(SELECT id FROM users WHERE email='second@example.test')").execute(&mut *revocation).await.unwrap();
    revocation.commit().await.unwrap();
    assert_eq!(queued.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(fixture.admins().await, [INITIAL_ADMIN]);
    let third_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email='third@example.test')")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert!(!third_exists);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn passwords_use_distinct_random_salts_and_hashes_never_leave_api() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    fixture
        .membership(&admin, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT admin_password_hash FROM users WHERE is_admin ORDER BY email")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
    let parsed: Vec<_> = hashes
        .iter()
        .map(|hash| argon2::PasswordHash::new(hash).unwrap())
        .collect();
    assert_ne!(parsed[0].salt, parsed[1].salt);
    for hash in &parsed {
        assert_eq!(hash.algorithm.as_str(), "argon2id");
        assert_eq!(hash.version, Some(19));
        assert_eq!(hash.params.get_decimal("m"), Some(19_456));
        assert_eq!(hash.params.get_decimal("t"), Some(2));
        assert_eq!(hash.params.get_decimal("p"), Some(1));
        let mut salt = [0; 64];
        assert_eq!(hash.salt.unwrap().decode_b64(&mut salt).unwrap().len(), 16);
        use argon2::PasswordVerifier;
        argon2::Argon2::default()
            .verify_password(PASSWORD.as_bytes(), hash)
            .unwrap();
    }
    let list = fixture
        .call(&admin, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    for entry in list["administrators"].as_array().unwrap() {
        assert_eq!(entry.as_object().unwrap().len(), 2);
        assert!(entry.get("id").is_some() && entry.get("email").is_some());
    }
    let me = fixture
        .call(&admin, Method::GET, "/api/auth/me", None, StatusCode::OK)
        .await;
    for value in [list.to_string(), me.to_string()] {
        assert!(!value.contains("argon2") && !value.contains(PASSWORD));
    }
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn admin_password_login_is_required_even_for_valid_customer_magic_link() {
    let fixture = Fixture::new().await;
    let customer_session = fixture.customer_login(INITIAL_ADMIN).await;
    fixture
        .call(
            &customer_session,
            Method::GET,
            ENDPOINT,
            None,
            StatusCode::FORBIDDEN,
        )
        .await;
    let me = fixture
        .call(
            &customer_session,
            Method::GET,
            "/api/auth/me",
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(me["user"]["is_admin"], false);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE admin_authenticated")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let admin = fixture.login(INITIAL_ADMIN).await;
    fixture
        .call(&admin, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn password_reset_changes_hash_revokes_sessions_and_rejects_old_password() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    fixture
        .membership(&admin, Method::POST, "second@example.test", StatusCode::OK)
        .await;
    let second = fixture.login("second@example.test").await;
    let before: String = sqlx::query_scalar(
        "SELECT admin_password_hash FROM users WHERE email='second@example.test'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    let replacement = "A new test password with spaces 2026";
    fixture
        .call(
            &admin,
            Method::PUT,
            "/api/admin/administrators/password",
            Some(json!({"email":" SECOND@example.test ","password":replacement})),
            StatusCode::OK,
        )
        .await;
    fixture
        .call(
            &second,
            Method::GET,
            "/api/auth/me",
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    let after: String = sqlx::query_scalar(
        "SELECT admin_password_hash FROM users WHERE email='second@example.test'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_ne!(before, after);
    fixture
        .call(
            &second,
            Method::POST,
            "/api/admin/auth/login",
            Some(json!({"email":"second@example.test","password":PASSWORD})),
            StatusCode::UNAUTHORIZED,
        )
        .await;
    fixture
        .call(
            &second,
            Method::POST,
            "/api/admin/auth/login",
            Some(json!({"email":"second@example.test","password":replacement})),
            StatusCode::OK,
        )
        .await;
    fixture
        .call(&second, Method::GET, ENDPOINT, None, StatusCode::OK)
        .await;
    fixture
        .call(
            &second,
            Method::PUT,
            "/api/admin/administrators/password",
            Some(json!({"email":"second@example.test","password":PASSWORD})),
            StatusCode::OK,
        )
        .await;
    fixture
        .call(
            &second,
            Method::GET,
            ENDPOINT,
            None,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn password_login_uses_generic_failures_and_rate_limits_accounts_and_globally() {
    let fixture = Fixture::new().await;
    let anonymous = fixture.client();
    let body = |email: &str| Some(json!({"email":email,"password":"Wrong password 2026"}));
    let known = fixture
        .call(
            &anonymous,
            Method::POST,
            "/api/admin/auth/login",
            body(INITIAL_ADMIN),
            StatusCode::UNAUTHORIZED,
        )
        .await;
    let unknown = fixture
        .call(
            &anonymous,
            Method::POST,
            "/api/admin/auth/login",
            body("unknown@example.test"),
            StatusCode::UNAUTHORIZED,
        )
        .await;
    assert_eq!(known, unknown);
    let key = format!("admin-login:{}", auth::token_hash(INITIAL_ADMIN));
    sqlx::query("UPDATE request_limits SET requests=10 WHERE key=$1")
        .bind(key)
        .execute(&fixture.pool)
        .await
        .unwrap();
    fixture
        .call(
            &anonymous,
            Method::POST,
            "/api/admin/auth/login",
            Some(json!({"email":INITIAL_ADMIN,"password":PASSWORD})),
            StatusCode::TOO_MANY_REQUESTS,
        )
        .await;
    sqlx::query("UPDATE request_limits SET requests=200 WHERE key='admin-login:global'")
        .execute(&fixture.pool)
        .await
        .unwrap();
    fixture
        .call(
            &anonymous,
            Method::POST,
            "/api/admin/auth/login",
            body("different@example.test"),
            StatusCode::TOO_MANY_REQUESTS,
        )
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn password_validation_and_authentication_require_mutation_csrf_headers() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    for password in [
        String::new(),
        "short".into(),
        "a".repeat(129),
        "🦀".repeat(129),
    ] {
        for (method, path) in [
            (Method::POST, ENDPOINT),
            (Method::PUT, "/api/admin/administrators/password"),
        ] {
            fixture
                .call(
                    &admin,
                    method,
                    path,
                    Some(json!({"email": INITIAL_ADMIN,"password":password})),
                    StatusCode::BAD_REQUEST,
                )
                .await;
        }
    }
    for (method, path) in [
        (Method::POST, "/api/admin/auth/login"),
        (Method::PUT, "/api/admin/administrators/password"),
    ] {
        for (header, value) in [
            ("x-brunch-request", "0"),
            ("origin", "https://untrusted.example"),
            ("sec-fetch-site", "cross-site"),
        ] {
            let response = admin
                .request(method.clone(), format!("{}{path}", fixture.url))
                .header(header, value)
                .json(&json!({"email":INITIAL_ADMIN,"password":PASSWORD}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
    fixture
        .call(
            &fixture.client(),
            Method::PUT,
            "/api/admin/administrators/password",
            Some(json!({"email":INITIAL_ADMIN,"password":PASSWORD})),
            StatusCode::UNAUTHORIZED,
        )
        .await;
    let customer = fixture.login("customer@example.test").await;
    fixture
        .call(
            &customer,
            Method::PUT,
            "/api/admin/administrators/password",
            Some(json!({"email":INITIAL_ADMIN,"password":PASSWORD})),
            StatusCode::FORBIDDEN,
        )
        .await;
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn secure_password_login_sets_short_lived_http_only_cookie() {
    let fixture = Fixture::with_secure_cookies(true).await;
    let response = fixture
        .client()
        .post(format!("{}/api/admin/auth/login", fixture.url))
        .json(&json!({"email": " ADMIN@EXAMPLE.TEST ","password":PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    for flag in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Max-Age=43200",
        "Path=/",
    ] {
        assert!(cookie.contains(flag));
    }
    assert!(!cookie.contains("Domain="));
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["user"]["email"], INITIAL_ADMIN);
    assert_eq!(result["user"]["is_admin"], true);
    fixture.close().await;
}

async fn wait_for_administrator_lock(pool: &PgPool) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let waiting: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=87654321 AND NOT granted)").fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("request should wait behind held administrator lock");
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn queued_membership_change_rejects_session_expired_while_waiting() {
    let fixture = Fixture::new().await;
    let admin = fixture.login(INITIAL_ADMIN).await;
    let mut tx = fixture.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(87654321)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let request = admin
        .post(format!("{}{ENDPOINT}", fixture.url))
        .json(&json!({"email":"second@example.test","password":PASSWORD}));
    let waiting = tokio::spawn(async move { request.send().await.unwrap() });
    wait_for_administrator_lock(&fixture.pool).await;
    // This expiration is later than the queued transaction's start, but past by release.
    sqlx::query("UPDATE sessions SET expires_at=clock_timestamp()+interval '100 milliseconds'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    tx.commit().await.unwrap();
    assert_eq!(waiting.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(fixture.admins().await, [INITIAL_ADMIN]);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn login_rechecks_password_hash_and_role_after_verification() {
    for revoke in [false, true] {
        let fixture = Fixture::new().await;
        let admin = fixture.login(INITIAL_ADMIN).await;
        fixture
            .membership(&admin, Method::POST, "second@example.test", StatusCode::OK)
            .await;
        let mut tx = fixture.pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(87654321)")
            .execute(&mut *tx)
            .await
            .unwrap();
        let request = fixture
            .client()
            .post(format!("{}/api/admin/auth/login", fixture.url))
            .json(&json!({"email":INITIAL_ADMIN,"password":PASSWORD}));
        let waiting = tokio::spawn(async move { request.send().await.unwrap() });
        wait_for_administrator_lock(&fixture.pool).await;
        if revoke {
            sqlx::query("UPDATE users SET is_admin=false,admin_password_hash=NULL WHERE email=$1")
                .bind(INITIAL_ADMIN)
                .execute(&mut *tx)
                .await
                .unwrap();
        } else {
            // Same password with a fresh salt still invalidates the verification snapshot.
            sqlx::query("UPDATE users SET admin_password_hash=(SELECT admin_password_hash FROM users WHERE email='second@example.test') WHERE email=$1").bind(INITIAL_ADMIN).execute(&mut *tx).await.unwrap();
        }
        sqlx::query("DELETE FROM sessions WHERE user_id=(SELECT id FROM users WHERE email=$1)")
            .bind(INITIAL_ADMIN)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(waiting.await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sessions WHERE user_id=(SELECT id FROM users WHERE email=$1)",
        )
        .bind(INITIAL_ADMIN)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn password_migration_revokes_legacy_admin_sessions_and_preserves_customers() {
    let fixture = Fixture::new().await;
    let _admin = fixture.login(INITIAL_ADMIN).await;
    let _customer = fixture.customer_login("customer@example.test").await;
    let before: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email=$1")
        .bind(INITIAL_ADMIN)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    // Recreate the precise pre-0011 auth schema within this disposable fixture.
    sqlx::raw_sql("ALTER TABLE users DROP CONSTRAINT administrator_requires_password; ALTER TABLE users DROP COLUMN admin_password_hash; ALTER TABLE sessions DROP COLUMN admin_authenticated;").execute(&fixture.pool).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0011_administrator_passwords.sql"
    ))
    .execute(&fixture.pool)
    .await
    .unwrap();
    assert!(fixture.admins().await.is_empty());
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT u.email FROM sessions s JOIN users u ON u.id=s.user_id")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(remaining, ["customer@example.test"]);
    auth::initialize_admin(&fixture.pool, INITIAL_ADMIN, PASSWORD)
        .await
        .unwrap();
    let after: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email=$1")
        .bind(INITIAL_ADMIN)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    fixture.close().await;
}
