//! Image upload boundaries and immutable public delivery against isolated PostgreSQL schemas.
use base64::{Engine, engine::general_purpose::STANDARD};
use brunch_server::{AppState, Config, app, auth, migrate};
use image::{ImageEncoder, ImageFormat};
use reqwest::{Client, Response, StatusCode};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{io::Cursor, str::FromStr, sync::Arc};
use tokio::task::JoinHandle;
use uuid::Uuid;

struct Fixture {
    url: String,
    pool: PgPool,
    database: PgPool,
    schema: String,
    server: JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("Set TEST_DATABASE_URL to a disposable local PostgreSQL database");
        let database = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let schema = format!("brunch_media_{}", Uuid::new_v4().simple());
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
            smtp_port: 9,
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
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            url,
            pool,
            database,
            schema,
            server,
        }
    }

    async fn client(&self, email: Option<&str>) -> Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-brunch-request", "1".parse().unwrap());
        if let Some(email) = email {
            let id: Uuid = sqlx::query_scalar("INSERT INTO users(id,email) VALUES ($1,$2) ON CONFLICT(email) DO UPDATE SET email=excluded.email RETURNING id")
                .bind(Uuid::new_v4()).bind(email).fetch_one(&self.pool).await.unwrap();
            let token = Uuid::new_v4().simple().to_string().repeat(2);
            sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,admin_authenticated) VALUES ($1,$2,now()+interval '1 hour',true)")
                .bind(auth::token_hash(&token)).bind(id).execute(&self.pool).await.unwrap();
            headers.insert("cookie", format!("brunch_session={token}").parse().unwrap());
        }
        Client::builder().default_headers(headers).build().unwrap()
    }

    async fn upload(&self, client: &Client, content: &[u8]) -> Response {
        client
            .post(format!("{}/api/admin/images", self.url))
            .json(&json!({"filename":"product.png","content_base64":STANDARD.encode(content)}))
            .send()
            .await
            .unwrap()
    }

    async fn close(self) {
        self.server.abort();
        let _ = self.server.await;
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.database)
            .await
            .unwrap();
        self.database.close().await;
    }
}

fn png(width: u32, height: u32, noise: bool) -> Vec<u8> {
    let mut state = 0x2345_6789_u32;
    let image = image::RgbaImage::from_fn(width, height, |_, _| {
        if noise {
            let mut pixel = [0; 4];
            for value in &mut pixel {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *value = state as u8;
            }
            image::Rgba(pixel)
        } else {
            image::Rgba([71, 128, 203, 130])
        }
    });
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn uploads_real_formats_strips_trailing_data_and_serves_immutable_public_images() {
    let fixture = Fixture::new().await;
    let admin = fixture.client(Some("admin@example.test")).await;
    let public = fixture.client(None).await;
    let mut source = png(128, 128, true);
    assert!(
        STANDARD.encode(&source).len() > 64 * 1024,
        "exercise route-specific body limit"
    );
    source.extend_from_slice(b"PRIVATE-TRAILING-METADATA");
    source.resize(40 * 1024 * 1024, 0);
    let response = admin
        .post(format!("{}/api/admin/images", fixture.url))
        .header("content-type", "application/octet-stream")
        .body(source.clone())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let uploaded: serde_json::Value = serde_json::from_str(&body).unwrap();
    let path = uploaded["url"].as_str().unwrap();
    assert!(path.starts_with("/api/images/"));
    let response = public
        .get(format!("{}{path}", fixture.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let etag = response.headers()["etag"].clone();
    let stored = response.bytes().await.unwrap();
    assert!(
        !stored
            .windows(b"PRIVATE-TRAILING-METADATA".len())
            .any(|bytes| bytes == b"PRIVATE-TRAILING-METADATA")
    );
    assert_eq!(
        image::load_from_memory(&stored).unwrap().to_rgba8(),
        image::load_from_memory(&source).unwrap().to_rgba8()
    );
    let cached = public
        .get(format!("{}{path}", fixture.url))
        .header("if-none-match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
    assert!(cached.bytes().await.unwrap().is_empty());

    let rgb = image::RgbImage::from_pixel(5, 3, image::Rgb([42, 86, 193]));
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode_image(&rgb)
        .unwrap();
    let mut webp = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .write_image(rgb.as_raw(), 5, 3, image::ColorType::Rgb8.into())
        .unwrap();
    for source in [jpeg, webp] {
        let response = fixture.upload(&admin, &source).await;
        assert_eq!(response.status(), StatusCode::OK);
        let uploaded: serde_json::Value = response.json().await.unwrap();
        let response = public
            .get(format!(
                "{}{}",
                fixture.url,
                uploaded["url"].as_str().unwrap()
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["content-type"], "image/jpeg");
        let decoded = image::load_from_memory(&response.bytes().await.unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (5, 3));
    }
    let mut rotated = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut rotated);
    encoder
        .set_exif_metadata(
            b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec(),
        )
        .unwrap();
    encoder.encode_image(&rgb).unwrap();
    let response = fixture.upload(&admin, &rotated).await;
    assert_eq!(response.status(), StatusCode::OK);
    let uploaded: serde_json::Value = response.json().await.unwrap();
    let response = public
        .get(format!(
            "{}{}",
            fixture.url,
            uploaded["url"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    let stored = response.bytes().await.unwrap();
    let decoded = image::load_from_memory(&stored).unwrap();
    assert_eq!(
        (decoded.width(), decoded.height()),
        (3, 5),
        "phone orientation is applied before metadata is removed"
    );
    assert!(!stored.windows(6).any(|bytes| bytes == b"Exif\0\0"));
    let created_by: Vec<String> =
        sqlx::query_scalar("SELECT u.email FROM images i JOIN users u ON i.created_by=u.id")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(created_by, vec!["admin@example.test"; 4]);
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and local PostgreSQL"]
async fn image_upload_rejects_unauthorized_svg_invalid_data_size_and_dimensions() {
    let fixture = Fixture::new().await;
    let admin = fixture.client(Some("admin@example.test")).await;
    let customer = fixture.client(Some("customer@example.test")).await;
    let public = fixture.client(None).await;
    let valid = png(2, 2, false);
    assert_eq!(
        fixture.upload(&public, &valid).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture.upload(&customer, &valid).await.status(),
        StatusCode::FORBIDDEN
    );
    let invalid = [
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.to_vec(),
        b"GIF89a0123456789".to_vec(),
        b"\x89PNG\r\n\x1a\ncorrupted".to_vec(),
        Vec::new(),
        png(16_385, 1, false),
    ];
    for bytes in invalid {
        let response = fixture.upload(&admin, &bytes).await;
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["error"].is_string());
    }
    let invalid_base64 = admin
        .post(format!("{}/api/admin/images", fixture.url))
        .json(&json!({"filename":"snack.png","content_base64":"%%%not-base64%%%"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_base64.status(), StatusCode::BAD_REQUEST);
    let oversized = admin
        .post(format!("{}/api/admin/images", fixture.url))
        .header("content-type", "application/octet-stream")
        .body(vec![0; 40 * 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let unknown = public
        .get(format!("{}/api/images/{}", fixture.url, Uuid::new_v4()))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "rejected uploads must not create database images");
    fixture.close().await;
}
