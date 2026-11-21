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

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn site_settings_validate_uploads_authorization_and_concurrent_updates() {
    let app = TestApp::new().await;
    let guest = app.client();
    let customer = app.login("hero-customer@example.test").await;
    let admin = app.login("admin@example.test").await;
    let initial = app.get(&admin, "/api/admin/site-settings").await;
    assert_eq!(initial["version"], 1);
    assert_eq!(initial["hero_image"], "");
    assert_eq!(
        initial["hero_image_alt"],
        "Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole"
    );
    for (client, status) in [
        (&guest, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        app.call(
            client,
            Method::GET,
            "/api/admin/site-settings",
            None,
            status,
        )
        .await;
        app.call(
            client,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&initial),
            status,
        )
        .await;
    }
    for (header, value) in [
        ("x-brunch-request", ""),
        ("origin", "https://another.example.test"),
    ] {
        let response = admin
            .put(format!("{}/api/admin/site-settings", app.url))
            .header(header, value)
            .json(&initial)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // The accepted URL comes from the existing sanitizer, not arbitrary remote content.
    let mut source = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut source, image::ImageFormat::Png)
        .unwrap();
    let upload = admin
        .post(format!("{}/api/admin/images", app.url))
        .header("content-type", "application/octet-stream")
        .body(source.into_inner())
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::OK);
    let uploaded: Value = upload.json().await.unwrap();
    let image_url = uploaded["url"].as_str().unwrap();
    let response = guest
        .get(format!("{}{image_url}", app.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");

    let mut update =
        json!({"hero_image":image_url,"hero_image_alt":"  Domácí houstičky  ","version":1});
    let invalid_images = [
        "https://example.test/photo.jpg".to_owned(),
        "//example.test/photo.jpg".to_owned(),
        "javascript:alert(1)".to_owned(),
        "data:image/svg+xml,<svg/>".to_owned(),
        "/images/brunch-spread.webp".to_owned(),
        "/api/images/not-a-uuid".to_owned(),
        format!("/api/images/{}", Uuid::new_v4()),
        format!("{image_url}?other=1"),
        format!("{image_url}#fragment"),
        format!("{image_url}/../other"),
        image_url.replace("/api/images/", "/api/images/%2e%2e/"),
        image_url.to_uppercase(),
        format!(
            "/api/images/{}",
            image_url["/api/images/".len()..].to_uppercase()
        ),
        format!(" {image_url}"),
        format!("{image_url} "),
        image_url.replace('-', ""),
    ];
    for invalid in invalid_images {
        update["hero_image"] = json!(invalid);
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&update),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    update["hero_image"] = json!(image_url);
    for invalid in [
        "".to_owned(),
        " \n\t ".to_owned(),
        "ž".repeat(501),
        "Popis\u{0000}".to_owned(),
    ] {
        update["hero_image_alt"] = json!(invalid);
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&update),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    update["hero_image_alt"] = json!(format!("  {}  ", "ž".repeat(500)));
    for invalid in [0, -1] {
        update["version"] = json!(invalid);
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&update),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert_eq!(app.get(&admin, "/api/admin/site-settings").await, initial);
    update["version"] = json!(1);
    let (a, b) = tokio::join!(
        admin
            .put(format!("{}/api/admin/site-settings", app.url))
            .json(&update)
            .send(),
        admin
            .put(format!("{}/api/admin/site-settings", app.url))
            .json(&update)
            .send()
    );
    let statuses = [a.unwrap().status(), b.unwrap().status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let saved = app.get(&admin, "/api/admin/site-settings").await;
    assert_eq!(saved["version"], 2);
    assert_eq!(saved["hero_image"], image_url);
    assert_eq!(saved["hero_image_alt"], "ž".repeat(500));
    let public = app.get(&guest, "/api/settings").await;
    assert_eq!(public["hero_image"], saved["hero_image"]);
    assert_eq!(public["hero_image_alt"], saved["hero_image_alt"]);
    assert!(public.get("version").is_none());
    assert!(public.get("updated_at").is_none());
    assert!(public.get("order_email").is_none());

    let products = app.get(&guest, "/api/products").await;
    let order = app
        .order(&guest, "hero-order@example.test", &products[0], 1)
        .await;
    app.post(&guest, "/api/orders", &order).await;
    let snapshot_sql = "SELECT jsonb_build_object(
        'products',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM products p),
        'prices',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM product_prices p),
        'images',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM images p),
        'orders',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM orders p),
        'items',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM order_items p),
        'mail',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM outbox p),
        'shop',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM shop_settings p),
        'areas',(SELECT jsonb_agg(to_jsonb(p) ORDER BY id) FROM delivery_areas p))";
    let before: Value = sqlx::query_scalar(snapshot_sql)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    migrate(&app.pool).await.unwrap();
    commerce::initialize(&app.pool, "unchanged@example.test", false)
        .await
        .unwrap();
    assert_eq!(
        app.get(&admin, "/api/admin/site-settings").await,
        saved,
        "restart must preserve the selected image and description"
    );
    let reset = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&json!({"hero_image":"","hero_image_alt":"","version":2})),
            StatusCode::OK,
        )
        .await;
    assert_eq!(reset["version"], 3);
    assert_eq!(reset["hero_image"], "");
    assert_eq!(reset["hero_image_alt"], initial["hero_image_alt"]);
    let public = app.get(&guest, "/api/settings").await;
    assert_eq!(public["hero_image"], "");
    assert_eq!(public["hero_image_alt"], initial["hero_image_alt"]);
    let after: Value = sqlx::query_scalar(snapshot_sql)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "site settings must not modify commerce data or stored photo bytes"
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn public_contacts_validate_publish_preserve_legacy_updates_and_conflict() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let initial = app.get(&admin, "/api/admin/site-settings").await;
    let fields = [
        "public_contact_email",
        "public_contact_phone",
        "seller_name",
        "seller_address",
        "seller_ico",
        "delivery_area_label",
    ];
    for field in fields {
        assert_eq!(initial[field], "");
    }
    let mut input = initial.clone();
    input["public_contact_email"] = json!(" INFO@EXAMPLE.TEST ");
    input["public_contact_phone"] = json!(" +420 777 123 456 ");
    input["seller_name"] = json!(" Testovací pekárna ");
    input["seller_address"] = json!(" Testovací 1\nPraha ");
    input["seller_ico"] = json!("12345678");
    input["delivery_area_label"] = json!(" Praha 1 až 5 ");
    let saved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&input),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["public_contact_email"], "info@example.test");
    assert_eq!(saved["seller_name"], "Testovací pekárna");
    assert_eq!(saved["seller_address"], "Testovací 1\nPraha");
    let public = app.get(&guest, "/api/settings").await;
    for field in fields {
        assert_eq!(public[field], saved[field], "public field {field}");
    }
    assert!(
        public.get("order_email").is_none(),
        "private order recipient is never the public contact"
    );
    for (field, value) in [
        ("public_contact_email", "not-email"),
        ("public_contact_phone", "javascript:123456"),
        ("public_contact_phone", "1"),
        ("seller_ico", "123"),
        ("seller_ico", "abcdefgh"),
        ("seller_name", "bad\u{0000}value"),
    ] {
        let mut invalid = saved.clone();
        invalid[field] = json!(value);
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&invalid),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(
            app.get(&admin, "/api/admin/site-settings").await,
            saved,
            "invalid contacts do not change settings"
        );
    }
    let legacy = json!({"hero_image":"", "hero_image_alt":"", "version":saved["version"]});
    let preserved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&legacy),
            StatusCode::OK,
        )
        .await;
    for field in fields {
        assert_eq!(
            preserved[field], saved[field],
            "legacy client preserves {field}"
        );
    }
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/site-settings",
        Some(&saved),
        StatusCode::CONFLICT,
    )
    .await;
    let mut cleared = preserved.clone();
    for field in fields {
        cleared[field] = json!("");
    }
    let cleared = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/site-settings",
            Some(&cleared),
            StatusCode::OK,
        )
        .await;
    for field in fields {
        assert_eq!(cleared[field], "");
    }
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn product_variants_validate_selection_and_preserve_order_snapshot() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let created = app.post(&admin, "/api/admin/products", &json!({"name":"Testovací box", "price_cents":43000, "variants":["  Šunka a sýr  ","Zeleninový"]})).await;
    assert_eq!(created["variants"], json!(["Šunka a sýr", "Zeleninový"]));
    let endpoint = format!("/api/admin/products/{}", created["id"]);
    for invalid in [
        json!([""]),
        json!(["Stejné", "Stejné"]),
        json!(["x".repeat(121)]),
        json!((0..31).map(|i| format!("Varianta {i}")).collect::<Vec<_>>()),
    ] {
        let mut update = product_input(&created);
        update["variants"] = invalid;
        app.call(
            &admin,
            Method::PUT,
            &endpoint,
            Some(&update),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let mail_before = app.count("outbox").await;
    let mut order_input = app
        .order(&guest, "variants@example.test", &created, 1)
        .await;
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&order_input),
        StatusCode::CONFLICT,
    )
    .await;
    order_input["items"][0]["variant"] = json!("Neexistující");
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&order_input),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("outbox").await, mail_before);
    order_input["items"][0]["variant"] = json!("Šunka a sýr");
    let order = app.post(&guest, "/api/orders", &order_input).await;
    assert_eq!(order["items"][0]["variant"], "Šunka a sýr");
    assert!(
        order["items"][0]["name"]
            .as_str()
            .unwrap()
            .contains("Šunka a sýr")
    );
    let mut update = product_input(&created);
    update.as_object_mut().unwrap().remove("variants");
    update["description"] = json!("Nový popis");
    let preserved = app
        .call(
            &admin,
            Method::PUT,
            &endpoint,
            Some(&update),
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        preserved["variants"], created["variants"],
        "legacy product editor preserves variants"
    );
    update = product_input(&preserved);
    update["variants"] = json!(["Jiná náplň"]);
    app.call(
        &admin,
        Method::PUT,
        &endpoint,
        Some(&update),
        StatusCode::OK,
    )
    .await;
    let replay = app.post(&guest, "/api/orders", &order_input).await;
    assert_eq!(
        replay["id"], order["id"],
        "a replay retains original accepted choice"
    );
    assert_eq!(replay["items"][0]["variant"], "Šunka a sýr");
    let summary = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(summary["items"][0]["variant"], "Šunka a sýr");
    assert_eq!(app.count("orders").await, 1);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn terms_publication_is_authorized_versioned_and_preserves_accepted_text() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let initial = app.get(&guest, "/api/terms").await;
    assert_eq!(initial["version"], 1);
    assert!(!initial["content"].as_str().unwrap().trim().is_empty());
    let response = guest
        .get(format!("{}/api/terms", app.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    app.call(
        &guest,
        Method::GET,
        "/api/admin/terms",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let update = json!({"version":1,"title":"Nové obchodní podmínky","content":"## Smlouva\n\nDohodnuté znění. <script>alert(1)</script>"});
    app.call(
        &guest,
        Method::PUT,
        "/api/admin/terms",
        Some(&update),
        StatusCode::UNAUTHORIZED,
    )
    .await;

    let product = &app.get(&guest, "/api/products").await[0];
    let mut input = app.order(&guest, "terms@example.test", product, 1).await;
    input["terms_version"] = json!(1);
    let order = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(order["terms_version"], 1);
    let mail: String =
        sqlx::query_scalar("SELECT body FROM outbox WHERE to_email='terms@example.test'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(mail.contains(initial["content"].as_str().unwrap()));
    assert!(mail.contains("Verze 1"));
    let published = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/terms",
            Some(&update),
            StatusCode::OK,
        )
        .await;
    assert_eq!(published["version"], 2);
    assert_eq!(published["content"], update["content"]);
    assert_eq!(app.get(&guest, "/api/terms?version=1").await, initial);
    assert_eq!(app.get(&guest, "/api/terms").await, published);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/terms",
        Some(&update),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["id"],
        order["id"],
        "a retry must preserve its already accepted version"
    );
    let detail = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(detail["terms_version"], 1);
    commerce::initialize(&app.pool, "unchanged@example.test", false)
        .await
        .unwrap();
    assert_eq!(
        app.get(&guest, "/api/terms").await,
        published,
        "restart must not overwrite edited terms"
    );

    let next = json!({"version":2,"title":"Podmínky potřetí","content":"Aktuální text."});
    let (a, b) = tokio::join!(
        admin
            .put(format!("{}/api/admin/terms", app.url))
            .json(&next)
            .send(),
        admin
            .put(format!("{}/api/admin/terms", app.url))
            .json(&next)
            .send()
    );
    let statuses = [a.unwrap().status(), b.unwrap().status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM terms_versions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(versions, 3);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn checkout_requires_current_terms_without_stock_or_mail_side_effects() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let product = &app.get(&guest, "/api/products").await[0];
    let mut input = app
        .order(&guest, "terms-reject@example.test", product, 1)
        .await;
    input.as_object_mut().unwrap().remove("terms_version");
    let before_stock: Option<i32> = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let before_mail = app.count("outbox").await;
    let response = guest
        .post(format!("{}/api/orders", app.url))
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    for version in [0, -1, 2] {
        input["terms_version"] = json!(version);
        app.call(
            &guest,
            Method::POST,
            "/api/orders",
            Some(&input),
            if version <= 0 {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::CONFLICT
            },
        )
        .await;
    }
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("outbox").await, before_mail);
    let after_stock: Option<i32> = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(after_stock, before_stock);
    for patch in [
        json!({"version":1,"title":" ","content":"Text"}),
        json!({"version":1,"title":"Název","content":"\n\t"}),
        json!({"version":1,"title":"x".repeat(161),"content":"Text"}),
        json!({"version":1,"title":"Název","content":"ž".repeat(100_001)}),
    ] {
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/terms",
            Some(&patch),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let long_content = "ž".repeat(40_000);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/terms",
        Some(&json!({"version":1,"title":"Delší podmínky","content":long_content})),
        StatusCode::OK,
    )
    .await;
    input["terms_version"] = json!(1);
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::CONFLICT,
    )
    .await;
    input["terms_version"] = json!(2);
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["terms_version"],
        2
    );
    app.call(
        &guest,
        Method::GET,
        "/api/terms?version=0",
        None,
        StatusCode::BAD_REQUEST,
    )
    .await;
    app.call(
        &guest,
        Method::GET,
        "/api/terms?version=999",
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn order_ip_uses_socket_peer_without_proxy_trust_and_stays_private() {
    let app = TestApp::new().await;
    let customer = app.login("client-ip@example.test").await;
    let admin = app.login("admin@example.test").await;
    let product = &app.get(&customer, "/api/products").await[0];
    let input = app
        .order(&customer, "client-ip@example.test", product, 1)
        .await;
    let response = customer
        .post(format!("{}/api/orders", app.url))
        .header("x-brunch-client-ip", "198.51.100.99")
        .header("x-forwarded-for", "203.0.113.99")
        .header("x-real-ip", "203.0.113.98")
        .header("forwarded", "for=203.0.113.97")
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let order: Value = response.json().await.unwrap();
    assert!(order.get("created_from_ip").is_none());
    let id: Uuid = order["id"].as_str().unwrap().parse().unwrap();
    let saved: Option<String> =
        sqlx::query_scalar("SELECT host(created_from_ip) FROM orders WHERE id=$1")
            .bind(id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(saved.as_deref(), Some("127.0.0.1"));
    let path = format!("/api/admin/orders/{id}");
    assert_eq!(app.get(&admin, &path).await["created_from_ip"], "127.0.0.1");
    app.call(&customer, Method::GET, &path, None, StatusCode::FORBIDDEN)
        .await;
    assert!(
        app.get(&customer, "/api/orders/mine")
            .await
            .to_string()
            .find("created_from_ip")
            .is_none()
    );
    assert!(
        app.get(&admin, "/api/admin/orders")
            .await
            .to_string()
            .find("created_from_ip")
            .is_none()
    );
    assert!(
        app.post(&customer, "/api/orders", &input)
            .await
            .get("created_from_ip")
            .is_none()
    );
    sqlx::query("UPDATE orders SET created_from_ip=NULL WHERE id=$1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(app.get(&admin, &path).await["created_from_ip"].is_null());
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn trusted_proxy_order_ip_supports_ipv4_ipv6_and_preserves_original_on_replay() {
    let app = TestApp::configured(true, true).await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let product = &app.get(&guest, "/api/products").await[0];
    let mut input = app.order(&guest, "proxy-ip@example.test", product, 1).await;
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::BAD_REQUEST,
    )
    .await;
    for address in [
        "198.51.100.20, 198.51.100.21",
        "198.51.100.0/24",
        "not-an-ip",
    ] {
        let response = guest
            .post(format!("{}/api/orders", app.url))
            .header("x-brunch-client-ip", address)
            .json(&input)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(app.count("orders").await, 0);
    let mut first_id = None;
    for (provided, canonical) in [
        ("198.51.100.20", "198.51.100.20"),
        ("2001:0db8:0:0:0:0:0:42", "2001:db8::42"),
    ] {
        input["idempotency_key"] = json!(Uuid::new_v4());
        let response = guest
            .post(format!("{}/api/orders", app.url))
            .header("x-brunch-client-ip", provided)
            .header("x-forwarded-for", "203.0.113.99")
            .json(&input)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let order: Value = response.json().await.unwrap();
        assert!(order.get("created_from_ip").is_none());
        let id: Uuid = order["id"].as_str().unwrap().parse().unwrap();
        first_id.get_or_insert(id);
        let saved: Option<String> =
            sqlx::query_scalar("SELECT host(created_from_ip) FROM orders WHERE id=$1")
                .bind(id)
                .fetch_one(&app.pool)
                .await
                .unwrap();
        assert_eq!(saved.as_deref(), Some(canonical));
        assert_eq!(
            app.get(&admin, &format!("/api/admin/orders/{id}")).await["created_from_ip"],
            canonical
        );
        let replay: Value = guest
            .post(format!("{}/api/orders", app.url))
            .header("x-brunch-client-ip", "203.0.113.77")
            .json(&input)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(replay["id"], order["id"]);
        assert!(replay.get("created_from_ip").is_none());
        // A retry of an already saved request does not depend on today's proxy configuration.
        assert_eq!(
            app.post(&guest, "/api/orders", &input).await["id"],
            order["id"]
        );
        let saved: Option<String> =
            sqlx::query_scalar("SELECT host(created_from_ip) FROM orders WHERE id=$1")
                .bind(id)
                .fetch_one(&app.pool)
                .await
                .unwrap();
        assert_eq!(saved.as_deref(), Some(canonical));
        let leaked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM outbox WHERE body LIKE '%' || $1 || '%' OR subject LIKE '%' || $1 || '%')")
            .bind(canonical).fetch_one(&app.pool).await.unwrap();
        assert!(!leaked, "IP must not be copied into customer or shop mail");
    }
    let mut supplier = app.get(&admin, "/api/admin/invoice-supplier").await;
    supplier["name"] = json!("Testovací dodavatel");
    supplier["address"] = json!("Praha 1");
    supplier["company_id"] = json!("12345678");
    let supplier = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/invoice-supplier",
            Some(&supplier),
            StatusCode::OK,
        )
        .await;
    let invoice = app.post(&admin, &format!("/api/admin/orders/{}/invoice", first_id.unwrap()), &json!({
        "supplier_version":supplier["version"], "customer_name":"Eva", "customer_address":"Praha",
        "customer_company_id":"", "customer_tax_id":"", "supply_date":input["delivery_date"],
        "due_date":input["delivery_date"], "vat_rates":{}
    })).await;
    assert!(!invoice.to_string().contains("198.51.100.20"));
    assert!(!invoice.to_string().contains("created_from_ip"));
    assert_eq!(app.count("orders").await, 2);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn upgrade_preserves_existing_products_photos_orders_and_settings() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let database = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .unwrap();
    let schema = format!("brunch_upgrade_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&database)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    let mut legacy = sqlx::migrate!();
    legacy.migrations = legacy
        .iter()
        .filter(|migration| migration.version <= 11)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    legacy.run(&pool).await.unwrap();
    sqlx::raw_sql(r#"
        INSERT INTO users(id,email) VALUES ('00000000-0000-0000-0000-000000000001','staff@example.test');
        INSERT INTO images(id,content_type,data,created_by) VALUES
            ('00000000-0000-0000-0000-000000000002','image/jpeg',decode('ffd8ffd9','hex'),'00000000-0000-0000-0000-000000000001');
        INSERT INTO products(name,category,price_cents,image,stock) VALUES
            ('Původní zboží','Pečivo',10450,'/api/images/00000000-0000-0000-0000-000000000002',31),
            ('Další pečivo','Pečivo',9990,'',NULL),('Bez kategorie','',5500,'',NULL);
        INSERT INTO product_prices(product_id,price_cents) VALUES (1,10450),(2,9990),(3,5500);
        INSERT INTO shop_settings(id,delivery_fee_cents,order_email) VALUES (1,2600,'staff@example.test');
        INSERT INTO delivery_areas(postal_code,label) VALUES ('11000','Praha');
        INSERT INTO orders(id,idempotency_key,request_payload,email,street,city,postal_code,delivery_date,delivery_slot,payment_method,status,subtotal_cents,delivery_fee_cents,total_cents) VALUES
            ('00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000004','{}','guest@example.test','Dlouhá','Praha','11000','2026-11-24','07:00–09:00','card_on_delivery','preparing',10450,2600,13050);
        INSERT INTO order_items(order_id,product_id,name,quantity,unit_price_cents,total_cents,tracked_stock) VALUES
            ('00000000-0000-0000-0000-000000000003',1,'Původní zboží',1,10450,10450,true);
        INSERT INTO order_history(order_id,status,payment_status) VALUES
            ('00000000-0000-0000-0000-000000000003','preparing','unpaid');
    "#).execute(&pool).await.unwrap();

    // Compare every existing field, including image bytes and historical prices.
    let comparisons = [
        ("products", " - 'minimum_quantity' - 'variants'"),
        ("images", ""),
        ("product_prices", ""),
        ("order_items", " - 'variant'"),
        (
            "shop_settings",
            " - 'delivery_slots' - 'additional_order_emails'",
        ),
        ("delivery_areas", " - 'delivery_slots'"),
        (
            "orders",
            " - 'status' - 'version' - 'updated_at' - 'created_from_ip' - 'terms_version' - 'user_id'",
        ),
    ];
    let mut before = Vec::new();
    for (table, exclusions) in comparisons {
        let query =
            format!("SELECT jsonb_agg(to_jsonb(p){exclusions} ORDER BY p.id) FROM {table} p");
        before.push(
            sqlx::query_scalar::<_, Value>(&query)
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
    }
    migrate(&pool).await.unwrap();
    commerce::initialize(&pool, "other@example.test", false)
        .await
        .unwrap();
    // A repeated application start must also preserve migrated data and history.
    migrate(&pool).await.unwrap();
    for ((table, exclusions), expected) in comparisons.into_iter().zip(before) {
        let query =
            format!("SELECT jsonb_agg(to_jsonb(p){exclusions} ORDER BY p.id) FROM {table} p");
        let actual: Value = sqlx::query_scalar(&query).fetch_one(&pool).await.unwrap();
        assert_eq!(actual, expected, "upgrade changed existing {table} fields");
    }
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT bool_and(cardinality(variants)=0) FROM products")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT bool_and(variant IS NULL) FROM order_items")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    let state: (String, i64) = sqlx::query_as("SELECT status,version FROM orders")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, ("confirmed".into(), 2));
    let history: Vec<String> = sqlx::query_scalar("SELECT status FROM order_history ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(history, ["preparing", "confirmed"]);
    let categories: Vec<String> = sqlx::query_scalar("SELECT name FROM categories ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(categories, ["", "Pečivo"]);
    let defaults: (bool, bool, bool) = sqlx::query_as("SELECT (SELECT bool_and(minimum_quantity=0) FROM products), (SELECT cardinality(delivery_slots)=3 FROM shop_settings), (SELECT delivery_slots IS NULL FROM delivery_areas)").fetch_one(&pool).await.unwrap();
    assert_eq!(defaults, (true, true, true));
    let site_defaults: (i32, String, i64) =
        sqlx::query_as("SELECT id,hero_image,version FROM site_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(site_defaults, (1, String::new(), 1));
    let recipients: Vec<String> =
        sqlx::query_scalar("SELECT additional_order_emails FROM shop_settings WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        recipients.is_empty(),
        "upgrading keeps the existing primary recipient only"
    );
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&database)
        .await
        .unwrap();
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to local PostgreSQL"]
async fn order_account_migration_backfills_emails_and_preserves_existing_identity_and_history() {
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let database = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .unwrap();
    let schema = format!("brunch_account_upgrade_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&database)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    let mut legacy = sqlx::migrate!();
    legacy.migrations = legacy
        .iter()
        .filter(|migration| migration.version <= 19)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    legacy.run(&pool).await.unwrap();
    sqlx::raw_sql(r#"
        INSERT INTO users(id,email,is_admin,admin_password_hash) VALUES
            ('00000000-0000-0000-0000-000000000001','admin@example.test',true,'preserved-password-hash'),
            ('00000000-0000-0000-0000-000000000002','existing@example.test',false,NULL);
        INSERT INTO sessions(token_hash,user_id,expires_at,admin_authenticated) VALUES
            ('preserved-session','00000000-0000-0000-0000-000000000001',now()+interval '1 hour',true);
        INSERT INTO orders(id,idempotency_key,request_payload,user_id,email,street,city,postal_code,delivery_date,delivery_slot,payment_method,subtotal_cents,delivery_fee_cents,total_cents)
        SELECT gen_random_uuid(),gen_random_uuid(),jsonb_build_object('email',email),user_id,email,'Test 1','Praha','11000','2026-12-01','07:00–09:00','cash_on_delivery',0,0,0
        FROM (VALUES
            ('New@Example.Test',NULL::uuid),
            (' new@example.test ',NULL::uuid),
            ('existing@example.test',NULL::uuid),
            ('existing@example.test','00000000-0000-0000-0000-000000000002'::uuid),
            ('admin@example.test',NULL::uuid),
            ('new@example.test','00000000-0000-0000-0000-000000000002'::uuid)
        ) AS legacy_orders(email,user_id);
        INSERT INTO order_history(order_id,status,payment_status,changed_by)
        SELECT id,status,payment_status,user_id FROM orders;
    "#).execute(&pool).await.unwrap();
    let snapshot_sql = "SELECT jsonb_build_object(
        'orders',(SELECT jsonb_agg(to_jsonb(o)-'user_id' ORDER BY id) FROM orders o),
        'history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY id) FROM order_history h),
        'existing_users',(SELECT jsonb_agg(to_jsonb(u) ORDER BY id) FROM users u WHERE email<>'new@example.test'),
        'sessions',(SELECT jsonb_agg(to_jsonb(s) ORDER BY token_hash) FROM sessions s))";
    let before: Value = sqlx::query_scalar(snapshot_sql)
        .fetch_one(&pool)
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    migrate(&pool).await.unwrap();
    let after: Value = sqlx::query_scalar(snapshot_sql)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "backfill must only change order ownership and add missing customers"
    );
    let owners: Vec<(String, i64)> = sqlx::query_as(
        "SELECT u.email,count(*) FROM orders o JOIN users u ON u.id=o.user_id GROUP BY u.email ORDER BY u.email",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        owners,
        [
            ("admin@example.test".into(), 1),
            ("existing@example.test".into(), 2),
            ("new@example.test".into(), 3)
        ]
    );
    let customer: (bool, Option<String>) = sqlx::query_as(
        "SELECT is_admin,admin_password_hash FROM users WHERE email='new@example.test'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(customer, (false, None));
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&database)
        .await
        .unwrap();
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to local PostgreSQL"]
async fn production_initialization_creates_settings_without_demo_and_preserves_existing_data() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .unwrap();
    let schema = format!("brunch_init_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&database)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&database_url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    commerce::initialize(&pool, "orders@brunch-ci.cz", false)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM products")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let settings: (i64, Option<i64>, String) = sqlx::query_as(
        "SELECT delivery_fee_cents,free_delivery_from_cents,order_email FROM shop_settings WHERE id=1"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(settings, (4900, Some(35000), "orders@brunch-ci.cz".into()));
    sqlx::query("INSERT INTO products(name,price_cents) VALUES ('Skutečné zboží',15000)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE shop_settings SET delivery_fee_cents=5900,order_email='kitchen@brunch-ci.cz',additional_order_emails=ARRAY['dispatch@brunch-ci.cz','owner@brunch-ci.cz'] WHERE id=1").execute(&pool).await.unwrap();
    commerce::initialize(&pool, "new-default@brunch-ci.cz", false)
        .await
        .unwrap();
    let products: Vec<(String, i64)> = sqlx::query_as("SELECT name,price_cents FROM products")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(products, vec![("Skutečné zboží".into(), 15000)]);
    let settings: (i64, String) =
        sqlx::query_as("SELECT delivery_fee_cents,order_email FROM shop_settings WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(settings, (5900, "kitchen@brunch-ci.cz".into()));
    let recipients: Vec<String> =
        sqlx::query_scalar("SELECT additional_order_emails FROM shop_settings WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recipients, ["dispatch@brunch-ci.cz", "owner@brunch-ci.cz"]);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&database)
        .await
        .unwrap();
    database.close().await;
}

fn product_input(value: &Value) -> Value {
    let mut value = value.clone();
    value.as_object_mut().unwrap().remove("id");
    value
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn broadcast_uses_distinct_order_addresses_and_retries_atomically() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let non_buyer = app.login("account-only@example.test").await;
    let route = "/api/admin/mail/broadcast";
    let payload = json!({"idempotency_key":Uuid::new_v4(),"subject":"Nové rozvozové hodiny","body":"V pondělí rozvážíme od osmi."});
    app.call(
        &non_buyer,
        Method::POST,
        route,
        Some(&payload),
        StatusCode::FORBIDDEN,
    )
    .await;
    app.call(&guest, Method::GET, route, None, StatusCode::UNAUTHORIZED)
        .await;
    assert_eq!(app.get(&admin, route).await["recipient_count"], 0);
    app.call(
        &admin,
        Method::POST,
        route,
        Some(&payload),
        StatusCode::BAD_REQUEST,
    )
    .await;

    let products = app.get(&guest, "/api/products").await;
    let mut order = app
        .order(&guest, "guest@example.test", &products[0], 1)
        .await;
    order["name"] = json!("Anna Testerová");
    order["phone"] = json!("+420 777 123 456");
    for email in [
        "guest@example.test",
        "Guest@Example.Test",
        "other@example.test",
    ] {
        order["idempotency_key"] = json!(Uuid::new_v4());
        order["email"] = json!(email);
        app.post(&guest, "/api/orders", &order).await;
    }
    assert_eq!(app.get(&admin, route).await["recipient_count"], 2);
    let (first, concurrent) = tokio::join!(
        app.post(&admin, route, &payload),
        app.post(&admin, route, &payload)
    );
    assert_eq!(first, json!({"recipient_count":2}));
    assert_eq!(concurrent, first);
    order["idempotency_key"] = json!(Uuid::new_v4());
    order["email"] = json!("later@example.test");
    app.post(&guest, "/api/orders", &order).await;
    assert_eq!(app.post(&admin, route, &payload).await, first);
    let mut changed = payload.clone();
    changed["body"] = json!("Jiná zpráva");
    app.call(
        &admin,
        Method::POST,
        route,
        Some(&changed),
        StatusCode::CONFLICT,
    )
    .await;
    let recipients: Vec<String> = sqlx::query_scalar(
        "SELECT to_email FROM outbox WHERE broadcast_id IS NOT NULL AND status='pending' ORDER BY to_email",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(recipients, ["guest@example.test", "other@example.test"]);
    app.close().await;
}

impl TestApp {
    async fn new() -> Self {
        Self::with_delivery_area(true).await
    }
    async fn with_delivery_area(allow_delivery: bool) -> Self {
        Self::configured(allow_delivery, false).await
    }
    async fn configured(allow_delivery: bool, trust_caddy_client_ip: bool) -> Self {
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
        auth::initialize_admin(&pool, "admin@example.test", "Test-only-admin-password-2026")
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}");
        let config = Config {
            app_env: brunch_server::config::AppEnvironment::Development,
            trust_caddy_client_ip,
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
            let mut body = body.clone();
            if method == Method::POST && path == "/api/orders" {
                body.as_object_mut()
                    .unwrap()
                    .entry("terms_version")
                    .or_insert(json!(1));
            }
            request = request.json(&body);
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
        let body: String = sqlx::query_scalar("SELECT body FROM outbox WHERE to_email=$1 AND subject='Přihlášení do Brunch Era' ORDER BY created_at DESC LIMIT 1")
            .bind(email.to_lowercase()).fetch_one(&self.pool).await.unwrap();
        assert!(body.ends_with("Brunch Era"));
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
        if email == "admin@example.test" {
            self.post(
                &client,
                "/api/admin/auth/login",
                &json!({"email": email, "password": "Test-only-admin-password-2026"}),
            )
            .await;
            return client;
        }
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
            "terms_version":1,
            "idempotency_key":Uuid::new_v4(),"email":email,"name":"Anna Testerová","phone":"+420 777 123 456","street":"Testovací 12",
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
async fn additional_order_recipients_validate_privacy_preservation_and_conflicts() {
    let app = TestApp::new().await;
    let guest = app.client();
    let customer = app.login("recipient-customer@example.test").await;
    let admin = app.login("admin@example.test").await;
    let initial = app.get(&admin, "/api/admin/settings").await;
    assert_eq!(initial["additional_order_emails"], json!([]));
    let mut update = initial.clone();
    update["order_email"] = json!("  ORDERS@Example.Test ");
    update["additional_order_emails"] = json!([
        " Chef@Example.Test ",
        "dispatch@example.test",
        "CHEF@example.test",
        " orders@example.test "
    ]);
    for (client, expected) in [
        (&guest, StatusCode::UNAUTHORIZED),
        (&customer, StatusCode::FORBIDDEN),
    ] {
        app.call(client, Method::GET, "/api/admin/settings", None, expected)
            .await;
        app.call(
            client,
            Method::PUT,
            "/api/admin/settings",
            Some(&update),
            expected,
        )
        .await;
    }
    assert_eq!(app.get(&admin, "/api/admin/settings").await, initial);
    let mut saved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&update),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["order_email"], "orders@example.test");
    assert_eq!(
        saved["additional_order_emails"],
        json!(["chef@example.test", "dispatch@example.test"])
    );
    let public = app.get(&guest, "/api/settings").await;
    for private_field in ["order_email", "additional_order_emails"] {
        assert!(
            public.get(private_field).is_none(),
            "{private_field} is private"
        );
    }
    for invalid in [
        json!(["not-an-email"]),
        json!([""]),
        json!(["ok@example.test\r\nBcc: hidden@example.test"]),
        json!(
            (0..20)
                .map(|i| format!("staff{i}@example.test"))
                .collect::<Vec<_>>()
        ),
    ] {
        let mut rejected = saved.clone();
        rejected["additional_order_emails"] = invalid;
        app.call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&rejected),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(
            app.get(&admin, "/api/admin/settings").await,
            saved,
            "invalid recipients cannot partially update settings or version"
        );
    }
    // Count unique addresses after normalization, including the primary recipient.
    let maximum: Vec<String> = (0..19).map(|i| format!("staff{i}@example.test")).collect();
    let mut duplicate_input = maximum.clone();
    duplicate_input.extend([
        " STAFF0@EXAMPLE.TEST ".into(),
        " ORDERS@EXAMPLE.TEST ".into(),
    ]);
    update = saved.clone();
    update["additional_order_emails"] = json!(duplicate_input);
    saved = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&update),
            StatusCode::OK,
        )
        .await;
    assert_eq!(saved["additional_order_emails"], json!(maximum));
    for omitted in [true, false] {
        update = saved.clone();
        if omitted {
            update
                .as_object_mut()
                .unwrap()
                .remove("additional_order_emails");
        } else {
            update["additional_order_emails"] = Value::Null;
        }
        update["delivery_fee_cents"] = json!(saved["delivery_fee_cents"].as_i64().unwrap() + 100);
        saved = app
            .call(
                &admin,
                Method::PUT,
                "/api/admin/settings",
                Some(&update),
                StatusCode::OK,
            )
            .await;
        assert_eq!(
            saved["additional_order_emails"],
            json!(maximum),
            "older clients preserve the list"
        );
    }
    let mut a = saved.clone();
    let mut b = saved.clone();
    a["additional_order_emails"] = json!(["first@example.test"]);
    b["additional_order_emails"] = json!(["second@example.test"]);
    let (a_response, b_response) = tokio::join!(
        admin
            .put(format!("{}/api/admin/settings", app.url))
            .json(&a)
            .send(),
        admin
            .put(format!("{}/api/admin/settings", app.url))
            .json(&b)
            .send()
    );
    let a_status = a_response.unwrap().status();
    let b_status = b_response.unwrap().status();
    assert!(
        (a_status == StatusCode::OK && b_status == StatusCode::CONFLICT)
            || (b_status == StatusCode::OK && a_status == StatusCode::CONFLICT)
    );
    saved = app.get(&admin, "/api/admin/settings").await;
    assert_eq!(
        saved["additional_order_emails"],
        if a_status == StatusCode::OK {
            a["additional_order_emails"].clone()
        } else {
            b["additional_order_emails"].clone()
        }
    );
    assert_eq!(
        saved["version"].as_i64().unwrap(),
        a["version"].as_i64().unwrap() + 1
    );
    update = saved;
    update["additional_order_emails"] = json!([]);
    let cleared = app
        .call(
            &admin,
            Method::PUT,
            "/api/admin/settings",
            Some(&update),
            StatusCode::OK,
        )
        .await;
    assert_eq!(cleared["additional_order_emails"], json!([]));
    assert_eq!(cleared["order_email"], "orders@example.test");
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn multiple_staff_order_mails_are_atomic_private_and_idempotent() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let mut settings = app.get(&admin, "/api/admin/settings").await;
    settings["additional_order_emails"] = json!([
        "chef@example.test",
        "dispatch@example.test",
        "CHEF@example.test",
        "orders@example.test"
    ]);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&settings),
        StatusCode::OK,
    )
    .await;
    sqlx::query("UPDATE products SET stock=5")
        .execute(&app.pool)
        .await
        .unwrap();
    let products = app.get(&guest, "/api/products").await;
    let input = app
        .order(&guest, "guest@example.test", &products[0], 1)
        .await;
    sqlx::query("CREATE FUNCTION fail_extra_recipient() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.to_email='dispatch@example.test' THEN RAISE EXCEPTION 'Injected extra recipient failure'; END IF; RETURN NEW; END $$").execute(&app.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_extra_recipient BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION fail_extra_recipient()").execute(&app.pool).await.unwrap();
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
    assert_eq!(app.count("outbox").await, 0);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(products[0]["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 5, "extra recipient failure must roll back stock too");
    sqlx::query("DROP TRIGGER fail_extra_recipient ON outbox")
        .execute(&app.pool)
        .await
        .unwrap();
    let (first, duplicate) = tokio::join!(
        app.post(&guest, "/api/orders", &input),
        app.post(&guest, "/api/orders", &input)
    );
    assert_eq!(first["id"], duplicate["id"]);
    assert_eq!(app.count("orders").await, 1);
    let mail: Vec<(String, String, String)> =
        sqlx::query_as("SELECT to_email,subject,body FROM outbox ORDER BY to_email")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        mail.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        [
            "chef@example.test",
            "dispatch@example.test",
            "guest@example.test",
            "orders@example.test"
        ]
    );
    for (_, _, body) in &mail {
        assert!(body.contains(first["number"].as_str().unwrap()));
        assert!(body.contains("Testovací 12"));
    }
    let customer_mail = mail
        .iter()
        .find(|row| row.0 == "guest@example.test")
        .unwrap();
    assert!(customer_mail.2.contains("orders@example.test"));
    assert!(!customer_mail.2.contains("chef@example.test"));
    assert!(!customer_mail.2.contains("dispatch@example.test"));
    let recipients: Vec<String> = mail
        .iter()
        .filter(|row| row.0 != "guest@example.test")
        .map(|row| row.1.clone())
        .collect();
    assert!(
        recipients.windows(2).all(|pair| pair[0] == pair[1]),
        "staff receive separate copies of the same notice"
    );
    app.post(&guest, "/api/orders", &input).await;
    assert_eq!(
        app.count("outbox").await,
        4,
        "replay cannot queue additional copies"
    );
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(products[0]["id"].as_i64().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 4);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn guest_checkout_totals_transactional_mail_idempotency_and_price_snapshot() {
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let product = &products[0];
    let input = app
        .order(&guest, " \tGUEST@Example.Test \n", product, 2)
        .await;
    let created = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(created["total_cents"], input["expected_total_cents"]);
    assert_eq!(created["payment_method"], "card_on_delivery");
    assert_eq!(created["payment_status"], "unpaid");
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("outbox").await, 2);
    assert_eq!(
        app.count("users").await,
        2,
        "the first order creates one customer account"
    );
    let customer: (Uuid, bool, Option<String>) = sqlx::query_as(
        "SELECT id,is_admin,admin_password_hash FROM users WHERE email='guest@example.test'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!customer.1);
    assert_eq!(customer.2, None);
    let ownership: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT o.user_id,h.changed_by FROM orders o JOIN order_history h ON h.order_id=o.id",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(ownership, (customer.0, None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions")
            .fetch_one(&app.pool)
            .await
            .unwrap(),
        0,
        "checkout must not authenticate the browser"
    );
    app.call(
        &guest,
        Method::GET,
        "/api/auth/me",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.call(
        &guest,
        Method::GET,
        "/api/orders/mine",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
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
    for body in &bodies {
        for expected in [
            format!(
                "Doručení: {} {}",
                input["delivery_date"].as_str().unwrap(),
                input["delivery_slot"].as_str().unwrap()
            ),
            format!("2 × {}", product["name"].as_str().unwrap()),
            "Celkem k úhradě při doručení: 287,00 Kč".into(),
            "Platba: Kartou při doručení".into(),
            "Adresa: Testovací 12, 11000 Praha".into(),
            "3. patro, výtah vlevo. Zvoňte Novák.".into(),
        ] {
            assert!(
                body.contains(&expected),
                "order email is missing {expected}: {body}"
            );
        }
    }
    let replay = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(replay["id"], created["id"]);
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("users").await, 2);
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
async fn admin_filters_delivery_day_with_search_status_and_stable_pagination() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let products = app.get(&guest, "/api/products").await;
    let options = app.get(&guest, "/api/delivery-options").await;
    let day = options["dates"][0]["value"].as_str().unwrap();
    let other_day = options["dates"][1]["value"].as_str().unwrap();
    let template = app
        .order(&guest, "daily@example.test", &products[0], 1)
        .await;
    let mut accepted_ids = Vec::new();
    let mut cancelled_id = Value::Null;
    for index in 0..54 {
        let mut input = template.clone();
        input["idempotency_key"] = json!(Uuid::new_v4());
        input["email"] = json!(format!("daily-{index}@example.test"));
        if index == 53 {
            input["delivery_date"] = json!(other_day);
        }
        let order = app.post(&guest, "/api/orders", &input).await;
        if index < 52 {
            accepted_ids.push(order["id"].as_str().unwrap().to_owned());
        } else if index == 52 {
            cancelled_id = order["id"].clone();
            app.call(&admin, Method::PATCH, &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
                Some(&json!({"status":"cancelled","payment_status":"unpaid","version":order["version"]})), StatusCode::OK).await;
        }
    }
    // Creation time must not drive the day filter. Equal timestamps also test the ID tiebreaker.
    sqlx::query("UPDATE orders SET created_at='2000-01-01T00:00:00Z'")
        .execute(&app.pool)
        .await
        .unwrap();
    let filter = format!("/api/admin/orders?delivery_date={day}&status=new&q=daily-");
    let first = app.get(&admin, &format!("{filter}&page=1")).await;
    let second = app.get(&admin, &format!("{filter}&page=2")).await;
    assert_eq!(first["total"], 52);
    assert_eq!(second["total"], 52);
    assert_eq!(first["orders"].as_array().unwrap().len(), 50);
    assert_eq!(second["orders"].as_array().unwrap().len(), 2);
    let found: Vec<String> = first["orders"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["orders"].as_array().unwrap())
        .map(|order| {
            assert_eq!(order["delivery_date"], day);
            assert_eq!(order["status"], "new");
            assert_eq!(order["items"].as_array().unwrap().len(), 1);
            order["id"].as_str().unwrap().to_owned()
        })
        .collect();
    accepted_ids.sort_by(|a, b| b.cmp(a));
    assert_eq!(found, accepted_ids);
    assert_eq!(app.get(&admin, &filter).await, first);
    let cancelled = app
        .get(
            &admin,
            &format!("/api/admin/orders?delivery_date={day}&status=cancelled"),
        )
        .await;
    assert_eq!(cancelled["total"], 1);
    assert_eq!(cancelled["orders"][0]["id"], cancelled_id);
    assert_eq!(
        app.get(&admin, &format!("/api/admin/orders?delivery_date={day}"))
            .await["total"],
        53
    );
    assert_eq!(
        app.get(
            &admin,
            &format!("/api/admin/orders?delivery_date={other_day}")
        )
        .await["total"],
        1
    );
    assert_eq!(
        app.get(
            &admin,
            &format!("/api/admin/orders?delivery_date={day}&q=daily-0%40example.test")
        )
        .await["total"],
        1
    );
    assert_eq!(
        app.get(&admin, "/api/admin/orders?delivery_date=2000-01-01")
            .await["total"],
        0
    );
    let all = app.get(&admin, "/api/admin/orders").await;
    assert_eq!(all["total"], 54);
    assert_eq!(
        app.get(&admin, "/api/admin/orders?delivery_date=").await,
        all
    );
    assert!(
        app.get(&admin, &format!("{filter}&page=3")).await["orders"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn delivery_day_filter_rejects_invalid_dates_and_requires_admin() {
    let app = TestApp::new().await;
    let anonymous = app.client();
    let customer = app.login("customer@example.test").await;
    let admin = app.login("admin@example.test").await;
    let path = "/api/admin/orders?delivery_date=2026-10-26";
    app.call(
        &anonymous,
        Method::GET,
        path,
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.call(&customer, Method::GET, path, None, StatusCode::FORBIDDEN)
        .await;
    for value in [
        "not-a-date",
        "2026-02-30",
        "2026-13-01",
        "2026-1-1",
        "26.10.2026",
        "2026-10-26T00:00:00Z",
        "0000-01-01",
        "+10000-01-01",
        "2026-10-26' OR true--",
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("delivery_date", value)
            .finish();
        app.call(
            &admin,
            Method::GET,
            &format!("/api/admin/orders?{query}"),
            None,
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert_eq!(
        app.get(&admin, "/api/admin/orders?delivery_date=2028-02-29")
            .await["total"],
        0
    );
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
async fn guest_checkout_preserves_administrator_account_without_granting_access() {
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let before: Value =
        sqlx::query_scalar("SELECT to_jsonb(u) FROM users u WHERE email='admin@example.test'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let input = app
        .order(&guest, " ADMIN@Example.Test ", &products[0], 1)
        .await;
    let order = app.post(&guest, "/api/orders", &input).await;
    let after: Value =
        sqlx::query_scalar("SELECT to_jsonb(u) FROM users u WHERE email='admin@example.test'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(after, before);
    assert_eq!(app.count("users").await, 1);
    app.call(
        &guest,
        Method::GET,
        "/api/orders/mine",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.call(
        &guest,
        Method::GET,
        "/api/admin/orders",
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let token = app.token(&guest, "admin@example.test").await;
    app.post(&guest, "/api/auth/verify", &json!({"token":token}))
        .await;
    assert_eq!(
        app.get(&guest, "/api/orders/mine").await[0]["id"],
        order["id"]
    );
    app.call(
        &guest,
        Method::GET,
        "/api/admin/orders",
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
    let admin = app.login("admin@example.test").await;
    assert_eq!(app.get(&admin, "/api/admin/orders").await["total"], 1);
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
    let mut repeated_input = guest_input.clone();
    repeated_input["idempotency_key"] = json!(Uuid::new_v4());
    repeated_input["email"] = json!(" ALICE@Example.Test ");
    let (first, second) = tokio::join!(
        app.post(&guest, "/api/orders", &guest_input),
        app.post(&guest, "/api/orders", &repeated_input)
    );
    assert_ne!(first["id"], second["id"]);
    assert_eq!(app.count("users").await, 2);
    let alice = app.login("alice@example.test").await;
    let bob = app.login("bob@example.test").await;
    assert_eq!(app.count("users").await, 3);
    let history = app.get(&alice, "/api/orders/mine").await;
    assert_eq!(
        history.as_array().unwrap().len(),
        2,
        "verified email ownership reveals all earlier orders for that email"
    );
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|order| order["id"] == first["id"])
    );
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|order| order["id"] == second["id"])
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
        3
    );
    assert_eq!(app.get(&bob, "/api/orders/mine").await, json!([]));
    let other_email = app.order(&alice, "bob@example.test", &products[0], 1).await;
    let other_order = app.post(&alice, "/api/orders", &other_email).await;
    let bob_history = app.get(&bob, "/api/orders/mine").await;
    assert_eq!(
        bob_history.as_array().unwrap().len(),
        1,
        "an order belongs to its email account, not the current browser session"
    );
    assert_eq!(bob_history[0]["id"], other_order["id"]);
    let owner_and_actor: (String, String) = sqlx::query_as(
        "SELECT owner.email,actor.email FROM orders o JOIN users owner ON owner.id=o.user_id JOIN order_history h ON h.order_id=o.id JOIN users actor ON actor.id=h.changed_by WHERE o.id=$1",
    )
    .bind(Uuid::parse_str(other_order["id"].as_str().unwrap()).unwrap())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        owner_and_actor,
        ("bob@example.test".into(), "alice@example.test".into())
    );
    assert_eq!(
        app.get(&alice, "/api/orders/mine")
            .await
            .as_array()
            .unwrap()
            .len(),
        3
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
async fn minimum_order_quantity_and_category_removal_preserve_catalog_and_orders() {
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let guest = app.client();
    let categories = "/api/admin/categories";
    app.call(
        &guest,
        Method::GET,
        categories,
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let category = app
        .post(&admin, categories, &json!({"name":"Snídaňové boxy"}))
        .await;
    let category_path = format!("{categories}/{}", category["id"]);
    let product = app
        .post(
            &admin,
            "/api/admin/products",
            &json!({
                "name":"Mini koláč", "category":"Snídaňové boxy", "stock":12, "minimum_quantity":6,
                "price_cents":2200, "image":"/images/mini.jpg"
            }),
        )
        .await;
    let product_path = format!("/api/admin/products/{}", product["id"]);
    let mut insufficient = app
        .order(&guest, "customer@example.test", &product, 5)
        .await;
    insufficient["name"] = json!("Jana Nová");
    insufficient["phone"] = json!("+420 777 123 456");
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&insufficient),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.count("orders").await, 0);
    let mut exact = app
        .order(&guest, "customer@example.test", &product, 6)
        .await;
    exact["name"] = insufficient["name"].clone();
    exact["phone"] = insufficient["phone"].clone();
    let order = app.post(&guest, "/api/orders", &exact).await;

    app.call(
        &admin,
        Method::PUT,
        &category_path,
        Some(&json!({"name":"Krabičky","version":1})),
        StatusCode::OK,
    )
    .await;
    let changed = app.get(&guest, "/api/products").await;
    let updated = changed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == product["id"])
        .unwrap();
    assert_eq!(updated["category"], "Krabičky");
    app.call(
        &admin,
        Method::DELETE,
        &category_path,
        Some(&json!({"version":1})),
        StatusCode::CONFLICT,
    )
    .await;
    app.call(
        &admin,
        Method::DELETE,
        &category_path,
        Some(&json!({"version":2})),
        StatusCode::OK,
    )
    .await;
    app.call(
        &admin,
        Method::PUT,
        &product_path,
        Some(&product_input(updated)),
        StatusCode::CONFLICT,
    )
    .await;
    let catalog = app.get(&guest, "/api/products").await;
    let retained = catalog
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == product["id"])
        .unwrap();
    assert_eq!(retained["category"], "");
    assert_eq!(retained["stock"], 6);
    for field in [
        "id",
        "name",
        "image",
        "price_cents",
        "minimum_quantity",
        "active",
    ] {
        assert_eq!(retained[field], product[field]);
    }
    let remaining = app.get(&admin, categories).await;
    assert!(
        !remaining
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "Krabičky")
    );
    let receipt = app
        .get(
            &admin,
            &format!("/api/admin/orders/{}", order["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(receipt["items"][0]["quantity"], 6);
    assert_eq!(receipt["items"][0]["product_id"], product["id"]);
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
        sqlx::query_scalar("SELECT to_email FROM outbox WHERE subject<>'Přihlášení do Brunch Era'")
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
    let mail_id: Uuid = sqlx::query_scalar("UPDATE outbox SET status='failed',attempts=8,last_error='Test SMTP failure' WHERE id=(SELECT id FROM outbox WHERE subject<>'Přihlášení do Brunch Era' LIMIT 1) RETURNING id").fetch_one(&app.pool).await.unwrap();
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
    for status in ["confirmed", "out_for_delivery", "delivered"] {
        if status == "confirmed" {
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
    assert_eq!(detail["history"].as_array().unwrap().len(), 4);
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
    for status in ["confirmed", "out_for_delivery"] {
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
        ("name", json!("  ")),
        ("phone", json!("")),
        ("phone", json!("chybí telefon")),
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
    assert_eq!(app.count("users").await, 1);
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
    let before_users = app.count("users").await;
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
    assert_eq!(app.count("users").await, before_users);
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
    assert_eq!(app.count("users").await, before_users + 1);
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
async fn new_orders_reject_past_today_and_weekends_but_replay_historical_success() {
    use chrono::{Datelike, Days, Utc, Weekday};
    let app = TestApp::new().await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let product = &products[0];
    let product_id = product["id"].as_i64().unwrap();
    sqlx::query("UPDATE products SET stock=5 WHERE id=$1")
        .bind(product_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let input = app.order(&guest, "dates@example.test", product, 1).await;
    let today = Utc::now()
        .with_timezone(&chrono_tz::Europe::Prague)
        .date_naive();
    let yesterday = today.pred_opt().unwrap();
    let saturday = (1..=7)
        .map(|days| today.checked_add_days(Days::new(days)).unwrap())
        .find(|day| day.weekday() == Weekday::Sat)
        .unwrap();
    let baseline_mail = app.count("outbox").await;
    // Direct POSTs exercise server validation independently of all browser controls.
    for date in [yesterday, today, saturday, saturday.succ_opt().unwrap()] {
        let mut invalid = input.clone();
        invalid["delivery_date"] = json!(date);
        app.call(
            &guest,
            Method::POST,
            "/api/orders",
            Some(&invalid),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert_eq!(app.count("orders").await, 0);
    assert_eq!(app.count("order_items").await, 0);
    assert_eq!(app.count("outbox").await, baseline_mail);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 5);

    let order = app.post(&guest, "/api/orders", &input).await;
    let id: Uuid = order["id"].as_str().unwrap().parse().unwrap();
    let mut historical: Value =
        sqlx::query_scalar("SELECT request_payload FROM orders WHERE id=$1")
            .bind(id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    // Model a previously accepted order whose delivery day has since passed.
    historical["delivery_date"] = json!(yesterday);
    sqlx::query("UPDATE orders SET delivery_date=$2,request_payload=$3 WHERE id=$1")
        .bind(id)
        .bind(yesterday)
        .bind(&historical)
        .execute(&app.pool)
        .await
        .unwrap();
    let replayed = app.post(&guest, "/api/orders", &historical).await;
    assert_eq!(replayed["id"], order["id"]);
    assert_eq!(replayed["delivery_date"], json!(yesterday));
    historical["idempotency_key"] = json!(Uuid::new_v4());
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&historical),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(app.count("orders").await, 1);
    assert_eq!(app.count("order_items").await, 1);
    assert_eq!(app.count("outbox").await, baseline_mail + 2);
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE id=$1")
        .bind(product_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stock, 4);
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn any_future_workday_uses_current_postal_or_default_delivery_slots() {
    use chrono::{Datelike, NaiveDate, Utc, Weekday};
    let app = TestApp::new().await;
    let admin = app.login("admin@example.test").await;
    let guest = app.client();
    let products = app.get(&guest, "/api/products").await;
    let mut input = app
        .order(&guest, "future@example.test", &products[0], 1)
        .await;
    let mut settings = app.get(&admin, "/api/admin/settings").await;
    settings["delivery_slots"] = json!(["06:30-08:45"]);
    app.call(
        &admin,
        Method::PUT,
        "/api/admin/settings",
        Some(&settings),
        StatusCode::OK,
    )
    .await;
    let mut area = app
        .post(
            &admin,
            "/api/admin/delivery-areas",
            &json!({
                "postal_code":"12000", "delivery_slots":["18:00-20:00"]
            }),
        )
        .await;
    let options = app.get(&guest, "/api/delivery-options").await;
    assert_eq!(options["slots"], json!(["06:30–08:45"]));
    assert_eq!(options["postal_slots"], json!({"12000":["18:00–20:00"]}));
    let year = Utc::now().year() + 2;
    let workday = (1..=7)
        .map(|day| NaiveDate::from_ymd_opt(year, 6, day).unwrap())
        .find(|date| date.weekday() == Weekday::Mon)
        .unwrap();
    input["delivery_date"] = json!(workday);
    input["delivery_slot"] = json!("18:00–20:00");
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::BAD_REQUEST,
    )
    .await;
    input["postal_code"] = json!("12000");
    let created = app.post(&guest, "/api/orders", &input).await;
    assert_eq!(created["delivery_date"], json!(workday));
    let id = area
        .as_object_mut()
        .unwrap()
        .remove("id")
        .unwrap()
        .as_i64()
        .unwrap();
    area["delivery_slots"] = Value::Null;
    app.call(
        &admin,
        Method::PUT,
        &format!("/api/admin/delivery-areas/{id}"),
        Some(&area),
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        app.post(&guest, "/api/orders", &input).await["id"],
        created["id"]
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
    input["delivery_slot"] = json!("06:30–08:45");
    let sunday = workday.pred_opt().unwrap();
    input["delivery_date"] = json!(sunday);
    app.call(
        &guest,
        Method::POST,
        "/api/orders",
        Some(&input),
        StatusCode::BAD_REQUEST,
    )
    .await;
    input["delivery_date"] = json!(workday);
    app.post(&guest, "/api/orders", &input).await;
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

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn bulk_order_updates_apply_once_and_require_matching_states_and_versions() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let products = app.get(&guest, "/api/products").await;
    let mut selected = Vec::new();
    for _ in 0..2 {
        let input = app
            .order(&guest, "bulk@example.test", &products[0], 1)
            .await;
        let order = app.post(&guest, "/api/orders", &input).await;
        selected.push(json!({"id": order["id"], "version": order["version"]}));
    }
    let endpoint = "/api/admin/orders/bulk-status";
    let change = |orders: &[Value], from: &str, to: &str| json!({"orders": orders, "from_status": from, "status": to});
    let first = change(&selected[0..1], "new", "confirmed");
    app.call(
        &guest,
        Method::POST,
        endpoint,
        Some(&first),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.post(&admin, endpoint, &first).await;
    let original = change(&selected, "new", "confirmed");
    app.call(
        &admin,
        Method::POST,
        endpoint,
        Some(&original),
        StatusCode::CONFLICT,
    )
    .await;
    selected[0]["version"] = json!(2);
    // Even fresh versions may not combine different starting states.
    app.call(
        &admin,
        Method::POST,
        endpoint,
        Some(&change(&selected, "new", "confirmed")),
        StatusCode::CONFLICT,
    )
    .await;
    app.post(
        &admin,
        endpoint,
        &change(&selected[1..], "new", "confirmed"),
    )
    .await;
    selected[1]["version"] = json!(2);
    let advance = change(&selected, "confirmed", "out_for_delivery");
    assert_eq!(app.post(&admin, endpoint, &advance).await["updated"], 2);
    let count = app.count("outbox").await;
    app.call(
        &admin,
        Method::POST,
        endpoint,
        Some(&advance),
        StatusCode::CONFLICT,
    )
    .await;
    for row in &mut selected {
        row["version"] = json!(3);
    }
    let mut finish = change(&selected, "out_for_delivery", "delivered");
    app.call(
        &admin,
        Method::POST,
        endpoint,
        Some(&finish),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(app.count("outbox").await, count);
    finish["confirm_payments"] = json!(true);
    assert_eq!(app.post(&admin, endpoint, &finish).await["updated"], 2);
    assert_eq!(app.count("outbox").await, count + 2);
    for row in selected {
        let detail = app
            .get(
                &admin,
                &format!("/api/admin/orders/{}", row["id"].as_str().unwrap()),
            )
            .await;
        assert_eq!(detail["status"], "delivered");
        assert_eq!(detail["payment_status"], "paid");
        assert_eq!(detail["history"].as_array().unwrap().len(), 4);
    }
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn issued_invoice_is_private_and_preserves_original_supplier_recipient_and_prices() {
    let app = TestApp::new().await;
    let guest = app.client();
    let admin = app.login("admin@example.test").await;
    let product = &app.get(&guest, "/api/products").await[0];
    let input = app.order(&guest, "invoice@example.test", product, 1).await;
    let order = app.post(&guest, "/api/orders", &input).await;
    let endpoint = format!(
        "/api/admin/orders/{}/invoice",
        order["id"].as_str().unwrap()
    );
    let route = "/api/admin/invoice-supplier";
    let mut supplier = app.get(&admin, route).await;
    supplier["name"] = json!("Testovací dodavatel");
    supplier["address"] = json!("Praha 1");
    supplier["company_id"] = json!("12345678");
    supplier["tax_id"] = json!("CZ12345678");
    supplier["vat_payer"] = json!(true);
    let mut supplier = app
        .call(&admin, Method::PUT, route, Some(&supplier), StatusCode::OK)
        .await;
    let mut issue = json!({"supplier_version":supplier["version"], "customer_name":"Eva", "customer_address":"Praha", "customer_company_id":"", "customer_tax_id":"", "supply_date":order["delivery_date"], "due_date":order["delivery_date"], "vat_rates":{}});
    assert!(app.get(&admin, &endpoint).await.is_null());
    app.call(
        &admin,
        Method::POST,
        &endpoint,
        Some(&issue),
        StatusCode::BAD_REQUEST,
    )
    .await;
    issue["vat_rates"][product["id"].to_string()] = json!(1200);
    if order["delivery_fee_cents"].as_i64().unwrap() > 0 {
        issue["vat_rates"]["delivery"] = json!(1200);
    }
    app.call(
        &guest,
        Method::POST,
        &endpoint,
        Some(&issue),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let (first, second) = tokio::join!(
        app.post(&admin, &endpoint, &issue),
        app.post(&admin, &endpoint, &issue)
    );
    assert_eq!(first, second, "concurrent requests must issue one invoice");
    assert_eq!(first["total_cents"], order["total_cents"]);
    assert_eq!(
        first["lines"][0]["unit_price_cents"],
        product["price_cents"]
    );
    assert!(first["number"].as_str().unwrap().starts_with("FA-"));
    supplier["name"] = json!("Změněné údaje");
    app.call(&admin, Method::PUT, route, Some(&supplier), StatusCode::OK)
        .await;
    assert_eq!(app.get(&admin, &endpoint).await, first);
    app.call(
        &guest,
        Method::GET,
        &endpoint,
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    app.close().await;
}

#[tokio::test]
#[ignore = "requires real PostgreSQL via TEST_DATABASE_URL"]
async fn terms_changed_while_checkout_waits_returns_conflict_without_creating_order() {
    let app = TestApp::new().await;
    let guest = app.client();
    let product = &app.get(&guest, "/api/products").await[0];
    let mut input = app
        .order(&guest, "terms-race@example.test", product, 1)
        .await;
    input["terms_version"] = json!(1);
    let mut publisher = app.pool.begin().await.unwrap();
    sqlx::query("SELECT version FROM terms_publication WHERE id=1 FOR UPDATE")
        .execute(&mut *publisher)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO terms_versions(version,title,content) VALUES (2,'Nové podmínky','Nové znění')",
    )
    .execute(&mut *publisher)
    .await
    .unwrap();
    sqlx::query("UPDATE terms_publication SET version=2 WHERE id=1")
        .execute(&mut *publisher)
        .await
        .unwrap();
    let url = format!("{}/api/orders", app.url);
    let request = tokio::spawn(async move { guest.post(url).json(&input).send().await.unwrap() });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE wait_event_type='Lock' AND query LIKE '%terms_publication%' AND pid<>pg_backend_pid())")
                .fetch_one(&app.pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("checkout must wait on the publisher's row lock");
    publisher.commit().await.unwrap();
    let response = request.await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("podmínky se změnily")
    );
    assert_eq!(app.count("orders").await, 0);
    app.close().await;
}
