use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};

use super::{collect, first, new_id, now};
use crate::{
    db::Db,
    error::{ApiError, ApiResult},
    models::{
        ConfirmPayment, CreateOrder, OrderDetail, OrderItemRow, OrderItemView, OrderQuery,
        OrderRow, OrderView, UpdateOrderStatus,
    },
    num::{MILLI, parse_milli, parse_tiyn},
    pricing::effective_price,
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(list_orders).post(create_order))
        .route("/{id}", get(get_order).patch(update_status))
        .route("/{id}/payment", post(confirm_payment))
}

const STATUSES: [&str; 8] = [
    "new",
    "confirmed",
    "assembled",
    "handed_over",
    "shipped",
    "delivered",
    "completed",
    "cancelled",
];

/// Статусы, на которых товар физически покидает склад. По условиям магазина
/// списание идёт «при выдаче или передаче в доставку», то есть раньше,
/// чем заказ доедет до клиента.
const SHIPPED_STATUSES: &[&str] = &["handed_over", "shipped", "delivered", "completed"];

const DELIVERY_METHODS: &[&str] = &["pickup", "courier", "cdek", "kazpost"];
/// Онлайн-оплаты в магазине нет: заказчик отказался от интеграции (26:01),
/// остаются наличные и перевод Kaspi по статическому QR.
const PAYMENT_METHODS: &[&str] = &["none", "cash", "kaspi_transfer", "online"];
const PAYMENT_STATUSES: &[&str] = &["unpaid", "paid", "refunded"];

const ORDER_COLS: &str = "id, order_no, client_id, client_name, client_phone, status,
     items_tiyn, delivery_tiyn, total_tiyn, paid_tiyn, channel, city, delivery_method,
     delivery_address, payment_method, payment_status,
     note, source, assigned_to, cancelled_reason, created_at, updated_at";

const ITEM_COLS: &str = "id, product_id, product_name, product_sku, unit, price_tiyn,
     quantity_milli, discount_percent, subtotal_tiyn";

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
        && !["site", "whatsapp", "instagram", "phone", "other"].contains(&src.as_str())
    {
        return Err(ApiError::BadRequest(format!("неизвестный источник: {src}")));
    }

    let delivery_method = body.delivery_method.as_deref().unwrap_or("pickup").to_string();
    if !DELIVERY_METHODS.contains(&delivery_method.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "неизвестный способ получения: {delivery_method}"
        )));
    }
    // Доставка без адреса уедет в никуда. У заказчика в таблице такие заказы
    // помечены «Уточнить у клиента», и менеджер обязан добрать адрес до
    // отправки — на сайте же это обычное обязательное поле (19:19).
    if delivery_method != "pickup"
        && body
            .delivery_address
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        return Err(ApiError::BadRequest(
            "для доставки нужен адрес или пункт выдачи".into(),
        ));
    }
    if body.client_phone.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(ApiError::BadRequest("телефон обязателен".into()));
    }

    let payment_method = body.payment_method.as_deref().unwrap_or("none").to_string();
    if !PAYMENT_METHODS.contains(&payment_method.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "неизвестный способ оплаты: {payment_method}"
        )));
    }
    if payment_method == "online" {
        return Err(ApiError::BadRequest(
            "онлайн-оплаты в магазине нет: наличные или перевод Kaspi".into(),
        ));
    }

    let delivery_tiyn = match body.delivery.as_deref() {
        Some(v) => parse_tiyn(v).map_err(|e| ApiError::BadRequest(format!("доставка: {e}")))?,
        None => 0,
    };
    if delivery_tiyn < 0 {
        return Err(ApiError::BadRequest("доставка не может быть отрицательной".into()));
    }

    let conn = db.connect()?;
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;

    if let Some(cid) = &body.client_id {
        let mut rows = tx
            .query("SELECT 1 FROM clients WHERE id = ?1", libsql::params![cid.clone()])
            .await?;
        if rows.next().await?.is_none() {
            return Err(ApiError::BadRequest(format!("клиент {cid} не найден")));
        }
    }

    let order_id = new_id();
    let ts = now();

    tx.execute(
        "INSERT INTO orders (id, client_id, client_name, client_phone, source, channel,
                             city, delivery_method, delivery_address, delivery_tiyn,
                             payment_method, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, IFNULL(?5, 'site'), ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        libsql::params![
            order_id.clone(),
            body.client_id.clone(),
            body.client_name.trim(),
            body.client_phone.clone(),
            body.source.clone(),
            body.city.clone(),
            delivery_method.clone(),
            body.delivery_address.clone(),
            delivery_tiyn,
            payment_method.clone(),
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
                "SELECT p.name, p.sku, p.unit, p.price_tiyn, p.stock_milli, p.reserved_milli,
                        p.is_archived, p.min_order_milli, p.order_step_milli,
                        p.wholesale_price_tiyn, p.wholesale_min_qty,
                        (SELECT cp.price_tiyn FROM client_prices cp
                          WHERE cp.product_id = p.id AND cp.client_id = ?2)
                 FROM products p WHERE p.id = ?1",
                libsql::params![item.product_id.clone(), body.client_id.clone()],
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
        let min_order: i64 = row.get(7)?;
        let step: i64 = row.get(8)?;
        let wholesale: Option<i64> = row.get(9)?;
        let wholesale_min: Option<i64> = row.get(10)?;
        let personal: Option<i64> = row.get(11)?;

        if archived != 0 {
            return Err(ApiError::BadRequest(format!("товар «{name}» снят с продажи")));
        }

        if qty < min_order {
            return Err(ApiError::BadRequest(format!(
                "«{name}»: минимальный заказ {} {unit}",
                crate::num::milli_to_string(min_order)
            )));
        }
        if step > 0 && (qty - min_order) % step != 0 {
            return Err(ApiError::BadRequest(format!(
                "«{name}»: количество задаётся шагом {} {unit}, начиная с {}",
                crate::num::milli_to_string(step),
                crate::num::milli_to_string(min_order)
            )));
        }

        let available = stock - reserved;
        if available < qty {
            return Err(ApiError::BadRequest(format!(
                "«{name}»: доступно {}, запрошено {}",
                crate::num::milli_to_string(available.max(0)),
                item.quantity
            )));
        }

        // Опт и персональная цена подставляются здесь, а не на фронте:
        // цену заказа считает тот, кто отвечает за деньги.
        let (price, price_kind) = effective_price(price, wholesale, wholesale_min, personal, qty);

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

        let mut view = OrderItemView::from(OrderItemRow {
            id: item_id,
            product_id: Some(item.product_id.clone()),
            product_name: name,
            product_sku: sku,
            unit,
            price_tiyn: price,
            quantity_milli: qty,
            discount_percent: 0,
            subtotal_tiyn: subtotal,
        });
        view.price_kind = Some(price_kind.as_str().to_string());
        saved.push(view);
    }

    tx.execute(
        "UPDATE orders SET items_tiyn = ?1, total_tiyn = ?1 + delivery_tiyn WHERE id = ?2",
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

    // Повторное движение по складу не делаем: признак — уже записанное списание.
    let written_off = {
        let mut rows = tx
            .query(
                "SELECT 1 FROM stock_movements
                 WHERE order_id = ?1 AND movement_type = 'out' LIMIT 1",
                libsql::params![id.clone()],
            )
            .await?;
        rows.next().await?.is_some()
    };

    if body.status == "cancelled" && current != "cancelled" && !written_off {
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

    if SHIPPED_STATUSES.contains(&body.status.as_str())
        && !SHIPPED_STATUSES.contains(&current.as_str())
        && !written_off
    {
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
                    -quantity_milli, 'выдача или передача в доставку', order_id, ?1
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


/// Подтверждение оплаты. Отдельный эндпоинт, потому что оплата у заказчика
/// живёт вне системы: QR статический, уведомления о платеже не приходит,
/// факт денег проставляет менеджер руками.
async fn confirm_payment(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<ConfirmPayment>,
) -> ApiResult<Json<OrderView>> {
    if !PAYMENT_STATUSES.contains(&body.payment_status.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "неизвестный статус оплаты: {}",
            body.payment_status
        )));
    }
    if let Some(m) = &body.payment_method {
        if !PAYMENT_METHODS.contains(&m.as_str()) {
            return Err(ApiError::BadRequest(format!("неизвестный способ оплаты: {m}")));
        }
        if m == "online" {
            return Err(ApiError::BadRequest(
                "онлайн-оплаты в магазине нет: наличные или перевод Kaspi".into(),
            ));
        }
    }

    let conn = db.connect()?;
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;

    let rows = tx
        .query(
            "SELECT total_tiyn, payment_status FROM orders WHERE id = ?1",
            libsql::params![id.clone()],
        )
        .await?;
    let Some(row) = ({
        let mut rows = rows;
        rows.next().await?
    }) else {
        return Err(ApiError::NotFound);
    };
    let total: i64 = row.get(0)?;
    let current: String = row.get(1)?;

    // Возврат — это возврат чего-то полученного. Если денег не было,
    // возвращать нечего, и такой заказ просто отменяют.
    if body.payment_status == "refunded" && current != "paid" {
        return Err(ApiError::BadRequest(
            "возврат возможен только по оплаченному заказу".into(),
        ));
    }

    let paid = match (body.paid.as_deref(), body.payment_status.as_str()) {
        (Some(v), _) => parse_tiyn(v).map_err(|e| ApiError::BadRequest(format!("оплачено: {e}")))?,
        (None, "paid") => total,
        (None, _) => 0,
    };
    if paid < 0 {
        return Err(ApiError::BadRequest("оплачено не может быть отрицательным".into()));
    }
    if paid > total {
        return Err(ApiError::BadRequest(format!(
            "оплачено {} больше итога заказа {}",
            crate::num::tiyn_to_string(paid),
            crate::num::tiyn_to_string(total)
        )));
    }

    tx.execute(
        "UPDATE orders SET payment_status = ?1,
                           payment_method = IFNULL(?2, payment_method),
                           paid_tiyn = ?3,
                           updated_at = ?4
         WHERE id = ?5",
        libsql::params![
            body.payment_status.clone(),
            body.payment_method.clone(),
            paid,
            now(),
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
