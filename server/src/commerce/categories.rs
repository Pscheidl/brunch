use super::clean;
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/categories", get(list).post(create))
        .route("/api/admin/categories/{id}", put(rename).delete(remove))
}

#[derive(Serialize, FromRow)]
struct Category {
    id: i64,
    name: String,
    version: i64,
    product_count: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameInput {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameInput {
    name: String,
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionInput {
    version: i64,
}

// Existing product API clients can create a category by entering a new name.
pub(super) async fn ensure(conn: &mut PgConnection, name: &str) -> ApiResult<()> {
    if sqlx::query("SELECT id FROM categories WHERE name=$1 FOR KEY SHARE")
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?
        .is_none()
    {
        sqlx::query("INSERT INTO categories(name) VALUES ($1) ON CONFLICT (name) DO NOTHING")
            .bind(name)
            .execute(conn)
            .await?;
    }
    Ok(())
}

fn duplicate(error: sqlx::Error) -> ApiError {
    if matches!(&error, sqlx::Error::Database(db) if db.is_unique_violation()) {
        ApiError::bad_request("Kategorie s tímto názvem již existuje.")
    } else {
        error.into()
    }
}

async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<Category>>> {
    auth::require_admin(&state, &headers).await?;
    let categories = sqlx::query_as(
        "SELECT c.id,c.name,c.version,count(p.id) AS product_count FROM categories c
         LEFT JOIN products p ON p.category=c.name WHERE c.name<>'' GROUP BY c.id ORDER BY c.name",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(categories))
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NameInput>,
) -> ApiResult<Json<Value>> {
    auth::require_admin(&state, &headers).await?;
    let name = clean(&input.name, "název kategorie", 80, true)?;
    let id: i64 = sqlx::query_scalar("INSERT INTO categories(name) VALUES ($1) RETURNING id")
        .bind(name)
        .fetch_one(&state.pool)
        .await
        .map_err(duplicate)?;
    Ok(Json(json!({"id":id})))
}

async fn rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<RenameInput>,
) -> ApiResult<Json<Value>> {
    auth::require_admin(&state, &headers).await?;
    let name = clean(&input.name, "název kategorie", 80, true)?;
    let result = sqlx::query(
        "UPDATE categories SET name=$3,version=version+1 WHERE id=$1 AND version=$2 AND name<>''",
    )
    .bind(id)
    .bind(input.version)
    .bind(name)
    .execute(&state.pool)
    .await
    .map_err(duplicate)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "Kategorie byla změněna nebo odstraněna.",
        ));
    }
    Ok(Json(json!({"id":id})))
}

async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<VersionInput>,
) -> ApiResult<Json<Value>> {
    auth::require_admin(&state, &headers).await?;
    let result = sqlx::query("DELETE FROM categories WHERE id=$1 AND version=$2 AND name<>''")
        .bind(id)
        .bind(input.version)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "Kategorie byla změněna nebo odstraněna.",
        ));
    }
    Ok(Json(json!({"id":id})))
}
