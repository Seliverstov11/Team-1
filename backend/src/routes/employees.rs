//! Сотрудники. Список видят все (имя ответственного в заказе), заводит
//! и меняет — владелец (ФТ-Р-04).

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use super::{collect, first, new_id, now};
use crate::{
    auth::{ADMIN, ANY_STAFF, Role, Staff, check_password_rules, hash_password},
    db::Db,
    error::{ApiError, ApiResult, bad},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::patch(update))
}

#[derive(Deserialize, Serialize)]
struct EmployeeView {
    id: String,
    name: String,
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    phone: Option<String>,
    is_active: bool,
    last_login_at: Option<String>,
}

#[derive(Deserialize)]
struct EmployeeRow {
    id: String,
    name: String,
    role: String,
    login: Option<String>,
    phone: Option<String>,
    is_active: i64,
    last_login_at: Option<String>,
}

fn view(r: EmployeeRow, with_login: bool) -> EmployeeView {
    EmployeeView {
        id: r.id,
        name: r.name,
        role: r.role,
        login: r.login.filter(|_| with_login),
        phone: r.phone,
        is_active: r.is_active != 0,
        last_login_at: r.last_login_at,
    }
}

const COLS: &str = "id, name, role, login, phone, is_active, last_login_at";

async fn load(conn: &libsql::Connection, id: &str) -> ApiResult<EmployeeRow> {
    let rows = conn.query(&format!("SELECT {COLS} FROM employees WHERE id = ?1"), libsql::params![id]).await?;
    first(rows).await?.ok_or(ApiError::NotFound)
}

async fn list(staff: Staff, State(db): State<Db>) -> ApiResult<Json<Vec<EmployeeView>>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let rows = conn.query(&format!("SELECT {COLS} FROM employees ORDER BY is_active DESC, role, name"), ()).await?;
    let admin = staff.is(Role::Admin);
    Ok(Json(collect::<EmployeeRow>(rows).await?.into_iter().map(|r| view(r, admin)).collect()))
}

#[derive(Deserialize)]
struct EmployeeInput {
    name: Option<String>,
    login: Option<String>,
    role: Option<String>,
    password: Option<String>,
    phone: Option<String>,
    is_active: Option<bool>,
}

fn check_role(r: &Option<String>) -> ApiResult<()> {
    if let Some(r) = r
        && Role::parse(r).is_none()
    {
        return Err(bad("роль: admin (владелец), manager или warehouse"));
    }
    Ok(())
}

async fn login_free(conn: &libsql::Connection, login: &str, except: Option<&str>) -> ApiResult<()> {
    let mut r = conn
        .query("SELECT 1 FROM employees WHERE login = ?1 AND (?2 IS NULL OR id <> ?2)", libsql::params![login, except])
        .await?;
    if r.next().await?.is_some() {
        return Err(ApiError::Conflict(format!("логин «{login}» уже занят")));
    }
    Ok(())
}

fn clean_login(l: &str) -> ApiResult<String> {
    let l = l.trim().to_lowercase();
    if l.len() < 3 || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
        return Err(bad("логин — латиница, цифры, точка или _, не короче 3 символов"));
    }
    Ok(l)
}

async fn create(staff: Staff, State(db): State<Db>, Json(b): Json<EmployeeInput>) -> ApiResult<Json<EmployeeView>> {
    staff.require(ADMIN)?;
    let name = b.name.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad("имя обязательно"));
    }
    let login = clean_login(b.login.as_deref().ok_or_else(|| bad("логин обязателен"))?)?;
    let role = b.role.clone().ok_or_else(|| bad("роль обязательна"))?;
    check_role(&Some(role.clone()))?;
    let pw = b.password.as_deref().ok_or_else(|| bad("пароль обязателен"))?;
    check_password_rules(pw)?;
    let conn = db.connect()?;
    login_free(&conn, &login, None).await?;
    let id = new_id();
    conn.execute(
        "INSERT INTO employees (id, name, login, role, phone, password_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        libsql::params![id.clone(), name, login, role, b.phone.clone(), hash_password(pw).await?, now()],
    )
    .await?;
    Ok(Json(view(load(&conn, &id).await?, true)))
}

async fn update(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<EmployeeInput>) -> ApiResult<Json<EmployeeView>> {
    staff.require(ADMIN)?;
    check_role(&b.role)?;
    if b.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return Err(bad("имя не может быть пустым"));
    }
    let conn = db.connect()?;
    let cur = load(&conn, &id).await?;
    let login = b.login.as_deref().map(clean_login).transpose()?;
    if let Some(l) = &login {
        login_free(&conn, l, Some(&id)).await?;
    }
    // Без владельца никто не сможет заводить сотрудников и менять цены.
    let loses_admin = cur.role == "admin"
        && cur.is_active == 1
        && (b.role.as_deref().is_some_and(|r| r != "admin") || b.is_active == Some(false));
    if loses_admin {
        let mut r = conn
            .query("SELECT COUNT(*) FROM employees WHERE role = 'admin' AND is_active = 1 AND id <> ?1", libsql::params![id.clone()])
            .await?;
        let others: i64 = r.next().await?.map(|r| r.get(0)).transpose()?.unwrap_or(0);
        if others == 0 {
            return Err(ApiError::Conflict("это последний владелец — сначала назначьте другого".into()));
        }
    }
    let hash = match b.password.as_deref() {
        Some(pw) => {
            check_password_rules(pw)?;
            Some(hash_password(pw).await?)
        }
        None => None,
    };
    conn.execute(
        "UPDATE employees SET name = IFNULL(?1, name), login = IFNULL(?2, login), role = IFNULL(?3, role),
                phone = IFNULL(?4, phone), is_active = IFNULL(?5, is_active), password_hash = IFNULL(?6, password_hash)
         WHERE id = ?7",
        libsql::params![
            b.name.as_deref().map(str::trim),
            login,
            b.role.clone(),
            b.phone.clone(),
            b.is_active.map(i64::from),
            hash.clone(),
            id.clone()
        ],
    )
    .await?;
    // Отключённый сотрудник и сменённый пароль — выход со всех устройств.
    if b.is_active == Some(false) || hash.is_some() || b.role.is_some() {
        conn.execute("DELETE FROM sessions WHERE kind = 'employee' AND subject_id = ?1", libsql::params![id.clone()]).await?;
    }
    Ok(Json(view(load(&conn, &id).await?, true)))
}
