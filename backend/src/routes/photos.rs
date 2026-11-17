//! Фото товара в CRM (#43): варианты с Pexels и загрузка выбранного или своего фото.
//! Каталог ведёт владелец (6.5, 6.7), поэтому оба запроса — только для него.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, header::CONTENT_TYPE},
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

/// `POST /api/catalog/products/{id}/photo`
///
/// - `Content-Type: application/json`, тело `{"pexels_id": 123}` — выбранный вариант;
/// - `Content-Type: image/jpeg | image/png | image/webp`, тело — сам файл.
///
/// Отвечает обновлённым товаром: в `image_url` — адрес фото в едином стиле.
async fn upload(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    load_product(&conn, &id).await?;

    let ctype = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .unwrap_or_default();

    let source = if ctype == "application/json" {
        let b: FromPexels =
            serde_json::from_slice(&body).map_err(|_| bad("нужно тело {\"pexels_id\": число}"))?;
        let cloud = ps::cloudinary()?;
        let original = ps::pexels_original(b.pexels_id).await?;
        (cloud, Source::Url(original))
    } else if ctype.starts_with("image/") {
        if body.is_empty() {
            return Err(bad("файл пустой"));
        }
        if body.len() > MAX_PHOTO_BYTES {
            return Err(ApiError::TooLarge("фото — не больше 10 МБ".into()));
        }
        let mime = ps::sniff_image(&body).ok_or_else(|| bad("нужно фото в формате JPG, PNG или WebP"))?;
        (ps::cloudinary()?, Source::File { bytes: body.to_vec(), mime })
    } else {
        return Err(bad("Content-Type: application/json с pexels_id или image/jpeg, image/png, image/webp"));
    };

    let (cloud, src) = source;
    let url = cloud.upload(&id, src).await?;
    conn.execute(
        "UPDATE products SET image_url = ?1, updated_at = ?2 WHERE id = ?3",
        libsql::params![url, now(), id.clone()],
    )
    .await?;
    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}
