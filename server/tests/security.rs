//! SMTP failure and privilege rotation regressions using isolated PostgreSQL schemas.
//! Run with TEST_DATABASE_URL=... cargo test --test security -- --ignored.
use axum::http::{HeaderMap, StatusCode};
use brunch_server::{AppState, Config, auth, mail, migrate};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    str::FromStr,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};
use uuid::Uuid;

struct Fixture {
    state: AppState,
    database: PgPool,
    schema: String,
}

impl Fixture {
    async fn new(smtp_port: u16) -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("Set TEST_DATABASE_URL to a disposable local PostgreSQL database");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_security_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&database)
            .await
            .unwrap();
        let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let config = Config {
            app_env: brunch_server::config::AppEnvironment::Development,
            payment_timeout_minutes: 60,
            teya: None,
            database_url,
            bind: "127.0.0.1:0".parse().unwrap(),
            public_url: "http://127.0.0.1:3100".into(),
            order_email: "orders@example.test".into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port,
            smtp_username: None,
            smtp_password: None,
            smtp_from: "brunch@example.test".into(),
            smtp_tls: false,
            secure_cookies: false,
            dist_dir: "../public".into(),
        };
        Self {
            state: AppState {
                pool,
                config: Arc::new(config),
            },
            database,
            schema,
        }
    }

    async fn enqueue(&self, subject: &str, body: &str) -> Uuid {
        let mut conn = self.state.pool.acquire().await.unwrap();
        mail::enqueue(&mut conn, "customer@example.test", subject, body)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT id FROM outbox ORDER BY created_at DESC LIMIT 1")
            .fetch_one(&mut *conn)
            .await
            .unwrap()
    }

    async fn close(self) {
        self.state.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.database)
            .await
            .unwrap();
        self.database.close().await;
    }
}

/// Pauses after receiving DATA, allowing the test to simulate a lease changing owners.
fn smtp_gate(
    accept: bool,
) -> (
    u16,
    tokio::sync::oneshot::Receiver<()>,
    mpsc::Sender<()>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (received_tx, received_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        stream.write_all(b"220 localhost SMTP\r\n").unwrap();
        let mut received_tx = Some(received_tx);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let command = line.to_ascii_uppercase();
            if command.starts_with("EHLO") || command.starts_with("HELO") {
                stream.write_all(b"250 localhost\r\n").unwrap();
            } else if command.starts_with("DATA") {
                stream.write_all(b"354 Send data\r\n").unwrap();
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == ".\r\n" {
                        break;
                    }
                }
                received_tx.take().unwrap().send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                stream
                    .write_all(if accept {
                        b"250 Queued\r\n"
                    } else {
                        b"550 Rejected\r\n"
                    })
                    .unwrap();
            } else if command.starts_with("QUIT") {
                stream.write_all(b"221 Bye\r\n").unwrap();
                break;
            } else {
                stream.write_all(b"250 Ok\r\n").unwrap();
            }
        }
    });
    (port, received_rx, release_tx, thread)
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn expired_login_mail_is_failed_and_redacted_without_smtp() {
    for subject in ["Přihlášení do Brunch", mail::LOGIN_SUBJECT] {
        let fixture = Fixture::new(9).await;
        let id = fixture
            .enqueue(subject, "secret login_token=expired-private-token")
            .await;
        sqlx::query("UPDATE outbox SET created_at=now()-interval '16 minutes' WHERE id=$1")
            .bind(id)
            .execute(&fixture.state.pool)
            .await
            .unwrap();
        let transport = mail::transport(&fixture.state.config).unwrap();
        assert_eq!(
            mail::deliver_batch(&fixture.state, &transport)
                .await
                .unwrap(),
            0
        );
        let (status, body, attempts, lease): (String, String, i32, Option<Uuid>) =
            sqlx::query_as("SELECT status,body,attempts,lease_token FROM outbox WHERE id=$1")
                .bind(id)
                .fetch_one(&fixture.state.pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        assert!(!body.contains("expired-private-token"));
        assert_eq!(attempts, 0);
        assert!(lease.is_none());
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn stale_smtp_success_and_failure_cannot_overwrite_a_new_lease() {
    for accepted in [true, false] {
        let (port, received, release, smtp_thread) = smtp_gate(accepted);
        let fixture = Fixture::new(port).await;
        let id = fixture.enqueue("Objednávka BR-1", "Order details").await;
        let state = fixture.state.clone();
        let transport = mail::transport(&state.config).unwrap();
        let worker =
            tokio::spawn(async move { mail::deliver_batch(&state, &transport).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(10), received)
            .await
            .unwrap()
            .unwrap();
        let new_lease = Uuid::new_v4();
        sqlx::query("UPDATE outbox SET lease_token=$2,locked_until=now()+interval '2 minutes',attempts=attempts+1 WHERE id=$1")
            .bind(id).bind(new_lease).execute(&fixture.state.pool).await.unwrap();
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), worker)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        smtp_thread.join().unwrap();
        let (status, lease, attempts, error): (String, Option<Uuid>, i32, Option<String>) =
            sqlx::query_as("SELECT status,lease_token,attempts,last_error FROM outbox WHERE id=$1")
                .bind(id)
                .fetch_one(&fixture.state.pool)
                .await
                .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(lease, Some(new_lease));
        assert_eq!(attempts, 2);
        assert!(error.is_none());
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn terminal_login_delivery_failure_removes_the_secret() {
    for subject in ["Přihlášení do Brunch", mail::LOGIN_SUBJECT] {
        let (port, received, release, smtp_thread) = smtp_gate(false);
        let fixture = Fixture::new(port).await;
        let id = fixture
            .enqueue(subject, "login_token=terminal-private-token")
            .await;
        sqlx::query("UPDATE outbox SET attempts=7 WHERE id=$1")
            .bind(id)
            .execute(&fixture.state.pool)
            .await
            .unwrap();
        let state = fixture.state.clone();
        let transport = mail::transport(&state.config).unwrap();
        let worker =
            tokio::spawn(async move { mail::deliver_batch(&state, &transport).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(10), received)
            .await
            .unwrap()
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), worker)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        smtp_thread.join().unwrap();
        let (status, body, attempts): (String, String, i32) =
            sqlx::query_as("SELECT status,body,attempts FROM outbox WHERE id=$1")
                .bind(id)
                .fetch_one(&fixture.state.pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        assert!(!body.contains("terminal-private-token"));
        assert_eq!(attempts, 8);
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn successful_login_delivery_redacts_both_brand_names() {
    for subject in ["Přihlášení do Brunch", mail::LOGIN_SUBJECT] {
        let (port, received, release, smtp_thread) = smtp_gate(true);
        let fixture = Fixture::new(port).await;
        let id = fixture
            .enqueue(subject, "login_token=sent-private-token")
            .await;
        let state = fixture.state.clone();
        let transport = mail::transport(&state.config).unwrap();
        let worker =
            tokio::spawn(async move { mail::deliver_batch(&state, &transport).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(10), received)
            .await
            .unwrap()
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), worker)
                .await
                .unwrap()
                .unwrap(),
            1
        );
        smtp_thread.join().unwrap();
        let (status, body): (String, String) =
            sqlx::query_as("SELECT status,body FROM outbox WHERE id=$1")
                .bind(id)
                .fetch_one(&fixture.state.pool)
                .await
                .unwrap();
        assert_eq!(status, "sent");
        assert!(!body.contains("sent-private-token"));
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn administrator_cannot_retry_login_mail_from_either_brand() {
    let fixture = Fixture::new(9).await;
    auth::initialize_admin(
        &fixture.state.pool,
        "admin@example.test",
        "Test-only-admin-password-2026",
    )
    .await
    .unwrap();
    let token = "c".repeat(64);
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,admin_authenticated) SELECT $1,id,now()+interval '1 hour',true FROM users WHERE email='admin@example.test'")
        .bind(auth::token_hash(&token)).execute(&fixture.state.pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = brunch_server::app(fixture.state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    for subject in [
        "Přihlášení do Brunch",
        mail::LOGIN_SUBJECT,
        "Nová objednávka BR-TEST",
    ] {
        let id = fixture.enqueue(subject, "test message").await;
        sqlx::query("UPDATE outbox SET status='failed',attempts=8 WHERE id=$1")
            .bind(id)
            .execute(&fixture.state.pool)
            .await
            .unwrap();
        let response = client
            .post(format!("{url}/api/admin/mail/{id}/retry"))
            .header("Cookie", format!("brunch_session={token}"))
            .header("X-Brunch-Request", "1")
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        let login = subject != "Nová objednávka BR-TEST";
        assert_eq!(
            response.status(),
            if login {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            }
        );
        let status: String = sqlx::query_scalar("SELECT status FROM outbox WHERE id=$1")
            .bind(id)
            .fetch_one(&fixture.state.pool)
            .await
            .unwrap();
        assert_eq!(status, if login { "failed" } else { "pending" });
    }
    server.abort();
    let _ = server.await;
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn initialization_refuses_to_replace_existing_administrators_or_sessions() {
    let fixture = Fixture::new(9).await;
    auth::initialize_admin(
        &fixture.state.pool,
        "old-admin@example.test",
        "Test-only-admin-password-2026",
    )
    .await
    .unwrap();
    let user_id: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email='old-admin@example.test'")
            .fetch_one(&fixture.state.pool)
            .await
            .unwrap();
    let token = "a".repeat(64);
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,admin_authenticated) VALUES ($1,$2,now()+interval '1 hour',true)")
        .bind(auth::token_hash(&token)).bind(user_id).execute(&fixture.state.pool).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("brunch_session={token}").parse().unwrap());
    assert!(auth::require_admin(&fixture.state, &headers).await.is_ok());
    let error = auth::initialize_admin(
        &fixture.state.pool,
        "new-admin@example.test",
        "Test-only-admin-password-2026",
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(auth::require_admin(&fixture.state, &headers).await.is_ok());
    let admins: Vec<String> = sqlx::query_scalar("SELECT email FROM users WHERE is_admin")
        .fetch_all(&fixture.state.pool)
        .await
        .unwrap();
    assert_eq!(admins, ["old-admin@example.test"]);
    fixture.close().await;
}
