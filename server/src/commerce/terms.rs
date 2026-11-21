use super::clean;
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::HeaderMap,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection};

#[derive(Serialize, FromRow)]
pub(super) struct Terms {
    pub version: i64,
    pub title: String,
    pub content: String,
    pub updated_at: DateTime<Utc>,
}

impl Terms {
    pub fn email_copy(&self) -> String {
        format!(
            "\n\n{}\nVerze {}\n\n{}",
            self.title, self.version, self.content
        )
    }
}

pub(super) async fn order_email_copy(
    conn: &mut PgConnection,
    version: Option<i64>,
) -> ApiResult<String> {
    let Some(version) = version else {
        return Ok(String::new());
    };
    let terms: Terms = sqlx::query_as("SELECT version,title,content,created_at AS updated_at FROM terms_versions WHERE version=$1")
        .bind(version)
        .fetch_one(conn)
        .await?;
    Ok(terms.email_copy())
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/terms", get(public_terms)).route(
        "/api/admin/terms",
        get(admin_terms)
            .put(update_terms)
            .layer(DefaultBodyLimit::max(512 * 1024)),
    )
}

// Called under the shop initialization advisory lock. Existing published and
// historical text is never overwritten by a new release or application restart.
pub(super) async fn initialize(conn: &mut PgConnection) -> ApiResult<()> {
    sqlx::query("INSERT INTO terms_versions(version,title,content) VALUES (1,$1,$2) ON CONFLICT (version) DO NOTHING")
        .bind("Všeobecné obchodní podmínky")
        .bind(include_str!("initial-terms-cs.txt").trim())
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO terms_publication(id,version) VALUES (1,1) ON CONFLICT (id) DO NOTHING",
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionQuery {
    version: Option<i64>,
}

async fn public_terms(
    State(state): State<AppState>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<Json<Terms>> {
    if query.version.is_some_and(|version| version <= 0) {
        return Err(ApiError::bad_request("Neplatná verze obchodních podmínek."));
    }
    let terms = if let Some(version) = query.version {
        sqlx::query_as("SELECT version,title,content,created_at AS updated_at FROM terms_versions WHERE version=$1")
            .bind(version)
            .fetch_optional(&state.pool)
            .await?
    } else {
        sqlx::query_as("SELECT t.version,t.title,t.content,t.created_at AS updated_at FROM terms_versions t JOIN terms_publication p ON p.version=t.version WHERE p.id=1")
            .fetch_optional(&state.pool)
            .await?
    };
    Ok(Json(terms.ok_or_else(|| {
        ApiError::not_found("Obchodní podmínky nejsou dostupné. Zkuste stránku obnovit.")
    })?))
}

async fn admin_terms(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Terms>> {
    auth::require_admin(&state, &headers).await?;
    public_terms(State(state), Query(VersionQuery { version: None })).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermsInput {
    version: i64,
    title: String,
    content: String,
}

async fn update_terms(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<TermsInput>,
) -> ApiResult<Json<Terms>> {
    let user = auth::require_admin(&state, &headers).await?;
    let title = clean(&input.title, "název obchodních podmínek", 160, true)?;
    if title.contains(['\n', '\r']) {
        return Err(ApiError::bad_request(
            "Název podmínek musí být na jednom řádku.",
        ));
    }
    let content = clean(
        &input.content.replace("\r\n", "\n").replace('\r', "\n"),
        "text obchodních podmínek",
        100_000,
        true,
    )?;
    let mut tx = state.pool.begin().await?;
    let previous: i64 =
        sqlx::query_scalar("SELECT version FROM terms_publication WHERE id=1 FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
    if input.version != previous {
        return Err(ApiError::conflict(
            "Obchodní podmínky mezitím upravil jiný správce. Načtěte aktuální verzi a znovu zkontrolujte změny.",
        ));
    }
    let next = previous
        .checked_add(1)
        .ok_or_else(|| ApiError::conflict("Další verzi podmínek nelze uložit."))?;
    let terms: Terms = sqlx::query_as("INSERT INTO terms_versions(version,title,content,created_by) VALUES ($1,$2,$3,$4) RETURNING version,title,content,created_at AS updated_at")
        .bind(next)
        .bind(title)
        .bind(content)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("UPDATE terms_publication SET version=$1 WHERE id=1")
        .bind(next)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(terms))
}

pub(super) async fn accepted_for_checkout(
    conn: &mut PgConnection,
    accepted_version: Option<i64>,
) -> ApiResult<Terms> {
    let accepted_version = accepted_version
        .filter(|version| *version > 0)
        .ok_or_else(|| {
            ApiError::bad_request(
                "Přečtěte si obchodní podmínky a potvrďte souhlas s jejich zněním.",
            )
        })?;
    // Publishing a new version waits until this order has committed. The order
    // therefore always records the exact current version accepted at checkout.
    let current_version: i64 =
        sqlx::query_scalar("SELECT version FROM terms_publication WHERE id=1 FOR SHARE")
            .fetch_one(&mut *conn)
            .await?;
    if accepted_version != current_version {
        return Err(ApiError::conflict(
            "Obchodní podmínky se změnily. Přečtěte si prosím aktuální znění a potvrďte souhlas znovu.",
        ));
    }
    // Fetch the immutable document only after locking its publication pointer.
    // A joined locking query could lose the old joined row while waiting for a
    // concurrent publisher and return 500 instead of a stale-consent conflict.
    let current: Terms = sqlx::query_as("SELECT version,title,content,created_at AS updated_at FROM terms_versions WHERE version=$1")
        .bind(current_version)
        .fetch_one(conn)
        .await?;
    Ok(current)
}
