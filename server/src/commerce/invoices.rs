use super::{OrderItem, clean, read_order};
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/invoice-supplier",
            get(supplier).put(update_supplier),
        )
        .route("/api/admin/orders/{id}/invoice", get(invoice).post(issue))
}

#[derive(Serialize, Deserialize, FromRow)]
#[serde(deny_unknown_fields)]
struct Supplier {
    name: String,
    address: String,
    company_id: String,
    tax_id: String,
    registration: String,
    vat_payer: bool,
    version: i64,
}

async fn supplier(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Supplier>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM invoice_supplier WHERE id=1")
            .fetch_one(&state.pool)
            .await?,
    ))
}

async fn update_supplier(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Supplier>,
) -> ApiResult<Json<Supplier>> {
    auth::require_admin(&state, &headers).await?;
    let supplier = sqlx::query_as("UPDATE invoice_supplier SET name=$1,address=$2,company_id=$3,tax_id=$4,registration=$5,vat_payer=$6,version=version+1 WHERE id=1 AND version=$7 RETURNING *")
        .bind(clean(&input.name, "název dodavatele", 200, true)?)
        .bind(clean(&input.address, "sídlo dodavatele", 500, true)?)
        .bind(clean(&input.company_id, "IČO dodavatele", 24, true)?)
        .bind(clean(&input.tax_id, "DIČ dodavatele", 30, input.vat_payer)?)
        .bind(clean(&input.registration, "zápis v rejstříku", 300, false)?)
        .bind(input.vat_payer).bind(input.version).fetch_optional(&state.pool).await?
        .ok_or_else(|| ApiError::conflict("Fakturační údaje se mezitím změnily."))?;
    Ok(Json(supplier))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvoiceInput {
    supplier_version: i64,
    customer_name: String,
    customer_address: String,
    customer_company_id: String,
    customer_tax_id: String,
    supply_date: NaiveDate,
    due_date: NaiveDate,
    // Rates are percentage points × 100, keyed by product ID or "delivery".
    vat_rates: BTreeMap<String, u16>,
}

async fn invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Option<Value>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(
        sqlx::query_scalar("SELECT document FROM invoices WHERE order_id=$1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?,
    ))
}

#[derive(Serialize)]
struct InvoiceLine {
    name: String,
    quantity: i32,
    unit_price_cents: i64,
    total_cents: i64,
    vat_rate: Option<u16>,
    net_cents: i64,
    vat_cents: i64,
}

fn line(
    name: String,
    quantity: i32,
    unit_price_cents: i64,
    total_cents: i64,
    rate: Option<u16>,
    vat_payer: bool,
) -> ApiResult<InvoiceLine> {
    if vat_payer != rate.is_some() || rate.is_some_and(|r| r > 10_000) {
        return Err(ApiError::bad_request(
            "Zadejte platnou sazbu DPH pro každou fakturovanou položku.",
        ));
    }
    let divisor = 10_000 + i64::from(rate.unwrap_or(0));
    let net_cents = (total_cents * 10_000 + divisor / 2) / divisor;
    Ok(InvoiceLine {
        name,
        quantity,
        unit_price_cents,
        total_cents,
        vat_rate: rate,
        net_cents,
        vat_cents: total_cents - net_cents,
    })
}

async fn issue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut input): Json<InvoiceInput>,
) -> ApiResult<Json<Value>> {
    let admin = auth::require_admin(&state, &headers).await?;
    let mut tx = state.pool.begin().await?;
    // Lock before checking existence: concurrent issuers and retries share one snapshot.
    sqlx::query("SELECT id FROM orders WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Objednávka neexistuje."))?;
    if let Some(existing) = sqlx::query_scalar("SELECT document FROM invoices WHERE order_id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
    {
        return Ok(Json(existing));
    }
    let supplier: Supplier = sqlx::query_as("SELECT * FROM invoice_supplier WHERE id=1 FOR SHARE")
        .fetch_one(&mut *tx)
        .await?;
    if supplier.name.is_empty() || supplier.address.is_empty() || supplier.company_id.is_empty() {
        return Err(ApiError::bad_request(
            "Před vystavením faktury uložte fakturační údaje dodavatele.",
        ));
    }
    if supplier.version != input.supplier_version {
        return Err(ApiError::conflict(
            "Údaje dodavatele se změnily. Otevřete fakturu znovu.",
        ));
    }
    let customer = json!({
        "name": clean(&input.customer_name, "odběratel", 250, true)?,
        "address": clean(&input.customer_address, "adresa odběratele", 500, true)?,
        "company_id": clean(&input.customer_company_id, "IČO odběratele", 24, false)?,
        "tax_id": clean(&input.customer_tax_id, "DIČ odběratele", 30, false)?,
    });
    if [input.due_date, input.supply_date]
        .iter()
        .any(|d| !(1..=9999).contains(&d.year()))
    {
        return Err(ApiError::bad_request("Neplatné datum na faktuře."));
    }
    let order = read_order(&mut tx, id, false).await?;
    let mut lines = Vec::new();
    for OrderItem {
        product_id,
        name,
        quantity,
        unit_price_cents,
        total_cents,
        ..
    } in order.items
    {
        lines.push(line(
            name,
            quantity,
            unit_price_cents,
            total_cents,
            input.vat_rates.remove(&product_id.to_string()),
            supplier.vat_payer,
        )?);
    }
    if order.delivery_fee_cents > 0 {
        lines.push(line(
            "Doprava".into(),
            1,
            order.delivery_fee_cents,
            order.delivery_fee_cents,
            input.vat_rates.remove("delivery"),
            supplier.vat_payer,
        )?);
    }
    if !input.vat_rates.is_empty() {
        return Err(ApiError::bad_request(
            "Sazby DPH neodpovídají položkám objednávky.",
        ));
    }
    let issue_date = Utc::now()
        .with_timezone(&chrono_tz::Europe::Prague)
        .date_naive();
    let sequence: i64 = sqlx::query_scalar("SELECT nextval('invoice_number_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let number = format!("FA-{}-{sequence:06}", issue_date.year());
    let document = json!({
        "number": number, "order_number": order.number, "issue_date": issue_date,
        "supply_date": input.supply_date, "due_date": input.due_date,
        "supplier": supplier, "customer": customer, "lines": lines,
        "payment_method": order.payment_method, "payment_status": order.payment_status,
        "fulfillment_method": order.fulfillment_method, "pickup_location": order.pickup_location,
        "total_cents": order.total_cents, "currency": "CZK",
    });
    sqlx::query("INSERT INTO invoices (order_id,number,issued_by,document) VALUES ($1,$2,$3,$4)")
        .bind(id)
        .bind(number)
        .bind(admin.id)
        .bind(&document)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(document))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tax_rounding_reconciles_to_line_gross_and_requires_explicit_rates() {
        let item = line("Čaj".into(), 2, 6050, 12100, Some(2100), true).unwrap();
        assert_eq!((item.net_cents, item.vat_cents), (10000, 2100));
        let item = line("Pečivo".into(), 1, 3190, 3190, Some(1200), true).unwrap();
        assert_eq!((item.net_cents, item.vat_cents), (2848, 342));
        assert!(line("A".into(), 1, 10, 10, Some(2100), false).is_err());
        assert!(line("A".into(), 1, 10, 10, None, true).is_err());
        assert!(line("A".into(), 1, 10, 10, Some(10001), true).is_err());
    }
}
