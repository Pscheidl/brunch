use crate::{ApiError, ApiResult, AppState, mail};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
}
#[derive(Deserialize)]
struct EmailRequest {
    email: String,
}
#[derive(Deserialize)]
struct VerifyRequest {
    token: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/request", post(request_link))
        .route("/api/auth/verify", post(verify))
        .route("/api/auth/me", get(me))
        .route("/api/auth/logout", post(logout))
}
pub fn normalize_email(value: &str) -> ApiResult<String> {
    let email = value.trim().to_lowercase();
    if email.len() > 254
        || email.len() < 3
        || email.contains(char::is_whitespace)
        || email.contains(['<', '>', '\r', '\n'])
        || email.parse::<lettre::Address>().is_err()
    {
        return Err(ApiError::bad_request("Zadejte platný e-mail."));
    }
    Ok(email)
}
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| p.trim().strip_prefix("brunch_session="))
}
pub async fn current_user(state: &AppState, headers: &HeaderMap) -> ApiResult<Option<User>> {
    let Some(token) = session_cookie(headers) else {
        return Ok(None);
    };
    if token.len() != 64 {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_,User>("SELECT u.id,u.email,u.is_admin FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>now()")
        .bind(token_hash(token)).fetch_optional(&state.pool).await?)
}
pub async fn require_user(state: &AppState, headers: &HeaderMap) -> ApiResult<User> {
    current_user(state, headers)
        .await?
        .ok_or_else(|| ApiError::unauthorized("Nejprve se přihlaste e-mailem."))
}
pub async fn require_admin(state: &AppState, headers: &HeaderMap) -> ApiResult<User> {
    let user = require_user(state, headers).await?;
    if !user.is_admin {
        return Err(ApiError::forbidden("Tato stránka je určena správci."));
    }
    Ok(user)
}
pub async fn bootstrap_admin(pool: &PgPool, email: &str) -> ApiResult<()> {
    let email = normalize_email(email)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(87654321)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET is_admin=false WHERE is_admin AND email<>$1")
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users (id,email,is_admin) VALUES ($1,$2,true) ON CONFLICT(email) DO UPDATE SET is_admin=true")
        .bind(Uuid::new_v4()).bind(email).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
/// Atomic fixed-window counters shared by all instances, keyed without raw email addresses.
pub async fn rate_limit(pool: &PgPool, key: &str, limit: i32, seconds: i32) -> ApiResult<bool> {
    let count:i32=sqlx::query_scalar("INSERT INTO request_limits (key) VALUES ($1) ON CONFLICT(key) DO UPDATE SET requests=CASE WHEN request_limits.window_start < now()-make_interval(secs=>$2) THEN 1 ELSE request_limits.requests+1 END,window_start=CASE WHEN request_limits.window_start < now()-make_interval(secs=>$2) THEN now() ELSE request_limits.window_start END RETURNING requests")
        .bind(key).bind(f64::from(seconds)).fetch_one(pool).await?;
    Ok(count <= limit)
}
async fn request_link(
    State(state): State<AppState>,
    Json(input): Json<EmailRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let email = normalize_email(&input.email)?;
    let response = Json(
        json!({"message":"Odkaz k přihlášení jsme zařadili k odeslání. Zkontrolujte svůj e-mail i složku nevyžádané pošty. Odkaz platí 15 minut."}),
    );
    if !rate_limit(
        &state.pool,
        &format!("login:{}", token_hash(&email)),
        4,
        900,
    )
    .await?
    {
        return Ok(response);
    }
    if !rate_limit(&state.pool, "login:global", 200, 3600).await? {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Odesílání je dočasně omezeno. Zkuste to prosím později.",
        ));
    }
    let token = new_token();
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO login_tokens(token_hash,email,expires_at) VALUES ($1,$2,now()+interval '15 minutes')")
        .bind(token_hash(&token)).bind(&email).execute(&mut *tx).await?;
    let body = format!(
        "Dobrý den,\n\npro přihlášení nebo vytvoření účtu Brunch Era otevřete:\n{}/?login_token={}\n\nOdkaz je jednorázový a platí 15 minut. Pokud jste o něj nežádali, e-mail ignorujte.",
        state.config.public_url, token
    );
    mail::enqueue(&mut tx, &email, mail::LOGIN_SUBJECT, &body).await?;
    tx.commit().await?;
    Ok(response)
}
async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<VerifyRequest>,
) -> ApiResult<Response> {
    if input.token.len() != 64 || !input.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "Odkaz není platný. Požádejte o nový.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    let email:Option<String>=sqlx::query_scalar("UPDATE login_tokens SET used_at=now() WHERE token_hash=$1 AND used_at IS NULL AND expires_at>now() RETURNING email")
        .bind(token_hash(&input.token)).fetch_optional(&mut *tx).await?;
    let email = email.ok_or_else(|| {
        ApiError::bad_request("Odkaz již vypršel nebo byl použit. Požádejte o nový.")
    })?;
    let user=sqlx::query_as::<_,User>("INSERT INTO users(id,email) VALUES ($1,$2) ON CONFLICT(email) DO UPDATE SET email=excluded.email RETURNING id,email,is_admin")
        .bind(Uuid::new_v4()).bind(email).fetch_one(&mut *tx).await?;
    if let Some(old) = session_cookie(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(token_hash(old))
            .execute(&mut *tx)
            .await?;
    }
    let token = new_token();
    let max_age = if user.is_admin { 43200 } else { 2_592_000 };
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES ($1,$2,now()+make_interval(secs=>$3))")
        .bind(token_hash(&token)).bind(user.id).bind(f64::from(max_age)).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut response = Json(json!({"user":user})).into_response();
    let cookie = format!(
        "brunch_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if state.config.secure_cookies {
            "; Secure"
        } else {
            ""
        }
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("hex token cookie"),
    );
    Ok(response)
}
async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(json!({"user":require_user(&state,&headers).await?})))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = session_cookie(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(token_hash(token))
            .execute(&state.pool)
            .await?;
    }
    let mut response = Json(json!({"ok":true})).into_response();
    let cookie = format!(
        "brunch_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if state.config.secure_cookies {
            "; Secure"
        } else {
            ""
        }
    );
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(response)
}
pub async fn cleanup(pool: &PgPool) -> ApiResult<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at<now()")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM login_tokens WHERE expires_at<now()-interval '1 day'")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM request_limits WHERE window_start<now()-interval '1 day'")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emails_normalized_and_headers_rejected() {
        assert_eq!(
            normalize_email(" Test@Example.cz ").unwrap(),
            "test@example.cz"
        );
        for value in [
            "x",
            "A <a@example.cz>",
            "a@example.cz\r\nBcc: x@example.cz",
            "a b@example.cz",
        ] {
            assert!(normalize_email(value).is_err(), "{value}");
        }
    }
    #[test]
    fn secrets_are_random_and_only_hashes_are_stored() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(a, token_hash(&a));
    }
}
