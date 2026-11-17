//! Фото товаров для CRM (#43): подбор на Pexels и хранение в Cloudinary.
//!
//! Обе интеграции бесплатные. Ключи — только в переменных окружения (H12):
//! - `PEXELS_API_KEY` — подбор вариантов по названию товара;
//! - `CLOUDINARY_URL` — `cloudinary://<api_key>:<api_secret>@<cloud_name>`, хранение фото.
//!
//! Без ключей соответствующие запросы отвечают 503, остальной API работает.
//! Хранить файлы на самом сервере нельзя: диск Render Free стирается при перезапуске.
//!
//! Единый стиль (ФТ-Т-07) задаёт трансформация Cloudinary в адресе фото:
//! квадрат 800×800, webp/avif по браузеру. Удаление фона —
//! `CLOUDINARY_REMOVE_BG=true`: включать после проверки, что эффект
//! `e_background_removal` доступен на аккаунте, иначе фото не откроется.
//! Без него фото обрезается до квадрата по главному объекту.
//! Массовое заполнение каталога делает скрипт `tools/photos` (#46).

use std::{sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{ApiError, ApiResult};

/// Фон подложки — brand.light, как у фото из скрипта #46.
const BRAND_BG: &str = "F7F4EC";
const SIZE: u32 = 800;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        // Без пула соединений: запросы к фото редкие, а соединение, открытое
        // в одном tokio-runtime, в другом не работает (так устроены тесты).
        reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP-клиент")
    })
}

fn upstream(service: &str, e: impl std::fmt::Display) -> ApiError {
    tracing::warn!("{service}: {e}");
    ApiError::Upstream(format!("{service} не ответил, попробуйте ещё раз"))
}

// ─── Pexels ──────────────────────────────────────────────────────────────────

/// Вариант фото для окна выбора в CRM.
#[derive(Debug, Serialize)]
pub struct Suggestion {
    pub id: u64,
    /// Миниатюра для сетки вариантов.
    pub thumb: String,
    /// Крупный вариант для предпросмотра.
    pub preview: String,
    pub alt: String,
    pub photographer: String,
    /// Страница фото на Pexels.
    pub url: String,
}

#[derive(Deserialize)]
struct PexelsSearch {
    #[serde(default)]
    photos: Vec<PexelsPhoto>,
}

#[derive(Deserialize)]
struct PexelsPhoto {
    id: u64,
    url: String,
    #[serde(default)]
    photographer: String,
    #[serde(default)]
    alt: String,
    src: PexelsSrc,
}

#[derive(Deserialize)]
struct PexelsSrc {
    original: String,
    large: String,
    medium: String,
}

struct Pexels {
    key: String,
    base: String,
}

fn pexels() -> ApiResult<Pexels> {
    let key = env("PEXELS_API_KEY")
        .ok_or_else(|| ApiError::Unavailable("подбор фото не настроен: нет PEXELS_API_KEY".into()))?;
    let base = env("PEXELS_API_BASE").unwrap_or_else(|| "https://api.pexels.com".into());
    Ok(Pexels { key, base: base.trim_end_matches('/').into() })
}

async fn pexels_get<T: for<'de> Deserialize<'de>>(p: &Pexels, path: &str, query: &[(&str, String)]) -> ApiResult<T> {
    let res = http()
        .get(format!("{}{path}", p.base))
        .header("Authorization", &p.key)
        .query(query)
        .send()
        .await
        .map_err(|e| upstream("Pexels", e))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(ApiError::BadRequest("фото на Pexels не найдено".into()));
    }
    if !status.is_success() {
        return Err(upstream("Pexels", format!("HTTP {status}")));
    }
    res.json().await.map_err(|e| upstream("Pexels", e))
}

/// Варианты по запросу. Запрос по-русски: Pexels понимает его с `locale=ru-RU`.
pub async fn suggest(q: &str, count: u32) -> ApiResult<Vec<Suggestion>> {
    let p = pexels()?;
    let found: PexelsSearch = pexels_get(
        &p,
        "/v1/search",
        &[("query", q.to_string()), ("per_page", count.to_string()), ("locale", "ru-RU".into())],
    )
    .await?;
    Ok(found
        .photos
        .into_iter()
        .map(|f| Suggestion {
            id: f.id,
            thumb: f.src.medium,
            preview: f.src.large,
            alt: f.alt,
            photographer: f.photographer,
            url: f.url,
        })
        .collect())
}

/// Адрес оригинала выбранного фото — его забирает Cloudinary.
pub async fn pexels_original(id: u64) -> ApiResult<String> {
    let p = pexels()?;
    let f: PexelsPhoto = pexels_get(&p, &format!("/v1/photos/{id}"), &[]).await?;
    Ok(f.src.original)
}

// ─── Cloudinary ──────────────────────────────────────────────────────────────

pub struct Cloudinary {
    cloud: String,
    key: String,
    secret: String,
    base: String,
    remove_bg: bool,
}

/// Разбор `cloudinary://<api_key>:<api_secret>@<cloud_name>`.
fn parse_url(url: &str) -> Option<(String, String, String)> {
    let rest = url.strip_prefix("cloudinary://")?;
    let (creds, cloud) = rest.rsplit_once('@')?;
    let (key, secret) = creds.split_once(':')?;
    let cloud = cloud.trim_end_matches('/');
    if key.is_empty() || secret.is_empty() || cloud.is_empty() {
        return None;
    }
    Some((key.into(), secret.into(), cloud.into()))
}

pub fn cloudinary() -> ApiResult<Cloudinary> {
    let url = env("CLOUDINARY_URL")
        .ok_or_else(|| ApiError::Unavailable("загрузка фото не настроена: нет CLOUDINARY_URL".into()))?;
    let (key, secret, cloud) = parse_url(&url).ok_or_else(|| {
        ApiError::Unavailable("CLOUDINARY_URL задан неверно: нужен вид cloudinary://ключ:секрет@облако".into())
    })?;
    let base = env("CLOUDINARY_API_BASE").unwrap_or_else(|| "https://api.cloudinary.com".into());
    let remove_bg = matches!(env("CLOUDINARY_REMOVE_BG").as_deref(), Some("1" | "true" | "yes"));
    Ok(Cloudinary { cloud, key, secret, base: base.trim_end_matches('/').into(), remove_bg })
}

/// Подпись запроса Cloudinary: параметры по алфавиту `k=v&…` + секрет, SHA-1 в hex.
pub fn sign(params: &[(&str, String)], secret: &str) -> String {
    let mut sorted: Vec<_> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let joined = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let digest = Sha1::digest(format!("{joined}{secret}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Что загружаем: адрес (Cloudinary заберёт сам) или файл.
pub enum Source {
    Url(String),
    File { bytes: Vec<u8>, mime: &'static str },
}

#[derive(Deserialize)]
struct Uploaded {
    secure_url: String,
}

impl Cloudinary {
    /// Трансформация единого стиля, вставляется в адрес фото после `/upload/`.
    fn transformation(&self) -> String {
        if self.remove_bg {
            format!("e_background_removal/c_pad,w_{SIZE},h_{SIZE},b_rgb:{BRAND_BG}/f_auto,q_auto")
        } else {
            format!("c_fill,g_auto,w_{SIZE},h_{SIZE}/f_auto,q_auto")
        }
    }

    /// Адрес для сайта: оригинал в Cloudinary + трансформация. В адресе есть
    /// версия загрузки, поэтому новое фото того же товара не застрянет в кэше.
    pub fn styled_url(&self, secure_url: &str) -> String {
        match secure_url.split_once("/image/upload/") {
            Some((head, tail)) => format!("{head}/image/upload/{}/{tail}", self.transformation()),
            None => secure_url.to_string(),
        }
    }

    /// Загрузить фото товара. Повторная загрузка заменяет прежнее фото.
    pub async fn upload(&self, product_id: &str, src: Source) -> ApiResult<String> {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let public_id = format!("bakeloom/products/{product_id}");
        let signed = [
            ("overwrite", "true".to_string()),
            ("public_id", public_id),
            ("timestamp", timestamp),
        ];
        let signature = sign(&signed, &self.secret);

        let mut form = reqwest::multipart::Form::new()
            .text("api_key", self.key.clone())
            .text("signature", signature);
        for (k, v) in signed {
            form = form.text(k, v);
        }
        form = match src {
            Source::Url(u) => form.text("file", u),
            Source::File { bytes, mime } => form.part(
                "file",
                reqwest::multipart::Part::bytes(bytes)
                    .file_name("photo")
                    .mime_str(mime)
                    .map_err(|e| ApiError::Other(e.into()))?,
            ),
        };

        let res = http()
            .post(format!("{}/v1_1/{}/image/upload", self.base, self.cloud))
            .multipart(form)
            .send()
            .await
            .map_err(|e| upstream("Cloudinary", e))?;
        let status = res.status();
        if status == reqwest::StatusCode::BAD_REQUEST {
            // Битый или неподдерживаемый файл — это ошибка запроса, а не сервиса.
            return Err(ApiError::BadRequest("Cloudinary не принял фото: проверьте файл".into()));
        }
        if !status.is_success() {
            return Err(upstream("Cloudinary", format!("HTTP {status}")));
        }
        let up: Uploaded = res.json().await.map_err(|e| upstream("Cloudinary", e))?;
        Ok(self.styled_url(&up.secure_url))
    }
}

/// Тип картинки по первым байтам: заголовку Content-Type не доверяем.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn разбор_cloudinary_url() {
        assert_eq!(
            parse_url("cloudinary://123:abc@demo"),
            Some(("123".into(), "abc".into(), "demo".into()))
        );
        assert_eq!(parse_url("cloudinary://123@demo"), None);
        assert_eq!(parse_url("https://123:abc@demo"), None);
    }

    #[test]
    fn подпись_как_в_документации_cloudinary() {
        // Пример из документации Cloudinary «Generating authentication signatures».
        let s = sign(
            &[
                ("timestamp", "1315060510".into()),
                ("public_id", "sample_image".into()),
                ("eager", "w_400,h_300,c_pad|w_260,h_200,c_crop".into()),
            ],
            "abcd",
        );
        assert_eq!(s, "bfd09f95f331f558cbd1320e67aa8d488770583e");
    }

    #[test]
    fn тип_по_первым_байтам() {
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<html>"), None);
    }
}
