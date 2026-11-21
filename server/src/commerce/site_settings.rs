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
    version: i64,
}

pub(super) async fn read(pool: &PgPool) -> ApiResult<SiteSettings> {
    Ok(
        sqlx::query_as("SELECT hero_image,hero_image_alt,version FROM site_settings WHERE id=1")
            .fetch_one(pool)
            .await?,
    )
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
    Json(mut input): Json<SiteSettings>,
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
    let updated = sqlx::query_as(
        "UPDATE site_settings SET hero_image=$1,hero_image_alt=$2,version=version+1,updated_at=now()
         WHERE id=1 AND version=$3 RETURNING hero_image,hero_image_alt,version",
    )
    .bind(input.hero_image)
    .bind(input.hero_image_alt)
    .bind(input.version)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        ApiError::conflict("Nastavení webu se mezitím změnilo. Načtěte aktuální údaje.")
    })?;
    Ok(Json(updated))
}
