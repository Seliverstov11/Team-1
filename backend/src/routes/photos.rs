//! Фото товара в CRM (#43): варианты с Pexels и загрузка выбранного или своего фото.
//! Каталог ведёт владелец (6.5, 6.7), поэтому оба запроса — только для него.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State, multipart::MultipartError},
    http::{StatusCode, header::CONTENT_TYPE},
    routing::{get, post},
};
use serde::Deserialize;

use super::catalog::{load_product, to_views};
use super::now;
use crate::{
    auth::{ADMIN, Staff},
    db::Db,
    error::{ApiError, ApiResult, bad},
    models::ProductView,
    photo_services::{self as ps, Source, Suggestion},
};

/// Своё фото — до 10 МБ: снимок с телефона обычно 2–6 МБ.
pub const MAX_PHOTO_BYTES: usize = 10 * 1024 * 1024;

pub fn router() -> Router<Db> {
    Router::new()
        .route("/photo-suggestions", get(suggestions))
        .route(
            "/products/{id}/photo",
            // Запас сверх лимита, чтобы на большой файл ответить понятным 413 из кода.
            post(upload).layer(DefaultBodyLimit::max(MAX_PHOTO_BYTES + 1024 * 1024)),
        )
}

#[derive(Deserialize)]
struct SuggestQuery {
    q: Option<String>,
    count: Option<u32>,
}

/// `GET /api/catalog/photo-suggestions?q=<название>&count=6`
async fn suggestions(staff: Staff, Query(p): Query<SuggestQuery>) -> ApiResult<Json<Vec<Suggestion>>> {
    staff.require(ADMIN)?;
    let q = p.q.as_deref().map(str::trim).unwrap_or_default();
    if q.is_empty() {
        return Err(bad("нужен запрос q — например, название товара"));
    }
    if q.chars().count() > 100 {
        return Err(bad("запрос — не длиннее 100 символов"));
    }
    let count = p.count.unwrap_or(6).clamp(1, 12);
    Ok(Json(ps::suggest(q, count).await?))
}

#[derive(Deserialize)]
struct FromPexels {
    pexels_id: u64,
}

/// Что прислал клиент — вариант с Pexels или свой файл.
enum Incoming {
    Pexels(u64),
    File(Vec<u8>),
}

fn check_file(bytes: Vec<u8>) -> ApiResult<(Vec<u8>, &'static str)> {
    if bytes.is_empty() {
        return Err(bad("файл пустой"));
    }
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(ApiError::TooLarge("фото — не больше 10 МБ".into()));
    }
    let mime = ps::sniff_image(&bytes)
        .ok_or_else(|| ApiError::UnsupportedMedia("нужно фото в формате JPG, PNG или WebP".into()))?;
    Ok((bytes, mime))
}

async fn read_multipart(req: Request) -> ApiResult<Incoming> {
    let mut form = Multipart::from_request(req, &()).await.map_err(|e| bad(format!("форма: {e}")))?;
    while let Some(field) = form.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("file") => {
                let bytes = field.bytes().await.map_err(multipart_error)?;
                return Ok(Incoming::File(bytes.to_vec()));
            }
            Some("pexels_id") => {
                let v = field.text().await.map_err(multipart_error)?;
                let id = v.trim().parse().map_err(|_| bad("pexels_id — число"))?;
                return Ok(Incoming::Pexels(id));
            }
            _ => {}
        }
    }
    Err(bad("в форме нужно поле file или pexels_id"))
}

/// Обрыв по лимиту тела axum помечает кодом 413 — отдаём его понятным текстом.
fn multipart_error(e: MultipartError) -> ApiError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::TooLarge("фото — не больше 10 МБ".into())
    } else {
        bad(format!("форма: {}", e.body_text()))
    }
}

/// `POST /api/catalog/products/{id}/photo` — три равноправных вида тела:
///
/// - `multipart/form-data` с полем `file` (своё фото) или `pexels_id` (выбранный вариант);
/// - `application/json`: `{"pexels_id": 123}`;
/// - `image/jpeg | image/png | image/webp`: тело — сам файл.
///
/// Тип файла проверяется по содержимому: JPG, PNG, WebP до 10 МБ, иначе 415 или 413.
/// Отвечает обновлённым товаром: в `image_url` — адрес фото в едином стиле.
async fn upload(staff: Staff, State(db): State<Db>, Path(id): Path<String>, req: Request) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    load_product(&conn, &id).await?;

    let ctype = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .unwrap_or_default();

    let incoming = match ctype.as_str() {
        "multipart/form-data" => read_multipart(req).await?,
        "application/json" => {
            let body = Bytes::from_request(req, &()).await.map_err(|_| ApiError::TooLarge("тело слишком большое".into()))?;
            let b: FromPexels = serde_json::from_slice(&body).map_err(|_| bad("нужно тело {\"pexels_id\": число}"))?;
            Incoming::Pexels(b.pexels_id)
        }
        c if c.starts_with("image/") => {
            let body = Bytes::from_request(req, &()).await.map_err(|_| ApiError::TooLarge("фото — не больше 10 МБ".into()))?;
            Incoming::File(body.to_vec())
        }
        _ => {
            return Err(ApiError::UnsupportedMedia(
                "нужна форма multipart с file или pexels_id, JSON с pexels_id или сам файл image/*".into(),
            ));
        }
    };

    let cloud = ps::cloudinary()?;
    let src = match incoming {
        Incoming::Pexels(pid) => Source::Url(ps::pexels_original(pid).await?),
        Incoming::File(bytes) => {
            let (bytes, mime) = check_file(bytes)?;
            Source::File { bytes, mime }
        }
    };
    let url = cloud.upload(&id, src).await?;
    conn.execute(
        "UPDATE products SET image_url = ?1, updated_at = ?2 WHERE id = ?3",
        libsql::params![url, now(), id.clone()],
    )
    .await?;
    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}
