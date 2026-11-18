//! «Сообщить о поступлении» (ФТ-К-06): покупатель оставляет телефон на товар,
//! которого нет в наличии; менеджер видит заявки в CRM и перезванивает, когда товар пришёл.
//! Это не предзаказ: товар не резервируется, денег не берём.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch},
};
use serde::{Deserialize, Serialize};

use super::{collect, first, new_id, now, phone_norm};
use crate::{
    auth::{MaybeActor, OFFICE, Staff},
    db::Db,
    error::{ApiError, ApiResult, bad},
    num::milli_to_string,
};

/// Сколько открытых заявок может висеть на одном телефоне — защита от спама формой
const MAX_WAITING_PER_PHONE: i64 = 30;

pub fn router() -> Router<Db> {
    Router::new().route("/", get(list).post(create)).route("/{id}", patch(update))
}

#[derive(Deserialize)]
struct RestockInput {
    product_id: String,
    name: Option<String>,
    phone: Option<String>,
    qty: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct RestockView {
    id: String,
    product_id: String,
    product_name: String,
    variant_name: Option<String>,
    sku: Option<String>,
    unit: String,
    available: String,
    client_id: Option<String>,
    name: String,
    phone: String,
    qty: Option<i64>,
    status: String,
    created_at: String,
    handled_by: Option<String>,
    handled_at: Option<String>,
}

const VIEW: &str = "SELECT r.id, r.product_id, p.name AS product_name, p.variant_name, p.sku, p.unit,
        p.stock_milli - p.reserved_milli AS available_milli, r.client_id, r.name, r.phone, r.qty, r.status,
        r.created_at, r.handled_by, r.handled_at
    FROM restock_requests r JOIN products p ON p.id = r.product_id";

#[derive(Deserialize)]
struct Row {
    id: String,
    product_id: String,
    product_name: String,
    variant_name: Option<String>,
    sku: Option<String>,
    unit: String,
    available_milli: i64,
    client_id: Option<String>,
    name: String,
    phone: String,
    qty: Option<i64>,
    status: String,
    created_at: String,
    handled_by: Option<String>,
    handled_at: Option<String>,
}

impl From<Row> for RestockView {
    fn from(r: Row) -> Self {
        RestockView {
            id: r.id,
            product_id: r.product_id,
            product_name: r.product_name,
            variant_name: r.variant_name,
            sku: r.sku,
            unit: r.unit,
            available: milli_to_string(r.available_milli.max(0)),
            client_id: r.client_id,
            name: r.name,
            phone: r.phone,
            qty: r.qty,
            status: r.status,
            created_at: r.created_at,
            handled_by: r.handled_by,
            handled_at: r.handled_at,
        }
    }
}

async fn load(conn: &libsql::Connection, id: &str) -> ApiResult<RestockView> {
    let rows = conn.query(&format!("{VIEW} WHERE r.id = ?1"), libsql::params![id]).await?;
    first::<Row>(rows).await?.map(Into::into).ok_or(ApiError::NotFound)
}

/// Оставить заявку. Без входа — имя и телефон обязательны; вошедший покупатель — из профиля.
/// Повторная заявка того же телефона на тот же товар не дублируется.
async fn create(actor: MaybeActor, State(db): State<Db>, Json(b): Json<RestockInput>) -> ApiResult<Json<RestockView>> {
    let conn = db.connect()?;
    let mut r = conn
        .query("SELECT 1 FROM products WHERE id = ?1 AND is_archived = 0", libsql::params![b.product_id.clone()])
        .await?;
    if r.next().await?.is_none() {
        return Err(bad("товар не найден"));
    }
    let (mut name, mut phone) = (b.name.clone(), b.phone.clone());
    let client_id = actor.customer().map(|c| c.id.clone());
    if let Some(cid) = &client_id {
        let mut r = conn.query("SELECT name, phone FROM clients WHERE id = ?1", libsql::params![cid.clone()]).await?;
        if let Some(row) = r.next().await? {
            name = name.filter(|n| !n.trim().is_empty()).or(Some(row.get::<String>(0)?));
            phone = phone.filter(|p| !p.trim().is_empty()).or(row.get::<Option<String>>(1)?);
        }
    }
    let name = name.map(|n| n.trim().to_string()).filter(|n| n.chars().count() >= 2).ok_or_else(|| bad("укажите имя"))?;
    let phone = phone.map(|p| p.trim().to_string()).unwrap_or_default();
    let norm = phone_norm(&phone).ok_or_else(|| bad("телефон в формате +7 (7XX) XXX-XX-XX"))?;
    if let Some(q) = b.qty
        && !(1..=100_000).contains(&q)
    {
        return Err(bad("количество — от 1"));
    }

    let mut r = conn
        .query(
            "SELECT id FROM restock_requests WHERE phone_norm = ?1 AND product_id = ?2 AND status = 'waiting'",
            libsql::params![norm.clone(), b.product_id.clone()],
        )
        .await?;
    if let Some(row) = r.next().await? {
        let id: String = row.get(0)?;
        if let Some(q) = b.qty {
            conn.execute("UPDATE restock_requests SET qty = ?1 WHERE id = ?2", libsql::params![q, id.clone()]).await?;
        }
        return Ok(Json(load(&conn, &id).await?));
    }
    let mut r = conn
        .query("SELECT COUNT(*) FROM restock_requests WHERE phone_norm = ?1 AND status = 'waiting'", libsql::params![norm.clone()])
        .await?;
    let waiting: i64 = r.next().await?.map(|row| row.get(0)).transpose()?.unwrap_or(0);
    if waiting >= MAX_WAITING_PER_PHONE {
        return Err(ApiError::TooManyRequests("слишком много заявок с этого телефона — менеджер уже свяжется с вами".into()));
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO restock_requests (id, product_id, client_id, name, phone, phone_norm, qty, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        libsql::params![id.clone(), b.product_id.clone(), client_id, name, phone, norm, b.qty, now()],
    )
    .await?;
    Ok(Json(load(&conn, &id).await?))
}

#[derive(Deserialize)]
struct ListQuery {
    /// waiting (по умолчанию) | notified | cancelled | all
    status: Option<String>,
}

async fn list(staff: Staff, State(db): State<Db>, Query(q): Query<ListQuery>) -> ApiResult<Json<Vec<RestockView>>> {
    staff.require(OFFICE)?;
    let status = q.status.unwrap_or_else(|| "waiting".into());
    let conn = db.connect()?;
    // Сначала то, что уже пришло на склад: этим клиентам пора звонить
    let rows = conn
        .query(
            &format!(
                "{VIEW} WHERE (?1 = 'all' OR r.status = ?1)
                 ORDER BY (p.stock_milli - p.reserved_milli > 0) DESC, r.created_at DESC LIMIT 500"
            ),
            libsql::params![status],
        )
        .await?;
    Ok(Json(collect::<Row>(rows).await?.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
struct RestockPatch {
    status: String,
}

async fn update(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<RestockPatch>) -> ApiResult<Json<RestockView>> {
    staff.require(OFFICE)?;
    if !["waiting", "notified", "cancelled"].contains(&b.status.as_str()) {
        return Err(bad("статус: waiting, notified или cancelled"));
    }
    let conn = db.connect()?;
    let n = conn
        .execute(
            "UPDATE restock_requests SET status = ?1, handled_by = ?2, handled_at = ?3 WHERE id = ?4",
            libsql::params![b.status.clone(), staff.id.clone(), now(), id.clone()],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(load(&conn, &id).await?))
}
