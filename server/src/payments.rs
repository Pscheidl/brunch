//! Teya Hosted Checkout. A browser return is never evidence of payment.
//! Both delivery and recovery use durable PostgreSQL work and authenticated session reads.
use crate::{ApiError, ApiResult, AppState, commerce, mail};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use rsa::{
    RsaPublicKey,
    pkcs1v15::{Signature, VerifyingKey},
    pkcs8::DecodePublicKey,
    signature::Verifier,
    traits::PublicKeyParts,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection};
use std::{env, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct TeyaConfig {
    pub environment: String,
    pub store_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub webhook_public_keys: Vec<RsaPublicKey>,
    pub api_base: String,
    pub token_url: String,
    pub checkout_host: String,
}
impl TeyaConfig {
    pub fn from_env(public_url: &str) -> Result<Option<Self>, String> {
        let Some(environment) = env::var("TEYA_ENVIRONMENT").ok().filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let suffix = match environment.as_str() {
            "staging" => "xyz",
            "production" => "com",
            _ => return Err("TEYA_ENVIRONMENT musí být staging nebo production".into()),
        };
        if !public_url.starts_with("https://") {
            return Err("Teya vyžaduje veřejné PUBLIC_URL s HTTPS pro návrat a webhook".into());
        }
        let required = |name: &str| {
            env::var(name)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("Chybí nastavení {name}"))
        };
        let secret_path = required("TEYA_CLIENT_SECRET_FILE")?;
        let dist_dir = env::var("DIST_DIR").unwrap_or_else(|_| "dist".into());
        let client_secret = read_client_secret(
            std::path::Path::new(&secret_path),
            &[
                std::path::Path::new(&dist_dir),
                std::path::Path::new("public"),
            ],
        )?;
        let mut webhook_public_keys = Vec::new();
        for name in [
            "TEYA_WEBHOOK_PUBLIC_KEY_FILE",
            "TEYA_WEBHOOK_PREVIOUS_PUBLIC_KEY_FILE",
        ] {
            let path = if name == "TEYA_WEBHOOK_PUBLIC_KEY_FILE" {
                required(name)?
            } else if let Ok(path) = env::var(name) {
                if path.is_empty() {
                    continue;
                }
                path
            } else {
                continue;
            };
            let pem = std::fs::read_to_string(path).map_err(|_| format!("Nelze číst {name}"))?;
            let key = RsaPublicKey::from_public_key_pem(&pem)
                .map_err(|_| format!("Neplatný RSA veřejný klíč v {name}"))?;
            if key.n().bits() < 2048 {
                return Err("Webhook klíč musí mít alespoň 2048 bitů".into());
            }
            webhook_public_keys.push(key);
        }
        Ok(Some(Self {
            store_id: required("TEYA_STORE_ID")?,
            client_id: required("TEYA_CLIENT_ID")?,
            client_secret,
            api_base: format!("https://api.teya.{suffix}"),
            token_url: format!("https://id.teya.{suffix}/oauth/v2/oauth-token"),
            checkout_host: format!("checkout.teya.{suffix}"),
            environment,
            webhook_public_keys,
        }))
    }
}

fn read_client_secret(
    path: &std::path::Path,
    public_dirs: &[&std::path::Path],
) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("TEYA_CLIENT_SECRET_FILE musí být absolutní cesta".into());
    }
    let resolved = path
        .canonicalize()
        .map_err(|_| "Nelze číst soubor s tajným klíčem Teya")?;
    if public_dirs
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .any(|dir| resolved.starts_with(dir))
    {
        return Err("Tajný klíč Teya nesmí být ve veřejném adresáři webu".into());
    }
    let file = std::fs::File::open(&resolved).map_err(|_| "Nelze číst tajný klíč Teya")?;
    let metadata = file.metadata().map_err(|_| "Nelze číst tajný klíč Teya")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Soubor TEYA_CLIENT_SECRET_FILE musí mít oprávnění 0600 nebo 0400".into());
        }
    }
    if !metadata.is_file() || metadata.len() > 8192 {
        return Err("Neplatný soubor s tajným klíčem Teya".into());
    }
    use std::io::Read;
    let mut secret = String::new();
    file.take(8193)
        .read_to_string(&mut secret)
        .map_err(|_| "Nelze číst tajný klíč Teya")?;
    if secret.len() > 8192 || secret.trim().is_empty() {
        return Err("Neplatný nebo prázdný tajný klíč Teya".into());
    }
    Ok(secret.trim().to_owned())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/orders/{id}/payment", post(payment_status))
        .route("/api/payments/teya/webhook", post(webhook))
}

// Called in the same transaction as the order and its stock reservation.
pub async fn enqueue_configured(
    conn: &mut PgConnection,
    id: Uuid,
    state: &AppState,
) -> ApiResult<()> {
    let config = state.config.teya.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Online platba není dostupná.",
        )
    })?;
    let total: i64 = sqlx::query_scalar("SELECT total_cents FROM orders WHERE id=$1 AND payment_method='card' AND status='awaiting_payment'")
        .bind(id).fetch_one(&mut *conn).await?;
    let return_url = format!("{}/?payment_return={id}", state.config.public_url);
    let body = json!({"amount":{"value":total,"currency":"CZK"},"type":"SALE","merchant_reference":id.to_string(),"store_id":config.store_id,"success_url":return_url,"cancel_url":return_url,"failure_url":return_url,"language":"cs-CZ","post_success_payment":"REDIRECT"});
    sqlx::query("INSERT INTO payment_attempts(order_id,idempotency_key,environment,store_id,request_body,expires_at) SELECT id,$2,$3,$4,$5,created_at+make_interval(mins=>$6) FROM orders WHERE id=$1")
        .bind(id).bind(Uuid::new_v4()).bind(&config.environment).bind(&config.store_id).bind(body).bind(state.config.payment_timeout_minutes as i32).execute(conn).await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentAccess {
    idempotency_key: Uuid,
}
#[derive(Serialize, FromRow)]
struct PaymentStatus {
    status: String,
    payment_status: String,
    checkout_url: Option<String>,
    payment_state: String,
    expires_at: DateTime<Utc>,
}
async fn payment_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<PaymentAccess>,
) -> ApiResult<Json<PaymentStatus>> {
    // The unguessable original checkout key is a capability, never put it in a URL or logs.
    // This endpoint exposes no address or contact data and performs no calls to Teya.
    let mut result: PaymentStatus = sqlx::query_as("SELECT o.status,o.payment_status,p.checkout_url,p.state AS payment_state,p.expires_at FROM orders o JOIN payment_attempts p ON p.order_id=o.id WHERE o.id=$1 AND o.idempotency_key=$2")
        .bind(id).bind(input.idempotency_key).fetch_optional(&state.pool).await?
        .ok_or_else(|| ApiError::not_found("Platba neexistuje."))?;
    if state.config.teya.is_none()
        || result.payment_status == "paid"
        || result.payment_state != "ready"
        || result.status != "awaiting_payment"
        || result.expires_at <= Utc::now()
    {
        result.checkout_url = None;
    }
    Ok(Json(result))
}

pub async fn can_cancel(conn: &mut PgConnection, id: Uuid) -> ApiResult<bool> {
    // Authenticated terminal sessions cannot receive another payment.
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payment_attempts WHERE order_id=$1 AND state IN ('expired','failed') AND checked_at IS NOT NULL)")
        .bind(id).fetch_one(conn).await?)
}

#[derive(Deserialize)]
struct Notification {
    event: String,
    data: NotificationData,
}
#[derive(Deserialize)]
struct NotificationData {
    transaction_id: String,
    session_id: String,
    merchant_reference: String,
    amount: Amount,
    status: String,
}
#[derive(Deserialize)]
struct Amount {
    value: i64,
    currency: String,
}

fn verify_signature(config: &TeyaConfig, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(encoded) = headers
        .get("x-teya-signature")
        .and_then(|s| s.to_str().ok())
    else {
        return false;
    };
    let Ok(bytes) = STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(bytes.as_slice()) else {
        return false;
    };
    config.webhook_public_keys.iter().any(|key| {
        VerifyingKey::<Sha256>::new(key.clone())
            .verify(body, &signature)
            .is_ok()
    })
}
async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let config = state.config.teya.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Příjem plateb není nakonfigurován.",
        )
    })?;
    if !verify_signature(config, &headers, &body) {
        return Err(ApiError::unauthorized("Neplatný podpis oznámení."));
    }
    let envelope: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("Neplatné oznámení."))?;
    if envelope.get("event").and_then(Value::as_str) != Some("payment.succeeded.v1") {
        return Ok(StatusCode::NO_CONTENT);
    }
    let notification: Notification = serde_json::from_value(envelope)
        .map_err(|_| ApiError::bad_request("Neplatné oznámení."))?;
    let data = &notification.data;
    if notification.event != "payment.succeeded.v1"
        || data.status != "SUCCESS"
        || !valid_id(&data.session_id)
        || !valid_id(&data.transaction_id)
    {
        return Err(ApiError::bad_request("Neplatné oznámení platby."));
    }
    let id = Uuid::parse_str(&data.merchant_reference)
        .map_err(|_| ApiError::bad_request("Neznámá reference."))?;
    let mut tx = state.pool.begin().await?;
    // Lock orders before attempts consistently with administration and reconciliation.
    let total: Option<i64> = sqlx::query_scalar(
        "SELECT total_cents FROM orders WHERE id=$1 AND payment_method='card' FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let attempt: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT environment,store_id,session_id FROM payment_attempts WHERE order_id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((environment, store_id, session_id)) = attempt else {
        return Err(ApiError::not_found("Neznámá platba."));
    };
    if total != Some(data.amount.value)
        || data.amount.currency != "CZK"
        || environment != config.environment
        || store_id != config.store_id
        || session_id.as_ref().is_some_and(|s| s != &data.session_id)
    {
        return Err(ApiError::conflict("Oznámení neodpovídá objednávce."));
    }
    let hash = hex::encode(Sha256::digest(&body));
    let inserted = sqlx::query("INSERT INTO payment_notifications(body_hash,order_id,session_id,transaction_id) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind(hash).bind(id).bind(&data.session_id).bind(&data.transaction_id).execute(&mut *tx).await?.rows_affected();
    if inserted > 0 {
        // Even if a create response was lost, a signed notification can supply the session ID.
        // The worker must verify store, reference, amount, SALE and SUCCESS through the API.
        sqlx::query("UPDATE payment_attempts SET candidate_session_id=CASE WHEN session_id IS NULL THEN $2 ELSE candidate_session_id END,state=CASE WHEN state='paid' THEN state ELSE 'pending' END,next_check_at=now() WHERE order_id=$1")
            .bind(id).bind(&data.session_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

#[derive(FromRow)]
struct Attempt {
    order_id: Uuid,
    idempotency_key: Uuid,
    environment: String,
    store_id: String,
    request_body: Value,
    session_id: Option<String>,
    candidate_session_id: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    failures: i32,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
}
#[derive(Deserialize)]
struct Created {
    session_id: String,
    session_url: String,
}
#[derive(Deserialize)]
struct BusinessData {
    id: String,
}
#[derive(Deserialize)]
struct Session {
    session_id: String,
    session_status: String,
    #[serde(default)]
    payment_status: String,
    transaction_id: Option<String>,
    merchant_reference: String,
    amount: Amount,
    business_data: BusinessData,
    #[serde(rename = "type")]
    kind: String,
}

fn checkout_url(config: &TeyaConfig, value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && (url.host_str() == Some(config.checkout_host.as_str())
                || (config.environment == "staging" && url.host_str() == Some("checkout.teya.com")))
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

// Safe error codes only. Never log or persist provider response bodies or bearer tokens.
async fn provider_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, &'static str> {
    if !response.status().is_success() {
        return Err("provider_http_error");
    }
    if response.content_length().is_some_and(|n| n > 64 * 1024) {
        return Err("provider_response_too_large");
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "provider_read_error")? {
        if body.len() + chunk.len() > 64 * 1024 {
            return Err("provider_response_too_large");
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "provider_invalid_response")
}

pub async fn process_batch(state: &AppState) -> ApiResult<()> {
    let Some(config) = &state.config.teya else {
        return Ok(());
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("BrunchEra/0.1 TeyaHostedCheckout")
        .build()
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Platební klient není dostupný.",
            )
        })?;
    for _ in 0..10 {
        let lease = Uuid::new_v4();
        let attempt: Option<Attempt> = sqlx::query_as("UPDATE payment_attempts SET lease_id=$1,lease_until=now()+interval '2 minutes' WHERE order_id=(SELECT order_id FROM payment_attempts WHERE state IN ('pending','ready') AND next_check_at<=now() AND (lease_until IS NULL OR lease_until<now()) ORDER BY next_check_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *")
            .bind(lease).fetch_optional(&state.pool).await?;
        let Some(attempt) = attempt else { break };
        if attempt.environment != config.environment || attempt.store_id != config.store_id {
            review(state, &attempt, lease, "configuration_changed").await?;
            continue;
        }
        match reconcile(state, config, &client, &attempt, lease).await {
            Ok(()) => (),
            Err(error) => {
                if error == "receipt_conflict" || error == "order_transition_error" {
                    review(state, &attempt, lease, error).await?;
                    continue;
                }
                let delay = (15_i64 * (1_i64 << attempt.failures.clamp(0, 8))).min(3600);
                sqlx::query("UPDATE payment_attempts SET lease_id=NULL,lease_until=NULL,failures=LEAST(failures+1,1000),last_error=$3,next_check_at=now()+make_interval(secs=>$4) WHERE order_id=$1 AND lease_id=$2")
                    .bind(attempt.order_id).bind(lease).bind(error).bind(delay as f64).execute(&state.pool).await?;
                tracing::warn!(order_id=%attempt.order_id, code=error, "Payment verification will retry");
            }
        }
    }
    Ok(())
}
async fn review(state: &AppState, attempt: &Attempt, lease: Uuid, error: &str) -> ApiResult<()> {
    let missing_session = matches!(error, "order_deadline_passed" | "create_outcome_unknown");
    // A webhook can persist a recoverable session after this worker read its attempt.
    // Decide under the row lock whether review is still needed, so that acknowledged
    // notifications always leave runnable work even if this stale worker finishes last.
    let result: Option<String> = sqlx::query_scalar("UPDATE payment_attempts SET state=CASE WHEN $4 AND (session_id IS NOT NULL OR candidate_session_id IS NOT NULL) THEN 'pending' ELSE 'review' END,last_error=CASE WHEN $4 AND (session_id IS NOT NULL OR candidate_session_id IS NOT NULL) THEN NULL ELSE $3 END,lease_id=NULL,lease_until=NULL,next_check_at=now() WHERE order_id=$1 AND lease_id=$2 RETURNING state")
        .bind(attempt.order_id).bind(lease).bind(error).bind(missing_session).fetch_optional(&state.pool).await?;
    if result.as_deref() == Some("review") {
        tracing::error!(order_id=%attempt.order_id, code=error, "Payment needs operator review");
    }
    Ok(())
}
async fn reconcile(
    state: &AppState,
    config: &TeyaConfig,
    client: &reqwest::Client,
    attempt: &Attempt,
    lease: Uuid,
) -> Result<(), &'static str> {
    if attempt.session_id.is_none()
        && attempt.candidate_session_id.is_none()
        && Utc::now() >= attempt.expires_at
    {
        review(state, attempt, lease, "order_deadline_passed")
            .await
            .map_err(|_| "database_error")?;
        return Ok(());
    }
    // Teya does not publish an idempotency retention period. Never automatically recreate
    // an ambiguous old request. A later signed webhook can recover its original session.
    if attempt.session_id.is_none()
        && attempt.candidate_session_id.is_none()
        && Utc::now() - attempt.created_at > chrono::Duration::hours(1)
    {
        review(state, attempt, lease, "create_outcome_unknown")
            .await
            .map_err(|_| "database_error")?;
        return Ok(());
    }
    let token: Token = provider_json(
        client
            .post(&config.token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &config.client_id),
                ("client_secret", &config.client_secret),
                ("scope", "checkout/sessions/create checkout/sessions/id/get"),
            ])
            .send()
            .await
            .map_err(|_| "provider_unavailable")?,
    )
    .await?;
    let is_candidate = attempt.session_id.is_none() && attempt.candidate_session_id.is_some();
    let session_id = if let Some(id) = attempt
        .session_id
        .as_ref()
        .or(attempt.candidate_session_id.as_ref())
    {
        id.clone()
    } else {
        // OAuth may itself take time. Recheck before creating a new payable session.
        if Utc::now() >= attempt.expires_at {
            review(state, attempt, lease, "order_deadline_passed")
                .await
                .map_err(|_| "database_error")?;
            return Ok(());
        }
        let created: Created = provider_json(
            client
                .post(format!("{}/v2/checkout/sessions", config.api_base))
                .bearer_auth(&token.access_token)
                .header("Idempotency-Key", attempt.idempotency_key.to_string())
                .json(&attempt.request_body)
                .send()
                .await
                .map_err(|_| "create_outcome_unknown")?,
        )
        .await?;
        if !valid_id(&created.session_id) || !checkout_url(config, &created.session_url) {
            return Err("unsafe_checkout_response");
        }
        let saved = sqlx::query("UPDATE payment_attempts SET session_id=$3,checkout_url=$4 WHERE order_id=$1 AND lease_id=$2 AND (session_id IS NULL OR session_id=$3)")
            .bind(attempt.order_id).bind(lease).bind(&created.session_id).bind(&created.session_url).execute(&state.pool).await.map_err(|_| "database_error")?.rows_affected();
        if saved != 1 {
            return Err("lease_or_session_changed");
        }
        created.session_id
    };
    let response = client
        .get(format!(
            "{}/v2/checkout/sessions/{session_id}",
            config.api_base
        ))
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|_| "provider_unavailable")?;
    if is_candidate && matches!(response.status().as_u16(), 403 | 404) {
        discard_candidate(state, attempt, lease, &session_id).await?;
        return Ok(());
    }
    let session: Session = provider_json(response).await?;
    let matches = session.session_id == session_id
        && session.kind == "SALE"
        && session.merchant_reference == attempt.order_id.to_string()
        && session.amount.currency == "CZK"
        && Some(session.amount.value) == attempt.request_body["amount"]["value"].as_i64()
        && session.business_data.id == attempt.store_id;
    if !matches && is_candidate {
        // A signing key might be shared by multiple merchants. An unverified candidate
        // must not replace our real session or permanently block creation/recovery.
        discard_candidate(state, attempt, lease, &session_id).await?;
        return Ok(());
    }
    if !matches {
        review(state, attempt, lease, "provider_data_mismatch")
            .await
            .map_err(|_| "database_error")?;
        return Ok(());
    }
    if is_candidate {
        let saved = sqlx::query("UPDATE payment_attempts SET session_id=$3,candidate_session_id=NULL WHERE order_id=$1 AND lease_id=$2 AND session_id IS NULL")
            .bind(attempt.order_id).bind(lease).bind(&session_id).execute(&state.pool).await.map_err(|_| "database_error")?.rows_affected();
        if saved != 1 {
            return Err("lease_or_session_changed");
        }
    }
    let next = match (
        session.session_status.as_str(),
        session.payment_status.as_str(),
    ) {
        ("COMPLETED", "SUCCESS") => "paid",
        ("COMPLETED", "FAILED") => "failed",
        ("EXPIRED", "NONE" | "FAILED") => "expired",
        ("ACTIVE", "" | "NONE" | "FAILED") => "ready",
        ("PROCESSING", "" | "NONE" | "FAILED") => "pending",
        _ => return Err("unrecognized_provider_status"),
    };
    let mut tx = state.pool.begin().await.map_err(|_| "database_error")?;
    let mut order_status: String =
        sqlx::query_scalar("SELECT status FROM orders WHERE id=$1 FOR UPDATE")
            .bind(attempt.order_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "database_error")?;
    let ownership: (Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT lease_id,session_id FROM payment_attempts WHERE order_id=$1 FOR UPDATE",
    )
    .bind(attempt.order_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "database_error")?;
    if ownership.0 != Some(lease) || ownership.1.as_deref() != Some(&session_id) {
        return Err("lease_or_session_changed");
    }
    // Evaluate wall-clock time only after both row locks have been acquired. PostgreSQL
    // now() is frozen at transaction start, which may precede a wait across the deadline.
    let deadline_passed: bool = sqlx::query_scalar(
        "SELECT expires_at<=clock_timestamp() FROM payment_attempts WHERE order_id=$1",
    )
    .bind(attempt.order_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| "database_error")?;
    if deadline_passed
        && order_status == "awaiting_payment"
        && commerce::expire_unpaid_order(&mut tx, attempt.order_id)
            .await
            .map_err(|_| "database_error")?
    {
        order_status = "expired".into();
    }
    let mut final_state = next;
    let mut final_error: Option<&str> = None;
    if next == "paid" {
        let transaction = session
            .transaction_id
            .as_ref()
            .filter(|s| valid_id(s))
            .ok_or("missing_transaction_id")?;
        let receipt_inserted = sqlx::query("INSERT INTO payment_receipts(order_id,transaction_id,session_id,amount_cents,currency) VALUES ($1,$2,$3,$4,'CZK') ON CONFLICT(order_id) DO NOTHING")
            .bind(attempt.order_id).bind(transaction).bind(&session_id).bind(session.amount.value).execute(&mut *tx).await.map_err(|error| if matches!(&error, sqlx::Error::Database(error) if error.is_unique_violation()) { "receipt_conflict" } else { "database_error" })?.rows_affected() > 0;
        let receipt_matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payment_receipts WHERE order_id=$1 AND transaction_id=$2 AND session_id=$3 AND amount_cents=$4)")
            .bind(attempt.order_id).bind(transaction).bind(&session_id).bind(session.amount.value).fetch_one(&mut *tx).await.map_err(|_| "database_error")?;
        if !receipt_matches {
            return Err("receipt_conflict");
        }
        if matches!(order_status.as_str(), "cancelled" | "expired") {
            // Preserve money received after fulfilment has already been closed and stock released.
            final_state = "review";
            final_error = Some(if order_status == "expired" {
                "paid_expired_order"
            } else {
                "paid_cancelled_order"
            });
            if receipt_inserted {
                notify_late_payment(&mut tx, attempt.order_id, &order_status)
                    .await
                    .map_err(|_| "database_error")?;
            }
            tracing::error!(order_id=%attempt.order_id, status=%order_status, "Late payment needs operator review");
        } else {
            commerce::payment_received(&mut tx, attempt.order_id)
                .await
                .map_err(|error| {
                    if error.status == StatusCode::CONFLICT {
                        "order_transition_error"
                    } else {
                        "database_error"
                    }
                })?;
        }
    }
    sqlx::query("UPDATE payment_attempts SET state=$3,checked_at=now(),next_check_at=now()+interval '30 seconds',lease_id=NULL,lease_until=NULL,failures=0,last_error=$4 WHERE order_id=$1 AND lease_id=$2")
        .bind(attempt.order_id).bind(lease).bind(final_state).bind(final_error).execute(&mut *tx).await.map_err(|_| "database_error")?;
    tx.commit().await.map_err(|_| "database_error")?;
    Ok(())
}

/// Separate from provider polling so an HTTP outage cannot prevent order expiration.
pub async fn expire_orders(state: &AppState) -> ApiResult<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT o.id FROM orders o JOIN payment_attempts p ON p.order_id=o.id WHERE o.status='awaiting_payment' AND o.payment_method='card' AND o.payment_status='unpaid' AND p.expires_at<=now() ORDER BY p.expires_at,o.id LIMIT 100")
        .fetch_all(&state.pool).await?;
    let mut failure = None;
    for id in ids {
        if let Err(error) = expire_one(state, id).await {
            tracing::error!(order_id=%id, "Order expiration failed and will retry");
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}

async fn expire_one(state: &AppState, id: Uuid) -> ApiResult<()> {
    let mut tx = state.pool.begin().await?;
    let locked: Option<Uuid> = sqlx::query_scalar("SELECT id FROM orders WHERE id=$1 AND status='awaiting_payment' AND payment_status='unpaid' FOR UPDATE SKIP LOCKED")
        .bind(id).fetch_optional(&mut *tx).await?;
    if locked.is_none() {
        return Ok(());
    }
    let due: bool = sqlx::query_scalar(
        "SELECT expires_at<=clock_timestamp() FROM payment_attempts WHERE order_id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if due {
        commerce::expire_unpaid_order(&mut tx, id).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn notify_late_payment(conn: &mut PgConnection, id: Uuid, status: &str) -> ApiResult<()> {
    let (number, email, amount): (String, String, i64) =
        sqlx::query_as("SELECT number,email,total_cents FROM orders WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let shop_email: String = sqlx::query_scalar("SELECT order_email FROM shop_settings WHERE id=1")
        .fetch_one(&mut *conn)
        .await?;
    let label = if status == "expired" {
        "vypršelé"
    } else {
        "zrušené"
    };
    mail::enqueue(conn, &shop_email, &format!("Pozdní platba objednávky {number}: nutná kontrola"),
        &format!("Teya potvrdila úhradu {},{:02} Kč u {label} objednávky {number}. Peníze jsou zaevidované, objednávka nebyla znovu přijata k vyřízení a sklad se znovu neodečetl. Ověřte situaci v portálu Teya, kontaktujte zákazníka a vyřešte případné vrácení platby.", amount / 100, amount % 100)).await?;
    mail::enqueue(conn, &email, &format!("Přijatá platba k uzavřené objednávce {number}"),
        &format!("Obdrželi jsme potvrzení vaší platby k {label} objednávce {number}. Platbu jsme zaevidovali a předali obsluze k vyřešení. Objednávka se automaticky znovu neotevřela. Znovu ji neplaťte. S dotazy se obraťte na {shop_email}." )).await?;
    Ok(())
}

async fn discard_candidate(
    state: &AppState,
    attempt: &Attempt,
    lease: Uuid,
    session_id: &str,
) -> Result<(), &'static str> {
    sqlx::query("UPDATE payment_attempts SET candidate_session_id=NULL,lease_id=NULL,lease_until=NULL,next_check_at=now()+interval '15 seconds',last_error='notification_store_mismatch' WHERE order_id=$1 AND lease_id=$2 AND candidate_session_id=$3")
        .bind(attempt.order_id).bind(lease).bind(session_id).execute(&state.pool).await.map_err(|_| "database_error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_redirects_require_exact_https_teya_host() {
        let mut config = TeyaConfig {
            environment: "production".into(),
            store_id: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            webhook_public_keys: vec![],
            api_base: String::new(),
            token_url: String::new(),
            checkout_host: "checkout.teya.com".into(),
        };
        assert!(checkout_url(
            &config,
            "https://checkout.teya.com/pay/session"
        ));
        for url in [
            "http://checkout.teya.com/pay/session",
            "https://checkout.teya.com.attacker.test/pay",
            "https://checkout.teya.com@attacker.test/pay",
            "https://attacker.test@checkout.teya.com/pay",
            "https://checkout.teya.com:8443/pay",
            "javascript:alert(1)",
            "https://checkout.teya.xyz/pay/session",
        ] {
            assert!(!checkout_url(&config, url), "unsafe redirect {url}");
        }
        config.environment = "staging".into();
        config.checkout_host = "checkout.teya.xyz".into();
        assert!(checkout_url(
            &config,
            "https://checkout.teya.xyz/pay/session"
        ));
        assert!(checkout_url(
            &config,
            "https://checkout.teya.com/pay/session"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn credential_file_requires_private_permissions_and_nonpublic_location() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let root = env::temp_dir().join(format!("brunch-secret-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("test-secret");
        fs::write(&path, "test-only-secret\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_client_secret(&path, &[]).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_client_secret(&path, &[]).unwrap(), "test-only-secret");
        assert!(read_client_secret(&path, &[root.as_path()]).is_err());
        let link = root.join("secret-link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_client_secret(&link, &[root.as_path()]).is_err());
        assert!(read_client_secret(std::path::Path::new("relative-secret"), &[]).is_err());
        fs::write(&path, " \n").unwrap();
        assert!(read_client_secret(&path, &[]).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
