use crate::{ApiError, ApiResult, AppState, mail};
use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
}
#[derive(Debug, Serialize, FromRow)]
struct Administrator {
    id: Uuid,
    email: String,
}

// Every administrator membership change and bootstrap takes this transaction lock.
const ADMINISTRATOR_LOCK: i64 = 87_654_321;
const ADMIN_SESSION_SECONDS: i32 = 43_200;
static PASSWORD_WORK: Semaphore = Semaphore::const_new(2);
// A syntactically valid dummy hash makes unknown accounts do the same expensive
// verification work. It is not a credential and is never stored on an account.
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZS1kdW1teS1zYWx0IQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const USER_SESSION_QUERY: &str = "SELECT u.id,u.email,(u.is_admin AND s.admin_authenticated AND u.admin_password_hash IS NOT NULL) AS is_admin FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>clock_timestamp()";

#[derive(Deserialize)]
struct PasswordRequest {
    email: String,
    password: String,
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
        .route("/api/admin/auth/login", post(password_login))
        .route(
            "/api/admin/administrators/password",
            put(change_administrator_password),
        )
        .route(
            "/api/admin/administrators",
            get(administrators)
                .post(add_administrator)
                .delete(remove_administrator),
        )
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
    Ok(sqlx::query_as::<_, User>(USER_SESSION_QUERY)
        .bind(token_hash(token))
        .fetch_optional(&state.pool)
        .await?)
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
fn password_engine() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19_456, 2, 1, None).expect("valid Argon2 parameters"),
    )
}

pub fn validate_admin_password(password: &str) -> ApiResult<()> {
    if password.len() > 512 || !(12..=128).contains(&password.chars().count()) {
        return Err(ApiError::bad_request("Heslo musí mít 12 až 128 znaků."));
    }
    Ok(())
}

fn password_busy() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "Přihlášení je dočasně vytížené. Zkuste to za chvíli.",
    )
}

async fn hash_password(password: &str) -> ApiResult<String> {
    validate_admin_password(password)?;
    let permit = PASSWORD_WORK.try_acquire().map_err(|_| password_busy())?;
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let salt = SaltString::generate(&mut OsRng);
        password_engine()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Heslo se nepodařilo bezpečně uložit.",
                )
            })
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Heslo se nepodařilo bezpečně uložit.",
        )
    })?
}

async fn password_matches(password: &str, encoded: &str) -> ApiResult<bool> {
    let permit = PASSWORD_WORK.try_acquire().map_err(|_| password_busy())?;
    let password = password.to_owned();
    let encoded = encoded.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&encoded).is_ok_and(|hash| {
            password_engine()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Přihlášení se nepodařilo ověřit.",
        )
    })
}

/// Explicit installation command only. Ordinary startup never changes admin roles.
pub async fn initialize_admin(pool: &PgPool, email: &str, password: &str) -> ApiResult<()> {
    let email = normalize_email(email)?;
    let hash = hash_password(password).await?;
    let mut tx = pool.begin().await?;
    administrator_lock(&mut tx).await?;
    let has_admin: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE is_admin)")
        .fetch_one(&mut *tx)
        .await?;
    if has_admin {
        return Err(ApiError::conflict(
            "Administrátor už existuje. Další účty spravujte v administraci.",
        ));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO users(id,email,is_admin,admin_password_hash) VALUES ($1,$2,true,$3) ON CONFLICT(email) DO UPDATE SET is_admin=true,admin_password_hash=excluded.admin_password_hash RETURNING id")
        .bind(Uuid::new_v4()).bind(email).bind(hash).fetch_one(&mut *tx).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn administrator_lock(tx: &mut Transaction<'_, Postgres>) -> ApiResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADMINISTRATOR_LOCK)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn password_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordRequest>,
) -> ApiResult<Response> {
    let invalid = || ApiError::unauthorized("Nesprávný e-mail nebo heslo.");
    let email = normalize_email(&input.email).map_err(|_| invalid())?;
    if input.password.len() > 512 || input.password.chars().count() > 128 {
        return Err(invalid());
    }
    if !rate_limit(
        &state.pool,
        &format!("admin-login:{}", token_hash(&email)),
        10,
        900,
    )
    .await?
        || !rate_limit(&state.pool, "admin-login:global", 200, 3600).await?
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Příliš mnoho pokusů o přihlášení. Zkuste to později.",
        ));
    }
    let stored: Option<(Uuid, String)> = sqlx::query_as("SELECT id,admin_password_hash FROM users WHERE email=$1 AND is_admin AND admin_password_hash IS NOT NULL")
        .bind(&email).fetch_optional(&state.pool).await?;
    let encoded = stored
        .as_ref()
        .map(|(_, hash)| hash.as_str())
        .unwrap_or(DUMMY_PASSWORD_HASH);
    let matches = password_matches(&input.password, encoded).await?;
    let Some((id, hash)) = stored.filter(|_| matches) else {
        return Err(invalid());
    };
    // A password reset or revocation may finish while Argon2 is running.
    let mut tx = state.pool.begin().await?;
    administrator_lock(&mut tx).await?;
    let user: User = sqlx::query_as(
        "SELECT id,email,is_admin FROM users WHERE id=$1 AND is_admin AND admin_password_hash=$2",
    )
    .bind(id)
    .bind(hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid)?;
    if let Some(old) = session_cookie(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(token_hash(old))
            .execute(&mut *tx)
            .await?;
    }
    let token = new_token();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,admin_authenticated) VALUES ($1,$2,clock_timestamp()+make_interval(secs=>$3),true)")
        .bind(token_hash(&token)).bind(id).bind(f64::from(ADMIN_SESSION_SECONDS)).execute(&mut *tx).await?;
    tx.commit().await?;
    session_response(&state, user, &token, ADMIN_SESSION_SECONDS)
}

fn session_response(
    state: &AppState,
    user: User,
    token: &str,
    max_age: i32,
) -> ApiResult<Response> {
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

async fn administrators(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&state, &headers).await?;
    let administrators: Vec<Administrator> =
        sqlx::query_as("SELECT id,email FROM users WHERE is_admin ORDER BY email,id")
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(json!({"administrators": administrators})))
}

async fn administrator_transaction<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> ApiResult<(Transaction<'a, Postgres>, User)> {
    let token = session_cookie(headers)
        .filter(|token| token.len() == 64)
        .ok_or_else(|| ApiError::unauthorized("Nejprve se přihlaste e-mailem."))?;
    let mut tx = state.pool.begin().await?;
    administrator_lock(&mut tx).await?;
    // Check the current session after acquiring the membership lock. A request
    // queued by an administrator whose rights were removed must not grant access.
    let actor = sqlx::query_as::<_, User>(USER_SESSION_QUERY)
        .bind(token_hash(token))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::unauthorized("Nejprve se přihlaste e-mailem."))?;
    if !actor.is_admin {
        return Err(ApiError::forbidden("Tato stránka je určena správci."));
    }
    Ok((tx, actor))
}

async fn add_administrator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&state, &headers).await?;
    let email = normalize_email(&input.email)?;
    let hash = hash_password(&input.password).await?;
    let (mut tx, actor) = administrator_transaction(&state, &headers).await?;
    let already_admin: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1 AND is_admin)")
            .bind(&email)
            .fetch_one(&mut *tx)
            .await?;
    if already_admin {
        return Err(ApiError::conflict(
            "Tento e-mail už patří administrátorovi.",
        ));
    }
    let administrator: Administrator = sqlx::query_as("INSERT INTO users(id,email,is_admin,admin_password_hash) VALUES ($1,$2,true,$3) ON CONFLICT(email) DO UPDATE SET is_admin=true,admin_password_hash=excluded.admin_password_hash RETURNING id,email")
        .bind(Uuid::new_v4()).bind(email).bind(hash).fetch_one(&mut *tx).await?;
    // Customer sessions must never gain administrator access after promotion.
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(administrator.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(actor_id = %actor.id, administrator_id = %administrator.id, "Administrator added");
    Ok(Json(json!({"administrator": administrator})))
}

async fn remove_administrator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let (mut tx, actor) = administrator_transaction(&state, &headers).await?;
    let email = normalize_email(&input.email)?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email=$1 AND is_admin")
        .bind(email)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Administrátor s tímto e-mailem neexistuje."))?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin")
        .fetch_one(&mut *tx)
        .await?;
    if count <= 1 {
        return Err(ApiError::conflict(
            "Posledního administrátora nelze odebrat.",
        ));
    }
    sqlx::query("UPDATE users SET is_admin=false,admin_password_hash=NULL WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(actor_id = %actor.id, administrator_id = %id, "Administrator removed");
    Ok(Json(json!({"ok": true})))
}
async fn change_administrator_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_admin(&state, &headers).await?;
    let email = normalize_email(&input.email)?;
    let hash = hash_password(&input.password).await?;
    let (mut tx, actor) = administrator_transaction(&state, &headers).await?;
    let id: Uuid = sqlx::query_scalar(
        "UPDATE users SET admin_password_hash=$1 WHERE email=$2 AND is_admin RETURNING id",
    )
    .bind(hash)
    .bind(email)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("Administrátor s tímto e-mailem neexistuje."))?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(actor_id = %actor.id, administrator_id = %id, "Administrator password reset");
    Ok(Json(json!({"ok": true})))
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
    let mut user=sqlx::query_as::<_,User>("INSERT INTO users(id,email) VALUES ($1,$2) ON CONFLICT(email) DO UPDATE SET email=excluded.email RETURNING id,email,is_admin")
        .bind(Uuid::new_v4()).bind(email).fetch_one(&mut *tx).await?;
    if let Some(old) = session_cookie(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(token_hash(old))
            .execute(&mut *tx)
            .await?;
    }
    let token = new_token();
    user.is_admin = false;
    let max_age = 2_592_000;
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES ($1,$2,now()+make_interval(secs=>$3))")
        .bind(token_hash(&token)).bind(user.id).bind(f64::from(max_age)).execute(&mut *tx).await?;
    tx.commit().await?;
    session_response(&state, user, &token, max_age)
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
    fn administrator_password_lengths_preserve_unicode_and_spaces() {
        for password in ["short".to_owned(), "a".repeat(129), "🦀".repeat(129)] {
            assert!(validate_admin_password(&password).is_err());
        }
        for password in [
            "a".repeat(12),
            "🦀".repeat(128),
            "a password with spaces".to_owned(),
        ] {
            assert!(validate_admin_password(&password).is_ok());
        }
    }
    #[tokio::test]
    async fn dummy_hash_is_valid_and_password_work_is_bounded() {
        let hash = PasswordHash::new(DUMMY_PASSWORD_HASH).unwrap();
        assert_eq!(hash.algorithm.as_str(), "argon2id");
        assert_eq!(hash.params.get_decimal("m"), Some(19_456));
        assert!(
            !password_matches("Unknown account test password", DUMMY_PASSWORD_HASH)
                .await
                .unwrap()
        );
        let _all_slots = PASSWORD_WORK.acquire_many(2).await.unwrap();
        assert_eq!(
            hash_password("A sufficiently long password")
                .await
                .unwrap_err()
                .status,
            StatusCode::TOO_MANY_REQUESTS
        );
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
