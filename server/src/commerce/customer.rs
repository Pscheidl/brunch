//! Authenticated customer self-service. Every order is resolved through its owner.
use super::{Order, attach_items, clean, normalize_postal_code};
use crate::{ApiError, ApiResult, AppState, auth, mail};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, patch, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/customer/addresses",
            get(addresses).post(create_address),
        )
        .route(
            "/api/customer/addresses/{id}",
            put(update_address).delete(delete_address),
        )
        .route(
            "/api/customer/billing",
            get(profile_billing).put(update_profile_billing),
        )
        .route("/api/customer/orders", get(orders))
        .route("/api/customer/orders/{id}/invoice", get(invoice))
        .route(
            "/api/customer/orders/{id}/billing",
            get(order_billing).put(update_order_billing),
        )
        .route(
            "/api/customer/orders/{id}/requests",
            get(order_requests).post(create_request),
        )
        .route("/api/admin/orders/{id}/billing", get(admin_order_billing))
        .route("/api/admin/customer-requests", get(admin_requests))
        .route("/api/admin/customer-requests/{id}", patch(resolve_request))
}

#[derive(Serialize, FromRow)]
struct Address {
    id: Uuid,
    label: String,
    name: String,
    phone: String,
    organization: String,
    street: String,
    city: String,
    postal_code: String,
    notes: String,
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressInput {
    label: String,
    name: String,
    phone: String,
    #[serde(default)]
    organization: String,
    street: String,
    city: String,
    postal_code: String,
    #[serde(default)]
    notes: String,
    version: Option<i64>,
}

impl AddressInput {
    fn normalize(&mut self) -> ApiResult<()> {
        self.label = clean(&self.label, "název místa", 80, true)?;
        self.name = clean(&self.name, "jméno příjemce", 200, true)?;
        self.phone = clean(&self.phone, "telefon", 40, true)?;
        if !self.phone.chars().any(|c| c.is_ascii_digit()) {
            return Err(ApiError::bad_request("Zadejte telefon pro kurýra."));
        }
        self.organization = clean(&self.organization, "firma", 200, false)?;
        self.street = clean(&self.street, "ulice a číslo", 250, true)?;
        self.city = clean(&self.city, "město", 100, true)?;
        self.postal_code = normalize_postal_code(&self.postal_code)?;
        self.notes = clean(&self.notes, "pokyny", 2000, false)?;
        Ok(())
    }
}

fn address_error(error: sqlx::Error) -> ApiError {
    if matches!(&error, sqlx::Error::Database(db) if db.is_unique_violation()) {
        ApiError::conflict("Místo s tímto názvem už máte uložené.")
    } else {
        error.into()
    }
}

async fn addresses(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Address>>> {
    let user = auth::require_user(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM customer_addresses WHERE user_id=$1 ORDER BY label,id")
            .bind(user.id)
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create_address(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<AddressInput>,
) -> ApiResult<Json<Address>> {
    let user = auth::require_user(&state, &headers).await?;
    input.normalize()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM customer_addresses WHERE user_id=$1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= 20 {
        return Err(ApiError::bad_request(
            "Uložit lze nejvýše 20 míst doručení.",
        ));
    }
    let label_key = input.label.to_lowercase();
    let address = sqlx::query_as("INSERT INTO customer_addresses (id,user_id,label,name,phone,organization,street,city,postal_code,notes,label_key) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *")
        .bind(Uuid::new_v4()).bind(user.id).bind(input.label).bind(input.name).bind(input.phone).bind(input.organization).bind(input.street).bind(input.city).bind(input.postal_code).bind(input.notes).bind(label_key)
        .fetch_one(&mut *tx).await.map_err(address_error)?;
    tx.commit().await?;
    Ok(Json(address))
}

async fn update_address(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut input): Json<AddressInput>,
) -> ApiResult<Json<Address>> {
    let user = auth::require_user(&state, &headers).await?;
    input.normalize()?;
    let label_key = input.label.to_lowercase();
    let address = sqlx::query_as("UPDATE customer_addresses SET label=$1,name=$2,phone=$3,organization=$4,street=$5,city=$6,postal_code=$7,notes=$8,label_key=$12,version=version+1,updated_at=now() WHERE id=$9 AND user_id=$10 AND version=$11 RETURNING *")
        .bind(input.label).bind(input.name).bind(input.phone).bind(input.organization).bind(input.street).bind(input.city).bind(input.postal_code).bind(input.notes).bind(id).bind(user.id).bind(input.version).bind(label_key)
        .fetch_optional(&state.pool).await.map_err(address_error)?
        .ok_or_else(|| ApiError::conflict("Místo už není dostupné nebo se změnilo. Načtěte adresář znovu."))?;
    Ok(Json(address))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionInput {
    version: i64,
}

async fn delete_address(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<VersionInput>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let result =
        sqlx::query("DELETE FROM customer_addresses WHERE id=$1 AND user_id=$2 AND version=$3")
            .bind(id)
            .bind(user.id)
            .bind(input.version)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Místo už není dostupné nebo se změnilo. Načtěte adresář znovu.",
        ));
    }
    Ok(Json(json!({"deleted":true})))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BillingInput {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub company_id: String,
    #[serde(default)]
    pub tax_id: String,
}

impl BillingInput {
    fn normalized(&self) -> ApiResult<Self> {
        Ok(Self {
            name: clean(&self.name, "odběratel", 250, true)?,
            address: clean(&self.address, "fakturační adresa", 500, true)?,
            company_id: clean(&self.company_id, "IČO", 24, false)?,
            tax_id: clean(&self.tax_id, "DIČ", 30, false)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BillingUpdate {
    name: String,
    address: String,
    #[serde(default)]
    company_id: String,
    #[serde(default)]
    tax_id: String,
    // Zero means creating the first record, a positive version means updating it.
    version: i64,
}
impl BillingUpdate {
    fn billing(&self) -> ApiResult<BillingInput> {
        BillingInput {
            name: self.name.clone(),
            address: self.address.clone(),
            company_id: self.company_id.clone(),
            tax_id: self.tax_id.clone(),
        }
        .normalized()
    }
}
#[derive(Serialize, FromRow)]
struct Billing {
    name: String,
    address: String,
    company_id: String,
    tax_id: String,
    version: i64,
}

pub(super) async fn save_checkout_billing(
    conn: &mut PgConnection,
    order_id: Uuid,
    billing: Option<&BillingInput>,
) -> ApiResult<()> {
    if let Some(billing) = billing {
        let billing = billing.normalized()?;
        sqlx::query("INSERT INTO order_billing (order_id,name,address,company_id,tax_id) VALUES ($1,$2,$3,$4,$5)")
            .bind(order_id).bind(billing.name).bind(billing.address).bind(billing.company_id).bind(billing.tax_id).execute(conn).await?;
    }
    Ok(())
}

async fn profile_billing(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Option<Billing>>> {
    let user = auth::require_user(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM customer_billing WHERE user_id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?,
    ))
}
async fn update_profile_billing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BillingUpdate>,
) -> ApiResult<Json<Billing>> {
    let user = auth::require_user(&state, &headers).await?;
    let data = input.billing()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let current: Option<i64> =
        sqlx::query_scalar("SELECT version FROM customer_billing WHERE user_id=$1")
            .bind(user.id)
            .fetch_optional(&mut *tx)
            .await?;
    if current.unwrap_or(0) != input.version {
        return Err(ApiError::conflict(
            "Fakturační údaje se změnily. Načtěte je znovu.",
        ));
    }
    let saved=sqlx::query_as("INSERT INTO customer_billing (user_id,name,address,company_id,tax_id) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (user_id) DO UPDATE SET name=$2,address=$3,company_id=$4,tax_id=$5,version=customer_billing.version+1 RETURNING *")
        .bind(user.id).bind(data.name).bind(data.address).bind(data.company_id).bind(data.tax_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(saved))
}
async fn owned_order(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
    lock: bool,
) -> ApiResult<(String, String)> {
    let query = if lock {
        "SELECT number,status FROM orders WHERE id=$1 AND user_id=$2 FOR UPDATE"
    } else {
        "SELECT number,status FROM orders WHERE id=$1 AND user_id=$2"
    };
    sqlx::query_as(query)
        .bind(id)
        .bind(user_id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("Objednávka není dostupná."))
}
async fn order_billing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Option<Billing>>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    owned_order(&mut conn, user.id, id, false).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM order_billing WHERE order_id=$1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    ))
}
async fn admin_order_billing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Option<Billing>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM order_billing WHERE order_id=$1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?,
    ))
}
async fn update_order_billing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<BillingUpdate>,
) -> ApiResult<Json<Billing>> {
    let user = auth::require_user(&state, &headers).await?;
    let data = input.billing()?;
    let mut tx = state.pool.begin().await?;
    owned_order(&mut tx, user.id, id, true).await?;
    let issued: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invoices WHERE order_id=$1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if issued {
        return Err(ApiError::conflict(
            "Faktura už byla vystavena. O opravu požádejte obsluhu.",
        ));
    }
    let current: Option<i64> =
        sqlx::query_scalar("SELECT version FROM order_billing WHERE order_id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if current.unwrap_or(0) != input.version {
        return Err(ApiError::conflict(
            "Fakturační údaje se změnily. Načtěte je znovu.",
        ));
    }
    let saved=sqlx::query_as("INSERT INTO order_billing (order_id,name,address,company_id,tax_id) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (order_id) DO UPDATE SET name=$2,address=$3,company_id=$4,tax_id=$5,version=order_billing.version+1 RETURNING *")
        .bind(id).bind(data.name).bind(data.address).bind(data.company_id).bind(data.tax_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(saved))
}
async fn invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Option<Value>>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    owned_order(&mut conn, user.id, id, false).await?;
    Ok(Json(
        sqlx::query_scalar("SELECT document FROM invoices WHERE order_id=$1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    ))
}

#[derive(Deserialize, Default)]
struct OrderQuery {
    page: Option<i64>,
    q: Option<String>,
}
async fn orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OrderQuery>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let page = super::page_number(query.page)?;
    let search = format!(
        "%{}%",
        clean(query.q.as_deref().unwrap_or(""), "hledání", 100, false)?
    );
    let mut conn = state.pool.acquire().await?;
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM orders WHERE user_id=$1 AND (number ILIKE $2 OR street ILIKE $2 OR organization ILIKE $2)")
        .bind(user.id).bind(&search).fetch_one(&mut *conn).await?;
    let mut items:Vec<Order>=sqlx::query_as("SELECT * FROM orders WHERE user_id=$1 AND (number ILIKE $2 OR street ILIKE $2 OR organization ILIKE $2) ORDER BY created_at DESC,id DESC LIMIT 20 OFFSET $3")
        .bind(user.id).bind(search).bind((page-1)*20).fetch_all(&mut *conn).await?;
    attach_items(&mut conn, &mut items).await?;
    let mut active:Vec<Order>=sqlx::query_as("SELECT * FROM orders WHERE user_id=$1 AND status IN ('awaiting_payment','new','confirmed','out_for_delivery') ORDER BY delivery_date,delivery_slot,created_at")
        .bind(user.id).fetch_all(&mut *conn).await?;
    attach_items(&mut conn, &mut active).await?;
    Ok(Json(
        json!({"items":items,"active":active,"total":total,"page":page}),
    ))
}

#[derive(Serialize, FromRow)]
struct CustomerRequest {
    id: Uuid,
    order_id: Uuid,
    order_number: String,
    email: String,
    kind: String,
    message: String,
    status: String,
    response: String,
    version: i64,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
}
const REQUEST_SELECT: &str = "SELECT r.id,r.order_id,o.number AS order_number,o.email,r.kind,r.message,r.status,r.response,r.version,r.created_at,r.resolved_at FROM customer_order_requests r JOIN orders o ON o.id=r.order_id";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestInput {
    idempotency_key: Uuid,
    kind: String,
    message: String,
}
async fn order_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<CustomerRequest>>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    owned_order(&mut conn, user.id, id, false).await?;
    Ok(Json(
        sqlx::query_as(&format!(
            "{REQUEST_SELECT} WHERE r.order_id=$1 AND r.user_id=$2 ORDER BY r.created_at DESC"
        ))
        .bind(id)
        .bind(user.id)
        .fetch_all(&mut *conn)
        .await?,
    ))
}
async fn create_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<RequestInput>,
) -> ApiResult<Json<CustomerRequest>> {
    let user = auth::require_user(&state, &headers).await?;
    let message = clean(&input.message, "popis požadované změny", 2000, true)?;
    if input.idempotency_key.is_nil() || !matches!(input.kind.as_str(), "change" | "cancel") {
        return Err(ApiError::bad_request("Vyberte platný druh požadavku."));
    }
    let mut tx = state.pool.begin().await?;
    // Serialize requests without blocking the FK key-share locks used by audit history.
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let (number, status) = owned_order(&mut tx, user.id, id, true).await?;
    let existing: Option<CustomerRequest> = sqlx::query_as(&format!(
        "{REQUEST_SELECT} WHERE r.user_id=$1 AND r.idempotency_key=$2"
    ))
    .bind(user.id)
    .bind(input.idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.order_id != id || existing.kind != input.kind || existing.message != message {
            return Err(ApiError::conflict(
                "Stejný požadavek už byl odeslán s jiným obsahem.",
            ));
        }
        return Ok(Json(existing));
    }
    if matches!(status.as_str(), "delivered" | "cancelled" | "expired") {
        return Err(ApiError::conflict(
            "Tato objednávka už je uzavřená. Kontaktujte obsluhu.",
        ));
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM customer_order_requests WHERE order_id=$1 AND status='pending')").bind(id).fetch_one(&mut *tx).await?;
    if pending {
        return Err(ApiError::conflict(
            "U objednávky už čeká požadavek na vyřízení. Vyčkejte na odpověď obsluhy.",
        ));
    }
    let request_id = Uuid::new_v4();
    sqlx::query("INSERT INTO customer_order_requests (id,idempotency_key,user_id,order_id,kind,message) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(request_id).bind(input.idempotency_key).bind(user.id).bind(id).bind(&input.kind).bind(&message).execute(&mut *tx).await?;
    let kind = if input.kind == "cancel" {
        "zrušení"
    } else {
        "změnu"
    };
    mail::enqueue_order_notification(&mut tx,&format!("Žádost o {kind} objednávky {number}"),&format!("Zákazník {} žádá o {kind} objednávky {number}.\n\n{message}\n\nPožadavek vyřiďte v administraci v sekci Požadavky zákazníků. Odeslání žádosti samo nemění objednávku ani její platbu.",user.email)).await?;
    let result = sqlx::query_as(&format!("{REQUEST_SELECT} WHERE r.id=$1"))
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(result))
}
async fn admin_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<CustomerRequest>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as(&format!(
            "{REQUEST_SELECT} ORDER BY (r.status='pending') DESC,r.created_at DESC LIMIT 500"
        ))
        .fetch_all(&state.pool)
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveInput {
    status: String,
    response: String,
    version: i64,
}
async fn resolve_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<ResolveInput>,
) -> ApiResult<Json<CustomerRequest>> {
    let admin = auth::require_admin(&state, &headers).await?;
    let response = clean(&input.response, "odpověď zákazníkovi", 2000, true)?;
    if !matches!(input.status.as_str(), "resolved" | "rejected") {
        return Err(ApiError::bad_request("Vyberte výsledek vyřízení."));
    }
    let mut tx = state.pool.begin().await?;
    let changed=sqlx::query("UPDATE customer_order_requests SET status=$1,response=$2,version=version+1,resolved_at=now(),resolved_by=$3 WHERE id=$4 AND status='pending' AND version=$5")
        .bind(&input.status).bind(&response).bind(admin.id).bind(id).bind(input.version).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Požadavek už byl vyřízen nebo se změnil. Načtěte seznam znovu.",
        ));
    }
    let result: CustomerRequest = sqlx::query_as(&format!("{REQUEST_SELECT} WHERE r.id=$1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    mail::enqueue(
        &mut tx,
        &result.email,
        &format!("Odpověď k objednávce {}", result.order_number),
        &format!(
            "Vaše žádost k objednávce {} byla {}.\n\n{}",
            result.order_number,
            if input.status == "resolved" {
                "vyřízena"
            } else {
                "zamítnuta"
            },
            response
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn billing_requires_identity_and_bounds_all_fields() {
        let mut input = BillingInput {
            name: " Firma ".into(),
            address: " Ulice 1 ".into(),
            company_id: String::new(),
            tax_id: String::new(),
        };
        assert_eq!(input.normalized().unwrap().name, "Firma");
        input.address = " ".into();
        assert!(input.normalized().is_err());
        input.address = "Ulice 1".into();
        input.tax_id = "1".repeat(31);
        assert!(input.normalized().is_err());
    }
}
