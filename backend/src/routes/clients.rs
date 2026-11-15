//! База клиентов: кто покупал, сколько, на какую сумму, долг по оплате,
//! персональные цены (ФТ-КЛ-01…03). Сайт и мессенджеры склеиваются по телефону.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};

use super::{collect, first, new_id, now, phone_norm};
use crate::{
    auth::{ADMIN, OFFICE, Staff, check_password_rules, hash_password},
    db::Db,
    error::{ApiError, ApiResult, bad},
    models::OrderBrief,
    num::{milli_to_string, parse_tiyn, tiyn_to_string},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(list_clients).post(create_client))
        .route("/{id}", get(get_client).patch(update_client))
        .route("/{id}/prices", get(list_prices))
        .route("/{id}/prices/{product_id}", put(set_price).delete(delete_price))
        .route("/{id}/password", post(set_password))
}

const CLIENT_TYPES: &[&str] = &["home_baker", "bakery", "cafe", "shop", "other"];

const CLIENT_SQL: &str = "SELECT c.id, c.name, c.phone, c.city, c.client_type, c.note, c.created_at,
       c.last_login_at, c.password_hash IS NOT NULL AS has_account,
       (SELECT COUNT(*) FROM orders o WHERE o.client_id = c.id AND o.status <> 'cancelled') AS orders_count,
       (SELECT IFNULL(SUM(o.total_tiyn), 0) FROM orders o
         WHERE o.client_id = c.id AND o.status <> 'cancelled') AS orders_tiyn,
       (SELECT IFNULL(SUM(o.total_tiyn), 0) FROM orders o
         WHERE o.client_id = c.id AND o.status IN ('handed_over','shipped','delivered','completed')) AS bought_tiyn,
       (SELECT IFNULL(SUM(MAX(o.total_tiyn - o.paid_tiyn, 0)), 0) FROM orders o
         WHERE o.client_id = c.id AND o.status <> 'cancelled' AND o.payment_status <> 'refunded') AS debt_tiyn,
       (SELECT MAX(o.created_at) FROM orders o WHERE o.client_id = c.id) AS last_order_at,
       (SELECT GROUP_CONCAT(DISTINCT o.channel) FROM orders o WHERE o.client_id = c.id) AS channels,
       (SELECT COUNT(*) FROM client_prices p WHERE p.client_id = c.id) AS personal_prices
     FROM clients c";

#[derive(Deserialize)]
struct ClientRow {
    id: String,
    name: String,
    phone: Option<String>,
    city: Option<String>,
    client_type: String,
    note: Option<String>,
    created_at: String,
    last_login_at: Option<String>,
    has_account: i64,
    orders_count: i64,
    orders_tiyn: i64,
    bought_tiyn: i64,
    debt_tiyn: i64,
    last_order_at: Option<String>,
    channels: Option<String>,
    personal_prices: i64,
}

#[derive(Serialize)]
pub struct ClientView {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub city: Option<String>,
    /// home_baker | bakery | cafe | shop | other
    pub client_type: String,
    pub note: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub has_account: bool,
    /// Заказы без отменённых.
    pub orders_count: i64,
    pub orders_total: String,
    /// Выкуплено: переданные, отправленные, доставленные и завершённые.
    pub bought_total: String,
    /// Не оплачено по действующим заказам.
    pub debt: String,
    pub last_order_at: Option<String>,
    pub channels: Vec<String>,
    pub personal_prices: i64,
}

impl From<ClientRow> for ClientView {
    fn from(r: ClientRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            phone: r.phone,
            city: r.city,
            client_type: r.client_type,
            note: r.note,
            created_at: r.created_at,
            last_login_at: r.last_login_at,
            has_account: r.has_account != 0,
            orders_count: r.orders_count,
            orders_total: tiyn_to_string(r.orders_tiyn),
            bought_total: tiyn_to_string(r.bought_tiyn),
            debt: tiyn_to_string(r.debt_tiyn),
            last_order_at: r.last_order_at,
            channels: r.channels.map(|c| c.split(',').map(String::from).collect()).unwrap_or_default(),
            personal_prices: r.personal_prices,
        }
    }
}

pub async fn load_client(conn: &libsql::Connection, id: &str) -> ApiResult<ClientView> {
    let rows = conn.query(&format!("{CLIENT_SQL} WHERE c.id = ?1"), libsql::params![id]).await?;
    first::<ClientRow>(rows).await?.map(ClientView::from).ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
struct ClientQuery {
    search: Option<String>,
    client_type: Option<String>,
    /// Только с долгом по оплате.
    debtors: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_clients(staff: Staff, State(db): State<Db>, Query(q): Query<ClientQuery>) -> ApiResult<Json<Vec<ClientView>>> {
    staff.require(OFFICE)?;
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let like = search.map(|s| format!("%{s}%"));
    let digits = search.and_then(phone_norm_partial);
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "SELECT * FROM ({CLIENT_SQL}) x
                 WHERE (?1 IS NULL OR x.name LIKE ?1 OR (?2 IS NOT NULL AND x.id IN
                        (SELECT id FROM clients WHERE phone_norm LIKE ?2)))
                   AND (?3 IS NULL OR x.client_type = ?3)
                   AND (?4 IS NULL OR x.debt_tiyn > 0)
                 ORDER BY x.last_order_at IS NULL, x.last_order_at DESC, x.name
                 LIMIT ?5 OFFSET ?6"
            ),
            libsql::params![
                like,
                digits,
                q.client_type.clone(),
                q.debtors.filter(|d| *d).map(|_| 1i64),
                q.limit.unwrap_or(50).clamp(1, 200),
                q.offset.unwrap_or(0).max(0)
            ],
        )
        .await?;
    Ok(Json(collect::<ClientRow>(rows).await?.into_iter().map(ClientView::from).collect()))
}

fn phone_norm_partial(s: &str) -> Option<String> {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    (d.len() >= 4).then(|| format!("%{}%", &d[d.len().saturating_sub(10)..]))
}

#[derive(Serialize)]
struct TopProduct {
    product_name: String,
    quantity: String,
    orders: i64,
}

#[derive(Serialize)]
struct ClientCard {
    #[serde(flatten)]
    client: ClientView,
    orders: Vec<OrderBrief>,
    /// Что берёт регулярно (Б: «что человек покупал, что берёт регулярно»).
    top_products: Vec<TopProduct>,
}

async fn get_client(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<ClientCard>> {
    staff.require(OFFICE)?;
    let conn = db.connect()?;
    let client = load_client(&conn, &id).await?;
    let mut rows = conn
        .query(
            "SELECT id, order_no, status, total_tiyn, created_at FROM orders
             WHERE client_id = ?1 ORDER BY created_at DESC LIMIT 50",
            libsql::params![id.clone()],
        )
        .await?;
    let mut orders = Vec::new();
    while let Some(r) = rows.next().await? {
        orders.push(OrderBrief { id: r.get(0)?, order_no: r.get(1)?, status: r.get(2)?, total: tiyn_to_string(r.get(3)?), created_at: r.get(4)? });
    }
    let mut rows = conn
        .query(
            "SELECT i.product_name, SUM(i.quantity_milli), COUNT(DISTINCT i.order_id)
             FROM order_items i JOIN orders o ON o.id = i.order_id
             WHERE o.client_id = ?1 AND o.status <> 'cancelled'
             GROUP BY i.product_name ORDER BY COUNT(DISTINCT i.order_id) DESC, SUM(i.quantity_milli) DESC LIMIT 10",
            libsql::params![id],
        )
        .await?;
    let mut top = Vec::new();
    while let Some(r) = rows.next().await? {
        top.push(TopProduct { product_name: r.get(0)?, quantity: milli_to_string(r.get(1)?), orders: r.get(2)? });
    }
    Ok(Json(ClientCard { client, orders, top_products: top }))
}

#[derive(Deserialize)]
struct ClientInput {
    name: Option<String>,
    phone: Option<String>,
    city: Option<String>,
    client_type: Option<String>,
    note: Option<String>,
}

fn check_type(t: &Option<String>) -> ApiResult<()> {
    if let Some(t) = t
        && !CLIENT_TYPES.contains(&t.as_str())
    {
        return Err(bad(format!("тип клиента: {}", CLIENT_TYPES.join(", "))));
    }
    Ok(())
}

async fn phone_taken(conn: &libsql::Connection, norm: &str, except: Option<&str>) -> ApiResult<()> {
    let mut r = conn
        .query(
            "SELECT name FROM clients WHERE phone_norm = ?1 AND (?2 IS NULL OR id <> ?2) LIMIT 1",
            libsql::params![norm, except],
        )
        .await?;
    if let Some(row) = r.next().await? {
        let name: String = row.get(0)?;
        return Err(ApiError::Conflict(format!("этот телефон уже у клиента «{name}»")));
    }
    Ok(())
}

async fn create_client(staff: Staff, State(db): State<Db>, Json(b): Json<ClientInput>) -> ApiResult<Json<ClientView>> {
    staff.require(OFFICE)?;
    let name = b.name.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad("имя обязательно"));
    }
    check_type(&b.client_type)?;
    let phone = b.phone.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let norm = phone.and_then(phone_norm);
    if phone.is_some() && norm.is_none() {
        return Err(bad("телефон указан неверно"));
    }
    let conn = db.connect()?;
    if let Some(n) = &norm {
        phone_taken(&conn, n, None).await?;
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO clients (id, name, phone, phone_norm, city, client_type, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        libsql::params![
            id.clone(),
            name,
            phone,
            norm,
            b.city.clone(),
            b.client_type.clone().unwrap_or_else(|| "other".into()),
            b.note.clone(),
            now()
        ],
    )
    .await?;
    Ok(Json(load_client(&conn, &id).await?))
}

async fn update_client(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<ClientInput>) -> ApiResult<Json<ClientView>> {
    staff.require(OFFICE)?;
    check_type(&b.client_type)?;
    if b.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return Err(bad("имя не может быть пустым"));
    }
    let conn = db.connect()?;
    load_client(&conn, &id).await?;
    let norm = match b.phone.as_deref().map(str::trim) {
        Some(p) => {
            let n = phone_norm(p).ok_or_else(|| bad("телефон указан неверно"))?;
            phone_taken(&conn, &n, Some(&id)).await?;
            Some(n)
        }
        None => None,
    };
    conn.execute(
        "UPDATE clients SET name = IFNULL(?1, name), phone = IFNULL(?2, phone), phone_norm = IFNULL(?3, phone_norm),
                city = IFNULL(?4, city), client_type = IFNULL(?5, client_type), note = IFNULL(?6, note)
         WHERE id = ?7",
        libsql::params![
            b.name.as_deref().map(str::trim),
            b.phone.as_deref().map(str::trim),
            norm,
            b.city.clone(),
            b.client_type.clone(),
            b.note.clone(),
            id.clone()
        ],
    )
    .await?;
    Ok(Json(load_client(&conn, &id).await?))
}

#[derive(Serialize)]
struct PriceView {
    product_id: String,
    product_name: String,
    sku: Option<String>,
    variant_name: Option<String>,
    /// Общая цена для сравнения.
    retail_price: String,
    price: String,
}

async fn list_prices(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<Vec<PriceView>>> {
    staff.require(OFFICE)?;
    let conn = db.connect()?;
    load_client(&conn, &id).await?;
    let mut rows = conn
        .query(
            "SELECT p.id, p.name, p.sku, p.variant_name, p.price_tiyn, cp.price_tiyn
             FROM client_prices cp JOIN products p ON p.id = cp.product_id
             WHERE cp.client_id = ?1 ORDER BY p.name, p.variant_name",
            libsql::params![id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(PriceView {
            product_id: r.get(0)?,
            product_name: r.get(1)?,
            sku: r.get(2)?,
            variant_name: r.get(3)?,
            retail_price: tiyn_to_string(r.get(4)?),
            price: tiyn_to_string(r.get(5)?),
        });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct PriceInput {
    price: String,
}

/// Персональную цену заводит владелец (ФТ-КЛ-03, ТЗ вопрос 4).
async fn set_price(
    staff: Staff,
    State(db): State<Db>,
    Path((id, product_id)): Path<(String, String)>,
    Json(b): Json<PriceInput>,
) -> ApiResult<Json<Vec<PriceView>>> {
    staff.require(ADMIN)?;
    let price = parse_tiyn(&b.price).map_err(|e| bad(format!("цена: {e}")))?;
    let conn = db.connect()?;
    load_client(&conn, &id).await?;
    let mut r = conn.query("SELECT 1 FROM products WHERE id = ?1", libsql::params![product_id.clone()]).await?;
    if r.next().await?.is_none() {
        return Err(bad(format!("товар {product_id} не найден")));
    }
    conn.execute(
        "INSERT INTO client_prices (client_id, product_id, price_tiyn, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (client_id, product_id) DO UPDATE SET price_tiyn = excluded.price_tiyn",
        libsql::params![id.clone(), product_id, price, now()],
    )
    .await?;
    list_prices(staff, State(db), Path(id)).await
}

async fn delete_price(
    staff: Staff,
    State(db): State<Db>,
    Path((id, product_id)): Path<(String, String)>,
) -> ApiResult<Json<Vec<PriceView>>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    conn.execute(
        "DELETE FROM client_prices WHERE client_id = ?1 AND product_id = ?2",
        libsql::params![id.clone(), product_id],
    )
    .await?;
    list_prices(staff, State(db), Path(id)).await
}

#[derive(Deserialize)]
struct PasswordInput {
    password: String,
}

/// Владелец выдаёт пароль клиенту с персональными ценами: сам такой клиент
/// зарегистрироваться не может, подтвердить телефон без SMS нечем.
async fn set_password(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<PasswordInput>) -> ApiResult<Json<ClientView>> {
    staff.require(ADMIN)?;
    check_password_rules(&b.password)?;
    let conn = db.connect()?;
    let c = load_client(&conn, &id).await?;
    if c.phone.as_deref().and_then(phone_norm).is_none() {
        return Err(bad("у клиента нет телефона — по нему он входит"));
    }
    let hash = hash_password(&b.password).await?;
    conn.execute("UPDATE clients SET password_hash = ?1 WHERE id = ?2", libsql::params![hash, id.clone()]).await?;
    conn.execute("DELETE FROM sessions WHERE kind = 'client' AND subject_id = ?1", libsql::params![id.clone()]).await?;
    Ok(Json(load_client(&conn, &id).await?))
}
