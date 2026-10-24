pub mod auth;
pub mod commerce;
pub mod config;
pub mod mail;
pub mod media;
pub mod payments;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
pub use config::Config;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::{compression::CompressionLayer, services::ServeDir};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
}
pub type ApiResult<T> = Result<T, ApiError>;
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}
impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn bad_request(s: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, s)
    }
    pub fn unauthorized(s: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, s)
    }
    pub fn forbidden(s: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, s)
    }
    pub fn conflict(s: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, s)
    }
    pub fn not_found(s: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, s)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error":self.message}))).into_response()
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(error = %error, "Database request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Požadavek se nepodařilo uložit. Zkuste to prosím znovu.",
        )
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!().run(pool).await
}

pub fn app(state: AppState) -> Router {
    let dist = state.config.dist_dir.clone();
    Router::new()
        .route("/api/health", get(health))
        .merge(auth::routes())
        .merge(commerce::routes())
        .merge(mail::routes())
        .merge(media::routes())
        .merge(payments::routes())
        .route(
            "/api/{*path}",
            get(|| async { ApiError::not_found("Rozhraní neexistuje") }),
        )
        .fallback_service(ServeDir::new(dist).append_index_html_on_directories(true))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}
async fn health(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(json!({"status":"ok"})))
}
async fn security(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    if is_api
        && !(request.method() == Method::POST
            && request.uri().path() == "/api/payments/teya/webhook")
        && !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        )
    {
        let headers = request.headers();
        if headers
            .get("x-brunch-request")
            .and_then(|x| x.to_str().ok())
            != Some("1")
        {
            return ApiError::forbidden("Chybí ochrana požadavku. Obnovte stránku.")
                .into_response();
        }
        if headers
            .get(header::ORIGIN)
            .is_some_and(|v| v.to_str().ok() != Some(state.config.public_url.as_str()))
            || headers
                .get("sec-fetch-site")
                .is_some_and(|v| v == "cross-site")
        {
            return ApiError::forbidden("Požadavek pochází z jiné stránky.").into_response();
        }
        if !headers
            .get(header::CONTENT_TYPE)
            .and_then(|x| x.to_str().ok())
            .is_some_and(|x| x.split(';').next() == Some("application/json"))
        {
            return ApiError::bad_request("Požadavek musí obsahovat JSON.").into_response();
        }
    }
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; font-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"));
    if is_api && !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if state.config.secure_cookies {
        h.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}
