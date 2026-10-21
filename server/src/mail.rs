use crate::{ApiError, ApiResult, AppState, Config, auth};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    transport::smtp::authentication::Credentials,
};
use serde::Serialize;
use serde_json::json;
use sqlx::{FromRow, PgConnection};
use std::time::Duration;
use uuid::Uuid;

pub async fn enqueue(
    conn: &mut PgConnection,
    to: &str,
    subject: &str,
    body: &str,
) -> ApiResult<()> {
    let to = auth::normalize_email(to)?;
    sqlx::query("INSERT INTO outbox(id,to_email,subject,body) VALUES ($1,$2,$3,$4)")
        .bind(Uuid::new_v4())
        .bind(to)
        .bind(subject)
        .bind(body)
        .execute(conn)
        .await?;
    Ok(())
}
pub fn transport(
    config: &Config,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, Box<dyn std::error::Error>> {
    let builder = if config.smtp_tls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.smtp_host)
    };
    let mut builder = builder
        .port(config.smtp_port)
        .timeout(Some(Duration::from_secs(20)));
    if let (Some(username), Some(password)) = (&config.smtp_username, &config.smtp_password) {
        builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
    }
    Ok(builder.build())
}
#[derive(FromRow)]
struct PendingMail {
    id: Uuid,
    to_email: String,
    subject: String,
    body: String,
    attempts: i32,
    lease_token: Uuid,
}
/// Claim one row at a time with a lease so crashed workers recover, SMTP is at-least-once.
/// A stable Message-ID helps recipients deduplicate retries after an ambiguous SMTP result.
pub async fn deliver_batch(
    state: &AppState,
    transport: &AsyncSmtpTransport<Tokio1Executor>,
) -> ApiResult<usize> {
    // Expired magic links must never remain queued for a delayed delivery or an admin retry.
    sqlx::query("UPDATE outbox SET status='failed',body='[Přihlašovací odkaz odstraněn po vypršení]',last_error='Přihlašovací odkaz vypršel. Požádejte o nový.',locked_until=NULL,lease_token=NULL WHERE status='pending' AND subject='Přihlášení do Brunch' AND created_at<=now()-interval '15 minutes' AND (locked_until IS NULL OR locked_until<now())")
        .execute(&state.pool).await?;
    let mut sent = 0;
    for _ in 0..20 {
        let lease_token = Uuid::new_v4();
        let row=sqlx::query_as::<_,PendingMail>("UPDATE outbox SET locked_until=now()+interval '2 minutes',lease_token=$1,attempts=attempts+1 WHERE id=(SELECT id FROM outbox WHERE status='pending' AND next_attempt_at<=now() AND (locked_until IS NULL OR locked_until<now()) AND (subject<>'Přihlášení do Brunch' OR created_at>now()-interval '15 minutes') ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,to_email,subject,body,attempts,lease_token")
            .bind(lease_token).fetch_optional(&state.pool).await?;
        let Some(row) = row else { break };
        let result = match (state.config.smtp_from.parse(), row.to_email.parse()) {
            (Ok(from), Ok(to)) => match Message::builder()
                .from(from)
                .to(to)
                .subject(&row.subject)
                .message_id(Some(format!("<{}@brunch.local>", row.id)))
                .body(row.body)
            {
                Ok(message) => {
                    match tokio::time::timeout(Duration::from_secs(45), transport.send(message))
                        .await
                    {
                        Ok(Ok(_)) => Ok(()),
                        Ok(Err(_)) => {
                            Err("SMTP server zprávu nepřijal nebo není dostupný".to_owned())
                        }
                        Err(_) => Err("SMTP server neodpověděl včas".to_owned()),
                    }
                }
                Err(_) => Err("Zprávu se nepodařilo sestavit".to_string()),
            },
            _ => Err("Neplatný odesílatel nebo příjemce".to_string()),
        };
        match result {
            Ok(()) => {
                let result = sqlx::query("UPDATE outbox SET status='sent',sent_at=now(),locked_until=NULL,lease_token=NULL,last_error=NULL,body=CASE WHEN subject='Přihlášení do Brunch' THEN '[Přihlašovací odkaz odstraněn po odeslání]' ELSE body END WHERE id=$1 AND lease_token=$2 AND status='pending'")
                    .bind(row.id).bind(row.lease_token).execute(&state.pool).await?;
                sent += result.rows_affected() as usize;
            }
            Err(error) => {
                let delay = (30_i64 * 2_i64.pow(row.attempts.min(7) as u32)).min(3600);
                sqlx::query("UPDATE outbox SET status=CASE WHEN attempts>=8 THEN 'failed' ELSE 'pending' END,last_error=$2,locked_until=NULL,lease_token=NULL,next_attempt_at=now()+make_interval(secs=>$3),body=CASE WHEN attempts>=8 AND subject='Přihlášení do Brunch' THEN '[Přihlašovací odkaz odstraněn po selhání odeslání]' ELSE body END WHERE id=$1 AND lease_token=$4 AND status='pending'")
                    .bind(row.id).bind(error).bind(delay as f64).bind(row.lease_token).execute(&state.pool).await?;
            }
        }
    }
    Ok(sent)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/mail", get(list))
        .route("/api/admin/mail/{id}/retry", post(retry))
}
#[derive(Serialize, FromRow)]
struct MailRow {
    id: Uuid,
    to_email: String,
    subject: String,
    status: String,
    attempts: i32,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<MailRow>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(sqlx::query_as("SELECT id,to_email,subject,status,attempts,last_error,created_at,sent_at FROM outbox ORDER BY created_at DESC LIMIT 100").fetch_all(&state.pool).await?))
}
async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    auth::require_admin(&state, &headers).await?;
    let result=sqlx::query("UPDATE outbox SET status='pending',attempts=0,next_attempt_at=now(),locked_until=NULL,lease_token=NULL,last_error=NULL WHERE id=$1 AND status='failed' AND subject<>'Přihlášení do Brunch'").bind(id).execute(&state.pool).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "Znovu lze odeslat jen selhanou objednávkovou zprávu. Pro přihlášení požádejte o nový odkaz.",
        ));
    }
    Ok(Json(json!({"ok":true})))
}
