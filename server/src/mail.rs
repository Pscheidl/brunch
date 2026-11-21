use crate::{ApiError, ApiResult, AppState, Config, auth};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::SinglePart,
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgConnection};
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

pub const LOGIN_SUBJECT: &str = "Přihlášení do Brunch Era";
// Keep queued links created before the rebrand protected until they are removed.
const LOGIN_SUBJECTS: &[&str] = &["Přihlášení do Brunch", LOGIN_SUBJECT];

/// The primary address is first and remains the customer-facing contact.
pub(crate) fn normalize_order_recipients(
    primary: &str,
    additional: &[String],
) -> ApiResult<Vec<String>> {
    let mut recipients = Vec::new();
    let mut seen = HashSet::new();
    for value in std::iter::once(primary).chain(additional.iter().map(String::as_str)) {
        let email = auth::normalize_email(value)?;
        if seen.insert(email.clone()) {
            recipients.push(email);
            if recipients.len() > 20 {
                return Err(ApiError::bad_request(
                    "Oznámení objednávek lze posílat nejvýše na 20 různých e-mailových adres.",
                ));
            }
        }
    }
    Ok(recipients)
}

/// Call within the order/payment transaction so all recipients are queued atomically.
/// Each recipient gets a separate message, with no other staff addresses exposed.
pub async fn enqueue_order_notification(
    conn: &mut PgConnection,
    subject: &str,
    body: &str,
) -> ApiResult<String> {
    let (primary, additional): (String, Vec<String>) =
        sqlx::query_as("SELECT order_email,additional_order_emails FROM shop_settings WHERE id=1")
            .fetch_one(&mut *conn)
            .await?;
    let recipients = normalize_order_recipients(&primary, &additional)?;
    let contact = recipients[0].clone();
    for recipient in recipients {
        enqueue(conn, &recipient, subject, body).await?;
    }
    Ok(contact)
}

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
        .bind(format!("{body}\n\nBrunch Era"))
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
    sqlx::query("UPDATE outbox SET status='failed',body='[Přihlašovací odkaz odstraněn po vypršení]',last_error='Přihlašovací odkaz vypršel. Požádejte o nový.',locked_until=NULL,lease_token=NULL WHERE status='pending' AND broadcast_id IS NULL AND subject=ANY($1) AND created_at<=now()-interval '15 minutes' AND (locked_until IS NULL OR locked_until<now())")
        .bind(LOGIN_SUBJECTS).execute(&state.pool).await?;
    let mut sent = 0;
    for _ in 0..20 {
        let lease_token = Uuid::new_v4();
        let row=sqlx::query_as::<_,PendingMail>("UPDATE outbox SET locked_until=now()+interval '2 minutes',lease_token=$1,attempts=attempts+1 WHERE id=(SELECT id FROM outbox WHERE status='pending' AND next_attempt_at<=now() AND (locked_until IS NULL OR locked_until<now()) AND (broadcast_id IS NOT NULL OR subject<>ALL($2) OR created_at>now()-interval '15 minutes') ORDER BY (broadcast_id IS NOT NULL),created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,to_email,subject,body,attempts,lease_token")
            .bind(lease_token).bind(LOGIN_SUBJECTS).fetch_optional(&state.pool).await?;
        let Some(row) = row else { break };
        let result = match (state.config.smtp_from.parse(), row.to_email.parse()) {
            (Ok(from), Ok(to)) => match Message::builder()
                .from(from)
                .to(to)
                .subject(&row.subject)
                .message_id(Some(format!("<{}@brunch.local>", row.id)))
                .singlepart(SinglePart::plain(row.body))
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
                let result = sqlx::query("UPDATE outbox SET status='sent',sent_at=now(),locked_until=NULL,lease_token=NULL,last_error=NULL,body=CASE WHEN broadcast_id IS NULL AND subject=ANY($3) THEN '[Přihlašovací odkaz odstraněn po odeslání]' ELSE body END WHERE id=$1 AND lease_token=$2 AND status='pending'")
                    .bind(row.id).bind(row.lease_token).bind(LOGIN_SUBJECTS).execute(&state.pool).await?;
                sent += result.rows_affected() as usize;
            }
            Err(error) => {
                let delay = (30_i64 * 2_i64.pow(row.attempts.min(7) as u32)).min(3600);
                sqlx::query("UPDATE outbox SET status=CASE WHEN attempts>=8 THEN 'failed' ELSE 'pending' END,last_error=$2,locked_until=NULL,lease_token=NULL,next_attempt_at=now()+make_interval(secs=>$3),body=CASE WHEN attempts>=8 AND broadcast_id IS NULL AND subject=ANY($5) THEN '[Přihlašovací odkaz odstraněn po selhání odeslání]' ELSE body END WHERE id=$1 AND lease_token=$4 AND status='pending'")
                    .bind(row.id).bind(error).bind(delay as f64).bind(row.lease_token).bind(LOGIN_SUBJECTS).execute(&state.pool).await?;
            }
        }
    }
    Ok(sent)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/mail", get(list))
        .route("/api/admin/mail/{id}/retry", post(retry))
        .route("/api/admin/mail/broadcast", get(audience).post(broadcast))
}
#[derive(Serialize, FromRow)]
struct MailRow {
    id: Uuid,
    to_email: String,
    subject: String,
    status: String,
    attempts: i32,
    can_retry: bool,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<MailRow>>> {
    auth::require_admin(&state, &headers).await?;
    Ok(Json(sqlx::query_as("SELECT id,to_email,subject,status,attempts,last_error,created_at,sent_at,status='failed' AND (broadcast_id IS NOT NULL OR subject<>ALL($1)) AS can_retry FROM outbox ORDER BY created_at DESC,id DESC LIMIT 100").bind(LOGIN_SUBJECTS).fetch_all(&state.pool).await?))
}

#[derive(Serialize)]
struct Audience {
    recipient_count: i64,
}

async fn audience(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Audience>> {
    auth::require_admin(&state, &headers).await?;
    let recipient_count = sqlx::query_scalar("SELECT count(*) FROM order_email_recipients")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(Audience { recipient_count }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BroadcastInput {
    idempotency_key: Uuid,
    subject: String,
    body: String,
}

async fn broadcast(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BroadcastInput>,
) -> ApiResult<Json<Audience>> {
    let admin = auth::require_admin(&state, &headers).await?;
    let subject = input.subject.trim();
    let body = input.body.trim();
    if subject.is_empty() || subject.chars().count() > 200 || subject.contains(char::is_control) {
        return Err(ApiError::bad_request(
            "Zadejte předmět na jednom řádku (nejvýše 200 znaků).",
        ));
    }
    if body.is_empty() || body.chars().count() > 12_000 || body.contains('\0') {
        return Err(ApiError::bad_request(
            "Zadejte text zprávy (nejvýše 12 000 znaků).",
        ));
    }
    let mut tx = state.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO mail_broadcasts(id,created_by,subject,body) VALUES ($1,$2,$3,$4) ON CONFLICT (id) DO NOTHING",
    )
    .bind(input.idempotency_key)
    .bind(admin.id)
    .bind(subject)
    .bind(body)
    .execute(&mut *tx)
    .await?;
    let recipient_count = if inserted.rows_affected() == 0 {
        sqlx::query_scalar(
            "SELECT recipient_count FROM mail_broadcasts WHERE id=$1 AND created_by=$2 AND subject=$3 AND body=$4",
        )
        .bind(input.idempotency_key)
        .bind(admin.id)
        .bind(subject)
        .bind(body)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::conflict("Tento požadavek již patří jiné zprávě."))?
    } else {
        let result = sqlx::query(
            "INSERT INTO outbox(id,to_email,subject,body,broadcast_id)
             SELECT gen_random_uuid(), email, $1, $2, $3 FROM order_email_recipients",
        )
        .bind(subject)
        .bind(format!("{body}\n\nBrunch Era"))
        .bind(input.idempotency_key)
        .execute(&mut *tx)
        .await?;
        let count = result.rows_affected() as i64;
        if count == 0 {
            return Err(ApiError::bad_request(
                "Zatím nejsou žádné objednávky s e-mailovou adresou.",
            ));
        }
        sqlx::query("UPDATE mail_broadcasts SET recipient_count=$2 WHERE id=$1")
            .bind(input.idempotency_key)
            .bind(count)
            .execute(&mut *tx)
            .await?;
        count
    };
    tx.commit().await?;
    Ok(Json(Audience { recipient_count }))
}

async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    auth::require_admin(&state, &headers).await?;
    let result=sqlx::query("UPDATE outbox SET status='pending',attempts=0,next_attempt_at=now(),locked_until=NULL,lease_token=NULL,last_error=NULL WHERE id=$1 AND status='failed' AND (broadcast_id IS NOT NULL OR subject<>ALL($2))").bind(id).bind(LOGIN_SUBJECTS).execute(&state.pool).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "Znovu lze odeslat jen zprávu, jejíž odeslání selhalo. Pro přihlášení požádejte o nový odkaz.",
        ));
    }
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::normalize_order_recipients;

    #[test]
    fn order_recipients_normalize_deduplicate_and_keep_primary_first() {
        let additional = [
            " Delivery@Example.test ",
            "SHOP@example.test",
            "delivery@example.test",
            "Kitchen@example.test",
        ]
        .map(String::from);
        assert_eq!(
            normalize_order_recipients(" Shop@Example.test ", &additional).unwrap(),
            [
                "shop@example.test",
                "delivery@example.test",
                "kitchen@example.test"
            ]
        );
        assert_eq!(
            normalize_order_recipients("delivery@example.test", &additional).unwrap(),
            [
                "delivery@example.test",
                "shop@example.test",
                "kitchen@example.test"
            ]
        );
        assert_eq!(
            normalize_order_recipients(" Shop@Example.test ", &[]).unwrap(),
            ["shop@example.test"]
        );
    }

    #[test]
    fn order_recipients_reject_invalid_addresses_and_header_injection() {
        for invalid in [
            "",
            " ",
            "not-an-email",
            "two@example.test,three@example.test",
            "Name <one@example.test>",
            "one@example.test\r\nBcc: other@example.test",
            "one @example.test",
        ] {
            assert!(
                normalize_order_recipients(invalid, &[]).is_err(),
                "{invalid:?}"
            );
            assert!(
                normalize_order_recipients("shop@example.test", &[invalid.to_owned()]).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn order_recipient_limit_counts_unique_addresses_including_primary() {
        let mut additional: Vec<String> =
            (1..20).map(|n| format!("staff{n}@example.test")).collect();
        additional.push(" SHOP@example.test ".into());
        additional.extend(vec!["STAFF1@example.test".into(); 30]);
        assert_eq!(
            normalize_order_recipients("shop@example.test", &additional)
                .unwrap()
                .len(),
            20
        );
        // Changing the primary can turn a previously duplicated address into a new recipient.
        assert!(normalize_order_recipients("new-shop@example.test", &additional).is_err());
        additional.push("staff20@example.test".into());
        assert!(normalize_order_recipients("shop@example.test", &additional).is_err());
    }
}
