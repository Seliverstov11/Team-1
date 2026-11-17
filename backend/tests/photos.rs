//! Фото товара (#43) с подставными Pexels и Cloudinary: настоящие ключи не нужны.
//! Отдельный файл — отдельный процесс, поэтому переменные окружения с ключами
//! не влияют на tests/api.rs, где проверяется ответ без ключей.

use std::sync::{Mutex, OnceLock};

use axum::{
    Json, Router,
    body::Body,
    extract::{Multipart, Path, Query},
    http::{Method, Request, StatusCode, HeaderMap},
    routing::{get, post},
};
use bakeloom_api::{db::Db, photo_services::sign};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PW: &str = "test-pass";
const PEXELS_KEY: &str = "test-pexels-key";
const CLOUD_SECRET: &str = "test-secret";

/// Что получил подставной Cloudinary: public_id → содержимое поля file.
#[derive(Clone, Debug)]
enum Got {
    Url(String),
    File(usize, String),
}
static UPLOADS: Mutex<Vec<(String, Got)>> = Mutex::new(Vec::new());

fn got(public_id: &str) -> Got {
    UPLOADS.lock().unwrap().iter().rev().find(|(id, _)| id == public_id).map(|(_, g)| g.clone()).unwrap()
}

// ─── Подставные сервисы ──────────────────────────────────────────────────────

fn photo(id: u64) -> Value {
    json!({
        "id": id, "url": format!("https://www.pexels.com/photo/{id}/"),
        "photographer": "Автор", "alt": "коробка",
        "src": { "original": format!("https://images.pexels.com/photos/{id}/orig.jpeg"),
                 "large": format!("https://images.pexels.com/photos/{id}/large.jpeg"),
                 "medium": format!("https://images.pexels.com/photos/{id}/medium.jpeg") }
    })
}

async fn pexels_search(h: HeaderMap, Query(q): Query<Vec<(String, String)>>) -> Result<Json<Value>, StatusCode> {
    if h.get("authorization").and_then(|v| v.to_str().ok()) != Some(PEXELS_KEY) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    assert_eq!(get("locale"), "ru-RU");
    if get("query") == "сбой" {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let n: u64 = get("per_page").parse().unwrap();
    Ok(Json(json!({ "photos": (1..=n).map(photo).collect::<Vec<_>>() })))
}

async fn pexels_photo(Path(id): Path<u64>) -> Result<Json<Value>, StatusCode> {
    if id == 404 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(photo(id)))
}

async fn cloudinary_upload(Path(cloud): Path<String>, mut m: Multipart) -> Result<Json<Value>, StatusCode> {
    assert_eq!(cloud, "demo");
    let (mut params, mut file, mut signature, mut api_key) = (Vec::new(), None, String::new(), String::new());
    while let Some(f) = m.next_field().await.unwrap() {
        let name = f.name().unwrap().to_string();
        if name == "file" {
            file = Some(match f.content_type().map(str::to_string) {
                Some(ct) => Got::File(f.bytes().await.unwrap().len(), ct),
                None => Got::Url(f.text().await.unwrap()),
            });
            continue;
        }
        let v = f.text().await.unwrap();
        match name.as_str() {
            "signature" => signature = v,
            "api_key" => api_key = v,
            _ => params.push((name, v)),
        }
    }
    assert_eq!(api_key, "test-key");
    let refs: Vec<(&str, String)> = params.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    if sign(&refs, CLOUD_SECRET) != signature {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let public_id = params.iter().find(|(k, _)| k == "public_id").unwrap().1.clone();
    assert!(params.iter().any(|(k, v)| k == "overwrite" && v == "true"));
    UPLOADS.lock().unwrap().push((public_id.clone(), file.unwrap()));
    Ok(Json(json!({ "secure_url": format!("https://res.cloudinary.com/demo/image/upload/v17/{public_id}.jpg") })))
}

/// Подставные сервисы живут в своём потоке весь прогон: у каждого теста свой runtime.
fn services() {
    static ADDR: OnceLock<()> = OnceLock::new();
    ADDR.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let app = Router::new()
                    .route("/v1/search", get(pexels_search))
                    .route("/v1/photos/{id}", get(pexels_photo))
                    .route("/v1_1/{cloud}/image/upload", post(cloudinary_upload));
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(l.local_addr().unwrap()).unwrap();
                axum::serve(l, app).await.unwrap();
            })
        });
        let addr = rx.recv().unwrap();
        // SAFETY: переменные выставляются один раз до первого запроса к API.
        unsafe {
            std::env::set_var("PEXELS_API_KEY", PEXELS_KEY);
            std::env::set_var("PEXELS_API_BASE", format!("http://{addr}"));
            std::env::set_var("CLOUDINARY_URL", format!("cloudinary://test-key:{CLOUD_SECRET}@demo"));
            std::env::set_var("CLOUDINARY_API_BASE", format!("http://{addr}"));
            std::env::remove_var("CLOUDINARY_REMOVE_BG");
        }
    });
}

// ─── Приложение ──────────────────────────────────────────────────────────────

struct T {
    app: Router,
    db: Db,
}

async fn setup() -> T {
    services();
    let path = std::env::temp_dir().join(format!("bakeloom-test-{}.db", uuid::Uuid::new_v4()));
    let db = bakeloom_api::db::connect_local(path.to_str().unwrap()).await.unwrap();
    let hash = bakeloom_api::auth::hash_password(PW).await.unwrap();
    db.connect().unwrap().execute("UPDATE employees SET password_hash = ?1", libsql::params![hash]).await.unwrap();
    T { app: bakeloom_api::app(db.clone()), db }
}

impl T {
    async fn raw(&self, method: Method, uri: &str, token: Option<&str>, ctype: &str, body: Vec<u8>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(uri).header("content-type", ctype);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let res = self.app.clone().oneshot(b.body(Body::from(body)).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
    async fn get(&self, uri: &str, token: &str) -> (StatusCode, Value) {
        self.raw(Method::GET, uri, Some(token), "application/json", vec![]).await
    }
    async fn login(&self, login: &str) -> String {
        let body = json!({ "login": login, "password": PW }).to_string().into_bytes();
        let (s, v) = self.raw(Method::POST, "/api/auth/login", None, "application/json", body).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["token"].as_str().unwrap().to_string()
    }
    /// Свой товар на каждый тест: подставной Cloudinary записывает загрузки
    /// по товару, а тесты идут параллельно.
    async fn product(&self, n: i64) -> String {
        let conn = self.db.connect().unwrap();
        let mut r = conn
            .query("SELECT id FROM products WHERE is_archived = 0 ORDER BY id LIMIT 1 OFFSET ?1", libsql::params![n])
            .await
            .unwrap();
        r.next().await.unwrap().unwrap().get::<String>(0).unwrap()
    }
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

// ─── Тесты ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn варианты_фото_с_pexels_только_владельцу() {
    let t = setup().await;
    let owner = t.login("owner").await;

    let (s, v) = t.get("/api/catalog/photo-suggestions?q=%D0%BA%D0%BE%D1%80%D0%BE%D0%B1%D0%BA%D0%B0", &owner).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 6, "по умолчанию 6 вариантов");
    assert_eq!(list[0]["id"], 1);
    assert_eq!(list[0]["thumb"], "https://images.pexels.com/photos/1/medium.jpeg");
    assert_eq!(list[0]["preview"], "https://images.pexels.com/photos/1/large.jpeg");
    assert_eq!(list[0]["photographer"], "Автор");

    let (_, v) = t.get("/api/catalog/photo-suggestions?q=box&count=50", &owner).await;
    assert_eq!(v.as_array().unwrap().len(), 12, "не больше 12");

    let (s, _) = t.get("/api/catalog/photo-suggestions?q=%20", &owner).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "пустой запрос");

    let (s, v) = t.get("/api/catalog/photo-suggestions?q=%D1%81%D0%B1%D0%BE%D0%B9", &owner).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "Pexels упал: {v}");

    for who in ["dana", "sklad"] {
        let tok = t.login(who).await;
        let (s, _) = t.get("/api/catalog/photo-suggestions?q=box", &tok).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who}");
    }
}

#[tokio::test]
async fn выбранный_вариант_уходит_в_cloudinary_и_становится_фото_товара() {
    let t = setup().await;
    let owner = t.login("owner").await;
    let id = t.product(0).await;
    let uri = format!("/api/catalog/products/{id}/photo");

    let (s, v) = t.raw(Method::POST, &uri, Some(&owner), "application/json", br#"{"pexels_id": 42}"#.to_vec()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["image_url"],
        format!("https://res.cloudinary.com/demo/image/upload/c_fill,g_auto,w_800,h_800/f_auto,q_auto/v17/bakeloom/products/{id}.jpg")
    );
    match got(&format!("bakeloom/products/{id}")) {
        Got::Url(u) => assert_eq!(u, "https://images.pexels.com/photos/42/orig.jpeg"),
        other => panic!("ждали адрес оригинала, пришло {other:?}"),
    }

    // Фото видно и на сайте без входа
    let (s, v) = t.raw(Method::GET, &format!("/api/catalog/products/{id}"), None, "application/json", vec![]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["image_url"].as_str().unwrap().contains("/c_fill,g_auto,w_800,h_800/"));

    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "application/json", br#"{"pexels_id": 404}"#.to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "такого фото на Pexels нет");
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "application/json", br#"{"id": 1}"#.to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "нет pexels_id");
    let (s, _) = t
        .raw(Method::POST, "/api/catalog/products/нет-такого/photo", Some(&owner), "application/json", br#"{"pexels_id": 1}"#.to_vec())
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn своё_фото_файлом_проверка_типа_и_размера() {
    let t = setup().await;
    let owner = t.login("owner").await;
    let id = t.product(1).await;
    let uri = format!("/api/catalog/products/{id}/photo");

    // Тип берётся по содержимому, а не по заголовку
    let (s, v) = t.raw(Method::POST, &uri, Some(&owner), "image/jpeg", PNG.to_vec()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    match got(&format!("bakeloom/products/{id}")) {
        Got::File(n, ct) => assert_eq!((n, ct.as_str()), (PNG.len(), "image/png")),
        other => panic!("ждали файл, пришло {other:?}"),
    }

    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "image/png", b"<html>not an image</html>".to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "не картинка");
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "image/png", vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "пустой файл");
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "text/plain", PNG.to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "не тот Content-Type");

    let mut big = PNG.to_vec();
    big.resize(10 * 1024 * 1024 + 1, 0);
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), "image/png", big).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "больше 10 МБ");

    for who in ["dana", "sklad"] {
        let tok = t.login(who).await;
        let (s, _) = t.raw(Method::POST, &uri, Some(&tok), "image/png", PNG.to_vec()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who}");
    }
}

/// Тело multipart/form-data, как его собирает браузер из FormData.
fn form(field: &str, value: &[u8], file: bool) -> (String, Vec<u8>) {
    let b = "----bakeloom-test-boundary";
    let mut body = format!("--{b}\r\n").into_bytes();
    if file {
        body.extend(format!("Content-Disposition: form-data; name=\"{field}\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").bytes());
    } else {
        body.extend(format!("Content-Disposition: form-data; name=\"{field}\"\r\n\r\n").bytes());
    }
    body.extend_from_slice(value);
    body.extend(format!("\r\n--{b}--\r\n").bytes());
    (format!("multipart/form-data; boundary={b}"), body)
}

#[tokio::test]
async fn форма_из_crm_файл_или_pexels_id() {
    let t = setup().await;
    let owner = t.login("owner").await;
    let id = t.product(2).await;
    let uri = format!("/api/catalog/products/{id}/photo");
    let key = format!("bakeloom/products/{id}");

    let (ct, body) = form("pexels_id", b"77", false);
    let (s, v) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(matches!(got(&key), Got::Url(u) if u == "https://images.pexels.com/photos/77/orig.jpeg"));

    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
    let (ct, body) = form("file", &jpeg, true);
    let (s, v) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(matches!(got(&key), Got::File(n, ref c) if n == jpeg.len() && c == "image/jpeg"));

    let (ct, body) = form("file", b"GIF89a", true);
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "GIF не принимаем");

    let (ct, body) = form("pexels_id", b"abc", false);
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "pexels_id не число");

    let (ct, body) = form("other", b"1", false);
    let (s, _) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "нет нужного поля");

    let mut big = jpeg.to_vec();
    big.resize(12 * 1024 * 1024, 0);
    let (ct, body) = form("file", &big, true);
    let (s, v) = t.raw(Method::POST, &uri, Some(&owner), &ct, body).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "больше лимита: {v}");
}
