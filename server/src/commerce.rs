use crate::{ApiError, ApiResult, AppState, auth, mail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use chrono::{DateTime, Datelike, Days, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

const MAX_PRICE: i64 = 10_000_000;
const MAX_TOTAL: i64 = 1_000_000_000;
const MAX_STOCK: i32 = 1_000_000;
const SLOTS: [&str; 3] = ["07:00–09:00", "09:00–11:00", "11:00–13:00"];
const STATUSES: [&str; 8] = [
    "awaiting_payment",
    "new",
    "confirmed",
    "preparing",
    "out_for_delivery",
    "delivered",
    "cancelled",
    "expired",
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/products", get(products))
        .route("/api/settings", get(settings))
        .route("/api/delivery-options", get(delivery_options))
        .route("/api/orders", post(create_order))
        .route("/api/orders/mine", get(my_orders))
        .route(
            "/api/admin/products",
            get(admin_products).post(create_product),
        )
        .route("/api/admin/products/{id}", put(update_product))
        .route("/api/admin/products/{id}/prices", get(price_history))
        .route("/api/admin/orders", get(admin_orders))
        .route(
            "/api/admin/orders/{id}",
            get(admin_order).patch(update_order),
        )
        .route(
            "/api/admin/settings",
            get(admin_settings).put(update_settings),
        )
        .route(
            "/api/admin/delivery-areas",
            get(admin_delivery_areas).post(create_delivery_area),
        )
        .route("/api/admin/delivery-areas/{id}", put(update_delivery_area))
        .route("/api/admin/customers", get(customers))
}

#[derive(Clone, Serialize, FromRow)]
struct Product {
    id: i64,
    name: String,
    description: String,
    category: String,
    tag: String,
    weight: String,
    price_cents: i64,
    vegan: bool,
    ingredients: String,
    allergens: String,
    image: String,
    active: bool,
    stock: Option<i32>,
    version: i64,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    weight: String,
    price_cents: i64,
    #[serde(default)]
    vegan: bool,
    #[serde(default)]
    ingredients: String,
    #[serde(default)]
    allergens: String,
    #[serde(default)]
    image: String,
    #[serde(default = "default_true")]
    active: bool,
    stock: Option<i32>,
    version: Option<i64>,
}

fn clean(value: &str, field: &str, max: usize, required: bool) -> ApiResult<String> {
    let value = value.trim();
    if (required && value.is_empty())
        || value.chars().count() > max
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError::bad_request(format!("Neplatné pole: {field}.")));
    }
    Ok(value.to_owned())
}

fn email(value: &str) -> ApiResult<String> {
    auth::normalize_email(value)
}

impl ProductInput {
    fn validate(&mut self) -> ApiResult<()> {
        self.name = clean(&self.name, "název", 200, true)?;
        self.description = clean(&self.description, "popis", 5000, false)?;
        self.category = clean(&self.category, "kategorie", 80, false)?;
        self.tag = clean(&self.tag, "štítek", 80, false)?;
        self.weight = clean(&self.weight, "hmotnost", 80, false)?;
        self.ingredients = clean(&self.ingredients, "složení", 5000, false)?;
        self.allergens = clean(&self.allergens, "alergeny", 1000, false)?;
        self.image = clean(&self.image, "obrázek", 500, false)?;
        if !(0..=MAX_PRICE).contains(&self.price_cents)
            || self.stock.is_some_and(|s| !(0..=MAX_STOCK).contains(&s))
        {
            return Err(ApiError::bad_request(
                "Cena nebo skladové množství jsou mimo povolený rozsah.",
            ));
        }
        if self.image.starts_with("//")
            || self.image.contains([':', '\\', '<', '>', '"', '\''])
            || self.image.chars().any(char::is_whitespace)
            || self.image.split('/').any(|s| s == "..")
        {
            return Err(ApiError::bad_request(
                "Obrázek musí být místní URL cesta bez nadřazených složek.",
            ));
        }
        Ok(())
    }
}

async fn products(State(state): State<AppState>) -> ApiResult<Json<Vec<Product>>> {
    Ok(Json(
        sqlx::query_as("SELECT * FROM products WHERE active ORDER BY id")
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn admin_products(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Product>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM products ORDER BY id")
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<ProductInput>,
) -> ApiResult<Json<Product>> {
    let admin = auth::require_admin(&state, &headers).await?;
    input.validate()?;
    let mut tx = state.pool.begin().await?;
    let product: Product = sqlx::query_as(
        "INSERT INTO products(name,description,category,tag,weight,price_cents,vegan,ingredients,allergens,image,active,stock) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING *",
    ).bind(&input.name).bind(&input.description).bind(&input.category).bind(&input.tag)
        .bind(&input.weight).bind(input.price_cents).bind(input.vegan).bind(&input.ingredients)
        .bind(&input.allergens).bind(&input.image).bind(input.active).bind(input.stock)
        .fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO product_prices(product_id,price_cents,changed_by) VALUES ($1,$2,$3)")
        .bind(product.id)
        .bind(product.price_cents)
        .bind(admin.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(product))
}

async fn update_product(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(mut input): Json<ProductInput>,
) -> ApiResult<Json<Product>> {
    let admin = auth::require_admin(&state, &headers).await?;
    input.validate()?;
    let version = input
        .version
        .filter(|v| *v > 0)
        .ok_or_else(|| ApiError::bad_request("Chybí verze produktu."))?;
    let mut tx = state.pool.begin().await?;
    let old: Product = sqlx::query_as("SELECT * FROM products WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Produkt neexistuje."))?;
    if old.version != version {
        return Err(ApiError::conflict(
            "Produkt se mezitím změnil. Načtěte aktuální údaje.",
        ));
    }
    let product = sqlx::query_as(
        "UPDATE products SET name=$2,description=$3,category=$4,tag=$5,weight=$6,price_cents=$7,vegan=$8,ingredients=$9,allergens=$10,image=$11,active=$12,stock=$13,version=version+1,updated_at=now() WHERE id=$1 RETURNING *",
    ).bind(id).bind(&input.name).bind(&input.description).bind(&input.category).bind(&input.tag)
        .bind(&input.weight).bind(input.price_cents).bind(input.vegan).bind(&input.ingredients)
        .bind(&input.allergens).bind(&input.image).bind(input.active).bind(input.stock)
        .fetch_one(&mut *tx).await?;
    if old.price_cents != input.price_cents {
        sqlx::query(
            "INSERT INTO product_prices(product_id,price_cents,changed_by) VALUES ($1,$2,$3)",
        )
        .bind(id)
        .bind(input.price_cents)
        .bind(admin.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(product))
}

#[derive(Serialize, FromRow)]
struct PriceEntry {
    id: i64,
    price_cents: i64,
    created_at: DateTime<Utc>,
    changed_by: Option<Uuid>,
}

async fn price_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<PriceEntry>>> {
    auth::require_admin(&state, &headers).await?;
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM products WHERE id=$1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?
    {
        return Err(ApiError::not_found("Produkt neexistuje."));
    }
    Ok(Json(sqlx::query_as("SELECT id,price_cents,created_at,changed_by FROM product_prices WHERE product_id=$1 ORDER BY id DESC")
        .bind(id).fetch_all(&state.pool).await?))
}

#[derive(Serialize, Deserialize, FromRow)]
#[serde(deny_unknown_fields)]
struct Settings {
    delivery_fee_cents: i64,
    free_delivery_from_cents: Option<i64>,
    order_email: String,
    version: i64,
}

async fn settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let (delivery_fee_cents, free_delivery_from_cents, delivery_postal_codes):
        (i64, Option<i64>, Vec<String>) = sqlx::query_as(
            "SELECT delivery_fee_cents,free_delivery_from_cents,ARRAY(SELECT postal_code FROM delivery_areas WHERE active ORDER BY postal_code) AS delivery_postal_codes FROM shop_settings WHERE id=1",
        ).fetch_one(&state.pool).await?;
    Ok(Json(
        json!({"delivery_fee_cents":delivery_fee_cents,"free_delivery_from_cents":free_delivery_from_cents,"delivery_postal_codes":delivery_postal_codes,"online_payment_enabled":state.config.teya.is_some()}),
    ))
}

fn normalize_postal_code(value: &str) -> ApiResult<String> {
    let postal_code: String = value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '\u{feff}')
        .collect();
    if postal_code.len() != 5
        || !postal_code
            .bytes()
            .all(|character| character.is_ascii_digit())
    {
        return Err(ApiError::bad_request("PSČ musí obsahovat pět číslic."));
    }
    Ok(postal_code)
}

#[derive(Serialize, FromRow)]
struct DeliveryArea {
    id: i64,
    postal_code: String,
    label: String,
    active: bool,
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryAreaInput {
    postal_code: String,
    #[serde(default)]
    label: String,
    #[serde(default = "default_true")]
    active: bool,
    version: Option<i64>,
}

impl DeliveryAreaInput {
    fn normalize(&mut self) -> ApiResult<()> {
        self.postal_code = normalize_postal_code(&self.postal_code)?;
        self.label = clean(&self.label, "popis oblasti rozvozu", 100, false)?;
        Ok(())
    }
}

fn delivery_area_error(error: sqlx::Error) -> ApiError {
    if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation()) {
        ApiError::conflict("Toto PSČ už je v oblastech rozvozu uvedeno. Upravte existující záznam.")
    } else {
        error.into()
    }
}

async fn admin_delivery_areas(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<DeliveryArea>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(sqlx::query_as("SELECT id,postal_code,label,active,version FROM delivery_areas ORDER BY postal_code,id")
        .fetch_all(&state.pool).await?))
}

async fn create_delivery_area(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<DeliveryAreaInput>,
) -> ApiResult<Json<DeliveryArea>> {
    auth::require_admin(&state, &headers).await?;
    input.normalize()?;
    let mut tx = state.pool.begin().await?;
    // All coverage changes serialize with checkout's shared settings lock.
    sqlx::query("SELECT id FROM shop_settings WHERE id=1 FOR UPDATE")
        .fetch_one(&mut *tx)
        .await?;
    let area = sqlx::query_as("INSERT INTO delivery_areas(postal_code,label,active) VALUES ($1,$2,$3) RETURNING id,postal_code,label,active,version")
        .bind(input.postal_code).bind(input.label).bind(input.active)
        .fetch_one(&mut *tx).await.map_err(delivery_area_error)?;
    tx.commit().await?;
    Ok(Json(area))
}

async fn update_delivery_area(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(mut input): Json<DeliveryAreaInput>,
) -> ApiResult<Json<DeliveryArea>> {
    auth::require_admin(&state, &headers).await?;
    input.normalize()?;
    let version = input
        .version
        .filter(|version| *version > 0)
        .ok_or_else(|| ApiError::bad_request("Chybí platná verze oblasti rozvozu."))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM shop_settings WHERE id=1 FOR UPDATE")
        .fetch_one(&mut *tx)
        .await?;
    let old: DeliveryArea = sqlx::query_as(
        "SELECT id,postal_code,label,active,version FROM delivery_areas WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("Oblast rozvozu neexistuje."))?;
    if old.version != version {
        return Err(ApiError::conflict(
            "Oblast rozvozu se mezitím změnila. Načtěte aktuální údaje.",
        ));
    }
    let area = sqlx::query_as("UPDATE delivery_areas SET postal_code=$2,label=$3,active=$4,version=version+1 WHERE id=$1 RETURNING id,postal_code,label,active,version")
        .bind(id).bind(input.postal_code).bind(input.label).bind(input.active)
        .fetch_one(&mut *tx).await.map_err(delivery_area_error)?;
    tx.commit().await?;
    Ok(Json(area))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Settings>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM shop_settings WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
    ))
}

async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    auth::require_admin(&state, &headers).await?;
    input.order_email = email(&input.order_email)?;
    if !(0..=MAX_PRICE).contains(&input.delivery_fee_cents)
        || input
            .free_delivery_from_cents
            .is_some_and(|v| !(0..=MAX_TOTAL).contains(&v))
        || input.version < 1
    {
        return Err(ApiError::bad_request(
            "Neplatné nastavení dopravy nebo verze.",
        ));
    }
    let updated = sqlx::query_as("UPDATE shop_settings SET delivery_fee_cents=$1,free_delivery_from_cents=$2,order_email=$3,version=version+1,updated_at=now() WHERE id=1 AND version=$4 RETURNING *")
        .bind(input.delivery_fee_cents).bind(input.free_delivery_from_cents).bind(input.order_email).bind(input.version)
        .fetch_optional(&state.pool).await?.ok_or_else(|| ApiError::conflict("Nastavení se mezitím změnilo. Načtěte aktuální údaje."))?;
    Ok(Json(updated))
}

fn easter_sunday(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("Gregorian Easter date")
}

fn working_day(date: NaiveDate) -> bool {
    let holiday = matches!(
        (date.month(), date.day()),
        (1, 1)
            | (5, 1)
            | (5, 8)
            | (7, 5)
            | (7, 6)
            | (9, 28)
            | (10, 28)
            | (11, 17)
            | (12, 24)
            | (12, 25)
            | (12, 26)
    );
    let easter = easter_sunday(date.year());
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
        && !holiday
        && Some(date) != easter.checked_sub_days(Days::new(2))
        && Some(date) != easter.checked_add_days(Days::new(1))
}

fn next_dates(today: NaiveDate) -> Vec<NaiveDate> {
    (1..30)
        .filter_map(|n| today.checked_add_days(Days::new(n)))
        .filter(|date| working_day(*date))
        .take(3)
        .collect()
}

async fn delivery_options() -> Json<Value> {
    let today = Utc::now()
        .with_timezone(&chrono_tz::Europe::Prague)
        .date_naive();
    let labels = [
        "Pondělí",
        "Úterý",
        "Středa",
        "Čtvrtek",
        "Pátek",
        "Sobota",
        "Neděle",
    ];
    let dates: Vec<_> = next_dates(today).into_iter().map(|date| {
        json!({"value":date.to_string(),"label":format!("{} {}. {}.",labels[date.weekday().num_days_from_monday() as usize],date.day(),date.month())})
    }).collect();
    Json(json!({"dates":dates,"slots":SLOTS}))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CartItem {
    product_id: i64,
    quantity: i32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OrderInput {
    idempotency_key: Uuid,
    email: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    organization: String,
    street: String,
    city: String,
    postal_code: String,
    #[serde(default)]
    notes: String,
    delivery_date: NaiveDate,
    delivery_slot: String,
    payment_method: String,
    items: Vec<CartItem>,
    expected_total_cents: i64,
}

impl OrderInput {
    fn normalize(&mut self) -> ApiResult<()> {
        self.email = email(&self.email)?;
        self.name = clean(&self.name, "jméno", 200, false)?;
        self.phone = clean(&self.phone, "telefon", 40, false)?;
        self.organization = clean(&self.organization, "firma", 200, false)?;
        self.street = clean(&self.street, "ulice a číslo", 250, true)?;
        self.city = clean(&self.city, "obec", 100, true)?;
        self.notes = clean(&self.notes, "poznámka k doručení", 2000, false)?;
        self.postal_code = normalize_postal_code(&self.postal_code)?;
        if !matches!(
            self.payment_method.as_str(),
            "card" | "card_on_delivery" | "cash_on_delivery"
        ) {
            return Err(ApiError::bad_request(
                "Vyberte platbu online nebo při doručení.",
            ));
        }
        if self.items.is_empty() || self.items.len() > 50 {
            return Err(ApiError::bad_request(
                "Košík musí obsahovat 1 až 50 různých produktů.",
            ));
        }
        let mut ids = HashSet::new();
        if self.items.iter().any(|item| {
            item.product_id <= 0
                || !(1..=99).contains(&item.quantity)
                || !ids.insert(item.product_id)
        }) {
            return Err(ApiError::bad_request(
                "Neplatné nebo duplicitní položky košíku. Množství musí být 1 až 99.",
            ));
        }
        self.items.sort_by_key(|i| i.product_id);
        if !(0..=MAX_TOTAL).contains(&self.expected_total_cents) {
            return Err(ApiError::bad_request("Neplatná očekávaná cena."));
        }
        if self.idempotency_key.is_nil() {
            return Err(ApiError::bad_request("Neplatný identifikátor požadavku."));
        }
        Ok(())
    }

    fn validate_delivery(&self) -> ApiResult<()> {
        let today = Utc::now()
            .with_timezone(&chrono_tz::Europe::Prague)
            .date_naive();
        if !next_dates(today).contains(&self.delivery_date)
            || !SLOTS.contains(&self.delivery_slot.as_str())
        {
            return Err(ApiError::bad_request("Zvolte dostupný den a čas doručení."));
        }
        Ok(())
    }
}

#[derive(Serialize, FromRow)]
struct Order {
    id: Uuid,
    number: String,
    email: String,
    name: String,
    phone: String,
    organization: String,
    street: String,
    city: String,
    postal_code: String,
    notes: String,
    delivery_date: NaiveDate,
    delivery_slot: String,
    payment_method: String,
    status: String,
    payment_status: String,
    subtotal_cents: i64,
    delivery_fee_cents: i64,
    total_cents: i64,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[sqlx(skip)]
    items: Vec<OrderItem>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<Vec<OrderHistory>>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    online_payment: Option<OnlinePayment>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, FromRow)]
struct OnlinePayment {
    state: String,
    expires_at: DateTime<Utc>,
    checked_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    cancellable: bool,
    #[sqlx(skip)]
    receipt: Option<PaymentReceipt>,
}

#[derive(Serialize, FromRow)]
struct PaymentReceipt {
    transaction_id: String,
    session_id: String,
    amount_cents: i64,
    currency: String,
    verified_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct OrderItem {
    #[serde(skip)]
    order_id: Uuid,
    product_id: i64,
    name: String,
    quantity: i32,
    unit_price_cents: i64,
    total_cents: i64,
}

#[derive(Serialize, FromRow)]
struct OrderHistory {
    status: String,
    payment_status: String,
    created_at: DateTime<Utc>,
    changed_by: Option<Uuid>,
}

async fn attach_items(conn: &mut PgConnection, orders: &mut [Order]) -> ApiResult<()> {
    if orders.is_empty() {
        return Ok(());
    }
    let ids: Vec<_> = orders.iter().map(|o| o.id).collect();
    let items: Vec<OrderItem> = sqlx::query_as("SELECT order_id,product_id,name,quantity,unit_price_cents,total_cents FROM order_items WHERE order_id=ANY($1) ORDER BY id")
        .bind(&ids).fetch_all(&mut *conn).await?;
    let online_ids: Vec<_> = orders
        .iter()
        .filter(|order| order.payment_method == "card")
        .map(|order| order.id)
        .collect();
    let mut deadlines: HashMap<Uuid, DateTime<Utc>> = if online_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
            "SELECT order_id,expires_at FROM payment_attempts WHERE order_id=ANY($1)",
        )
        .bind(&online_ids)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .collect()
    };
    let mut grouped: HashMap<Uuid, Vec<OrderItem>> = HashMap::new();
    for item in items {
        grouped.entry(item.order_id).or_default().push(item);
    }
    for order in orders {
        order.items = grouped.remove(&order.id).unwrap_or_default();
        order.payment_expires_at = deadlines.remove(&order.id);
    }
    Ok(())
}

async fn read_order(conn: &mut PgConnection, id: Uuid, history: bool) -> ApiResult<Order> {
    let mut order: Order = sqlx::query_as("SELECT * FROM orders WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| ApiError::not_found("Objednávka neexistuje."))?;
    attach_items(conn, std::slice::from_mut(&mut order)).await?;
    if history {
        order.history = Some(sqlx::query_as("SELECT status,payment_status,created_at,changed_by FROM order_history WHERE order_id=$1 ORDER BY id")
            .bind(id).fetch_all(&mut *conn).await?);
        if order.payment_method == "card" {
            order.online_payment = sqlx::query_as("SELECT state,expires_at,checked_at,last_error,(state IN ('failed','expired') AND checked_at IS NOT NULL) AS cancellable FROM payment_attempts WHERE order_id=$1")
                .bind(id).fetch_optional(&mut *conn).await?;
            if let Some(payment) = &mut order.online_payment {
                payment.receipt = sqlx::query_as("SELECT transaction_id,session_id,amount_cents,currency,verified_at FROM payment_receipts WHERE order_id=$1")
                    .bind(id).fetch_optional(&mut *conn).await?;
            }
        }
    }
    Ok(order)
}

fn delivery_fee(subtotal: i64, settings: &Settings) -> i64 {
    if settings
        .free_delivery_from_cents
        .is_some_and(|threshold| subtotal >= threshold)
    {
        0
    } else {
        settings.delivery_fee_cents
    }
}

async fn create_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<OrderInput>,
) -> ApiResult<Json<Order>> {
    input.normalize()?;
    let user = auth::current_user(&state, &headers).await?;
    let user_id = user
        .filter(|u| u.email.eq_ignore_ascii_case(&input.email))
        .map(|u| u.id);
    let payload =
        serde_json::to_value(&input).map_err(|_| ApiError::bad_request("Neplatná objednávka."))?;
    let mut tx = state.pool.begin().await?;
    // Serializes retries even before the unique idempotency row exists.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(input.idempotency_key.to_string())
        .execute(&mut *tx)
        .await?;
    if let Some((id, previous)) = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id,request_payload FROM orders WHERE idempotency_key=$1",
    )
    .bind(input.idempotency_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        if previous != payload {
            return Err(ApiError::conflict(
                "Tento identifikátor již patří jiné objednávce.",
            ));
        }
        let order = read_order(&mut tx, id, false).await?;
        tx.commit().await?;
        return Ok(Json(order));
    }
    let online = input.payment_method == "card";
    if online && state.config.teya.is_none() {
        return Err(ApiError::conflict(
            "Online platba momentálně není dostupná. Zvolte platbu při doručení.",
        ));
    }
    input.validate_delivery()?;
    let settings: Settings = sqlx::query_as("SELECT * FROM shop_settings WHERE id=1 FOR SHARE")
        .fetch_one(&mut *tx)
        .await?;
    let supported: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM delivery_areas WHERE postal_code=$1 AND active)",
    )
    .bind(&input.postal_code)
    .fetch_one(&mut *tx)
    .await?;
    if !supported {
        return Err(ApiError::bad_request(
            "Na zadané PSČ momentálně nerozvážíme. Zadejte adresu v podporované oblasti rozvozu.",
        ));
    }
    let ids: Vec<_> = input.items.iter().map(|i| i.product_id).collect();
    // The stable lock order prevents concurrent checkouts from overselling or deadlocking.
    let products: Vec<Product> =
        sqlx::query_as("SELECT * FROM products WHERE id=ANY($1) ORDER BY id FOR UPDATE")
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?;
    let by_id: BTreeMap<i64, Product> = products.into_iter().map(|p| (p.id, p)).collect();
    let mut subtotal: i64 = 0;
    for item in &input.items {
        let product = by_id
            .get(&item.product_id)
            .filter(|p| p.active)
            .ok_or_else(|| {
                ApiError::conflict("Některý produkt již není v nabídce. Aktualizujte košík.")
            })?;
        if product.stock.is_some_and(|stock| stock < item.quantity) {
            return Err(ApiError::conflict(format!(
                "Produkt {} není dostupný v požadovaném množství.",
                product.name
            )));
        }
        subtotal = subtotal
            .checked_add(product.price_cents * i64::from(item.quantity))
            .filter(|sum| *sum <= MAX_TOTAL)
            .ok_or_else(|| ApiError::bad_request("Objednávka překračuje maximální částku."))?;
    }
    let fee = delivery_fee(subtotal, &settings);
    let total = subtotal
        .checked_add(fee)
        .filter(|v| *v <= MAX_TOTAL)
        .ok_or_else(|| ApiError::bad_request("Objednávka překračuje maximální částku."))?;
    if total != input.expected_total_cents {
        return Err(ApiError::conflict(
            "Cena objednávky se změnila. Aktualizujte košík a potvrďte aktuální cenu.",
        ));
    }
    if online && total == 0 {
        return Err(ApiError::bad_request(
            "Objednávku s nulovou částkou potvrďte s platbou při doručení.",
        ));
    }
    // Counters share this transaction, so replays and rejected orders do not consume limits.
    for (key, limit) in [
        ("checkout:global".to_owned(), 500_i32),
        (format!("checkout:{}", auth::token_hash(&input.email)), 20),
    ] {
        let count: i32 = sqlx::query_scalar("INSERT INTO request_limits(key) VALUES ($1) ON CONFLICT(key) DO UPDATE SET requests=CASE WHEN request_limits.window_start < now()-interval '1 hour' THEN 1 ELSE request_limits.requests+1 END,window_start=CASE WHEN request_limits.window_start < now()-interval '1 hour' THEN now() ELSE request_limits.window_start END RETURNING requests")
            .bind(key).fetch_one(&mut *tx).await?;
        if count > limit {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Přijímání objednávek je dočasně omezeno. Zkuste to prosím později.",
            ));
        }
    }
    let id = Uuid::new_v4();
    let initial_status = if online { "awaiting_payment" } else { "new" };
    sqlx::query("INSERT INTO orders(id,idempotency_key,request_payload,user_id,email,name,phone,organization,street,city,postal_code,notes,delivery_date,delivery_slot,payment_method,subtotal_cents,delivery_fee_cents,total_cents,status) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)")
        .bind(id).bind(input.idempotency_key).bind(payload).bind(user_id).bind(&input.email)
        .bind(&input.name).bind(&input.phone).bind(&input.organization).bind(&input.street)
        .bind(&input.city).bind(&input.postal_code).bind(&input.notes).bind(input.delivery_date)
        .bind(&input.delivery_slot).bind(&input.payment_method).bind(subtotal).bind(fee).bind(total)
        .bind(initial_status)
        .execute(&mut *tx).await?;
    for item in &input.items {
        let product = &by_id[&item.product_id];
        sqlx::query("INSERT INTO order_items(order_id,product_id,name,quantity,unit_price_cents,total_cents,tracked_stock) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(id).bind(product.id).bind(&product.name).bind(item.quantity).bind(product.price_cents)
            .bind(product.price_cents * i64::from(item.quantity)).bind(product.stock.is_some()).execute(&mut *tx).await?;
        if product.stock.is_some() {
            sqlx::query(
                "UPDATE products SET stock=stock-$2,version=version+1,updated_at=now() WHERE id=$1",
            )
            .bind(product.id)
            .bind(item.quantity)
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query("INSERT INTO order_history(order_id,status,payment_status,changed_by) VALUES ($1,$2,'unpaid',$3)")
        .bind(id).bind(initial_status).bind(user_id).execute(&mut *tx).await?;
    if online {
        crate::payments::enqueue_configured(&mut tx, id, &state).await?;
    }
    let order = read_order(&mut tx, id, false).await?;
    let summary = order_summary(&order);
    if online {
        mail::enqueue(&mut tx, &order.email, &format!("Objednávka {} čeká na zaplacení", order.number),
            &format!("Objednávku jsme uložili a čekáme na potvrzení online platby. K vyřízení ji přijmeme až po úspěšné úhradě. Platbu dokončete v platební bráně otevřené z objednávky.\n\n{summary}\n\nPokud už jste zaplatili, další platbu neprovádějte a vyčkejte na potvrzení. S dotazy napište na {} a uveďte číslo objednávky.", settings.order_email)).await?;
    } else {
        mail::enqueue(
            &mut tx,
            &settings.order_email,
            &format!("Nová objednávka {}", order.number),
            &format!(
                "Nová objednávka k vyřízení.\n\n{summary}\n\nAdministrace: {}/admin/",
                state.config.public_url.trim_end_matches('/')
            ),
        )
        .await?;
        mail::enqueue(&mut tx, &order.email, &format!("Přijali jsme objednávku {}", order.number),
            &format!("Děkujeme za objednávku. Přijali jsme ji ke zpracování.\nZvolený čas doručení je požadované okno, které potvrdíme při vyřízení.\n\n{summary}\n\nPotřebujete něco změnit? Napište nám na {} a uveďte číslo objednávky.", settings.order_email)).await?;
    }
    tx.commit().await?;
    Ok(Json(order))
}

fn money(cents: i64) -> String {
    format!("{},{:02} Kč", cents / 100, cents % 100)
}

// The caller keeps restoration and the terminal order transition in one transaction.
async fn restore_order_stock(conn: &mut PgConnection, id: Uuid) -> ApiResult<()> {
    let restored: bool =
        sqlx::query_scalar("SELECT stock_restored FROM orders WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    if restored {
        return Ok(());
    }
    // Restore only quantities deducted at checkout. Unlimited stock stays unlimited.
    let items: Vec<(i64, i32)> = sqlx::query_as("SELECT product_id,quantity FROM order_items WHERE order_id=$1 AND tracked_stock ORDER BY product_id")
        .bind(id).fetch_all(&mut *conn).await?;
    for (product_id, quantity) in items {
        let stock: Option<i32> =
            sqlx::query_scalar("SELECT stock FROM products WHERE id=$1 FOR UPDATE")
                .bind(product_id)
                .fetch_one(&mut *conn)
                .await?;
        if let Some(stock) = stock {
            if stock > MAX_STOCK - quantity {
                return Err(ApiError::conflict(
                    "Zásoba po vrácení překračuje limit. Upravte skladové množství.",
                ));
            }
            sqlx::query(
                "UPDATE products SET stock=stock+$2,version=version+1,updated_at=now() WHERE id=$1",
            )
            .bind(product_id)
            .bind(quantity)
            .execute(&mut *conn)
            .await?;
        }
    }
    sqlx::query("UPDATE orders SET stock_restored=true WHERE id=$1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

// Called in the maintenance transaction after its deadline check and payment-attempt lock.
pub(crate) async fn expire_unpaid_order(conn: &mut PgConnection, id: Uuid) -> ApiResult<bool> {
    let Some(order) = sqlx::query_as::<_, Order>("SELECT * FROM orders WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(false);
    };
    if order.payment_method != "card"
        || order.status != "awaiting_payment"
        || order.payment_status != "unpaid"
    {
        return Ok(false);
    }
    let has_receipt: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payment_receipts WHERE order_id=$1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    if has_receipt {
        return Ok(false);
    }
    restore_order_stock(conn, id).await?;
    sqlx::query("UPDATE orders SET status='expired',stock_restored=true,version=version+1,updated_at=now() WHERE id=$1")
        .bind(id).execute(&mut *conn).await?;
    sqlx::query(
        "INSERT INTO order_history(order_id,status,payment_status) VALUES ($1,'expired','unpaid')",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let order_email: String =
        sqlx::query_scalar("SELECT order_email FROM shop_settings WHERE id=1")
            .fetch_one(&mut *conn)
            .await?;
    mail::enqueue(conn, &order.email, &format!("Objednávka {}: Vypršela", order.number),
        &format!("Vypršela lhůta pro online zaplacení objednávky {}. Objednávku už nepřijmeme k vyřízení a rezervované zboží jsme uvolnili.\n\nPokud jste již zaplatili, další platbu neprovádějte. Napište na {} a uveďte číslo objednávky, abychom mohli přijatou platbu prověřit.\n\nPokud jste nezaplatili a máte o zboží zájem, vytvořte novou objednávku.", order.number, order_email)).await?;
    Ok(true)
}

// The caller must hold a transaction containing the verified payment record.
// Repeated provider notifications cannot produce duplicate confirmations.
pub(crate) async fn payment_received(conn: &mut PgConnection, id: Uuid) -> ApiResult<()> {
    let old: Order = sqlx::query_as("SELECT * FROM orders WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| ApiError::not_found("Objednávka neexistuje."))?;
    if old.payment_method != "card" {
        return Err(ApiError::conflict(
            "Objednávka není určena k online platbě.",
        ));
    }
    let receipt_matches: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM payment_receipts WHERE order_id=$1 AND amount_cents=$2 AND currency='CZK')",
    )
    .bind(id)
    .bind(old.total_cents)
    .fetch_one(&mut *conn)
    .await?;
    if !receipt_matches {
        return Err(ApiError::conflict(
            "Chybí ověřený záznam platby odpovídající částce objednávky.",
        ));
    }
    if old.payment_status == "paid"
        && matches!(
            old.status.as_str(),
            "new" | "confirmed" | "preparing" | "out_for_delivery" | "delivered"
        )
    {
        return Ok(());
    }
    if old.status != "awaiting_payment" || old.payment_status != "unpaid" {
        return Err(ApiError::conflict(
            "Platbu nelze přiřadit k aktuálnímu stavu objednávky. Je nutná kontrola obsluhou.",
        ));
    }
    sqlx::query("UPDATE orders SET status='new',payment_status='paid',version=version+1,updated_at=now() WHERE id=$1")
        .bind(id).execute(&mut *conn).await?;
    sqlx::query(
        "INSERT INTO order_history(order_id,status,payment_status) VALUES ($1,'new','paid')",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let order = read_order(conn, id, false).await?;
    let order_email: String =
        sqlx::query_scalar("SELECT order_email FROM shop_settings WHERE id=1")
            .fetch_one(&mut *conn)
            .await?;
    let summary = order_summary(&order);
    mail::enqueue(conn, &order_email, &format!("Zaplacená objednávka {} k vyřízení", order.number),
        &format!("Online platba byla ověřena. Objednávka je přijata k vyřízení. Při doručení již nic nevybírejte.\n\n{summary}")).await?;
    mail::enqueue(conn, &order.email, &format!("Přijali jsme platbu a objednávku {}", order.number),
        &format!("Děkujeme, online platba byla přijata a objednávku jsme přijali ke zpracování. Při doručení již nic neplatíte.\nZvolený čas doručení je požadované okno, které potvrdíme při vyřízení.\n\n{summary}\n\nPotřebujete něco změnit? Napište nám na {order_email} a uveďte číslo objednávky.")).await?;
    Ok(())
}

fn order_summary(order: &Order) -> String {
    let lines = order
        .items
        .iter()
        .map(|i| {
            format!(
                "{} × {} | {} / ks | {}",
                i.quantity,
                i.name,
                money(i.unit_price_cents),
                money(i.total_cents)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let payment = match order.payment_method.as_str() {
        "card" => "Online kartou",
        "card_on_delivery" => "Kartou při doručení",
        _ => "Hotově při doručení",
    };
    let total_label = if order.payment_status == "paid" {
        "Celkem uhrazeno"
    } else if order.payment_method == "card" {
        "Celkem k úhradě online"
    } else {
        "Celkem k úhradě při doručení"
    };
    format!(
        "Objednávka: {}\n\n{}\n\nZboží: {}\nDoprava: {}\n{}: {}\nPlatba: {}\n\nDoručení: {} {}\nJméno: {}\nFirma: {}\nAdresa: {}, {} {}\nE-mail: {}\nTelefon: {}\nPokyny k doručení: {}",
        order.number,
        lines,
        money(order.subtotal_cents),
        money(order.delivery_fee_cents),
        total_label,
        money(order.total_cents),
        payment,
        order.delivery_date,
        order.delivery_slot,
        order.name,
        order.organization,
        order.street,
        order.postal_code,
        order.city,
        order.email,
        order.phone,
        order.notes
    )
}

async fn my_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Order>>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    let mut orders =
        sqlx::query_as("SELECT * FROM orders WHERE user_id=$1 ORDER BY created_at DESC LIMIT 100")
            .bind(user.id)
            .fetch_all(&mut *conn)
            .await?;
    attach_items(&mut conn, &mut orders).await?;
    Ok(Json(orders))
}

#[derive(Deserialize, Default)]
struct PageQuery {
    page: Option<i64>,
    status: Option<String>,
    q: Option<String>,
}

fn page_number(page: Option<i64>) -> ApiResult<i64> {
    match page.unwrap_or(1) {
        page @ 1..=1_000_000 => Ok(page),
        _ => Err(ApiError::bad_request("Neplatná stránka.")),
    }
}

async fn admin_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_admin(&state, &headers).await?;
    let page = page_number(query.page)?;
    let status = query.status.filter(|s| !s.is_empty());
    if status
        .as_ref()
        .is_some_and(|s| !STATUSES.contains(&s.as_str()))
    {
        return Err(ApiError::bad_request("Neplatný stav objednávky."));
    }
    let q = clean(query.q.as_deref().unwrap_or(""), "hledání", 200, false)?;
    // Literal matching keeps wildcard characters from expanding arbitrary searches.
    let q = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut conn = state.pool.acquire().await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM orders WHERE ($1::text IS NULL OR status=$1) AND (number ILIKE $2 OR email ILIKE $2 OR name ILIKE $2 OR street ILIKE $2 OR city ILIKE $2)")
        .bind(&status).bind(&q).fetch_one(&mut *conn).await?;
    let mut orders = sqlx::query_as("SELECT * FROM orders WHERE ($1::text IS NULL OR status=$1) AND (number ILIKE $2 OR email ILIKE $2 OR name ILIKE $2 OR street ILIKE $2 OR city ILIKE $2) ORDER BY created_at DESC,id DESC LIMIT 50 OFFSET $3")
        .bind(&status).bind(&q).bind((page - 1) * 50).fetch_all(&mut *conn).await?;
    attach_items(&mut conn, &mut orders).await?;
    Ok(Json(json!({"orders":orders,"total":total,"page":page})))
}

async fn admin_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Order>> {
    auth::require_admin(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    Ok(Json(read_order(&mut conn, id, true).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderUpdate {
    status: String,
    payment_status: String,
    version: i64,
}

fn validate_transition(
    payment_method: &str,
    old_status: &str,
    old_payment: &str,
    input: &OrderUpdate,
) -> ApiResult<()> {
    if !STATUSES.contains(&input.status.as_str())
        || !matches!(input.payment_status.as_str(), "unpaid" | "paid")
    {
        return Err(ApiError::bad_request(
            "Neplatný stav objednávky nebo platby.",
        ));
    }
    if matches!(old_status, "cancelled" | "delivered" | "expired") {
        return Err(ApiError::conflict(
            "Dokončenou, zrušenou nebo vypršelou objednávku nelze měnit.",
        ));
    }
    if input.status == "expired" {
        return Err(ApiError::conflict(
            "Vypršení online objednávky nastavuje systém po uplynutí lhůty k zaplacení.",
        ));
    }
    if payment_method == "card" && old_payment != input.payment_status {
        return Err(ApiError::conflict(
            "Stav online platby může potvrdit pouze platební brána.",
        ));
    }
    if old_payment == "paid" && input.payment_status != "paid" {
        return Err(ApiError::conflict(
            "Uhrazenou objednávku nelze označit za neuhrazenou bez vrácení platby.",
        ));
    }
    if input.status == "cancelled" {
        if old_payment == "paid" || input.payment_status == "paid" {
            return Err(ApiError::conflict(
                "Uhrazenou objednávku nelze zrušit bez vyřešení vrácení platby.",
            ));
        }
        return Ok(());
    }
    if old_status == "awaiting_payment" {
        if payment_method == "card" && old_payment == "unpaid" && input.status == "awaiting_payment"
        {
            return Ok(());
        }
        return Err(ApiError::conflict(
            "Objednávku lze přijmout k vyřízení až po potvrzení online platby.",
        ));
    }
    if payment_method == "card" && old_payment != "paid" {
        return Err(ApiError::conflict(
            "Objednávku lze vyřizovat až po potvrzení online platby.",
        ));
    }
    let next = match old_status {
        "new" => "confirmed",
        "confirmed" => "preparing",
        "preparing" => "out_for_delivery",
        "out_for_delivery" => "delivered",
        _ => return Err(ApiError::conflict("Neplatný přechod stavu.")),
    };
    if input.status != old_status && input.status != next {
        return Err(ApiError::conflict(
            "Stavy objednávky je nutné měnit postupně.",
        ));
    }
    if old_payment != "paid"
        && input.payment_status == "paid"
        && input.status != "out_for_delivery"
        && input.status != "delivered"
    {
        return Err(ApiError::conflict(
            "Platbu při doručení lze potvrdit až během rozvozu.",
        ));
    }
    if input.status == "delivered" && input.payment_status != "paid" {
        return Err(ApiError::conflict(
            "Při dokončení doručení potvrďte také přijetí platby.",
        ));
    }
    Ok(())
}

fn status_label(status: &str) -> &'static str {
    match status {
        "awaiting_payment" => "Čeká na zaplacení",
        "confirmed" => "Potvrzena",
        "preparing" => "Připravujeme",
        "out_for_delivery" => "Na cestě",
        "delivered" => "Doručena",
        "cancelled" => "Zrušena",
        "expired" => "Vypršela",
        _ => "Přijata",
    }
}

async fn update_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<OrderUpdate>,
) -> ApiResult<Json<Order>> {
    let admin = auth::require_admin(&state, &headers).await?;
    let mut tx = state.pool.begin().await?;
    let old: Order = sqlx::query_as("SELECT * FROM orders WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Objednávka neexistuje."))?;
    if old.version != input.version {
        return Err(ApiError::conflict(
            "Objednávka se mezitím změnila. Načtěte aktuální údaje.",
        ));
    }
    validate_transition(
        &old.payment_method,
        &old.status,
        &old.payment_status,
        &input,
    )?;
    if old.status == input.status && old.payment_status == input.payment_status {
        let order = read_order(&mut tx, id, true).await?;
        tx.commit().await?;
        return Ok(Json(order));
    }
    if input.status == "cancelled" {
        if old.payment_method == "card" && !crate::payments::can_cancel(&mut tx, id).await? {
            return Err(ApiError::conflict(
                "Online platba ještě může být dokončena. Objednávku lze zrušit až po ověřeném ukončení platby.",
            ));
        }
        restore_order_stock(&mut tx, id).await?;
    }
    sqlx::query("UPDATE orders SET status=$2,payment_status=$3,version=version+1,updated_at=now(),stock_restored=($2='cancelled') WHERE id=$1")
        .bind(id).bind(&input.status).bind(&input.payment_status).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO order_history(order_id,status,payment_status,changed_by) VALUES ($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(&input.status)
    .bind(&input.payment_status)
    .bind(admin.id)
    .execute(&mut *tx)
    .await?;
    let order = read_order(&mut tx, id, true).await?;
    if old.status != order.status {
        let label = status_label(&order.status);
        let payment_note = if order.payment_status == "paid" {
            "Platba byla přijata."
        } else if order.status == "cancelled" {
            "Objednávka byla zrušena. Nic neplaťte."
        } else {
            "Platíte až při doručení."
        };
        mail::enqueue(&mut tx, &order.email, &format!("Objednávka {}: {}", order.number, label),
            &format!("Vaše objednávka {} má nový stav: {}.\n{}\nPožadované doručení: {} {}\nCelková částka: {}\n\nDěkujeme.",order.number,label,payment_note,order.delivery_date,order.delivery_slot,money(order.total_cents))).await?;
    }
    tx.commit().await?;
    Ok(Json(order))
}

#[derive(Serialize, FromRow)]
struct Customer {
    id: Uuid,
    email: String,
    is_admin: bool,
    created_at: DateTime<Utc>,
    order_count: i64,
}

async fn customers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_admin(&state, &headers).await?;
    let page = page_number(query.page)?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&state.pool)
        .await?;
    let customers: Vec<Customer> = sqlx::query_as("SELECT u.id,u.email,u.is_admin,u.created_at,(SELECT count(*) FROM orders o WHERE o.user_id=u.id) AS order_count FROM users u ORDER BY u.created_at DESC,u.id DESC LIMIT 50 OFFSET $1")
        .bind((page - 1) * 50).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"customers":customers,"total":total,"page":page}),
    ))
}

pub async fn seed(pool: &PgPool, order_email: &str) -> ApiResult<()> {
    let order_email = email(order_email)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(541847121)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO shop_settings(id,delivery_fee_cents,free_delivery_from_cents,order_email) VALUES (1,4900,35000,$1) ON CONFLICT (id) DO NOTHING")
        .bind(order_email).execute(&mut *tx).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM products")
        .fetch_one(&mut *tx)
        .await?;
    if count == 0 {
        let products = [
            (
                "Avokádový kváskový sendvič",
                "Cottage, křupavá ředkvička a pořádná porce avokáda.",
                "Slané",
                "Z naší pece",
                "180 g",
                11900_i64,
                false,
                "Celozrnný kváskový chléb, avokádo, cottage, ředkvička, rukola, citron, semínka.",
                "Lepek (pšenice, žito), mléko, sezam.",
            ),
            (
                "Jogurt, granola & lesní ovoce",
                "Bílý jogurt, naše pečená granola a šťavnaté ovoce.",
                "Sladké",
                "Ranní radost",
                "220 g",
                8900,
                false,
                "Bílý jogurt, ovesné vločky, med, mandle, maliny, borůvky.",
                "Mléko, lepek (oves), skořápkové plody (mandle).",
            ),
            (
                "Hummus bowl se zeleninou",
                "Jemný hummus, pečená cizrna a čerstvá zelenina.",
                "Bowls",
                "Rostlinné",
                "250 g",
                12900,
                true,
                "Cizrna, sezamová pasta tahini, olivový olej, citron, okurka, mrkev, listový salát.",
                "Sezam.",
            ),
            (
                "Banánový chlebíček",
                "Vláčný, voňavý po banánech. S vlašskými ořechy.",
                "Sladké",
                "Z naší pece",
                "100 g",
                6500,
                true,
                "Banány, pšeničná mouka, ovesný nápoj, vlašské ořechy, řepkový olej, třtinový cukr, skořice.",
                "Lepek (pšenice, oves), skořápkové plody (vlašské ořechy).",
            ),
            (
                "Celozrnný bagel s lučinou",
                "Čerstvý sýr, okurka a klíčky v měkkém bagelu.",
                "Slané",
                "Z naší pece",
                "160 g",
                9900,
                false,
                "Celozrnný pšeničný bagel, čerstvý sýr, okurka, klíčky, sezam a lněné semínko.",
                "Lepek (pšenice), mléko, sezam.",
            ),
            (
                "Pomeranč, mrkev & zázvor",
                "Svěží ovocná šťáva s malým zázvorovým probuzením.",
                "Nápoje",
                "Svěží doušek",
                "250 ml",
                6900,
                true,
                "Pomerančová šťáva, mrkvová šťáva, zázvor, citronová šťáva.",
                "Bez alergenů v ukázkové receptuře.",
            ),
        ];
        for (name, description, category, tag, weight, price, vegan, ingredients, allergens) in
            products
        {
            let id: i64 = sqlx::query_scalar("INSERT INTO products(name,description,category,tag,weight,price_cents,vegan,ingredients,allergens) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
                .bind(name).bind(description).bind(category).bind(tag).bind(weight).bind(price).bind(vegan)
                .bind(ingredients).bind(allergens).fetch_one(&mut *tx).await?;
            sqlx::query("INSERT INTO product_prices(product_id,price_cents) VALUES ($1,$2)")
                .bind(id)
                .bind(price)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postal_codes_normalize_whitespace_and_bom_but_require_ascii_digits() {
        for input in [
            "11000",
            " 110 00 ",
            "\t11\n0\r00",
            "\u{a0}110\u{202f}00",
            "\u{feff}11\u{feff}000\u{feff}",
        ] {
            assert_eq!(normalize_postal_code(input).unwrap(), "11000");
        }
        for input in [
            "",
            "1100",
            "110000",
            "11-000",
            "11a00",
            "１１０００",
            "١١٠٠٠",
            "11\u{200b}000",
        ] {
            assert!(
                normalize_postal_code(input).is_err(),
                "accepted invalid postal code {input:?}"
            );
        }
    }

    #[test]
    fn delivery_area_normalization_preserves_default_flags_and_limits_labels() {
        let mut input: DeliveryAreaInput =
            serde_json::from_value(json!({"postal_code":"\u{feff}110 00"})).unwrap();
        input.normalize().unwrap();
        assert_eq!(input.postal_code, "11000");
        assert!(input.label.is_empty());
        assert!(input.active);
        input.label = "a".repeat(101);
        assert!(input.normalize().is_err());
    }

    #[test]
    fn delivery_skips_weekends_and_czech_holidays() {
        let days = next_dates(NaiveDate::from_ymd_opt(2026, 4, 2).unwrap());
        assert_eq!(days[0], NaiveDate::from_ymd_opt(2026, 4, 7).unwrap());
        let christmas = next_dates(NaiveDate::from_ymd_opt(2026, 12, 23).unwrap());
        assert_eq!(christmas[0], NaiveDate::from_ymd_opt(2026, 12, 28).unwrap());
    }

    #[test]
    fn shipping_threshold_is_inclusive() {
        let settings = Settings {
            delivery_fee_cents: 4900,
            free_delivery_from_cents: Some(35000),
            order_email: String::new(),
            version: 1,
        };
        assert_eq!(delivery_fee(34999, &settings), 4900);
        assert_eq!(delivery_fee(35000, &settings), 0);
        assert_eq!(delivery_fee(35001, &settings), 0);
    }

    #[test]
    fn delivery_payment_and_terminal_states_are_enforced() {
        let update = |status: &str, payment: &str| OrderUpdate {
            status: status.to_owned(),
            payment_status: payment.to_owned(),
            version: 1,
        };
        assert!(
            validate_transition(
                "cash_on_delivery",
                "new",
                "unpaid",
                &update("confirmed", "unpaid")
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "new",
                "unpaid",
                &update("delivered", "paid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "confirmed",
                "unpaid",
                &update("confirmed", "paid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "out_for_delivery",
                "unpaid",
                &update("delivered", "unpaid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "out_for_delivery",
                "unpaid",
                &update("delivered", "paid")
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "preparing",
                "unpaid",
                &update("cancelled", "unpaid")
            )
            .is_ok()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "out_for_delivery",
                "paid",
                &update("cancelled", "unpaid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "delivered",
                "paid",
                &update("cancelled", "unpaid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "cash_on_delivery",
                "cancelled",
                "unpaid",
                &update("new", "unpaid")
            )
            .is_err()
        );
    }

    #[test]
    fn online_orders_wait_for_verified_payment_and_admin_cannot_record_it() {
        let update = |status: &str, payment: &str| OrderUpdate {
            status: status.to_owned(),
            payment_status: payment.to_owned(),
            version: 1,
        };
        assert!(
            validate_transition(
                "card",
                "awaiting_payment",
                "unpaid",
                &update("awaiting_payment", "unpaid")
            )
            .is_ok()
        );
        for status in [
            "new",
            "confirmed",
            "preparing",
            "out_for_delivery",
            "delivered",
        ] {
            for payment in ["unpaid", "paid"] {
                assert!(
                    validate_transition(
                        "card",
                        "awaiting_payment",
                        "unpaid",
                        &update(status, payment)
                    )
                    .is_err(),
                    "admin advanced unpaid online order to {status}/{payment}"
                );
            }
        }
        assert!(
            validate_transition(
                "card",
                "out_for_delivery",
                "unpaid",
                &update("delivered", "paid")
            )
            .is_err()
        );
        assert!(
            validate_transition(
                "card",
                "awaiting_payment",
                "unpaid",
                &update("awaiting_payment", "paid")
            )
            .is_err()
        );
        // Cancellation requires an additional provider-terminal check in update_order.
        assert!(
            validate_transition(
                "card",
                "awaiting_payment",
                "unpaid",
                &update("cancelled", "unpaid")
            )
            .is_ok()
        );
    }

    #[test]
    fn online_payment_stays_paid_while_order_advances() {
        for (before, after) in [
            ("new", "confirmed"),
            ("confirmed", "preparing"),
            ("preparing", "out_for_delivery"),
            ("out_for_delivery", "delivered"),
        ] {
            let mut update = OrderUpdate {
                status: after.to_owned(),
                payment_status: "paid".to_owned(),
                version: 1,
            };
            assert!(validate_transition("card", before, "paid", &update).is_ok());
            update.payment_status = "unpaid".to_owned();
            assert!(validate_transition("card", before, "paid", &update).is_err());
            update.status = "cancelled".to_owned();
            update.payment_status = "paid".to_owned();
            assert!(validate_transition("card", before, "paid", &update).is_err());
        }
    }

    #[test]
    fn delivery_orders_cannot_enter_online_waiting_state() {
        let update = OrderUpdate {
            status: "awaiting_payment".to_owned(),
            payment_status: "unpaid".to_owned(),
            version: 1,
        };
        for payment_method in ["cash_on_delivery", "card_on_delivery"] {
            assert!(validate_transition(payment_method, "new", "unpaid", &update).is_err());
        }
    }

    #[test]
    fn expired_online_orders_are_terminal_for_every_admin_transition() {
        for status in STATUSES {
            for payment_status in ["unpaid", "paid"] {
                let update = OrderUpdate {
                    status: status.to_owned(),
                    payment_status: payment_status.to_owned(),
                    version: 1,
                };
                assert!(
                    validate_transition("card", "expired", "unpaid", &update).is_err(),
                    "expired order changed to {status}/{payment_status}"
                );
            }
        }
        assert_eq!(status_label("expired"), "Vypršela");
    }

    #[test]
    fn only_maintenance_can_expire_an_online_order() {
        let update = OrderUpdate {
            status: "expired".to_owned(),
            payment_status: "unpaid".to_owned(),
            version: 1,
        };
        assert!(validate_transition("card", "awaiting_payment", "unpaid", &update).is_err());
        for payment_method in ["cash_on_delivery", "card_on_delivery"] {
            for before in ["new", "confirmed", "preparing", "out_for_delivery"] {
                assert!(
                    validate_transition(payment_method, before, "unpaid", &update).is_err(),
                    "delivery order expired from {before}"
                );
            }
        }
    }

    #[test]
    fn duplicate_cart_items_and_invalid_addresses_are_rejected() {
        let mut input: OrderInput = serde_json::from_value(json!({"idempotency_key":Uuid::new_v4(),"email":"Guest@Example.cz ","street":"Test 1","city":"Praha","postal_code":"110 00","delivery_date":"2026-10-22","delivery_slot":SLOTS[0],"payment_method":"cash_on_delivery","items":[{"product_id":1,"quantity":2}],"expected_total_cents":23800})).unwrap();
        assert!(input.normalize().is_ok());
        assert_eq!(input.email, "guest@example.cz");
        assert_eq!(input.postal_code, "11000");
        input.payment_method = "card".to_owned();
        assert!(input.normalize().is_ok());
        input.items.push(CartItem {
            product_id: 1,
            quantity: 1,
        });
        assert!(input.normalize().is_err());
        input.items.pop();
        input.postal_code = "abcde".to_owned();
        assert!(input.normalize().is_err());
    }
}
