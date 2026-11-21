//! Public HTML and discovery checks using HTTP and an isolated PostgreSQL schema.
use brunch_server::{AppState, Config, app, commerce, migrate};
use reqwest::{Client, Response, StatusCode};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::PathBuf, str::FromStr, sync::Arc};
use tokio::task::JoinHandle;
use uuid::Uuid;

const PUBLIC_URL: &str = "https://shop.example.test";

struct TestApp {
    url: String,
    pool: PgPool,
    database: PgPool,
    schema: String,
    dist: PathBuf,
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
        let schema = format!("brunch_seo_test_{}", Uuid::new_v4().simple());
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
        commerce::initialize(&pool, "private-order-recipient@example.test", false)
            .await
            .unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let dist = std::env::temp_dir().join(&schema);
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::copy(root.join("index.html"), dist.join("index.html")).unwrap();
        for private in ["admin", "customer", "team"] {
            std::fs::create_dir_all(dist.join(private)).unwrap();
            std::fs::copy(
                root.join("public").join(private).join("index.html"),
                dist.join(private).join("index.html"),
            )
            .unwrap();
        }
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
            public_url: PUBLIC_URL.into(),
            order_email: "private-order-recipient@example.test".into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port: 1025,
            smtp_username: None,
            smtp_password: None,
            smtp_from: "brunch@example.test".into(),
            smtp_tls: false,
            secure_cookies: false,
            dist_dir: dist.to_string_lossy().into_owned(),
        };
        let router = app(AppState {
            pool: pool.clone(),
            config: Arc::new(config),
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            pool,
            database,
            schema,
            dist,
            task,
        }
    }

    async fn product(&self, name: &str, active: bool) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO products(name, description, price_cents, image, ingredients, allergens, weight, active)
             VALUES ($1, 'Čerstvá houstička s bylinkovým sýrem.', 4950, '/api/images/11111111-1111-4111-8111-111111111111',
             'Pšeničná mouka, sýr, pažitka', 'Lepek, mléko', '80 g', $2) RETURNING id",
        )
        .bind(name)
        .bind(active)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn get(&self, path: &str) -> Response {
        Client::new()
            .get(format!("{}{path}", self.url))
            .send()
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
        std::fs::remove_dir_all(self.dist).unwrap();
    }
}

fn title(html: &str) -> &str {
    html.split("<title>")
        .nth(1)
        .unwrap()
        .split("</title>")
        .next()
        .unwrap()
}

fn canonical(html: &str) -> &str {
    html.split("<link")
        .find(|tag| tag.split('>').next().unwrap().contains("rel=\"canonical\""))
        .unwrap()
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn homepage_has_public_catalog_without_javascript_for_people_and_crawlers() {
    let app = TestApp::new().await;
    let active = app.product("Bylinková houstička", true).await;
    let sold_out = app.product("Vyprodaná houstička", true).await;
    app.product("NEVEŘEJNÝ ARCHIVOVANÝ PRODUKT", false).await;
    sqlx::query("UPDATE products SET stock=0 WHERE id=$1")
        .bind(sold_out)
        .execute(&app.pool)
        .await
        .unwrap();
    let response = app.get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = response.text().await.unwrap();
    assert!(html.contains("id=\"server-content\""));
    assert!(html.contains("Bylinková houstička"));
    assert!(html.contains("Vyprodaná houstička"));
    assert!(html.contains("Čerstvá houstička s bylinkovým sýrem."));
    assert!(html.contains(&format!("href=\"/?product={active}\"")));
    assert!(html.contains("/api/images/11111111-1111-4111-8111-111111111111"));
    assert!(html.contains("49,50"));
    assert_eq!(canonical(&html), format!("{PUBLIC_URL}/"));
    assert!(!html.contains("NEVEŘEJNÝ ARCHIVOVANÝ PRODUKT"));
    assert!(!html.contains("private-order-recipient@example.test"));
    assert!(!html.contains("noindex"));
    let crawler = Client::new()
        .get(format!("{}/", app.url))
        .header("user-agent", "Googlebot")
        .header("host", "attacker.example")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        html, crawler,
        "people and crawlers must receive identical HTML"
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn product_pages_have_specific_metadata_and_reject_nonexistent_products() {
    let app = TestApp::new().await;
    let active = app.product("Sýrová houstička", true).await;
    let archived = app.product("Archivovaná houstička", false).await;
    let response = app
        .get(&format!("/?product={active}&utm_source=test"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = response.text().await.unwrap();
    assert!(title(&html).contains("Sýrová houstička"));
    assert_eq!(canonical(&html), format!("{PUBLIC_URL}/?product={active}"));
    assert!(html.contains("Pšeničná mouka, sýr, pažitka"));
    assert!(html.contains("Lepek, mléko"));
    assert!(html.contains("80 g"));
    assert!(!html.contains("utm_source"));

    for query in [
        format!("product={archived}"),
        "product=9223372036854775807".into(),
        "product=not-a-number".into(),
        "product=".into(),
        "product=-1".into(),
        "product=0".into(),
        format!("product={active}&product={archived}"),
    ] {
        let response = app.get(&format!("/?{query}")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{query}");
        let header_noindex = response
            .headers()
            .get("x-robots-tag")
            .is_some_and(|value| value.to_str().unwrap().contains("noindex"));
        let html = response.text().await.unwrap();
        assert!(
            header_noindex || html.contains("content=\"noindex"),
            "{query}"
        );
        assert!(!html.contains("Archivovaná houstička"));
    }
    let ordinary_query = app.get("/?utm_source=test").await.text().await.unwrap();
    assert_eq!(canonical(&ordinary_query), format!("{PUBLIC_URL}/"));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn sitemap_tracks_active_catalog_without_private_or_host_injected_urls() {
    let app = TestApp::new().await;
    let active = app.product("Houstička", true).await;
    let archived = app.product("Archiv", false).await;
    let response = Client::new()
        .get(format!("{}/sitemap.xml", app.url))
        .header("host", "attacker.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("xml")
    );
    let xml = response.text().await.unwrap();
    assert!(xml.contains("http://www.sitemaps.org/schemas/sitemap/0.9"));
    assert!(xml.contains(&format!("<loc>{PUBLIC_URL}/</loc>")));
    assert!(xml.contains(&format!("<loc>{PUBLIC_URL}/?product={active}</loc>")));
    assert!(!xml.contains(&format!("?product={archived}")));
    for private in [
        "/admin",
        "/customer",
        "/team",
        "attacker.example",
        "private-order-recipient",
    ] {
        assert!(!xml.contains(private));
    }
    sqlx::query("UPDATE products SET active=false WHERE id=$1")
        .bind(active)
        .execute(&app.pool)
        .await
        .unwrap();
    let updated = app.get("/sitemap.xml").await.text().await.unwrap();
    assert!(!updated.contains(&format!("?product={active}")));
    assert!(updated.contains(&format!("<loc>{PUBLIC_URL}/</loc>")));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn metadata_and_visible_content_escape_untrusted_product_text() {
    let app = TestApp::new().await;
    let id = app
        .product("Sýr & <script>alert(1)</script> \"premium\"", true)
        .await;
    sqlx::query("UPDATE products SET description=$1, ingredients=$2, allergens=$3 WHERE id=$4")
        .bind("Popis <img src=x onerror=alert(2)> & chuť")
        .bind("Mouka </p><script>alert(3)</script>")
        .bind("Lepek <b>mléko</b>")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    for path in ["/".to_owned(), format!("/?product={id}")] {
        let html = app.get(&path).await.text().await.unwrap();
        assert!(html.contains("Sýr &amp; &lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(2)&gt; &amp; chuť"));
        for executable in [
            "<script>alert(1)</script>",
            "<img src=x onerror=alert(2)>",
            "<script>alert(3)</script>",
        ] {
            assert!(!html.contains(executable), "{path}: {executable}");
        }
    }
    let product_html = app
        .get(&format!("/?product={id}"))
        .await
        .text()
        .await
        .unwrap();
    assert!(product_html.contains("Mouka &lt;/p&gt;&lt;script&gt;alert(3)&lt;/script&gt;"));
    assert!(product_html.contains("Lepek &lt;b&gt;mléko&lt;/b&gt;"));
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn robots_exposes_sitemap_and_private_pages_keep_noindex() {
    let app = TestApp::new().await;
    let response = app.get("/robots.txt").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    let robots = response.text().await.unwrap();
    assert!(robots.contains("User-agent: *"));
    assert!(robots.contains(&format!("Sitemap: {PUBLIC_URL}/sitemap.xml")));
    for line in robots.lines().map(str::trim) {
        assert!(!matches!(
            line,
            "Disallow: /" | "Disallow: /*" | "Disallow: /api/"
        ));
    }
    for private in ["admin", "customer", "team"] {
        let response = app.get(&format!("/{private}/")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = response.text().await.unwrap();
        assert!(html.contains("name=\"robots\" content=\"noindex,nofollow\""));
    }
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn missing_page_template_is_an_error_instead_of_an_indexable_empty_page() {
    let app = TestApp::new().await;
    std::fs::remove_file(app.dist.join("index.html")).unwrap();
    let response = app.get("/").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    app.close().await;
}
