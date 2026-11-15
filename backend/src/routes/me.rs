//! Профиль покупателя: свои заказы, сколько заказов, сколько выкуплено
//! (ФТ-ЛК-01). Только для вошедшего покупателя.

use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;

use super::{
    clients::{ClientView, load_client},
    collect,
};
use crate::{
    auth::Customer,
    db::Db,
    error::{ApiResult, bad},
    models::{ORDER_COLS, ORDER_FROM, OrderRow, OrderView},
};

pub fn router() -> Router<Db> {
    Router::new().route("/", get(profile).patch(update_profile)).route("/orders", get(my_orders))
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
