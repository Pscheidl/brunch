use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_SOURCE: usize = 2 * 1024 * 1024;
const MAX_ENCODED: usize = MAX_SOURCE.div_ceil(3) * 4;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;
const MAX_PIXELS: u64 = 16_000_000;
static IMAGE_WORKERS: Semaphore = Semaphore::const_new(2);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/images",
            post(upload).layer(DefaultBodyLimit::max(3 * 1024 * 1024)),
        )
        .route("/api/images/{id}", get(read))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageInput {
    filename: String,
    content_base64: String,
}

struct SanitizedImage {
    content_type: &'static str,
    data: Vec<u8>,
}

/// Enforces the output cap during compression rather than after an unbounded allocation.
#[derive(Default)]
struct BoundedOutput(Vec<u8>);

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_OUTPUT.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "Encoded image exceeds the output limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn sanitize_image(source: &[u8]) -> ApiResult<SanitizedImage> {
    if source.is_empty() || source.len() > MAX_SOURCE {
        return Err(ApiError::bad_request("Obrázek smí mít nejvýše 2 MB."));
    }
    let format = image::guess_format(source)
        .map_err(|_| ApiError::bad_request("Vyberte skutečný obrázek JPEG, PNG nebo WebP."))?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(ApiError::bad_request(
            "Podporovány jsou pouze obrázky JPEG, PNG a WebP.",
        ));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(128 * 1024 * 1024);
    let reader = || {
        let mut reader = ImageReader::with_format(Cursor::new(source), format);
        reader.limits(limits.clone());
        reader
    };
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|_| ApiError::bad_request("Obrázek je poškozený nebo má příliš velké rozměry."))?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(ApiError::bad_request(
            "Obrázek smí mít nejvýše 4096 pixelů na straně a 16 megapixelů.",
        ));
    }
    let orientation = reader()
        .into_decoder()
        .and_then(|mut decoder| decoder.orientation())
        .map_err(|_| ApiError::bad_request("Obrázek obsahuje neplatné údaje o otočení."))?;
    let mut decoded = reader()
        .decode()
        .map_err(|_| ApiError::bad_request("Obrázek nelze bezpečně načíst. Zvolte jiný soubor."))?;
    decoded.apply_orientation(orientation);
    let (width, height) = (decoded.width(), decoded.height());
    let mut output = BoundedOutput::default();
    let content_type;
    let encoded = if decoded.color().has_alpha() {
        content_type = "image/png";
        let rgba = decoded.to_rgba8();
        image::codecs::png::PngEncoder::new(&mut output).write_image(
            rgba.as_raw(),
            width,
            height,
            image::ColorType::Rgba8.into(),
        )
    } else {
        content_type = "image/jpeg";
        let rgb = decoded.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 85).encode_image(&rgb)
    };
    encoded.map_err(|_| {
        ApiError::bad_request("Obrázek je po úpravě příliš velký. Zmenšete jeho rozměry.")
    })?;
    Ok(SanitizedImage {
        content_type,
        data: output.0,
    })
}

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ImageInput>,
) -> ApiResult<Json<Value>> {
    let admin = auth::require_admin(&state, &headers).await?;
    if input.filename.trim().is_empty()
        || input.filename.chars().count() > 200
        || input.filename.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request("Neplatný název obrázku."));
    }
    if input.content_base64.len() > MAX_ENCODED {
        return Err(ApiError::bad_request("Obrázek smí mít nejvýše 2 MB."));
    }
    let permit = IMAGE_WORKERS.try_acquire().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Právě zpracováváme jiné obrázky. Zkuste to za chvíli.",
        )
    })?;
    let source = STANDARD
        .decode(&input.content_base64)
        .map_err(|_| ApiError::bad_request("Neplatná data obrázku."))?;
    let sanitized = tokio::task::spawn_blocking(move || {
        // Retain the processing permit even if the HTTP request is disconnected.
        let _permit = permit;
        sanitize_image(&source)
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Obrázek se nepodařilo zpracovat.",
        )
    })??;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO images(id,content_type,data,created_by) VALUES ($1,$2,$3,$4)")
        .bind(id)
        .bind(sanitized.content_type)
        .bind(sanitized.data)
        .bind(admin.id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"url":format!("/api/images/{id}")})))
}

async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    let (content_type, data): (String, Vec<u8>) =
        sqlx::query_as("SELECT content_type,data FROM images WHERE id=$1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::not_found("Obrázek neexistuje."))?;
    let etag = format!("\"{id}\"");
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag);
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        data.into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).map_err(|_| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Obrázek není dostupný.")
        })?,
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("UUID ETag"),
    );
    Ok(response)
}
