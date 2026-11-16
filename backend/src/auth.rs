//! Вход сотрудников и покупателей, роли, проверка прав на сервере.
//!
//! Права проверяет сервер, а не интерфейс: скрытая кнопка — не защита
//! (ФТ-Р-05). Каждый закрытый обработчик принимает `Staff` и вызывает
//! `require` со списком ролей, которым действие разрешено.
//!
//! Токен — случайная строка, которую клиент шлёт в `Authorization: Bearer`.
//! В базе лежит только его SHA-256: утечка таблицы сессий не даёт войти.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::request::Parts,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    db::Db,
    error::{ApiError, ApiResult, bad},
    routes::{now, phone_norm},
};

// ─── Роли ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Владелец магазина: все права, каталог, цены, сотрудники.
    Admin,
    /// Заказы, клиенты, оплаты.
    Manager,
    /// Склад и сборка. Оплаты не видит, цены не меняет.
    Warehouse,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "manager" => Some(Role::Manager),
            "warehouse" => Some(Role::Warehouse),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Manager => "manager",
            Role::Warehouse => "warehouse",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Role::Admin => "владелец",
            Role::Manager => "менеджер",
            Role::Warehouse => "кладовщик",
        }
    }
}

pub const ADMIN: &[Role] = &[Role::Admin];
pub const OFFICE: &[Role] = &[Role::Admin, Role::Manager];
pub const STOREKEEPING: &[Role] = &[Role::Admin, Role::Warehouse];
pub const ANY_STAFF: &[Role] = &[Role::Admin, Role::Manager, Role::Warehouse];

// ─── Кто делает запрос ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Staff {
    pub id: String,
    pub name: String,
    pub role: Role,
}

impl Staff {
    pub fn require(&self, roles: &[Role]) -> ApiResult<()> {
        if roles.contains(&self.role) {
            return Ok(());
        }
        let allowed = roles.iter().map(|r| r.title()).collect::<Vec<_>>().join(", ");
        Err(ApiError::Forbidden(format!(
            "недостаточно прав: действие доступно ролям «{allowed}»"
        )))
    }

    pub fn is(&self, role: Role) -> bool {
        self.role == role
    }
}

#[derive(Debug, Clone)]
pub struct Customer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Actor {
    Staff(Staff),
    Customer(Customer),
}

/// Для публичных запросов: вход необязателен. Просроченный или чужой
/// токен — это просто гость, а не ошибка, чтобы сайт не ломался.
pub struct MaybeActor(pub Option<Actor>);

impl MaybeActor {
    pub fn staff(&self) -> Option<&Staff> {
        match &self.0 {
            Some(Actor::Staff(s)) => Some(s),
            _ => None,
        }
    }

    pub fn customer(&self) -> Option<&Customer> {
        match &self.0 {
            Some(Actor::Customer(c)) => Some(c),
            _ => None,
        }
    }
}

/// Кто записывается в журнал заказа.
pub struct ActorRef {
    pub kind: &'static str,
    pub id: Option<String>,
    pub name: Option<String>,
}

impl ActorRef {
    pub fn system() -> Self {
        Self { kind: "system", id: None, name: None }
    }
}

impl From<&Staff> for ActorRef {
    fn from(s: &Staff) -> Self {
        Self { kind: "employee", id: Some(s.id.clone()), name: Some(s.name.clone()) }
    }
}

impl From<&MaybeActor> for ActorRef {
    fn from(a: &MaybeActor) -> Self {
        match &a.0 {
            Some(Actor::Staff(s)) => s.into(),
            Some(Actor::Customer(c)) => Self {
                kind: "client",
                id: Some(c.id.clone()),
                name: Some(c.name.clone()),
            },
            None => Self { kind: "client", id: None, name: None },
        }
    }
}

fn bearer(parts: &Parts) -> Option<String> {
    let v = parts.headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let token = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

async fn resolve(db: &Db, token: &str) -> ApiResult<Option<Actor>> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT s.kind, s.subject_id,
                    e.name, e.role, e.is_active,
                    c.name
             FROM sessions s
             LEFT JOIN employees e ON s.kind = 'employee' AND e.id = s.subject_id
             LEFT JOIN clients   c ON s.kind = 'client'   AND c.id = s.subject_id
             WHERE s.token_hash = ?1 AND s.expires_at > ?2",
            libsql::params![token_hash(token), now()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let kind: String = row.get(0)?;
    let id: String = row.get(1)?;
    if kind == "employee" {
        let name: Option<String> = row.get(2)?;
        let role: Option<String> = row.get(3)?;
        let active: Option<i64> = row.get(4)?;
        match (name, role.as_deref().and_then(Role::parse), active) {
            (Some(name), Some(role), Some(1)) => Ok(Some(Actor::Staff(Staff { id, name, role }))),
            _ => Ok(None),
        }
    } else {
        let name: Option<String> = row.get(5)?;
        Ok(name.map(|name| Actor::Customer(Customer { id, name })))
    }
}

impl FromRequestParts<Db> for MaybeActor {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, db: &Db) -> Result<Self, Self::Rejection> {
        let Some(token) = bearer(parts) else {
            return Ok(MaybeActor(None));
        };
        Ok(MaybeActor(resolve(db, &token).await?))
    }
}

impl FromRequestParts<Db> for Staff {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, db: &Db) -> Result<Self, Self::Rejection> {
        let token = bearer(parts)
            .ok_or_else(|| ApiError::Unauthorized("нужен вход сотрудника".into()))?;
        match resolve(db, &token).await? {
            Some(Actor::Staff(s)) => Ok(s),
            Some(Actor::Customer(_)) => {
                Err(ApiError::Forbidden("раздел доступен только сотрудникам".into()))
            }
            None => Err(ApiError::Unauthorized("сессия истекла, войдите заново".into())),
        }
    }
}

impl FromRequestParts<Db> for Customer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, db: &Db) -> Result<Self, Self::Rejection> {
        let token =
            bearer(parts).ok_or_else(|| ApiError::Unauthorized("нужен вход покупателя".into()))?;
        match resolve(db, &token).await? {
            Some(Actor::Customer(c)) => Ok(c),
            Some(Actor::Staff(_)) => {
                Err(ApiError::Forbidden("раздел доступен только покупателю".into()))
            }
            None => Err(ApiError::Unauthorized("сессия истекла, войдите заново".into())),
        }
    }
}

// ─── Пароли и токены ─────────────────────────────────────────────────────────

// ─── Ограничение попыток входа ───────────────────────────────────────────────
// Пауза после серии неверных паролей по одному логину или телефону: перебор
// прямо в API без неё ничем не ограничен (ревью PR #13, задача #35).
// Счётчик в памяти процесса — для одного сервера этого достаточно.

const MAX_FAILS: u32 = 5;
const LOCK: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Неудачи старше этого срока забываются: опечатки раз в день не копятся.
const FORGET: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct LoginLimiter(std::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>);

impl LoginLimiter {
    fn check(&self, key: &str) -> ApiResult<()> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((fails, last)) = map.get(key)
            && *fails >= MAX_FAILS
        {
            let left = LOCK.saturating_sub(last.elapsed());
            if !left.is_zero() {
                let minutes = left.as_secs().div_ceil(60).max(1);
                return Err(ApiError::TooManyRequests(format!(
                    "слишком много неверных попыток — попробуйте через {minutes} мин."
                )));
            }
        }
        Ok(())
    }

    fn fail(&self, key: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(key.to_string()).or_insert((0, std::time::Instant::now()));
        if entry.1.elapsed() > FORGET || (entry.0 >= MAX_FAILS && entry.1.elapsed() > LOCK) {
            entry.0 = 0;
        }
        entry.0 += 1;
        entry.1 = std::time::Instant::now();
    }

    fn success(&self, key: &str) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
    }
}

pub const MIN_PASSWORD: usize = 6;

pub fn check_password_rules(pw: &str) -> ApiResult<()> {
    if pw.chars().count() < MIN_PASSWORD {
        return Err(bad(format!("пароль — не короче {MIN_PASSWORD} символов")));
    }
    Ok(())
}

/// Argon2 нарочно медленный, поэтому считается вне потоков обработки запросов.
pub async fn hash_password(pw: &str) -> ApiResult<String> {
    let pw = pw.to_string();
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(pw.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| ApiError::Other(anyhow::anyhow!("хеш пароля: {e}")))
    })
    .await
    .map_err(|e| ApiError::Other(anyhow::anyhow!("хеш пароля: {e}")))?
}

pub async fn verify_password(hash: &str, pw: &str) -> bool {
    let (hash, pw) = (hash.to_string(), pw.to_string());
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn open_session(db: &Db, kind: &str, subject_id: &str) -> ApiResult<(String, String)> {
    let token = new_token();
    let expires = (chrono::Utc::now() + chrono::Duration::days(crate::config::get().session_days))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO sessions (token_hash, kind, subject_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![token_hash(&token), kind, subject_id, now(), expires.clone()],
    )
    .await?;
    Ok((token, expires))
}

/// Пароли в репозиторий не кладём (H12). При старте всем сотрудникам без
/// пароля выставляется пароль из BOOTSTRAP_PASSWORD; дальше владелец
/// меняет их через API. Без переменной вход сотрудников не настроен.
pub async fn bootstrap_passwords(db: &Db) -> anyhow::Result<()> {
    let Some(pw) = std::env::var("BOOTSTRAP_PASSWORD").ok().filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let conn = db.connect()?;
    let mut rows = conn
        .query("SELECT id FROM employees WHERE password_hash IS NULL", ())
        .await?;
    let mut ids = Vec::new();
    while let Some(r) = rows.next().await? {
        ids.push(r.get::<String>(0)?);
    }
    for id in &ids {
        let h = hash_password(&pw).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE employees SET password_hash = ?1 WHERE id = ?2",
            libsql::params![h, id.clone()],
        )
        .await?;
    }
    if !ids.is_empty() {
        tracing::info!("начальный пароль выставлен сотрудникам: {}", ids.len());
    }
    Ok(())
}

// ─── Маршруты ────────────────────────────────────────────────────────────────

pub fn router() -> Router<Db> {
    Router::new()
        .route("/login", post(staff_login))
        .route("/client/login", post(client_login))
        .route("/client/register", post(client_register))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

#[derive(Deserialize)]
pub struct StaffLogin {
    pub login: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ClientLogin {
    pub phone: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ClientRegister {
    pub name: String,
    pub phone: String,
    pub password: String,
    pub city: Option<String>,
}

#[derive(Serialize)]
pub struct Session {
    pub token: String,
    pub expires_at: String,
    pub kind: &'static str,
    pub user: UserView,
}

#[derive(Serialize)]
pub struct UserView {
    pub id: String,
    pub name: String,
    /// Для сотрудника — admin | manager | warehouse, для покупателя — client.
    pub role: String,
}

const WRONG: &str = "неверный логин или пароль";

async fn staff_login(State(db): State<Db>, Json(b): Json<StaffLogin>) -> ApiResult<Json<Session>> {
    let key = format!("staff:{}", b.login.trim().to_lowercase());
    db.logins.check(&key)?;
    let conn = db.connect()?;
    let (id, name, role, hash, active) = {
        let mut rows = conn
            .query(
                "SELECT id, name, role, password_hash, is_active FROM employees WHERE login = ?1",
                libsql::params![b.login.trim().to_lowercase()],
            )
            .await?;
        let Some(r) = rows.next().await? else {
            db.logins.fail(&key);
            return Err(ApiError::Unauthorized(WRONG.into()));
        };
        (r.get::<String>(0)?, r.get::<String>(1)?, r.get::<String>(2)?, r.get::<Option<String>>(3)?, r.get::<i64>(4)?)
    };
    let ok = match hash {
        Some(h) => verify_password(&h, &b.password).await,
        None => false,
    };
    if !ok || active != 1 {
        db.logins.fail(&key);
        return Err(ApiError::Unauthorized(WRONG.into()));
    }
    db.logins.success(&key);
    conn.execute(
        "UPDATE employees SET last_login_at = ?1 WHERE id = ?2",
        libsql::params![now(), id.clone()],
    )
    .await?;
    let (token, expires_at) = open_session(&db, "employee", &id).await?;
    Ok(Json(Session { token, expires_at, kind: "employee", user: UserView { id, name, role } }))
}

async fn client_login(State(db): State<Db>, Json(b): Json<ClientLogin>) -> ApiResult<Json<Session>> {
    let norm = phone_norm(&b.phone).ok_or_else(|| bad("телефон указан неверно"))?;
    let key = format!("client:{norm}");
    db.logins.check(&key)?;
    let conn = db.connect()?;
    let (id, name, hash) = {
        let mut rows = conn
            .query(
                "SELECT id, name, password_hash FROM clients
                 WHERE phone_norm = ?1 AND password_hash IS NOT NULL",
                libsql::params![norm],
            )
            .await?;
        let Some(r) = rows.next().await? else {
            db.logins.fail(&key);
            return Err(ApiError::Unauthorized("неверный телефон или пароль".into()));
        };
        (r.get::<String>(0)?, r.get::<String>(1)?, r.get::<String>(2)?)
    };
    if !verify_password(&hash, &b.password).await {
        db.logins.fail(&key);
        return Err(ApiError::Unauthorized("неверный телефон или пароль".into()));
    }
    db.logins.success(&key);
    conn.execute(
        "UPDATE clients SET last_login_at = ?1 WHERE id = ?2",
        libsql::params![now(), id.clone()],
    )
    .await?;
    let (token, expires_at) = open_session(&db, "client", &id).await?;
    Ok(Json(Session {
        token,
        expires_at,
        kind: "client",
        user: UserView { id, name, role: "client".into() },
    }))
}

/// Регистрация покупателя. Если клиент с этим телефоном уже есть в базе
/// (заказывал раньше), учётная запись привязывается к нему, и история его
/// заказов сразу видна в профиле. Исключение — клиент с персональными
/// ценами: подтвердить, что телефон его, без SMS нельзя, поэтому такой
/// учётке пароль выставляет владелец.
async fn client_register(
    State(db): State<Db>,
    Json(b): Json<ClientRegister>,
) -> ApiResult<Json<Session>> {
    let name = b.name.trim();
    if name.is_empty() {
        return Err(bad("имя обязательно"));
    }
    let norm = phone_norm(&b.phone).ok_or_else(|| bad("телефон указан неверно"))?;
    check_password_rules(&b.password)?;
    let hash = hash_password(&b.password).await?;

    let conn = db.connect()?;
    let existing = {
        let mut rows = conn
            .query(
                "SELECT c.id, c.password_hash IS NOT NULL,
                        EXISTS (SELECT 1 FROM client_prices p WHERE p.client_id = c.id)
                 FROM clients c WHERE c.phone_norm = ?1 LIMIT 1",
                libsql::params![norm.clone()],
            )
            .await?;
        match rows.next().await? {
            Some(r) => Some((r.get::<String>(0)?, r.get::<i64>(1)?, r.get::<i64>(2)?)),
            None => None,
        }
    };
    let id = match existing {
        Some((id, has_password, has_prices)) => {
            if has_password == 1 {
                return Err(ApiError::Conflict(
                    "этот телефон уже зарегистрирован — войдите по паролю".into(),
                ));
            }
            if has_prices == 1 {
                return Err(ApiError::Conflict(
                    "для этого телефона настроены персональные цены — пароль выдаст менеджер магазина"
                        .into(),
                ));
            }
            conn.execute(
                "UPDATE clients SET password_hash = ?1, city = IFNULL(city, ?2) WHERE id = ?3",
                libsql::params![hash, b.city.clone(), id.clone()],
            )
            .await?;
            id
        }
        None => {
            let id = crate::routes::new_id();
            conn.execute(
                "INSERT INTO clients (id, name, phone, phone_norm, city, password_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                libsql::params![id.clone(), name, b.phone.trim(), norm, b.city.clone(), hash, now()],
            )
            .await?;
            id
        }
    };
    let (token, expires_at) = open_session(&db, "client", &id).await?;
    Ok(Json(Session {
        token,
        expires_at,
        kind: "client",
        user: UserView { id, name: name.to_string(), role: "client".into() },
    }))
}

async fn logout(
    State(db): State<Db>,
    parts: axum::http::HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(token) = parts
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        let conn = db.connect()?;
        conn.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            libsql::params![token_hash(token.trim())],
        )
        .await?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn me(actor: MaybeActor) -> ApiResult<Json<UserView>> {
    match actor.0 {
        Some(Actor::Staff(s)) => Ok(Json(UserView { id: s.id, name: s.name, role: s.role.as_str().into() })),
        Some(Actor::Customer(c)) => Ok(Json(UserView { id: c.id, name: c.name, role: "client".into() })),
        None => Err(ApiError::Unauthorized("вход не выполнен".into())),
    }
}
