use super::clean;
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

const DEFAULT_HERO_ALT: &str =
    "Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole";

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/admin/site-settings",
        get(admin_settings).put(update_settings),
    )
}

#[derive(Deserialize, Serialize, FromRow)]
#[serde(deny_unknown_fields)]
pub(super) struct SiteSettings {
    pub(super) hero_image: String,
    pub(super) hero_image_alt: String,
    pub(super) public_contact_email: String,
    pub(super) public_contact_phone: String,
    pub(super) seller_name: String,
    pub(super) seller_address: String,
    pub(super) seller_ico: String,
    pub(super) delivery_area_label: String,
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteSettingsInput {
    hero_image: String,
    hero_image_alt: String,
    version: i64,
    public_contact_email: Option<String>,
    public_contact_phone: Option<String>,
    seller_name: Option<String>,
    seller_address: Option<String>,
    seller_ico: Option<String>,
    delivery_area_label: Option<String>,
}

pub(super) async fn read(pool: &PgPool) -> ApiResult<SiteSettings> {
    Ok(sqlx::query_as("SELECT * FROM site_settings WHERE id=1")
        .fetch_one(pool)
        .await?)
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SiteSettings>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(read(&state.pool).await?))
}

async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<SiteSettingsInput>,
) -> ApiResult<Json<SiteSettings>> {
    auth::require_admin(&state, &headers).await?;
    if input.version < 1 {
        return Err(ApiError::bad_request("Neplatná verze nastavení webu."));
    }
    if input.hero_image.is_empty() {
        input.hero_image_alt = DEFAULT_HERO_ALT.to_owned();
    } else {
        let id = input
            .hero_image
            .strip_prefix("/api/images/")
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|id| input.hero_image == format!("/api/images/{id}"))
            .ok_or_else(|| ApiError::bad_request("Vyberte fotografii nahranou v administraci."))?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM images WHERE id=$1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
        if !exists {
            return Err(ApiError::bad_request("Nahraná fotografie neexistuje."));
        }
        input.hero_image_alt = clean(&input.hero_image_alt, "popis fotografie", 500, true)?;
    }
    let current = read(&state.pool).await?;
    let contact_email = match input.public_contact_email {
        Some(value) if !value.trim().is_empty() => auth::normalize_email(&value)?,
        Some(_) => String::new(),
        None => current.public_contact_email,
    };
    let contact_phone = clean(
        input
            .public_contact_phone
            .as_deref()
            .unwrap_or(&current.public_contact_phone),
        "veřejný telefon",
        40,
        false,
    )?;
    if !contact_phone.is_empty()
        && (contact_phone.chars().filter(char::is_ascii_digit).count() < 6
            || !contact_phone
                .chars()
                .all(|c| c.is_ascii_digit() || "+ ()-/".contains(c)))
    {
        return Err(ApiError::bad_request("Zadejte platný veřejný telefon."));
    }
    let seller_name = clean(
        input.seller_name.as_deref().unwrap_or(&current.seller_name),
        "název provozovatele",
        200,
        false,
    )?;
    let seller_address = clean(
        input
            .seller_address
            .as_deref()
            .unwrap_or(&current.seller_address),
        "sídlo provozovatele",
        500,
        false,
    )?;
    let seller_ico = clean(
        input.seller_ico.as_deref().unwrap_or(&current.seller_ico),
        "IČO provozovatele",
        24,
        false,
    )?;
    if !seller_ico.is_empty()
        && (seller_ico.len() != 8 || !seller_ico.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(ApiError::bad_request("IČO musí obsahovat osm číslic."));
    }
    let delivery_area_label = clean(
        input
            .delivery_area_label
            .as_deref()
            .unwrap_or(&current.delivery_area_label),
        "oblast rozvozu",
        200,
        false,
    )?;
    let updated = sqlx::query_as(
        "UPDATE site_settings SET hero_image=$1,hero_image_alt=$2,version=version+1,updated_at=now(),
         public_contact_email=$4,public_contact_phone=$5,seller_name=$6,seller_address=$7,seller_ico=$8,delivery_area_label=$9
         WHERE id=1 AND version=$3 RETURNING *",
    )
    .bind(input.hero_image)
    .bind(input.hero_image_alt)
    .bind(input.version)
    .bind(contact_email)
    .bind(contact_phone)
    .bind(seller_name)
    .bind(seller_address)
    .bind(seller_ico)
    .bind(delivery_area_label)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        ApiError::conflict("Nastavení webu se mezitím změnilo. Načtěte aktuální údaje.")
    })?;
    Ok(Json(updated))
}
