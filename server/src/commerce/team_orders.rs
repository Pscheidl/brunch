//! Shared selection only. The organizer creates the paid order through checkout.
use super::clean;
use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/team-orders", get(list).post(create))
        .route("/api/team-orders/{id}", get(detail))
        .route("/api/team-orders/{id}/close", post(close))
        .route("/api/team-orders/{id}/reopen", post(reopen))
        .route(
            "/api/team-orders/{id}/contributions/{contribution_id}",
            axum::routing::put(edit_contribution),
        )
        .route("/api/team-orders/{id}/export", get(export))
        .route("/api/team-joins/{token}", get(join).put(contribute))
}

#[derive(FromRow, Serialize)]
struct Team {
    id: Uuid,
    name: String,
    invite_token: String,
    deadline: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    order_id: Option<Uuid>,
    revision: i64,
    created_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Item {
    product_id: i64,
    quantity: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
}
#[derive(FromRow, Serialize)]
struct Contribution {
    id: Uuid,
    name: String,
    items: Value,
    updated_at: DateTime<Utc>,
}
#[derive(FromRow)]
struct ProductAvailability {
    name: String,
    active: bool,
    stock: Option<i32>,
    minimum_quantity: i32,
    variants: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTeam {
    name: String,
    /// A Prague local time, not the browser's local time zone.
    deadline: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    name: String,
    items: Vec<Item>,
    #[serde(default)]
    contribution_token: Option<String>,
    #[serde(default)]
    client_key: Option<String>,
}

fn secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}
fn missing() -> ApiError {
    ApiError::not_found("Týmový košík nebyl nalezen nebo k němu nemáte přístup.")
}
fn deadline(value: &str, now: DateTime<Utc>) -> ApiResult<DateTime<Utc>> {
    let local = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .map_err(|_| ApiError::bad_request("Zvolte datum a čas uzávěrky v českém čase."))?;
    let result = chrono_tz::Europe::Prague
        .from_local_datetime(&local)
        .single()
        .ok_or_else(|| {
            ApiError::bad_request("Tento čas při změně letního času není jednoznačný. Zvolte jiný.")
        })?
        .with_timezone(&Utc);
    if result < now + Duration::minutes(5) || result > now + Duration::days(31) {
        return Err(ApiError::bad_request(
            "Uzávěrka musí být za 5 minut až 31 dní.",
        ));
    }
    Ok(result)
}
fn validate_items(items: &[Item]) -> ApiResult<()> {
    let mut ids = HashSet::new();
    if items.len() > 50
        || items.iter().any(|item| {
            item.product_id <= 0
                || !(1..=99).contains(&item.quantity)
                || !ids.insert(item.product_id)
                || item
                    .variant
                    .as_ref()
                    .is_some_and(|v| v.is_empty() || v.chars().count() > 120)
        })
    {
        return Err(ApiError::bad_request(
            "Vyberte nejvýše 50 různých položek, každou v počtu 1 až 99.",
        ));
    }
    Ok(())
}
async fn rate(state: &AppState, key: &str, maximum: i32) -> ApiResult<()> {
    if !auth::rate_limit(&state.pool, key, maximum, 3600).await? {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Příliš mnoho změn týmového košíku. Zkuste to později.",
        ));
    }
    Ok(())
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<Team>>> {
    let user = auth::require_user(&state, &headers).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT * FROM team_orders WHERE owner_id=$1 ORDER BY created_at DESC LIMIT 100",
        )
        .bind(user.id)
        .fetch_all(&state.pool)
        .await?,
    ))
}
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewTeam>,
) -> ApiResult<Json<Team>> {
    let user = auth::require_user(&state, &headers).await?;
    rate(&state, &format!("team-create:{}", user.id), 20).await?;
    let name = clean(&input.name, "název košíku", 100, true)?;
    let deadline = deadline(&input.deadline, Utc::now())?;
    let mut tx = state.pool.begin().await?;
    // Serialize creations per owner, including the active-team bound.
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM team_orders WHERE owner_id=$1 AND closed_at IS NULL AND deadline>clock_timestamp()")
        .bind(user.id).fetch_one(&mut *tx).await?;
    if count >= 10 {
        return Err(ApiError::bad_request(
            "Můžete mít nejvýše deset otevřených týmových košíků.",
        ));
    }
    let team = sqlx::query_as("INSERT INTO team_orders(id,owner_id,name,invite_token,deadline) VALUES($1,$2,$3,$4,$5) RETURNING *")
        .bind(Uuid::new_v4()).bind(user.id).bind(name).bind(secret()).bind(deadline).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(team))
}
async fn owner_team(conn: &mut PgConnection, id: Uuid, owner: Uuid, lock: bool) -> ApiResult<Team> {
    let query = if lock {
        "SELECT * FROM team_orders WHERE id=$1 AND owner_id=$2 FOR UPDATE"
    } else {
        "SELECT * FROM team_orders WHERE id=$1 AND owner_id=$2"
    };
    sqlx::query_as(query)
        .bind(id)
        .bind(owner)
        .fetch_optional(conn)
        .await?
        .ok_or_else(missing)
}
async fn contributions(conn: &mut PgConnection, id: Uuid) -> ApiResult<Vec<Contribution>> {
    Ok(sqlx::query_as("SELECT id,name,items,updated_at FROM team_contributions WHERE team_id=$1 ORDER BY created_at,id")
        .bind(id).fetch_all(conn).await?)
}
async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut conn = state.pool.acquire().await?;
    let team = owner_team(&mut conn, id, user.id, false).await?;
    let entries = contributions(&mut conn, id).await?;
    let selected_products: Vec<super::Product> = sqlx::query_as("SELECT p.* FROM products p WHERE p.id IN (SELECT (item->>'product_id')::bigint FROM team_contributions c, jsonb_array_elements(c.items) item WHERE c.team_id=$1)")
        .bind(id).fetch_all(&mut *conn).await?;
    Ok(Json(
        json!({"team":team,"contributions":entries,"products":selected_products}),
    ))
}
async fn invite(conn: &mut PgConnection, token: &str, lock: bool) -> ApiResult<Team> {
    if !valid_token(token) {
        return Err(missing());
    }
    let query = if lock {
        "SELECT * FROM team_orders WHERE invite_token=$1 FOR UPDATE"
    } else {
        "SELECT * FROM team_orders WHERE invite_token=$1"
    };
    sqlx::query_as(query)
        .bind(token)
        .fetch_optional(conn)
        .await?
        .ok_or_else(missing)
}
async fn join(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let team = invite(&mut conn, &token, false).await?;
    let own_token = headers
        .get("x-team-contribution")
        .and_then(|h| h.to_str().ok())
        .filter(|t| valid_token(t));
    let own: Option<Contribution> = if let Some(token) = own_token {
        sqlx::query_as("SELECT id,name,items,updated_at FROM team_contributions WHERE team_id=$1 AND token_hash=$2")
            .bind(team.id).bind(auth::token_hash(token)).fetch_optional(&mut *conn).await?
    } else {
        None
    };
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM team_contributions WHERE team_id=$1 AND jsonb_array_length(items)>0",
    )
    .bind(team.id)
    .fetch_one(&mut *conn)
    .await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *conn)
        .await?;
    Ok(Json(
        json!({"id":team.id,"name":team.name,"deadline":team.deadline,"closed":team.closed_at.is_some(),"ordered":team.order_id.is_some(),"accepting":team.order_id.is_none() && team.closed_at.is_none() && team.deadline>now,"contributor_count":count,"own":own}),
    ))
}
async fn aggregate(
    conn: &mut PgConnection,
    team_id: Uuid,
    require_minimum: bool,
) -> ApiResult<Vec<Item>> {
    let selections: Vec<Value> =
        sqlx::query_scalar("SELECT items FROM team_contributions WHERE team_id=$1")
            .bind(team_id)
            .fetch_all(&mut *conn)
            .await?;
    let mut combined = BTreeMap::<i64, (i32, BTreeSet<Option<String>>)>::new();
    for selection in selections {
        let items: Vec<Item> = serde_json::from_value(selection)
            .map_err(|_| ApiError::conflict("Uložený týmový košík nelze načíst."))?;
        for item in items {
            let entry = combined.entry(item.product_id).or_default();
            entry.0 += item.quantity;
            entry.1.insert(item.variant);
        }
    }
    if combined.len() > 50 {
        return Err(ApiError::bad_request(
            "Společný košík může mít nejvýše 50 různých položek.",
        ));
    }
    let mut result = Vec::new();
    for (id, (quantity, variants)) in combined {
        let product: Option<ProductAvailability> = sqlx::query_as("SELECT name,active,stock,minimum_quantity,variants FROM products WHERE id=$1 FOR SHARE")
            .bind(id).fetch_optional(&mut *conn).await?;
        let Some(ProductAvailability {
            name,
            active,
            stock,
            minimum_quantity: minimum,
            variants: allowed,
        }) = product
        else {
            return Err(ApiError::conflict("Některá položka již není v nabídce."));
        };
        if !active || quantity > stock.unwrap_or(99).min(99) {
            return Err(ApiError::conflict(format!(
                "U položky {name} není dostupný požadovaný počet. Upravte výběr účastníků."
            )));
        }
        if variants.iter().any(|variant| match variant {
            Some(value) => !allowed.contains(value),
            None => !allowed.is_empty(),
        }) {
            return Err(ApiError::conflict(format!(
                "U položky {name} zvolte dostupnou variantu."
            )));
        }
        if require_minimum && variants.len() > 1 {
            return Err(ApiError::conflict(format!(
                "U položky {name} kolegové vybrali různé varianty. Před uzavřením sjednoťte variantu v jejich výběrech."
            )));
        }
        if require_minimum && quantity < minimum.max(1) {
            return Err(ApiError::conflict(format!(
                "U položky {name} je minimální odběr {} ks. Doplňte výběr před uzavřením.",
                minimum.max(1)
            )));
        }
        result.push(Item {
            product_id: id,
            quantity,
            variant: variants.into_iter().next().flatten(),
        });
    }
    Ok(result)
}
async fn contribute(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(input): Json<Selection>,
) -> ApiResult<Json<Value>> {
    if !valid_token(&token) {
        return Err(missing());
    }
    let name = clean(&input.name, "jméno účastníka", 80, true)?;
    validate_items(&input.items)?;
    if input
        .contribution_token
        .as_deref()
        .is_some_and(|t| !valid_token(t))
    {
        return Err(ApiError::forbidden("Neplatný klíč vlastního výběru."));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM team_orders WHERE invite_token=$1)")
            .bind(&token)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(missing());
    }
    rate(
        &state,
        &format!("team-write:{}", auth::token_hash(&token)),
        1000,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    let team = invite(&mut tx, &token, true).await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    if team.closed_at.is_some() || team.deadline <= now {
        return Err(ApiError::conflict(
            "Výběr je uzavřený. Domluvte se s organizátorem.",
        ));
    }
    if input.client_key.as_deref().is_some_and(|t| !valid_token(t))
        || (input.client_key.is_some() && input.contribution_token.is_some())
    {
        return Err(ApiError::bad_request("Neplatný klíč výběru."));
    }
    let contribution_token = input
        .contribution_token
        .clone()
        .or(input.client_key)
        .unwrap_or_else(secret);
    let token_hash = auth::token_hash(&contribution_token);
    let items = json!(input.items);
    let existing: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT team_id,id FROM team_contributions WHERE token_hash=$1")
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.is_some_and(|(team_id, _)| team_id != team.id) {
        return Err(ApiError::forbidden("Tento klíč patří jinému košíku."));
    }
    let id = if input.contribution_token.is_some() || existing.is_some() {
        sqlx::query_scalar::<_, Uuid>("UPDATE team_contributions SET name=$3,items=$4,updated_at=clock_timestamp() WHERE team_id=$1 AND token_hash=$2 RETURNING id")
            .bind(team.id).bind(&token_hash).bind(name).bind(items).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::forbidden("Tento výběr nemůžete upravovat."))?
    } else {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM team_contributions WHERE team_id=$1")
                .bind(team.id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= 50 {
            return Err(ApiError::bad_request(
                "Košík už má maximální počet 50 účastníků.",
            ));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO team_contributions(id,team_id,token_hash,name,items) VALUES($1,$2,$3,$4,$5)")
            .bind(id).bind(team.id).bind(&token_hash).bind(name).bind(items).execute(&mut *tx).await?;
        id
    };
    aggregate(&mut tx, team.id, false).await?;
    let before_commit: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    if team.deadline <= before_commit {
        return Err(ApiError::conflict(
            "Uzávěrka výběru právě uplynula. Změna nebyla uložena.",
        ));
    }
    sqlx::query("UPDATE team_orders SET revision=revision+1 WHERE id=$1")
        .bind(team.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id":id,"contribution_token":contribution_token}),
    ))
}
async fn edit_contribution(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, contribution_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<Selection>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let name = clean(&input.name, "jméno účastníka", 80, true)?;
    validate_items(&input.items)?;
    let mut tx = state.pool.begin().await?;
    let team = owner_team(&mut tx, id, user.id, true).await?;
    if team.closed_at.is_some() {
        return Err(ApiError::conflict("Košík je již uzavřený."));
    }
    // The owner can repair several unavailable selections after the cutoff.
    // Full availability and minimum validation runs atomically when closing.
    let result = sqlx::query("UPDATE team_contributions SET name=$3,items=$4,updated_at=clock_timestamp() WHERE team_id=$1 AND id=$2")
        .bind(id).bind(contribution_id).bind(name).bind(json!(input.items)).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(missing());
    }
    sqlx::query("UPDATE team_orders SET revision=revision+1 WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":contribution_id})))
}
async fn close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut tx = state.pool.begin().await?;
    let team = owner_team(&mut tx, id, user.id, true).await?;
    if team.closed_at.is_none() {
        if aggregate(&mut tx, id, true).await?.is_empty() {
            return Err(ApiError::bad_request("Týmový košík je prázdný."));
        }
        sqlx::query(
            "UPDATE team_orders SET closed_at=clock_timestamp(),revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"id":id,"closed":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reopen {
    deadline: String,
}
async fn reopen(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Reopen>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let cutoff = deadline(&input.deadline, Utc::now())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let team = owner_team(&mut tx, id, user.id, true).await?;
    if team.order_id.is_some() {
        return Err(ApiError::conflict(
            "Tento týmový košík již byl objednán. Pro další nákup založte nový.",
        ));
    }
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM team_orders WHERE owner_id=$1 AND id<>$2 AND closed_at IS NULL AND deadline>clock_timestamp()")
        .bind(user.id).bind(id).fetch_one(&mut *tx).await?;
    if active >= 10 {
        return Err(ApiError::bad_request(
            "Můžete mít nejvýše deset otevřených týmových košíků.",
        ));
    }
    sqlx::query(
        "UPDATE team_orders SET closed_at=NULL,deadline=$2,revision=revision+1 WHERE id=$1",
    )
    .bind(id)
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"closed":false})))
}
async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let user = auth::require_user(&state, &headers).await?;
    let mut tx = state.pool.begin().await?;
    let team = owner_team(&mut tx, id, user.id, true).await?;
    if team.order_id.is_some() {
        return Err(ApiError::conflict(
            "Tento týmový košík již byl objednán. Najdete jej ve svých objednávkách.",
        ));
    }
    if team.closed_at.is_none() {
        return Err(ApiError::conflict("Nejprve uzavřete výběr kolegů."));
    }
    let items = aggregate(&mut tx, id, true).await?;
    if items.is_empty() {
        return Err(ApiError::bad_request("Týmový košík je prázdný."));
    }
    let entries = contributions(&mut tx, id).await?;
    let ids: Vec<i64> = items.iter().map(|item| item.product_id).collect();
    let product_names: BTreeMap<i64, String> =
        sqlx::query_as::<_, (i64, String)>("SELECT id,name FROM products WHERE id=ANY($1)")
            .bind(ids)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    let mut notes = format!("Společná objednávka: {}.\n", team.name);
    for entry in entries {
        let selected: Vec<Item> = serde_json::from_value(entry.items).unwrap_or_default();
        if selected.is_empty() {
            continue;
        }
        let parts: Vec<String> = selected
            .iter()
            .map(|i| {
                format!(
                    "{}× {}{}",
                    i.quantity,
                    product_names
                        .get(&i.product_id)
                        .map(String::as_str)
                        .unwrap_or("Svačina"),
                    i.variant
                        .as_ref()
                        .map(|v| format!(" ({v})"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let line = format!("{}: {}\n", entry.name, parts.join(", "));
        if notes.chars().count() + line.chars().count() > 1850 {
            notes.push_str("Úplný rozpis má organizátor v týmovém košíku.\n");
            break;
        }
        notes.push_str(&line);
    }
    tx.commit().await?;
    Ok(Json(
        json!({"team_id":id,"revision":team.revision,"items":items,"notes":notes}),
    ))
}

/// Called inside the same transaction that persists the actual order and outbox.
/// The team lock prevents two tabs or checkout keys from ordering one team twice.
pub(super) async fn lock_checkout(
    conn: &mut PgConnection,
    team_id: Uuid,
    actor_id: Option<Uuid>,
    revision: i64,
) -> ApiResult<()> {
    let owner = actor_id.ok_or_else(|| {
        ApiError::unauthorized("Před odesláním týmové objednávky se přihlaste jako organizátor.")
    })?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(owner)
        .fetch_one(&mut *conn)
        .await?;
    let team = owner_team(conn, team_id, owner, true).await?;
    if team.revision != revision {
        return Err(ApiError::conflict(
            "Výběr týmu se od přenesení změnil. Vyprázdněte původní košík a přeneste aktuální výběr znovu.",
        ));
    }
    if team.closed_at.is_none() {
        return Err(ApiError::conflict("Nejprve uzavřete výběr kolegů."));
    }
    if team.order_id.is_some() {
        return Err(ApiError::conflict(
            "Tento týmový košík již byl objednán. Zkontrolujte své objednávky, nevytvářejte druhou.",
        ));
    }
    Ok(())
}
pub(super) async fn claim_checkout(
    conn: &mut PgConnection,
    team_id: Uuid,
    actor_id: Option<Uuid>,
    order_id: Uuid,
    revision: i64,
) -> ApiResult<()> {
    lock_checkout(conn, team_id, actor_id, revision).await?;
    sqlx::query("UPDATE team_orders SET order_id=$2 WHERE id=$1")
        .bind(team_id)
        .bind(order_id)
        .execute(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_uses_prague_and_rejects_ambiguous_dst() {
        let now = DateTime::parse_from_rfc3339("2026-11-21T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            deadline("2026-11-21T12:00", now).unwrap().to_rfc3339(),
            "2026-11-21T11:00:00+00:00"
        );
        assert!(deadline("2026-11-21T11:04", now).is_err());
        let autumn = DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(deadline("2026-10-25T02:30", autumn).is_err());
        assert!(deadline("2027-01-01T12:00", now).is_err());
    }
    #[test]
    fn variant_length_matches_product_catalog_limit() {
        for length in [101, 120] {
            assert!(
                validate_items(&[Item {
                    product_id: 1,
                    quantity: 1,
                    variant: Some("ž".repeat(length)),
                }])
                .is_ok(),
                "a {length}-character catalog variant must work in a team cart"
            );
        }
        assert!(
            validate_items(&[Item {
                product_id: 1,
                quantity: 1,
                variant: Some("ž".repeat(121)),
            }])
            .is_err()
        );
    }
    #[test]
    fn selections_reject_duplicates_and_out_of_range_counts() {
        assert!(
            validate_items(&[Item {
                product_id: 1,
                quantity: 0,
                variant: None
            }])
            .is_err()
        );
        assert!(
            validate_items(&[Item {
                product_id: 1,
                quantity: 100,
                variant: None
            }])
            .is_err()
        );
        assert!(
            validate_items(&[
                Item {
                    product_id: 1,
                    quantity: 1,
                    variant: None
                },
                Item {
                    product_id: 1,
                    quantity: 1,
                    variant: None
                }
            ])
            .is_err()
        );
        assert!(validate_items(&[]).is_ok());
        assert!(
            validate_items(&[Item {
                product_id: 1,
                quantity: 99,
                variant: None
            }])
            .is_ok()
        );
    }
}
