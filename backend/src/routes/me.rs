//! Профиль покупателя: свои заказы, сколько заказов, сколько выкуплено
//! (ФТ-ЛК-01). Только для вошедшего покупателя.

use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;

use super::{
    clients::{ClientView, load_client},
    collect, now, placeholders,
};
use crate::{
    auth::Customer,
    db::Db,
    error::{ApiResult, bad},
    models::{ORDER_COLS, ORDER_FROM, OrderRow, OrderView},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(profile).patch(update_profile))
        .route("/orders", get(my_orders))
        .route("/favorites", get(favorites).put(set_favorites))
        .route("/cart", get(cart).put(set_cart))
}

async fn profile(me: Customer, State(db): State<Db>) -> ApiResult<Json<ClientView>> {
    let conn = db.connect()?;
    Ok(Json(load_client(&conn, &me.id).await?))
}

#[derive(Deserialize)]
struct ProfilePatch {
    name: Option<String>,
    city: Option<String>,
}

async fn update_profile(me: Customer, State(db): State<Db>, Json(b): Json<ProfilePatch>) -> ApiResult<Json<ClientView>> {
    if b.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return Err(bad("имя не может быть пустым"));
    }
    let conn = db.connect()?;
    conn.execute(
        "UPDATE clients SET name = IFNULL(?1, name), city = IFNULL(?2, city) WHERE id = ?3",
        libsql::params![b.name.as_deref().map(str::trim), b.city.clone(), me.id.clone()],
    )
    .await?;
    Ok(Json(load_client(&conn, &me.id).await?))
}

async fn my_orders(me: Customer, State(db): State<Db>) -> ApiResult<Json<Vec<OrderView>>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!("SELECT {ORDER_COLS} FROM {ORDER_FROM} WHERE o.client_id = ?1 ORDER BY o.created_at DESC LIMIT 100"),
            libsql::params![me.id],
        )
        .await?;
    let orders: Vec<OrderRow> = collect(rows).await?;
    Ok(Json(orders.into_iter().map(OrderView::from).collect()))
}

// ---------------------------------------------------------------- избранное и корзина на всех устройствах
// Сайт хранит их в браузере; у вошедшего покупателя — ещё и здесь, чтобы телефон и компьютер видели одно и то же.
// PUT заменяет список целиком: сайт сам сливает своё с серверным при входе.

const MAX_FAVORITES: usize = 500;
const MAX_CART_LINES: usize = 300;

async fn favorites(me: Customer, State(db): State<Db>) -> ApiResult<Json<Vec<String>>> {
    let conn = db.connect()?;
    let mut rows = conn
        .query("SELECT item_key FROM client_favorites WHERE client_id = ?1 ORDER BY created_at", libsql::params![me.id])
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(r.get::<String>(0)?);
    }
    Ok(Json(out))
}

async fn set_favorites(me: Customer, State(db): State<Db>, Json(keys): Json<Vec<String>>) -> ApiResult<Json<Vec<String>>> {
    let mut keys: Vec<String> = keys.into_iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty() && k.len() <= 100).collect();
    keys.dedup();
    if keys.len() > MAX_FAVORITES {
        return Err(bad(format!("в избранном не больше {MAX_FAVORITES} товаров")));
    }
    let conn = db.connect()?;
    let tx = conn.transaction().await?;
    tx.execute("DELETE FROM client_favorites WHERE client_id = ?1", libsql::params![me.id.clone()]).await?;
    let at = now();
    for k in &keys {
        tx.execute(
            "INSERT OR IGNORE INTO client_favorites (client_id, item_key, created_at) VALUES (?1, ?2, ?3)",
            libsql::params![me.id.clone(), k.clone(), at.clone()],
        )
        .await?;
    }
    tx.commit().await?;
    favorites(me, State(db)).await
}

async fn cart(me: Customer, State(db): State<Db>) -> ApiResult<Json<BTreeMap<String, i64>>> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT c.product_id, c.qty FROM client_cart c JOIN products p ON p.id = c.product_id
             WHERE c.client_id = ?1 AND p.is_archived = 0",
            libsql::params![me.id],
        )
        .await?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next().await? {
        out.insert(r.get::<String>(0)?, r.get::<i64>(1)?);
    }
    Ok(Json(out))
}

/// { id товара: штук }. Неизвестные и снятые с продажи товары молча отбрасываются —
/// корзина могла пролежать в браузере дольше, чем товар был в каталоге
async fn set_cart(me: Customer, State(db): State<Db>, Json(items): Json<BTreeMap<String, i64>>) -> ApiResult<Json<BTreeMap<String, i64>>> {
    if items.len() > MAX_CART_LINES {
        return Err(bad(format!("в корзине не больше {MAX_CART_LINES} позиций")));
    }
    let items: Vec<(String, i64)> = items.into_iter().filter(|(_, q)| *q > 0).map(|(k, q)| (k, q.min(1_000_000))).collect();
    let conn = db.connect()?;
    let known: Vec<String> = if items.is_empty() {
        Vec::new()
    } else {
        let sql = format!("SELECT id FROM products WHERE is_archived = 0 AND id IN ({})", placeholders(1, items.len()));
        let params: Vec<libsql::Value> = items.iter().map(|(k, _)| libsql::Value::Text(k.clone())).collect();
        let mut rows = conn.query(&sql, params).await?;
        let mut v = Vec::new();
        while let Some(r) = rows.next().await? {
            v.push(r.get::<String>(0)?);
        }
        v
    };
    let tx = conn.transaction().await?;
    tx.execute("DELETE FROM client_cart WHERE client_id = ?1", libsql::params![me.id.clone()]).await?;
    let at = now();
    for (pid, qty) in items.iter().filter(|(k, _)| known.contains(k)) {
        tx.execute(
            "INSERT INTO client_cart (client_id, product_id, qty, updated_at) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![me.id.clone(), pid.clone(), *qty, at.clone()],
        )
        .await?;
    }
    tx.commit().await?;
    cart(me, State(db)).await
}
