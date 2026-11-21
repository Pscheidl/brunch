use super::{clean, normalize_postal_code, normalize_slots};
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool};

#[derive(Serialize, Deserialize, FromRow)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    pub enabled: bool,
    pub name: String,
    pub street: String,
    pub city: String,
    pub postal_code: String,
    pub instructions: String,
    pub parking_free: bool,
    pub slots: Vec<String>,
    pub version: i64,
}

impl Settings {
    fn normalize(&mut self) -> ApiResult<()> {
        self.name = clean(&self.name, "název odběrného místa", 200, self.enabled)?;
        self.street = clean(&self.street, "ulice odběrného místa", 250, self.enabled)?;
        self.city = clean(&self.city, "obec odběrného místa", 100, self.enabled)?;
        self.instructions = clean(&self.instructions, "pokyny k osobnímu odběru", 2000, false)?;
        self.postal_code = self.postal_code.trim().to_owned();
        if self.enabled || !self.postal_code.is_empty() {
            self.postal_code = normalize_postal_code(&self.postal_code)?;
        }
        if self.version < 1 || self.slots.len() > 24 {
            return Err(ApiError::bad_request(
                "Neplatná verze nastavení nebo příliš mnoho oken odběru.",
            ));
        }
        if self.enabled || !self.slots.is_empty() {
            normalize_slots(&mut self.slots).map_err(|_| ApiError::bad_request("Zadejte platná nepřekrývající se časová okna osobního odběru, například 08:00–12:00."))?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Value {
        json!({"name":self.name,"street":self.street,"city":self.city,"postal_code":self.postal_code,"instructions":self.instructions,"parking_free":self.parking_free})
    }
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/admin/pickup-settings", get(admin_get).put(update))
}

pub(super) async fn read(pool: &PgPool) -> ApiResult<Settings> {
    Ok(sqlx::query_as("SELECT * FROM pickup_settings WHERE id=1")
        .fetch_one(pool)
        .await?)
}

pub(super) async fn for_checkout(
    conn: &mut PgConnection,
    version: Option<i64>,
) -> ApiResult<Settings> {
    let settings: Settings = sqlx::query_as("SELECT * FROM pickup_settings WHERE id=1 FOR SHARE")
        .fetch_one(conn)
        .await?;
    if !settings.enabled {
        return Err(ApiError::conflict(
            "Osobní odběr momentálně není dostupný. Zvolte doručení nebo objednávku dokončete později.",
        ));
    }
    if version != Some(settings.version) {
        return Err(ApiError::conflict(
            "Odběrné místo nebo časy odběru se změnily. Obnovte údaje a potvrďte aktuální místo a čas.",
        ));
    }
    Ok(settings)
}

async fn admin_get(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Settings>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(read(&state.pool).await?))
}

async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    auth::require_admin(&state, &headers).await?;
    input.normalize()?;
    let result = sqlx::query_as("UPDATE pickup_settings SET enabled=$1,name=$2,street=$3,city=$4,postal_code=$5,instructions=$6,parking_free=$7,slots=$8,version=version+1 WHERE id=1 AND version=$9 RETURNING *")
        .bind(input.enabled).bind(input.name).bind(input.street).bind(input.city).bind(input.postal_code).bind(input.instructions).bind(input.parking_free).bind(input.slots).bind(input.version)
        .fetch_optional(&state.pool).await?
        .ok_or_else(|| ApiError::conflict("Nastavení osobního odběru se mezitím změnilo. Načtěte aktuální údaje."))?;
    Ok(Json(result))
}
