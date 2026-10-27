use brunch_server::{AppState, Config, mail, migrate};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{str::FromStr, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to local PostgreSQL"]
async fn smtp_delivery_retry_and_crashed_worker_lease() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let control = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("mail_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&control)
        .await
        .unwrap();
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // First attempt targets a closed port. No message is lost or falsely marked sent.
    drop(listener);
    let config = Config {
        app_env: brunch_server::config::AppEnvironment::Development,
        payment_timeout_minutes: 60,
        teya: None,
        database_url: url,
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: "http://localhost".into(),
        order_email: "orders@example.test".into(),
        admin_email: "admin@example.test".into(),
        smtp_host: "127.0.0.1".into(),
        smtp_port: port,
        smtp_username: None,
        smtp_password: None,
        smtp_from: "brunch@example.test".into(),
        smtp_tls: false,
        secure_cookies: false,
        dist_dir: "missing-test-dist".into(),
    };
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config),
    };
    let mut tx = pool.begin().await.unwrap();
    mail::enqueue(
        &mut tx,
        "customer@example.test",
        "Objednávka BR-TEST",
        "Doručení do druhého patra. Celkem 168 Kč.",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let transport = mail::transport(&state.config).unwrap();
    assert_eq!(mail::deliver_batch(&state, &transport).await.unwrap(), 0);
    let row: (String, i32, bool) =
        sqlx::query_as("SELECT status,attempts,next_attempt_at>now() FROM outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, ("pending".into(), 1, true));
    // Expired lease emulates a worker dying during delivery. Another worker must recover it.
    sqlx::query("UPDATE outbox SET next_attempt_at=now(),locked_until=now()-interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    let smtp = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        writer
            .write_all(b"220 localhost ESMTP test\r\n")
            .await
            .unwrap();
        let mut message = String::new();
        let mut data = false;
        while let Some(line) = lines.next_line().await.unwrap() {
            if data {
                if line == "." {
                    data = false;
                    writer.write_all(b"250 queued\r\n").await.unwrap();
                } else {
                    message.push_str(&line);
                    message.push('\n');
                }
            } else if line.starts_with("EHLO") {
                writer
                    .write_all(b"250-localhost\r\n250 8BITMIME\r\n")
                    .await
                    .unwrap();
            } else if line == "DATA" {
                data = true;
                writer.write_all(b"354 go ahead\r\n").await.unwrap();
            } else if line == "QUIT" {
                writer.write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                writer.write_all(b"250 ok\r\n").await.unwrap();
            }
        }
        message
    });
    assert_eq!(mail::deliver_batch(&state, &transport).await.unwrap(), 1);
    let message = smtp.await.unwrap();
    assert!(message.contains("customer@example.test"));
    assert!(message.contains("Message-ID:"));
    assert!(message.contains("Brunch Era"));
    let (headers, body) = message.split_once("\n\n").expect("SMTP message has a body");
    assert!(headers.contains("MIME-Version: 1.0"), "{headers}");
    assert!(
        headers.contains("Content-Type: text/plain; charset=utf-8"),
        "Recipients need the UTF-8 charset to decode Czech text: {headers}"
    );
    assert!(headers.contains("Content-Transfer-Encoding: quoted-printable"));
    // These are the actual UTF-8 bytes transported over SMTP, not ISO-8859-1 text.
    assert!(body.contains("Doru=C4=8Den=C3=AD do druh=C3=A9ho patra. Celkem 168 K=C4=8D."));
    let row: (String, i32, bool) =
        sqlx::query_as("SELECT status,attempts,sent_at IS NOT NULL FROM outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, ("sent".into(), 2, true));
    assert_eq!(mail::deliver_batch(&state, &transport).await.unwrap(), 0);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&control)
        .await
        .unwrap();
    control.close().await;
}
