use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};

use super::{collect, first, new_id, now};
use crate::{
    db::Db,
    error::{ApiError, ApiResult},
    models::{
        CreateOrder, OrderDetail, OrderItemRow, OrderItemView, OrderQuery, OrderRow, OrderView,
        UpdateOrderStatus,
    },
    num::{MILLI, parse_milli},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(list_orders).post(create_order))
        .route("/{id}", get(get_order).patch(update_status))
}

const STATUSES: [&str; 6] = [
    "new",
    "confirmed",
    "assembling",
    "ready",
    "delivered",
    "cancelled",
];

const ORDER_COLS: &str = "id, client_id, client_name, client_phone, status, total_tiyn,
     note, source, assigned_to, cancelled_reason, created_at, updated_at";

const ITEM_COLS: &str =
    "id, product_id, product_name, product_sku, unit, price_tiyn, quantity_milli, subtotal_tiyn";

async fn list_orders(
    State(db): State<Db>,
    Query(q): Query<OrderQuery>,
) -> ApiResult<Json<Vec<OrderView>>> {
    let conn = db.connect()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let rows = conn
        .query(
            &format!(
                "SELECT {ORDER_COLS} FROM orders
                 WHERE (?1 IS NULL OR status = ?1)
                 ORDER BY created_at DESC
                 LIMIT ?2 OFFSET ?3"
            ),
            libsql::params![q.status, limit, offset],
        )
        .await?;

    let orders: Vec<OrderRow> = collect(rows).await?;
    Ok(Json(orders.into_iter().map(OrderView::from).collect()))
}

async fn get_order(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<OrderDetail>> {
    let conn = db.connect()?;

    let rows = conn
        .query(
            &format!("SELECT {ORDER_COLS} FROM orders WHERE id = ?1"),
            libsql::params![id.clone()],
        )
        .await?;
    let order: OrderRow = first(rows).await?.ok_or(ApiError::NotFound)?;

    let rows = conn
        .query(
            &format!("SELECT {ITEM_COLS} FROM order_items WHERE order_id = ?1 ORDER BY created_at"),
            libsql::params![id],
        )
        .await?;
    let items: Vec<OrderItemRow> = collect(rows).await?;

    Ok(Json(OrderDetail {
        order: OrderView::from(order),
        items: items.into_iter().map(OrderItemView::from).collect(),
    }))
}

/// Резервирует товар под заказ. Либо проходит весь заказ целиком, либо
/// не меняется ничего: при нехватке любой позиции транзакция откатывается,
/// поэтому два одновременных заказа не уведут остаток в минус.
async fn create_order(
    State(db): State<Db>,
    Json(body): Json<CreateOrder>,
) -> ApiResult<Json<OrderDetail>> {
    if body.items.is_empty() {
        return Err(ApiError::BadRequest("заказ без позиций".into()));
    }
    if body.client_name.trim().is_empty() {
        return Err(ApiError::BadRequest("имя клиента обязательно".into()));
    }
    if let Some(src) = &body.source
        && !["site", "whatsapp", "phone", "other"].contains(&src.as_str())
    {
        return Err(ApiError::BadRequest(format!("неизвестный источник: {src}")));
    }

    let conn = db.connect()?;
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;

    let order_id = new_id();
    let ts = now();

    tx.execute(
        "INSERT INTO orders (id, client_name, client_phone, source, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        libsql::params![
            order_id.clone(),
            body.client_name.trim(),
            body.client_phone.clone(),
            body.source.clone(),
            body.note.clone(),
            ts.clone()
        ],
    )
    .await?;

    let mut total: i64 = 0;
    let mut saved: Vec<OrderItemView> = Vec::new();

    for item in &body.items {
        let qty = parse_milli(&item.quantity)
            .map_err(|e| ApiError::BadRequest(format!("количество: {e}")))?;
        if qty <= 0 {
            return Err(ApiError::BadRequest("количество должно быть больше нуля".into()));
        }

        let rows = tx
            .query(
                "SELECT name, sku, unit, price_tiyn, stock_milli, reserved_milli, is_archived
                 FROM products WHERE id = ?1",
                libsql::params![item.product_id.clone()],
            )
            .await?;
        let Some(row) = ({
            let mut rows = rows;
            rows.next().await?
        }) else {
            return Err(ApiError::BadRequest(format!(
                "товар {} не найден",
                item.product_id
            )));
        };

        let name: String = row.get(0)?;
        let sku: Option<String> = row.get(1)?;
        let unit: String = row.get(2)?;
        let price: i64 = row.get(3)?;
        let stock: i64 = row.get(4)?;
        let reserved: i64 = row.get(5)?;
        let archived: i64 = row.get(6)?;

        if archived != 0 {
            return Err(ApiError::BadRequest(format!("товар «{name}» снят с продажи")));
        }

        let available = stock - reserved;
        if available < qty {
            return Err(ApiError::BadRequest(format!(
                "«{name}»: доступно {}, запрошено {}",
                crate::num::milli_to_string(available.max(0)),
                item.quantity
            )));
        }

        let subtotal = price * qty / MILLI;
        total += subtotal;

        tx.execute(
            "UPDATE products SET reserved_milli = reserved_milli + ?1, updated_at = ?2
             WHERE id = ?3",
            libsql::params![qty, ts.clone(), item.product_id.clone()],
        )
        .await?;

        tx.execute(
            "INSERT INTO stock_movements
             (id, product_id, movement_type, quantity_milli, reason, order_id, created_at)
             VALUES (?1, ?2, 'reserve', ?3, 'резерв под заказ', ?4, ?5)",
            libsql::params![
                new_id(),
                item.product_id.clone(),
                qty,
                order_id.clone(),
                ts.clone()
            ],
        )
        .await?;

        let item_id = new_id();
        tx.execute(
            "INSERT INTO order_items
             (id, order_id, product_id, product_name, product_sku, unit,
              price_tiyn, quantity_milli, subtotal_tiyn, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            libsql::params![
                item_id.clone(),
                order_id.clone(),
                item.product_id.clone(),
                name.clone(),
                sku.clone(),
                unit.clone(),
                price,
                qty,
                subtotal,
                ts.clone()
            ],
        )
        .await?;

        saved.push(OrderItemView::from(OrderItemRow {
            id: item_id,
            product_id: Some(item.product_id.clone()),
            product_name: name,
            product_sku: sku,
            unit,
            price_tiyn: price,
            quantity_milli: qty,
            subtotal_tiyn: subtotal,
        }));
    }

    tx.execute(
        "UPDATE orders SET total_tiyn = ?1 WHERE id = ?2",
        libsql::params![total, order_id.clone()],
    )
    .await?;

    tx.commit().await?;

    let rows = conn
        .query(
            &format!("SELECT {ORDER_COLS} FROM orders WHERE id = ?1"),
            libsql::params![order_id],
        )
        .await?;
    let order: OrderRow = first(rows).await?.ok_or(ApiError::NotFound)?;

    Ok(Json(OrderDetail {
        order: OrderView::from(order),
        items: saved,
    }))
}

/// Отмена снимает резерв. Выдача списывает остаток и снимает резерв.
/// Повторный перевод в тот же статус ничего не делает со складом.
async fn update_status(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<UpdateOrderStatus>,
) -> ApiResult<Json<OrderView>> {
    if !STATUSES.contains(&body.status.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "неизвестный статус: {}",
            body.status
        )));
    }

    let conn = db.connect()?;
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;

    let rows = tx
        .query(
            "SELECT status FROM orders WHERE id = ?1",
            libsql::params![id.clone()],
        )
        .await?;
    let Some(row) = ({
        let mut rows = rows;
        rows.next().await?
    }) else {
        return Err(ApiError::NotFound);
    };
    let current: String = row.get(0)?;

    let ts = now();

    if body.status == "cancelled" && current != "cancelled" && current != "delivered" {
        tx.execute(
            "UPDATE products SET reserved_milli = reserved_milli - (
                 SELECT IFNULL(SUM(quantity_milli), 0) FROM order_items
                 WHERE order_id = ?1 AND product_id = products.id
             ), updated_at = ?2
             WHERE id IN (SELECT product_id FROM order_items WHERE order_id = ?1)",
            libsql::params![id.clone(), ts.clone()],
        )
        .await?;

        tx.execute(
            "INSERT INTO stock_movements
             (id, product_id, movement_type, quantity_milli, reason, order_id, created_at)
             SELECT lower(hex(randomblob(16))), product_id, 'unreserve',
                    -quantity_milli, ?1, order_id, ?2
             FROM order_items WHERE order_id = ?3",
            libsql::params![
                format!(
                    "отмена заказа: {}",
                    body.cancelled_reason.as_deref().unwrap_or("причина не указана")
                ),
                ts.clone(),
                id.clone()
            ],
        )
        .await?;
    }

    if body.status == "delivered" && current != "delivered" {
        tx.execute(
            "UPDATE products SET
                 stock_milli = stock_milli - (
                     SELECT IFNULL(SUM(quantity_milli), 0) FROM order_items
                     WHERE order_id = ?1 AND product_id = products.id
                 ),
                 reserved_milli = reserved_milli - (
                     SELECT IFNULL(SUM(quantity_milli), 0) FROM order_items
                     WHERE order_id = ?1 AND product_id = products.id
                 ),
                 updated_at = ?2
             WHERE id IN (SELECT product_id FROM order_items WHERE order_id = ?1)",
            libsql::params![id.clone(), ts.clone()],
        )
        .await?;

        tx.execute(
            "INSERT INTO stock_movements
             (id, product_id, movement_type, quantity_milli, reason, order_id, created_at)
             SELECT lower(hex(randomblob(16))), product_id, 'out',
                    -quantity_milli, 'выдача заказа', order_id, ?1
             FROM order_items WHERE order_id = ?2",
            libsql::params![ts.clone(), id.clone()],
        )
        .await?;
    }

    tx.execute(
        "UPDATE orders SET status = ?1,
                           cancelled_reason = ?2,
                           assigned_to = IFNULL(?3, assigned_to),
                           updated_at = ?4
         WHERE id = ?5",
        libsql::params![
            body.status.clone(),
            body.cancelled_reason.clone(),
            body.assigned_to.clone(),
            ts,
            id.clone()
        ],
    )
    .await?;

    tx.commit().await?;

    let rows = conn
        .query(
            &format!("SELECT {ORDER_COLS} FROM orders WHERE id = ?1"),
            libsql::params![id],
        )
        .await?;
    let order: OrderRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(OrderView::from(order)))
}
