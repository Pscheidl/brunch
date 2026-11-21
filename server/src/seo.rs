//! Public HTML is rendered for every visitor, including clients without JavaScript.
//! The interactive storefront replaces it only after its data has loaded.
use crate::{ApiError, ApiResult, AppState};
use axum::{
    Router,
    extract::{RawQuery, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use sqlx::FromRow;
use std::fmt::Write;

const HOME_TITLE: &str = "Brunch Era · Dobrá pauza začíná tady";
const HOME_DESCRIPTION: &str = "Poctivé houstičky z pekárny Mysterium Gaudi podle originální domácí receptury. Čerstvé svačiny až k vám do kanceláře.";
const HEAD_START: &str = "<!--seo-head-start-->";
const HEAD_END: &str = "<!--seo-head-end-->";
const CONTENT: &str = "<!--server-content-->";

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(storefront))
        .route("/robots.txt", get(robots))
        .route("/sitemap.xml", get(sitemap))
}

#[derive(FromRow)]
struct Product {
    id: i64,
    name: String,
    description: String,
    category: String,
    weight: String,
    price_cents: i64,
    ingredients: String,
    allergens: String,
    image: String,
    stock: Option<i32>,
    minimum_quantity: i32,
    banquet: bool,
    variants: Vec<String>,
}

#[derive(FromRow)]
struct Site {
    hero_image: String,
    hero_image_alt: String,
    delivery_area_label: String,
    public_contact_email: String,
    public_contact_phone: String,
    enabled: bool,
    name: String,
    street: String,
    city: String,
    postal_code: String,
    instructions: String,
    parking_free: bool,
    slots: Vec<String>,
    banquet_minimum_quantity: i32,
}

fn escape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#39;"),
            // Keep XML 1.0 sitemap values and HTML text free of invalid controls.
            c if (!c.is_control() || matches!(c, '\t' | '\n' | '\r'))
                && !matches!(c, '\u{fffe}' | '\u{ffff}') =>
            {
                result.push(c)
            }
            _ => {}
        }
    }
    result
}

fn local_image(value: &str) -> Option<&str> {
    let path = value.trim();
    (!path.is_empty() && !path.starts_with("//") && !path.contains([':', '\\'])).then_some(path)
}

fn money(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("{} Kč", cents / 100)
    } else {
        format!("{},{:02} Kč", cents / 100, cents % 100)
    }
}

fn product_id(query: Option<&str>) -> Result<Option<i64>, ()> {
    let mut values = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter(|(key, _)| key == "product");
    let Some((_, value)) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .map(Some)
        .ok_or(())
}

fn error_page(status: StatusCode) -> Response {
    let (title, message) = if status == StatusCode::NOT_FOUND {
        (
            "Svačina není v nabídce",
            "Tento produkt už není dostupný. Vyberte si z aktuální nabídky.",
        )
    } else {
        (
            "Nabídku se nepodařilo načíst",
            "Zkuste prosím stránku za chvíli obnovit.",
        )
    };
    (status, [(header::CACHE_CONTROL, "no-store"), (header::HeaderName::from_static("x-robots-tag"), "noindex")], Html(format!(
        "<!doctype html><html lang=\"cs\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{title} | Brunch Era</title></head><body><main><h1>{title}</h1><p>{message}</p><a href=\"/\">Zpět na nabídku</a></main></body></html>"
    ))).into_response()
}

async fn storefront(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let id = match product_id(query.as_deref()) {
        Ok(id) => id,
        Err(()) => return error_page(StatusCode::NOT_FOUND),
    };
    match render(&state, id).await {
        Ok(Some(html)) => ([(header::CACHE_CONTROL, "no-cache")], Html(html)).into_response(),
        Ok(None) => error_page(StatusCode::NOT_FOUND),
        Err(error) => {
            tracing::error!(error = %error, "Public HTML rendering failed");
            error_page(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn render(state: &AppState, id: Option<i64>) -> ApiResult<Option<String>> {
    let products: Vec<Product> = sqlx::query_as("SELECT id,name,description,category,weight,price_cents,ingredients,allergens,image,stock,minimum_quantity,banquet,variants FROM products WHERE active ORDER BY id")
        .fetch_all(&state.pool).await?;
    let selected = id.and_then(|id| products.iter().find(|p| p.id == id));
    if id.is_some() && selected.is_none() {
        return Ok(None);
    }
    let site: Site = sqlx::query_as("SELECT s.hero_image,s.hero_image_alt,s.delivery_area_label,s.public_contact_email,s.public_contact_phone,p.enabled,p.name,p.street,p.city,p.postal_code,p.instructions,p.parking_free,p.slots,b.banquet_minimum_quantity FROM site_settings s CROSS JOIN pickup_settings p CROSS JOIN shop_settings b WHERE s.id=1 AND p.id=1 AND b.id=1")
        .fetch_one(&state.pool).await?;
    let template =
        tokio::fs::read_to_string(std::path::Path::new(&state.config.dist_dir).join("index.html"))
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "Storefront template unavailable");
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Šablona webu není dostupná",
                )
            })?;
    let title = selected.map_or_else(
        || HOME_TITLE.to_owned(),
        |p| format!("{} | Brunch Era", p.name),
    );
    let description = selected.map_or_else(
        || HOME_DESCRIPTION.to_owned(),
        |p| {
            let text = if p.description.trim().is_empty() {
                &p.name
            } else {
                &p.description
            };
            text.chars().take(160).collect()
        },
    );
    let origin = state.config.public_url.trim_end_matches('/');
    let canonical = selected.map_or_else(
        || format!("{origin}/"),
        |p| format!("{origin}/?product={}", p.id),
    );
    let mut head = format!(
        "<title>{}</title><meta name=\"description\" content=\"{}\"><meta name=\"robots\" content=\"index,follow\"><link rel=\"canonical\" href=\"{}\"><meta property=\"og:site_name\" content=\"Brunch Era\"><meta property=\"og:title\" content=\"{}\"><meta property=\"og:description\" content=\"{}\"><meta property=\"og:type\" content=\"website\"><meta property=\"og:url\" content=\"{}\">",
        escape(&title),
        escape(&description),
        escape(&canonical),
        escape(&title),
        escape(&description),
        escape(&canonical)
    );
    let photo = selected
        .and_then(|p| local_image(&p.image))
        .or_else(|| local_image(&site.hero_image))
        .unwrap_or("/images/brunch-spread.webp");
    if let Ok(url) = url::Url::parse(&format!("{origin}/")).and_then(|base| base.join(photo)) {
        let _ = write!(
            head,
            "<meta property=\"og:image\" content=\"{}\">",
            escape(url.as_str())
        );
    }
    let content = public_content(&products, selected, &site);
    let Some((before, rest)) = template.split_once(HEAD_START) else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Chybí začátek SEO šablony",
        ));
    };
    let Some((_, after)) = rest.split_once(HEAD_END) else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Chybí konec SEO šablony",
        ));
    };
    if !after.contains(CONTENT) {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Chybí veřejný obsah šablony",
        ));
    }
    Ok(Some(format!(
        "{before}{head}{}",
        after.replacen(CONTENT, &content, 1)
    )))
}

fn public_content(products: &[Product], selected: Option<&Product>, site: &Site) -> String {
    let mut html = String::from(
        "<div id=\"server-content\"><header class=\"site-header\"><div class=\"header-inner page-width\"><a class=\"brand\" href=\"/\"><img class=\"brand-wordmark\" src=\"/brand/wordmark.svg\" width=\"704\" height=\"168\" alt=\"Brunch Éra\"></a><nav aria-label=\"Hlavní navigace\"><a href=\"/#nabidka\">Naše nabídka</a></nav></div></header><main>",
    );
    if let Some(product) = selected {
        html.push_str("<section class=\"page-width server-product\"><a class=\"text-button\" href=\"/#nabidka\">Zpět na nabídku</a>");
        let _ = write!(html, "<h1>{}</h1>", escape(&product.name));
        product_content(&mut html, product, true, site.banquet_minimum_quantity);
        html.push_str("</section>");
    } else {
        let photo = local_image(&site.hero_image).unwrap_or("/images/brunch-spread.webp");
        let _ = write!(
            html,
            "<section class=\"hero page-width\"><div class=\"hero-copy\"><div class=\"eyebrow\">Z PEKÁRNY MYSTERIUM GAUDI</div><h1>Dobrá pauza.<br><span>Lepší den.</span></h1><p>Jedinečné houstičky z pekárny Mysterium Gaudi, pečené podle originální domácí receptury. Poctivý základ našich svačin, doplněný čerstvými surovinami. Až k vám do kanceláře.</p><a class=\"button primary hero-cta\" href=\"#nabidka\">Vybrat svačiny</a></div><div class=\"hero-visual\"><img src=\"{}\" alt=\"{}\" width=\"1536\" height=\"1024\" fetchpriority=\"high\"></div></section>",
            escape(photo),
            escape(&site.hero_image_alt)
        );
    }
    html.push_str("<section class=\"catalog page-width\" id=\"nabidka\"><div class=\"section-heading\"><h2>Na co máte chuť?</h2></div><div class=\"product-grid\">");
    for product in products {
        html.push_str("<article class=\"product-card\">");
        product_content(&mut html, product, false, site.banquet_minimum_quantity);
        html.push_str("</article>");
    }
    if products.is_empty() {
        html.push_str("<p>Nabídku právě připravujeme. Zkuste to prosím později.</p>");
    }
    html.push_str("</div><p class=\"catalog-disclaimer\">Konečné ceny včetně případné DPH. Fotografie jsou ilustrační.</p></section><section class=\"page-width server-delivery\"><h2>Rozvoz a osobní odběr</h2><p>Vyberte si svačiny, způsob převzetí a dostupný termín. Dostupnost rozvozu ověříte při zadání adresy.</p>");
    if !site.delivery_area_label.is_empty() {
        let _ = write!(html, "<p>{}</p>", escape(&site.delivery_area_label));
    }
    if site.enabled {
        let _ = write!(
            html,
            "<h3>Osobní odběr</h3><p>{}<br>{}, {} {}</p><p>Časová okna odběru: {}</p>",
            escape(&site.name),
            escape(&site.street),
            escape(&site.postal_code),
            escape(&site.city),
            escape(&site.slots.join(", "))
        );
        if site.parking_free {
            html.push_str("<p>Parkování zdarma.</p>");
        }
        if !site.instructions.is_empty() {
            let _ = write!(html, "<p>{}</p>", escape(&site.instructions));
        }
    }
    if !site.public_contact_email.is_empty() || !site.public_contact_phone.is_empty() {
        let _ = write!(
            html,
            "<h3>Kontakt</h3><p>{} {}</p>",
            escape(&site.public_contact_email),
            escape(&site.public_contact_phone)
        );
    }
    html.push_str("</section></main><footer class=\"site-footer page-width\"><p>Brunch Era · Dobrá pauza začíná tady.</p><a href=\"/obchodni-podminky/\">Obchodní podmínky</a></footer></div>");
    html
}

fn product_content(html: &mut String, product: &Product, detail: bool, banquet_minimum: i32) {
    if let Some(image) = local_image(&product.image) {
        let _ = write!(
            html,
            "<a class=\"product-photo photo-frame\" href=\"/?product={}\"><img class=\"catalog-photo\" src=\"{}\" alt=\"{}\" loading=\"lazy\" decoding=\"async\"></a>",
            product.id,
            escape(image),
            escape(&product.name)
        );
    }
    html.push_str("<div class=\"product-info\">");
    if !detail {
        let _ = write!(
            html,
            "<h3><a href=\"/?product={}\">{}</a></h3>",
            product.id,
            escape(&product.name)
        );
    }
    let _ = write!(
        html,
        "<p>{}</p><p class=\"product-meta\">{} {}</p><p class=\"product-price\">{} / ks</p>",
        escape(&product.description),
        escape(&product.category),
        escape(&product.weight),
        money(product.price_cents)
    );
    if product.minimum_quantity > 1 {
        let _ = write!(
            html,
            "<p>Minimální odběr {} ks</p>",
            product.minimum_quantity
        );
    }
    if product
        .stock
        .is_some_and(|stock| stock < product.minimum_quantity.max(1))
    {
        html.push_str("<p>Vyprodáno</p>");
    }
    if detail {
        if product.banquet {
            let _ = write!(
                html,
                "<p>Banketní pečivo objednávejte celkem od {banquet_minimum} ks. Do společného minima můžete kombinovat různé druhy.</p>"
            );
        }
        let ingredients = if product.ingredients.trim().is_empty() {
            "Složení zatím nebylo doplněno. Před objednáním si je prosím ověřte u obsluhy."
        } else {
            &product.ingredients
        };
        let allergens = if product.allergens.trim().is_empty() {
            "Informace o alergenech si prosím ověřte u obsluhy."
        } else {
            &product.allergens
        };
        let _ = write!(
            html,
            "<h2>Co je uvnitř</h2><p>{}</p><h2>Alergeny</h2><p>{}</p>",
            escape(ingredients),
            escape(allergens)
        );
        if !product.variants.is_empty() {
            let _ = write!(
                html,
                "<h2>Varianty</h2><p>{}</p>",
                escape(&product.variants.join(", "))
            );
        }
    }
    html.push_str("</div>");
}

async fn robots(State(state): State<AppState>) -> Response {
    // Private pages carry noindex. Do not disallow them here: crawlers must read it.
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
            state.config.public_url.trim_end_matches('/')
        ),
    )
        .into_response()
}

async fn sitemap(State(state): State<AppState>) -> ApiResult<Response> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM products WHERE active ORDER BY id")
        .fetch_all(&state.pool)
        .await?;
    let origin = escape(state.config.public_url.trim_end_matches('/'));
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>{origin}/</loc></url>"
    );
    for id in ids {
        let _ = write!(xml, "<url><loc>{origin}/?product={id}</loc></url>");
    }
    xml.push_str("</urlset>");
    Ok((
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        xml,
    )
        .into_response())
}
