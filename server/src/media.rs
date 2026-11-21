use crate::{ApiError, ApiResult, AppState, auth};
use axum::{
    Json, Router,
    body::{Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits,
    imageops::FilterType,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_SOURCE: usize = 40 * 1024 * 1024;
const MAX_ENCODED: usize = MAX_SOURCE.div_ceil(3) * 4;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 52_000_000;
const DISPLAY_DIMENSION: u32 = 1440;
// The application shares a 640 MiB container with checkout and the mail worker.
static IMAGE_WORKERS: Semaphore = Semaphore::const_new(1);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/images", post(upload))
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
        return Err(ApiError::bad_request("Obrázek smí mít nejvýše 40 MB."));
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
    limits.max_alloc = Some(224 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(source), format);
    reader.limits(limits.clone());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| ApiError::bad_request("Obrázek je poškozený nebo má příliš velké rozměry."))?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ApiError::bad_request(
            "Obrázek smí mít nejvýše 16 384 pixelů na straně a 52 megapixelů.",
        ));
    }
    limits
        .reserve(decoder.total_bytes())
        .and_then(|()| decoder.set_limits(limits))
        .map_err(|_| ApiError::bad_request("Obrázek má pro zpracování příliš velké rozměry."))?;
    let orientation = decoder
        .orientation()
        .map_err(|_| ApiError::bad_request("Obrázek obsahuje neplatné údaje o otočení."))?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| ApiError::bad_request("Obrázek nelze bezpečně načíst. Zvolte jiný soubor."))?;
    // Pre-shrink large originals to bound the temporary buffers used by Lanczos resampling.
    if width.max(height) > 2 * DISPLAY_DIMENSION {
        decoded = decoded.thumbnail(2 * DISPLAY_DIMENSION, 2 * DISPLAY_DIMENSION);
    }
    if decoded.width().max(decoded.height()) > DISPLAY_DIMENSION {
        decoded = decoded.resize(DISPLAY_DIMENSION, DISPLAY_DIMENSION, FilterType::Lanczos3);
    }
    decoded.apply_orientation(orientation);
    let (width, height) = (decoded.width(), decoded.height());
    let mut output = BoundedOutput::default();
    let content_type;
    let has_transparency =
        decoded.color().has_alpha() && decoded.pixels().any(|(_, _, p)| p[3] < 255);
    let encoded = if has_transparency {
        content_type = "image/png";
        image::codecs::png::PngEncoder::new(&mut output).write_image(
            decoded.into_rgba8().as_raw(),
            width,
            height,
            image::ColorType::Rgba8.into(),
        )
    } else {
        content_type = "image/jpeg";
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 88)
            .encode_image(&decoded.into_rgb8())
    };
    encoded.map_err(|_| {
        ApiError::bad_request("Obrázek je po úpravě příliš velký. Zmenšete jeho rozměry.")
    })?;
    Ok(SanitizedImage {
        content_type,
        data: output.0,
    })
}

fn legacy_json_image(body: Bytes) -> ApiResult<Bytes> {
    let input: ImageInput = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("Neplatné údaje o obrázku."))?;
    drop(body);
    if input.filename.trim().is_empty()
        || input.filename.chars().count() > 200
        || input.filename.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request("Neplatný název obrázku."));
    }
    if input.content_base64.len() > MAX_ENCODED {
        return Err(ApiError::bad_request("Obrázek smí mít nejvýše 40 MB."));
    }
    STANDARD
        .decode(input.content_base64)
        .map(Bytes::from)
        .map_err(|_| ApiError::bad_request("Neplatná data obrázku."))
}

async fn upload(State(state): State<AppState>, request: Request) -> ApiResult<Json<Value>> {
    let admin = auth::require_admin(&state, request.headers()).await?;
    // Authenticate and acquire the worker before buffering any part of a large upload.
    let binary = request
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value == "application/octet-stream");
    let permit = IMAGE_WORKERS.try_acquire().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Právě zpracováváme jiné obrázky. Zkuste to za chvíli.",
        )
    })?;
    let limit = if binary {
        MAX_SOURCE
    } else {
        MAX_ENCODED + 4096
    };
    let body = to_bytes(request.into_body(), limit).await.map_err(|_| {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Obrázek smí mít nejvýše 40 MB.",
        )
    })?;
    let sanitized = tokio::task::spawn_blocking(move || {
        // Retain the processing permit even if the HTTP request is disconnected.
        let _permit = permit;
        let source = if binary {
            body
        } else {
            legacy_json_image(body)?
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_phone_photo_is_resized_compressed_and_oriented() {
        let rgb = image::RgbImage::from_pixel(4800, 1200, image::Rgb([81, 144, 62]));
        let mut source = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut source);
        encoder
            .set_exif_metadata(
                b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec(),
            )
            .unwrap();
        encoder.encode_image(&rgb).unwrap();
        source.resize(MAX_SOURCE, 0);
        let result = sanitize_image(&source).unwrap();
        assert_eq!(result.content_type, "image/jpeg");
        assert!(result.data.len() < 100_000);
        let img = image::load_from_memory(&result.data).unwrap();
        assert_eq!(img.dimensions(), (360, 1440));
        assert!(!result.data.windows(6).any(|part| part == b"Exif\0\0"));
        source.push(0);
        assert!(sanitize_image(&source).is_err());
    }
}
