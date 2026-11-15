//! Заказы: оформление, жизненный цикл, сборка, оплата, журнал.
//!
//! Склад и деньги меняются только здесь и только в транзакции: либо
//! проходит всё действие, либо не меняется ничего.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};

use super::{collect, first, new_id, now, phone_norm};
use crate::{
    auth::{ANY_STAFF, ActorRef, MaybeActor, OFFICE, Role, Staff},
    db::Db,
    error::{ApiError, ApiResult, bad},
    models::{
        ConfirmPayment, CreateOrder, EventRow, EventView, ITEM_COLS, ITEM_FROM, ORDER_COLS,
        ORDER_FROM, OrderBrief, OrderDetail, OrderItemRow, OrderItemView, OrderQuery, OrderRow,
        OrderView, UpdateOrder,
    },
    num::{MILLI, milli_to_string, parse_milli, parse_tiyn, tiyn_to_string},
    pricing::effective_price,
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/", get(list_orders).post(create_order))
        .route("/track", get(track_order))
        .route("/{id}", get(get_order).patch(update_order))
        .route("/{id}/payment", post(confirm_payment))
        .route("/{id}/events", get(list_events))
        .route("/{id}/comments", post(add_comment))
        .route("/{id}/items", post(add_item))
        .route("/{id}/items/{item_id}", patch(update_item).delete(remove_item))
        .route("/{id}/items/{item_id}/pick", patch(pick_item))
        .route("/{id}/invoice", get(super::invoice::invoice))
}

// ─── Статусы ─────────────────────────────────────────────────────────────────

pub const STATUSES: [&str; 8] = [
    "new", "confirmed", "assembled", "handed_over", "shipped", "delivered", "completed", "cancelled",
];

pub fn status_title(s: &str) -> &'static str {
    match s {
        "new" => "Новый",
        "confirmed" => "Подтверждён",
        "assembled" => "Собран",
        "handed_over" => "Передан в доставку",
        "shipped" => "Отправлен",
        "delivered" => "Доставлен",
        "completed" => "Завершён",
        "cancelled" => "Отменён",
        _ => "—",
    }
}

/// Куда можно перейти из статуса (ФТ-З-03). Перескочить из «Новый» в
/// «Завершён» нельзя; после передачи в доставку отменить нельзя — товар уже
/// списан со склада. Самовывоз: «Передан» (выдан клиенту) → «Завершён».
fn next_statuses(from: &str) -> &'static [&'static str] {
    match from {
        "new" => &["confirmed", "cancelled"],
        "confirmed" => &["assembled", "cancelled"],
        "assembled" => &["handed_over", "cancelled"],
        "handed_over" => &["shipped", "delivered", "completed"],
        "shipped" => &["delivered", "completed"],
        "delivered" => &["completed"],
        _ => &[],
    }
}

/// Кто может перевести заказ в статус. Кладовщик отмечает сборку и выдачу.
fn roles_for(to: &str) -> &'static [Role] {
    match to {
        "assembled" | "handed_over" => ANY_STAFF,
        _ => OFFICE,
    }
}

/// Состав можно менять, пока заказ не собран: при нехватке на сборке
/// менеджер правит его прямо в карточке (ФТ-М-07).
const EDITABLE: &[&str] = &["new", "confirmed"];
/// Доставку и адрес можно менять, пока товар не передан.
const BEFORE_HANDOVER: &[&str] = &["new", "confirmed", "assembled"];

const CHANNELS: &[&str] = &["site", "site_legacy", "whatsapp", "instagram", "phone", "other"];
const DELIVERY_METHODS: &[&str] = &["pickup", "courier", "cdek", "kazpost"];
/// Онлайн-оплаты нет (ФТ-П-01): наличные и Kaspi по статическому QR.
const PAYMENT_METHODS: &[&str] = &["none", "cash", "kaspi_transfer"];
const PAYMENT_STATUSES: &[&str] = &["unpaid", "partial", "paid", "refunded"];
const ADDRESS_UNKNOWN: &str = "Уточнить у клиента";
const SHORTAGE_PREFIX: &str = "Не хватает: ";

// ─── Загрузка ────────────────────────────────────────────────────────────────

pub async fn load_order(conn: &libsql::Connection, id: &str) -> ApiResult<OrderRow> {
    let rows = conn
        .query(&format!("SELECT {ORDER_COLS} FROM {ORDER_FROM} WHERE o.id = ?1"), libsql::params![id])
        .await?;
    first(rows).await?.ok_or(ApiError::NotFound)
}

pub async fn load_items(conn: &libsql::Connection, id: &str) -> ApiResult<Vec<OrderItemRow>> {
    let rows = conn
        .query(
            &format!("SELECT {ITEM_COLS} FROM {ITEM_FROM} WHERE i.order_id = ?1 ORDER BY i.created_at, i.id"),
            libsql::params![id],
        )
        .await?;
    collect(rows).await
}

async fn load_events(conn: &libsql::Connection, id: &str, with_money: bool) -> ApiResult<Vec<EventView>> {
    let rows = conn
        .query(
            "SELECT id, kind, message, amount_tiyn, actor_kind, actor_name, created_at
             FROM order_events WHERE order_id = ?1 AND (?2 = 1 OR kind <> 'payment')
             ORDER BY created_at, rowid",
            libsql::params![id, with_money as i64],
        )
        .await?;
    Ok(collect::<EventRow>(rows).await?.into_iter().map(EventView::from).collect())
}

async fn client_orders(conn: &libsql::Connection, client_id: &str, except: &str) -> ApiResult<Vec<OrderBrief>> {
    let mut rows = conn
        .query(
            "SELECT id, order_no, status, total_tiyn, created_at FROM orders
             WHERE client_id = ?1 AND id <> ?2 ORDER BY created_at DESC LIMIT 20",
            libsql::params![client_id, except],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(OrderBrief {
            id: r.get(0)?,
            order_no: r.get(1)?,
            status: r.get(2)?,
            total: tiyn_to_string(r.get(3)?),
            created_at: r.get(4)?,
        });
    }
    Ok(out)
}

fn view_for(staff: Option<&Staff>, row: OrderRow) -> OrderView {
    let v = OrderView::from(row);
    if staff.is_some_and(|s| s.is(Role::Warehouse)) { v.without_payment() } else { v }
}

async fn detail(conn: &libsql::Connection, id: &str, staff: Option<&Staff>) -> ApiResult<OrderDetail> {
    let row = load_order(conn, id).await?;
    let client_id = row.client_id.clone();
    let items = load_items(conn, id).await?.into_iter().map(OrderItemView::from).collect();
    let (events, history) = match staff {
        Some(s) => {
            let money = !s.is(Role::Warehouse);
            let events = Some(load_events(conn, id, money).await?);
            let history = match (&client_id, money) {
                (Some(c), true) => Some(client_orders(conn, c, id).await?),
                _ => None,
            };
            (events, history)
        }
        None => (None, None),
    };
    Ok(OrderDetail { order: view_for(staff, row), items, events, client_orders: history })
}

// ─── Журнал ──────────────────────────────────────────────────────────────────

pub async fn log_event(
    conn: &libsql::Connection,
    order_id: &str,
    actor: &ActorRef,
    kind: &str,
    message: &str,
    amount: Option<i64>,
    ts: &str,
) -> ApiResult<()> {
    conn.execute(
        "INSERT INTO order_events (id, order_id, kind, message, amount_tiyn, actor_kind,
                                   actor_id, actor_name, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            new_id(),
            order_id,
            kind,
            message,
            amount,
            actor.kind,
            actor.id.clone(),
            actor.name.clone(),
            ts
        ],
    )
    .await?;
    Ok(())
}

// ─── Расчёты ─────────────────────────────────────────────────────────────────

/// Скидка в процентах по строке ставится менеджером (ФТ-Ц-08).
pub fn line_subtotal(price: i64, qty: i64, discount: i64) -> i64 {
    price * qty / MILLI * (100 - discount) / 100
}

fn check_qty(name: &str, unit: &str, qty: i64, min: i64, step: i64) -> ApiResult<()> {
    if qty <= 0 {
        return Err(bad("количество должно быть больше нуля"));
    }
    if qty < min {
        return Err(bad(format!("«{name}»: минимальный заказ {} {unit}", milli_to_string(min))));
    }
    if step > 0 && (qty - min) % step != 0 {
        return Err(bad(format!(
            "«{name}»: количество задаётся шагом {} {unit}, начиная с {}",
            milli_to_string(step),
            milli_to_string(min)
        )));
    }
    Ok(())
}

fn parse_qty(s: &str) -> ApiResult<i64> {
    parse_milli(s).map_err(|e| bad(format!("количество: {e}")))
}

async fn recalc_totals(conn: &libsql::Connection, order_id: &str, ts: &str) -> ApiResult<()> {
    conn.execute(
        "UPDATE orders SET
            items_tiyn = (SELECT IFNULL(SUM(subtotal_tiyn), 0) FROM order_items WHERE order_id = ?1),
            total_tiyn = (SELECT IFNULL(SUM(subtotal_tiyn), 0) FROM order_items WHERE order_id = ?1)
                         + delivery_tiyn,
            updated_at = ?2
         WHERE id = ?1",
        libsql::params![order_id, ts],
    )
    .await?;
    Ok(())
}

/// Признак «проблемный» по нехватке ставится и снимается сам: как только
/// менеджер поправил состав, проблема по сборке закрыта. Проблему,
/// поставленную вручную, снимает только человек.
async fn sync_shortage(conn: &libsql::Connection, order_id: &str, ts: &str) -> ApiResult<()> {
    let mut rows = conn
        .query(
            "SELECT product_name, short_milli, unit FROM order_items
             WHERE order_id = ?1 AND short_milli > 0 ORDER BY created_at",
            libsql::params![order_id],
        )
        .await?;
    let mut parts = Vec::new();
    while let Some(r) = rows.next().await? {
        let (name, short, unit): (String, i64, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        parts.push(format!("{name} — {} {unit}", milli_to_string(short)));
    }
    if parts.is_empty() {
        conn.execute(
            "UPDATE orders SET is_problem = 0, problem_note = NULL, updated_at = ?2
             WHERE id = ?1 AND problem_note LIKE 'Не хватает: %'",
            libsql::params![order_id, ts],
        )
        .await?;
    } else {
        conn.execute(
            "UPDATE orders SET is_problem = 1, problem_note = ?2, updated_at = ?3 WHERE id = ?1",
            libsql::params![order_id, format!("{SHORTAGE_PREFIX}{}", parts.join("; ")), ts],
        )
        .await?;
    }
    Ok(())
}

async fn next_order_no(conn: &libsql::Connection) -> ApiResult<String> {
    let mut rows = conn
        .query(
            "SELECT IFNULL(MAX(CAST(order_no AS INTEGER)), 1000) + 1 FROM orders
             WHERE order_no GLOB '[0-9]*'",
            (),
        )
        .await?;
    let n: i64 = rows.next().await?.map(|r| r.get(0)).transpose()?.unwrap_or(1001);
    Ok(n.to_string())
}

struct Priced {
    name: String,
    sku: Option<String>,
    unit: String,
    price: i64,
    kind: &'static str,
}

/// Проверяет товар и количество и считает цену строки. Цену заказа
/// считает сервер, а не сайт (ФТ-Ц-04).
async fn price_line(
    conn: &libsql::Connection,
    product_id: &str,
    qty: i64,
    price_client: Option<&str>,
    extra_reserved: i64,
) -> ApiResult<Priced> {
    let mut rows = conn
        .query(
            "SELECT p.name, p.sku, p.unit, p.price_tiyn, p.stock_milli, p.reserved_milli,
                    p.is_archived, p.min_order_milli, p.order_step_milli,
                    p.wholesale_price_tiyn, p.wholesale_min_qty, p.variant_name,
                    (SELECT cp.price_tiyn FROM client_prices cp
                      WHERE cp.product_id = p.id AND cp.client_id = ?2)
             FROM products p WHERE p.id = ?1",
            libsql::params![product_id, price_client],
        )
        .await?;
    let Some(r) = rows.next().await? else {
        return Err(bad(format!("товар {product_id} не найден")));
    };
    let base: String = r.get(0)?;
    let variant: Option<String> = r.get(11)?;
    let name = match variant {
        Some(v) if !v.is_empty() => format!("{base}, {v}"),
        _ => base,
    };
    let unit: String = r.get(2)?;
    if r.get::<i64>(6)? != 0 {
        return Err(bad(format!("товар «{name}» снят с продажи")));
    }
    check_qty(&name, &unit, qty, r.get(7)?, r.get(8)?)?;
    let available = r.get::<i64>(4)? - r.get::<i64>(5)? - extra_reserved;
    if available < qty {
        return Err(bad(format!(
            "«{name}»: доступно {}, запрошено {}",
            milli_to_string(available.max(0)),
            milli_to_string(qty)
        )));
    }
    let wholesale_on = crate::config::get().wholesale_enabled;
    let (wholesale, wmin) = if wholesale_on { (r.get(9)?, r.get(10)?) } else { (None, None) };
    let (price, kind) = effective_price(r.get(3)?, wholesale, wmin, r.get(12)?, qty);
    Ok(Priced { name, sku: r.get(1)?, unit, price, kind: kind.as_str() })
}

async fn reserve(conn: &libsql::Connection, product_id: &str, qty: i64, order_id: &str, ts: &str) -> ApiResult<()> {
    if qty == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE products SET reserved_milli = reserved_milli + ?1, updated_at = ?2 WHERE id = ?3",
        libsql::params![qty, ts, product_id],
    )
    .await?;
    let (kind, reason) = if qty > 0 { ("reserve", "резерв под заказ") } else { ("unreserve", "снятие резерва") };
    conn.execute(
        "INSERT INTO stock_movements (id, product_id, movement_type, quantity_milli, reason, order_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        libsql::params![new_id(), product_id, kind, qty, reason, order_id, ts],
    )
    .await?;
    Ok(())
}

// ─── Список и карточка ───────────────────────────────────────────────────────

async fn list_orders(
    staff: Staff,
    State(db): State<Db>,
    Query(q): Query<OrderQuery>,
) -> ApiResult<Json<Vec<OrderView>>> {
    staff.require(ANY_STAFF)?;
    if staff.is(Role::Warehouse) && q.payment_status.is_some() {
        return Err(ApiError::Forbidden("кладовщику фильтр по оплате недоступен".into()));
    }
    for (v, allowed, what) in [
        (&q.status, &STATUSES[..], "статус"),
        (&q.payment_status, PAYMENT_STATUSES, "статус оплаты"),
        (&q.channel, CHANNELS, "канал"),
    ] {
        if let Some(v) = v
            && !allowed.contains(&v.as_str())
        {
            return Err(bad(format!("{what}: неизвестное значение «{v}»")));
        }
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let like = search.map(|s| format!("%{s}%"));
    let digits = search.and_then(|s| {
        let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        (d.len() >= 4).then(|| format!("%{}%", &d[d.len().saturating_sub(10)..]))
    });

    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "SELECT {ORDER_COLS} FROM {ORDER_FROM}
                 WHERE (?1 IS NULL OR o.status = ?1)
                   AND (?2 IS NULL OR o.payment_status = ?2)
                   AND (?3 IS NULL OR o.channel = ?3)
                   AND (?4 IS NULL OR o.assigned_to = ?4)
                   AND (?5 IS NULL OR o.client_id = ?5)
                   AND (?6 IS NULL OR o.is_urgent = ?6)
                   AND (?7 IS NULL OR o.is_problem = ?7)
                   AND (?8 IS NULL OR o.order_no LIKE ?8 OR o.client_name LIKE ?8
                        OR (?9 IS NOT NULL AND REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(
                              IFNULL(o.client_phone, ''), ' ', ''), '-', ''), '(', ''), ')', ''), '+', '')
                            LIKE ?9))
                   AND (?10 IS NULL OR o.created_at >= ?10)
                   AND (?11 IS NULL OR o.created_at < date(?11, '+1 day'))
                 ORDER BY o.is_urgent DESC, o.created_at DESC
                 LIMIT ?12 OFFSET ?13"
            ),
            libsql::params![
                q.status.clone(),
                q.payment_status.clone(),
                q.channel.clone(),
                q.assigned_to.clone(),
                q.client_id.clone(),
                q.urgent.map(i64::from),
                q.problem.map(i64::from),
                like,
                digits,
                q.date_from.clone(),
                q.date_to.clone(),
                limit,
                offset
            ],
        )
        .await?;
    let orders: Vec<OrderRow> = collect(rows).await?;
    Ok(Json(orders.into_iter().map(|o| view_for(Some(&staff), o)).collect()))
}

/// Сотрудник видит любой заказ с журналом, покупатель — только свой.
async fn get_order(actor: MaybeActor, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<OrderDetail>> {
    let conn = db.connect()?;
    if let Some(s) = actor.staff() {
        return Ok(Json(detail(&conn, &id, Some(s)).await?));
    }
    let Some(c) = actor.customer() else {
        return Err(ApiError::Unauthorized("нужен вход".into()));
    };
    let row = load_order(&conn, &id).await?;
    if row.client_id.as_deref() != Some(c.id.as_str()) {
        return Err(ApiError::NotFound);
    }
    Ok(Json(detail(&conn, &id, None).await?))
}

// ─── Оформление ──────────────────────────────────────────────────────────────

/// Заказ с сайта, от вошедшего покупателя или от менеджера из мессенджера.
/// Резерв ставится сразу при оформлении (ТЗ, вопрос 3): либо проходит весь
/// заказ, либо не меняется ничего, поэтому два одновременных заказа не
/// продадут один остаток дважды (ФТ-З-06).
async fn create_order(actor: MaybeActor, State(db): State<Db>, Json(body): Json<CreateOrder>) -> ApiResult<Json<OrderDetail>> {
    if body.items.is_empty() {
        return Err(bad("заказ без позиций"));
    }
    let client_name = body.client_name.trim();
    if client_name.is_empty() {
        return Err(bad("имя клиента обязательно"));
    }
    let phone = body.client_phone.as_deref().map(str::trim).unwrap_or("");
    if phone.is_empty() {
        return Err(bad("телефон обязателен"));
    }
    let staff = actor.staff().cloned();
    if let Some(s) = &staff {
        s.require(OFFICE)?;
    }

    // Канал: с сайта — всегда site; другие каналы заводит менеджер (ФТ-М-04).
    let channel = match (&staff, body.source.as_deref()) {
        (Some(_), Some(src)) => src.to_string(),
        (Some(_), None) => "phone".to_string(),
        (None, _) => "site".to_string(),
    };
    if !CHANNELS.contains(&channel.as_str()) {
        return Err(bad(format!("неизвестный канал: {channel}")));
    }
    let delivery_method = body.delivery_method.as_deref().unwrap_or("pickup").to_string();
    if !DELIVERY_METHODS.contains(&delivery_method.as_str()) {
        return Err(bad(format!("неизвестный способ получения: {delivery_method}")));
    }
    let address = body.delivery_address.as_deref().map(str::trim).filter(|a| !a.is_empty());
    // С сайта адрес обязателен для доставки (ФТ-О-02). Менеджер может завести
    // заказ из WhatsApp без адреса — тогда «Уточнить у клиента» (ФТ-О-06).
    let address = match (address, delivery_method.as_str(), &staff) {
        (Some(a), _, _) => Some(a.to_string()),
        (None, "pickup", _) => None,
        (None, _, Some(_)) => Some(ADDRESS_UNKNOWN.to_string()),
        (None, _, None) => return Err(bad("для доставки нужен адрес или пункт выдачи")),
    };
    let payment_method = body.payment_method.as_deref().unwrap_or("none").to_string();
    if payment_method == "online" {
        return Err(bad("онлайн-оплаты в магазине нет: наличные или перевод Kaspi"));
    }
    if !PAYMENT_METHODS.contains(&payment_method.as_str()) {
        return Err(bad(format!("неизвестный способ оплаты: {payment_method}")));
    }
    let delivery_tiyn = match body.delivery.as_deref() {
        Some(v) => parse_tiyn(v).map_err(|e| bad(format!("доставка: {e}")))?,
        None => 0,
    };
    if staff.is_none() && (body.is_urgent.is_some() || body.assigned_to.is_some()) {
        return Err(ApiError::Forbidden("срочность и ответственного ставит менеджер".into()));
    }

    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let ts = now();
    let norm = phone_norm(phone);

    // Чьи персональные цены применить: покупателя по входу или клиента,
    // за которого заказ заводит менеджер. Гость с чужим телефоном договорных
    // цен не получает.
    let price_client: Option<String> = match (actor.customer(), &staff) {
        (Some(c), _) => Some(c.id.clone()),
        (None, Some(_)) => body.client_id.clone(),
        _ => None,
    };
    if let Some(cid) = &price_client {
        let mut r = tx.query("SELECT 1 FROM clients WHERE id = ?1", libsql::params![cid.clone()]).await?;
        if r.next().await?.is_none() {
            return Err(bad(format!("клиент {cid} не найден")));
        }
    }
    // Каждый заказ привязывается к карточке клиента по телефону: так CRM
    // помнит, что человек покупал, даже если он заказывал гостем (ФТ-КЛ-01).
    let link_client = match &price_client {
        Some(c) => c.clone(),
        None => {
            let found = match &norm {
                Some(n) => {
                    let mut r = tx
                        .query("SELECT id FROM clients WHERE phone_norm = ?1 LIMIT 1", libsql::params![n.clone()])
                        .await?;
                    r.next().await?.map(|r| r.get::<String>(0)).transpose()?
                }
                None => None,
            };
            match found {
                Some(id) => id,
                None => {
                    let id = new_id();
                    tx.execute(
                        "INSERT INTO clients (id, name, phone, phone_norm, city, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        libsql::params![id.clone(), client_name, phone, norm.clone(), body.city.clone(), ts.clone()],
                    )
                    .await?;
                    id
                }
            }
        }
    };

    if let Some(a) = &body.assigned_to {
        ensure_employee(&tx, a).await?;
    }
    let assigned = body
        .assigned_to
        .clone()
        .or_else(|| staff.as_ref().filter(|s| s.is(Role::Manager)).map(|s| s.id.clone()));

    let order_id = new_id();
    let order_no = next_order_no(&tx).await?;
    tx.execute(
        "INSERT INTO orders (id, order_no, client_id, client_name, client_phone, source, channel,
                             city, delivery_method, delivery_address, delivery_tiyn,
                             payment_method, note, is_urgent, assigned_to, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16)",
        libsql::params![
            order_id.clone(),
            order_no.clone(),
            link_client,
            client_name,
            phone,
            body.source.clone(),
            channel.clone(),
            body.city.clone(),
            delivery_method,
            address,
            delivery_tiyn,
            payment_method,
            body.note.clone(),
            body.is_urgent.unwrap_or(false) as i64,
            assigned,
            ts.clone()
        ],
    )
    .await?;

    // Одна и та же позиция дважды в заказе резервировалась бы с ошибкой.
    let mut taken: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for item in &body.items {
        let qty = parse_qty(&item.quantity)?;
        let already = taken.get(&item.product_id).copied().unwrap_or(0);
        let p = price_line(&tx, &item.product_id, qty, price_client.as_deref(), already).await?;
        *taken.entry(item.product_id.clone()).or_default() += qty;
        let subtotal = line_subtotal(p.price, qty, 0);
        reserve(&tx, &item.product_id, qty, &order_id, &ts).await?;
        tx.execute(
            "INSERT INTO order_items (id, order_id, product_id, product_name, product_sku, unit,
                                      price_tiyn, quantity_milli, subtotal_tiyn, price_kind, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            libsql::params![
                new_id(),
                order_id.clone(),
                item.product_id.clone(),
                p.name,
                p.sku,
                p.unit,
                p.price,
                qty,
                subtotal,
                p.kind,
                ts.clone()
            ],
        )
        .await?;
    }
    recalc_totals(&tx, &order_id, &ts).await?;
    let how = match channel.as_str() {
        "site" => "с сайта".to_string(),
        other => format!("менеджером, канал {other}"),
    };
    log_event(&tx, &order_id, &ActorRef::from(&actor), "created", &format!("Заказ №{order_no} оформлен {how}"), None, &ts).await?;
    tx.commit().await?;

    let d = detail(&conn, &order_id, staff.as_ref()).await?;
    crate::notify::order_created(&d);
    Ok(Json(d))
}

async fn ensure_employee(conn: &libsql::Connection, id: &str) -> ApiResult<String> {
    let mut r = conn
        .query("SELECT name FROM employees WHERE id = ?1 AND is_active = 1", libsql::params![id])
        .await?;
    match r.next().await? {
        Some(row) => Ok(row.get(0)?),
        None => Err(bad(format!("сотрудник {id} не найден"))),
    }
}

// ─── Изменение заказа ────────────────────────────────────────────────────────

async fn update_order(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(b): Json<UpdateOrder>,
) -> ApiResult<Json<OrderDetail>> {
    staff.require(ANY_STAFF)?;
    let office_only = b.assigned_to.is_some()
        || b.is_urgent.is_some()
        || b.is_problem.is_some()
        || b.problem_note.is_some()
        || b.note.is_some()
        || b.city.is_some()
        || b.delivery_method.is_some()
        || b.delivery_address.is_some()
        || b.delivery.is_some();
    if office_only {
        staff.require(OFFICE)?;
    }

    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let cur = load_order(&tx, &id).await?;
    let actor = ActorRef::from(&staff);
    let ts = now();

    if let Some(a) = &b.assigned_to {
        let (value, msg) = if a.trim().is_empty() {
            (None, "Ответственный снят".to_string())
        } else {
            let name = ensure_employee(&tx, a).await?;
            (Some(a.clone()), format!("Ответственный: {name}"))
        };
        tx.execute("UPDATE orders SET assigned_to = ?1, updated_at = ?2 WHERE id = ?3", libsql::params![value, ts.clone(), id.clone()]).await?;
        log_event(&tx, &id, &actor, "assigned", &msg, None, &ts).await?;
    }
    if let Some(u) = b.is_urgent
        && u != (cur.is_urgent != 0)
    {
        tx.execute("UPDATE orders SET is_urgent = ?1, updated_at = ?2 WHERE id = ?3", libsql::params![u as i64, ts.clone(), id.clone()]).await?;
        log_event(&tx, &id, &actor, "urgent", if u { "Отмечен срочным" } else { "Снята отметка «срочный»" }, None, &ts).await?;
    }
    if b.is_problem.is_some() || b.problem_note.is_some() {
        let flag = b.is_problem.unwrap_or(true);
        let note = if flag { b.problem_note.clone().or(cur.problem_note.clone()) } else { None };
        tx.execute(
            "UPDATE orders SET is_problem = ?1, problem_note = ?2, updated_at = ?3 WHERE id = ?4",
            libsql::params![flag as i64, note.clone(), ts.clone(), id.clone()],
        )
        .await?;
        let msg = if flag {
            format!("Проблема: {}", note.as_deref().unwrap_or("без описания"))
        } else {
            "Проблема решена".to_string()
        };
        log_event(&tx, &id, &actor, "problem", &msg, None, &ts).await?;
    }
    if let Some(n) = &b.note {
        tx.execute("UPDATE orders SET note = ?1, updated_at = ?2 WHERE id = ?3", libsql::params![n.clone(), ts.clone(), id.clone()]).await?;
    }
    if b.city.is_some() || b.delivery_method.is_some() || b.delivery_address.is_some() || b.delivery.is_some() {
        if !BEFORE_HANDOVER.contains(&cur.status.as_str()) {
            return Err(ApiError::Conflict("заказ уже передан — доставку менять поздно".into()));
        }
        let method = b.delivery_method.clone().unwrap_or(cur.delivery_method.clone());
        if !DELIVERY_METHODS.contains(&method.as_str()) {
            return Err(bad(format!("неизвестный способ получения: {method}")));
        }
        let address = b.delivery_address.clone().or(cur.delivery_address.clone());
        let delivery = match &b.delivery {
            Some(v) => parse_tiyn(v).map_err(|e| bad(format!("доставка: {e}")))?,
            None => cur.delivery_tiyn,
        };
        tx.execute(
            "UPDATE orders SET city = IFNULL(?1, city), delivery_method = ?2, delivery_address = ?3,
                               delivery_tiyn = ?4, updated_at = ?5 WHERE id = ?6",
            libsql::params![b.city.clone(), method.clone(), address.clone(), delivery, ts.clone(), id.clone()],
        )
        .await?;
        recalc_totals(&tx, &id, &ts).await?;
        let msg = format!(
            "Доставка: {method}{}, {} ₸",
            address.map(|a| format!(", {a}")).unwrap_or_default(),
            tiyn_to_string(delivery)
        );
        log_event(&tx, &id, &actor, "delivery", &msg, None, &ts).await?;
    }
    if let Some(to) = &b.status
        && to != &cur.status
    {
        let cur = load_order(&tx, &id).await?;
        change_status(&tx, &cur, to, b.cancelled_reason.clone(), &staff, &ts).await?;
    }
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

async fn change_status(
    tx: &libsql::Connection,
    cur: &OrderRow,
    to: &str,
    reason: Option<String>,
    staff: &Staff,
    ts: &str,
) -> ApiResult<()> {
    if !STATUSES.contains(&to) {
        return Err(bad(format!("неизвестный статус: {to}")));
    }
    if !next_statuses(&cur.status).contains(&to) {
        return Err(ApiError::Conflict(format!(
            "нельзя перевести заказ из «{}» в «{}»",
            status_title(&cur.status),
            status_title(to)
        )));
    }
    staff.require(roles_for(to))?;
    let id = cur.id.as_str();

    match to {
        "cancelled" => {
            // Деньги получены — сначала возврат (ФТ-П-05).
            if matches!(cur.payment_status.as_str(), "paid" | "partial") {
                return Err(ApiError::Conflict(
                    "заказ оплачен — сначала оформите возврат, затем отмену".into(),
                ));
            }
            let items = load_items(tx, id).await?;
            for i in items.iter().filter(|i| i.product_id.is_some()) {
                reserve(tx, i.product_id.as_deref().unwrap_or_default(), -i.quantity_milli, id, ts).await?;
            }
        }
        "assembled" => {
            let items = load_items(tx, id).await?;
            if items.iter().any(|i| i.short_milli > 0) {
                return Err(ApiError::Conflict("при сборке не хватило товара — сначала поправьте состав".into()));
            }
            if cur.is_problem != 0 {
                return Err(ApiError::Conflict("заказ отмечен проблемным — сначала решите проблему".into()));
            }
            tx.execute("UPDATE order_items SET picked = 1 WHERE order_id = ?1", libsql::params![id]).await?;
        }
        "handed_over" => {
            if cur.is_problem != 0 {
                return Err(ApiError::Conflict("заказ отмечен проблемным — сначала решите проблему".into()));
            }
            // Менеджер обязан получить адрес до отправки (ФТ-О-06).
            let addr = cur.delivery_address.as_deref().map(str::trim).unwrap_or("");
            if cur.delivery_method != "pickup" && (addr.is_empty() || addr == ADDRESS_UNKNOWN) {
                return Err(ApiError::Conflict("нужен адрес доставки — уточните его у клиента".into()));
            }
            // Списание товара и резерва при передаче (ФТ-З-05). Повтора не будет:
            // в handed_over заказ попадает один раз, назад переходов нет.
            let items = load_items(tx, id).await?;
            for i in &items {
                let Some(pid) = i.product_id.as_deref() else { continue };
                let mut r = tx
                    .query("SELECT stock_milli, name FROM products WHERE id = ?1", libsql::params![pid])
                    .await?;
                if let Some(row) = r.next().await? {
                    let stock: i64 = row.get(0)?;
                    if stock < i.quantity_milli {
                        let name: String = row.get(1)?;
                        return Err(ApiError::Conflict(format!(
                            "«{name}»: на складе {}, а в заказе {} — проведите приход или поправьте заказ",
                            milli_to_string(stock),
                            milli_to_string(i.quantity_milli)
                        )));
                    }
                }
                tx.execute(
                    "UPDATE products SET stock_milli = stock_milli - ?1,
                            reserved_milli = MAX(reserved_milli - ?1, 0), updated_at = ?2 WHERE id = ?3",
                    libsql::params![i.quantity_milli, ts, pid],
                )
                .await?;
                tx.execute(
                    "INSERT INTO stock_movements (id, product_id, movement_type, quantity_milli, reason,
                                                  order_id, employee_id, created_at)
                     VALUES (?1, ?2, 'out', ?3, 'выдача или передача в доставку', ?4, ?5, ?6)",
                    libsql::params![new_id(), pid, -i.quantity_milli, id, staff.id.clone(), ts],
                )
                .await?;
            }
        }
        _ => {}
    }

    tx.execute(
        "UPDATE orders SET status = ?1, cancelled_reason = CASE WHEN ?1 = 'cancelled' THEN ?2 ELSE cancelled_reason END,
                updated_at = ?3 WHERE id = ?4",
        libsql::params![to, reason.clone(), ts, id],
    )
    .await?;
    let mut msg = format!("Статус: {}", status_title(to));
    if let (Some(r), "cancelled") = (&reason, to) {
        msg.push_str(&format!(", причина: {r}"));
    }
    log_event(tx, id, &ActorRef::from(staff), "status", &msg, None, ts).await
}

// ─── Состав заказа ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AddItem {
    product_id: String,
    quantity: String,
}

#[derive(Deserialize)]
struct UpdateItem {
    quantity: Option<String>,
    discount_percent: Option<i64>,
}

async fn editable(tx: &libsql::Connection, id: &str) -> ApiResult<OrderRow> {
    let o = load_order(tx, id).await?;
    if !EDITABLE.contains(&o.status.as_str()) {
        return Err(ApiError::Conflict(format!(
            "заказ в статусе «{}» — состав уже не меняется",
            status_title(&o.status)
        )));
    }
    Ok(o)
}

async fn load_item(tx: &libsql::Connection, order_id: &str, item_id: &str) -> ApiResult<OrderItemRow> {
    let rows = tx
        .query(
            &format!("SELECT {ITEM_COLS} FROM {ITEM_FROM} WHERE i.order_id = ?1 AND i.id = ?2"),
            libsql::params![order_id, item_id],
        )
        .await?;
    first(rows).await?.ok_or(ApiError::NotFound)
}

async fn add_item(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<AddItem>) -> ApiResult<Json<OrderDetail>> {
    staff.require(OFFICE)?;
    let qty = parse_qty(&b.quantity)?;
    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let o = editable(&tx, &id).await?;
    let ts = now();
    let p = price_line(&tx, &b.product_id, qty, o.client_id.as_deref(), 0).await?;
    let subtotal = line_subtotal(p.price, qty, 0);
    reserve(&tx, &b.product_id, qty, &id, &ts).await?;
    tx.execute(
        "INSERT INTO order_items (id, order_id, product_id, product_name, product_sku, unit,
                                  price_tiyn, quantity_milli, subtotal_tiyn, price_kind, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        libsql::params![new_id(), id.clone(), b.product_id.clone(), p.name.clone(), p.sku, p.unit.clone(), p.price, qty, subtotal, p.kind, ts.clone()],
    )
    .await?;
    recalc_totals(&tx, &id, &ts).await?;
    let msg = format!("Добавлено: {} — {} {}", p.name, milli_to_string(qty), p.unit);
    log_event(&tx, &id, &ActorRef::from(&staff), "items", &msg, None, &ts).await?;
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

/// Правка строки: количество (например, годных оказалось меньше) и скидка.
/// Цена строки остаётся той, что была при оформлении (ФТ-Ц-05).
async fn update_item(
    staff: Staff,
    State(db): State<Db>,
    Path((id, item_id)): Path<(String, String)>,
    Json(b): Json<UpdateItem>,
) -> ApiResult<Json<OrderDetail>> {
    staff.require(OFFICE)?;
    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    editable(&tx, &id).await?;
    let item = load_item(&tx, &id, &item_id).await?;
    let ts = now();
    let mut qty = item.quantity_milli;
    let mut notes = Vec::new();

    if let Some(q) = &b.quantity {
        qty = parse_qty(q)?;
        let delta = qty - item.quantity_milli;
        if let Some(pid) = item.product_id.as_deref() {
            let mut r = tx
                .query(
                    "SELECT min_order_milli, order_step_milli, stock_milli - reserved_milli FROM products WHERE id = ?1",
                    libsql::params![pid],
                )
                .await?;
            if let Some(row) = r.next().await? {
                check_qty(&item.product_name, &item.unit, qty, row.get(0)?, row.get(1)?)?;
                let available: i64 = row.get(2)?;
                if delta > available {
                    return Err(bad(format!(
                        "«{}»: можно добавить ещё {}, запрошено {}",
                        item.product_name,
                        milli_to_string(available.max(0)),
                        milli_to_string(delta)
                    )));
                }
            }
            reserve(&tx, pid, delta, &id, &ts).await?;
        } else if qty <= 0 {
            return Err(bad("количество должно быть больше нуля"));
        } else if delta > 0 {
            return Err(bad("товара нет в каталоге — количество можно только уменьшить"));
        }
        // Нехватка по строке закрыта правкой. Если добавили — собрать заново.
        tx.execute(
            "UPDATE order_items SET quantity_milli = ?1, short_milli = 0,
                    picked = CASE WHEN ?2 > 0 THEN 0 ELSE picked END WHERE id = ?3",
            libsql::params![qty, delta, item_id.clone()],
        )
        .await?;
        notes.push(format!(
            "{}: {} → {} {}",
            item.product_name,
            milli_to_string(item.quantity_milli),
            milli_to_string(qty),
            item.unit
        ));
    }
    let discount = b.discount_percent.unwrap_or(item.discount_percent);
    if !(0..=100).contains(&discount) {
        return Err(bad("скидка — от 0 до 100 %"));
    }
    if b.discount_percent.is_some() && discount != item.discount_percent {
        notes.push(format!("{}: скидка {} %", item.product_name, discount));
    }
    tx.execute(
        "UPDATE order_items SET discount_percent = ?1, subtotal_tiyn = ?2 WHERE id = ?3",
        libsql::params![discount, line_subtotal(item.price_tiyn, qty, discount), item_id],
    )
    .await?;
    recalc_totals(&tx, &id, &ts).await?;
    sync_shortage(&tx, &id, &ts).await?;
    if !notes.is_empty() {
        log_event(&tx, &id, &ActorRef::from(&staff), "items", &notes.join("; "), None, &ts).await?;
    }
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

async fn remove_item(
    staff: Staff,
    State(db): State<Db>,
    Path((id, item_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderDetail>> {
    staff.require(OFFICE)?;
    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    editable(&tx, &id).await?;
    let item = load_item(&tx, &id, &item_id).await?;
    let mut r = tx.query("SELECT COUNT(*) FROM order_items WHERE order_id = ?1", libsql::params![id.clone()]).await?;
    let count: i64 = r.next().await?.map(|r| r.get(0)).transpose()?.unwrap_or(0);
    if count <= 1 {
        return Err(ApiError::Conflict("это последняя позиция — заказ без позиций нужно отменить".into()));
    }
    let ts = now();
    if let Some(pid) = item.product_id.as_deref() {
        reserve(&tx, pid, -item.quantity_milli, &id, &ts).await?;
    }
    tx.execute("DELETE FROM order_items WHERE id = ?1", libsql::params![item_id]).await?;
    recalc_totals(&tx, &id, &ts).await?;
    sync_shortage(&tx, &id, &ts).await?;
    let msg = format!("Убрано: {} — {} {}", item.product_name, milli_to_string(item.quantity_milli), item.unit);
    log_event(&tx, &id, &ActorRef::from(&staff), "items", &msg, None, &ts).await?;
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

#[derive(Deserialize)]
struct PickItem {
    picked: bool,
    /// Сколько не хватило: по учёту 4, годных 3 — нехватка 1.
    short_qty: Option<String>,
}

/// Отметка кладовщика при сборке (ФТ-СК-07). Нехватка делает заказ
/// проблемным, менеджер видит, на каком этапе и чего не хватило (ФТ-З-08).
async fn pick_item(
    staff: Staff,
    State(db): State<Db>,
    Path((id, item_id)): Path<(String, String)>,
    Json(b): Json<PickItem>,
) -> ApiResult<Json<OrderDetail>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let o = load_order(&tx, &id).await?;
    if o.status != "confirmed" {
        return Err(ApiError::Conflict(format!(
            "собирают подтверждённый заказ, а этот в статусе «{}»",
            status_title(&o.status)
        )));
    }
    let item = load_item(&tx, &id, &item_id).await?;
    let short = match b.short_qty.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => parse_qty(s)?,
        None => 0,
    };
    if short > item.quantity_milli {
        return Err(bad("нехватка больше, чем позиция в заказе"));
    }
    let picked = b.picked && short == 0;
    let ts = now();
    tx.execute(
        "UPDATE order_items SET picked = ?1, short_milli = ?2 WHERE id = ?3",
        libsql::params![picked as i64, short, item_id],
    )
    .await?;
    sync_shortage(&tx, &id, &ts).await?;
    let msg = if short > 0 {
        format!("Сборка: не хватает {} — {} {}", item.product_name, milli_to_string(short), item.unit)
    } else if picked {
        format!("Собрано: {}", item.product_name)
    } else {
        format!("Снята отметка сборки: {}", item.product_name)
    };
    log_event(&tx, &id, &ActorRef::from(&staff), "picking", &msg, None, &ts).await?;
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

// ─── Оплата ──────────────────────────────────────────────────────────────────

fn payment_title(s: &str) -> &'static str {
    match s {
        "unpaid" => "не оплачен",
        "partial" => "оплачен частично",
        "paid" => "оплачен",
        "refunded" => "возврат",
        _ => "—",
    }
}

/// Оплату отмечает менеджер вручную: QR статический, система о платеже
/// не узнаёт (ФТ-П-03). Хранится полученная сумма (ФТ-П-04).
async fn confirm_payment(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<ConfirmPayment>,
) -> ApiResult<Json<OrderDetail>> {
    staff.require(OFFICE)?;
    let status = body.payment_status.as_str();
    if !PAYMENT_STATUSES.contains(&status) {
        return Err(bad(format!("неизвестный статус оплаты: {status}")));
    }
    if let Some(m) = &body.payment_method {
        if m == "online" {
            return Err(bad("онлайн-оплаты в магазине нет: наличные или перевод Kaspi"));
        }
        if !PAYMENT_METHODS.contains(&m.as_str()) {
            return Err(bad(format!("неизвестный способ оплаты: {m}")));
        }
    }

    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let o = load_order(&tx, &id).await?;
    if o.status == "cancelled" && status != "refunded" {
        return Err(ApiError::Conflict("заказ отменён — оплату по нему не отмечают".into()));
    }
    let method = body.payment_method.clone().unwrap_or(o.payment_method.clone());
    let given = body
        .paid
        .as_deref()
        .map(|v| parse_tiyn(v).map_err(|e| bad(format!("оплачено: {e}"))))
        .transpose()?;

    let paid = match status {
        "paid" => {
            let p = given.unwrap_or(o.total_tiyn);
            if p != o.total_tiyn {
                return Err(bad(format!(
                    "оплачено {} не равно итогу {} — для предоплаты укажите partial",
                    tiyn_to_string(p),
                    tiyn_to_string(o.total_tiyn)
                )));
            }
            p
        }
        "partial" => {
            let p = given.ok_or_else(|| bad("для частичной оплаты укажите сумму"))?;
            if p <= 0 || p >= o.total_tiyn {
                return Err(bad(format!("частичная оплата — больше нуля и меньше итога {}", tiyn_to_string(o.total_tiyn))));
            }
            p
        }
        "unpaid" => {
            if o.payment_status == "refunded" {
                return Err(ApiError::Conflict("по заказу оформлен возврат".into()));
            }
            0
        }
        _ => {
            if !matches!(o.payment_status.as_str(), "paid" | "partial") {
                return Err(ApiError::Conflict("возврат возможен только по оплаченному заказу".into()));
            }
            0
        }
    };
    if matches!(status, "paid" | "partial") && method == "none" {
        return Err(bad("укажите способ оплаты: cash или kaspi_transfer"));
    }

    let ts = now();
    tx.execute(
        "UPDATE orders SET payment_status = ?1, payment_method = ?2, paid_tiyn = ?3, updated_at = ?4 WHERE id = ?5",
        libsql::params![status, method.clone(), paid, ts.clone(), id.clone()],
    )
    .await?;
    let delta = paid - o.paid_tiyn;
    let mut msg = format!("Оплата: {}", payment_title(status));
    if paid > 0 {
        msg.push_str(&format!(", получено {} ₸ ({method})", tiyn_to_string(paid)));
    }
    if status == "refunded" {
        msg.push_str(&format!(", возвращено {} ₸", tiyn_to_string(o.paid_tiyn)));
    }
    log_event(&tx, &id, &ActorRef::from(&staff), "payment", &msg, (delta != 0).then_some(delta), &ts).await?;
    tx.commit().await?;
    Ok(Json(detail(&conn, &id, Some(&staff)).await?))
}

// ─── Журнал и комментарии ────────────────────────────────────────────────────

async fn list_events(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<Vec<EventView>>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    load_order(&conn, &id).await?;
    Ok(Json(load_events(&conn, &id, !staff.is(Role::Warehouse)).await?))
}

#[derive(Deserialize)]
struct Comment {
    text: String,
}

async fn add_comment(staff: Staff, State(db): State<Db>, Path(id): Path<String>, Json(b): Json<Comment>) -> ApiResult<Json<Vec<EventView>>> {
    staff.require(ANY_STAFF)?;
    let text = b.text.trim();
    if text.is_empty() {
        return Err(bad("пустой комментарий"));
    }
    let conn = db.connect()?;
    load_order(&conn, &id).await?;
    let ts = now();
    log_event(&conn, &id, &ActorRef::from(&staff), "comment", text, None, &ts).await?;
    conn.execute("UPDATE orders SET updated_at = ?1 WHERE id = ?2", libsql::params![ts, id.clone()]).await?;
    Ok(Json(load_events(&conn, &id, !staff.is(Role::Warehouse)).await?))
}

// ─── Отслеживание покупателем ────────────────────────────────────────────────

#[derive(Deserialize)]
struct TrackQuery {
    order_no: String,
    phone: String,
}

#[derive(Serialize)]
struct TrackItem {
    name: String,
    quantity: String,
    unit: String,
}

#[derive(Serialize)]
struct TrackStep {
    status: String,
    title: &'static str,
    at: String,
}

#[derive(Serialize)]
struct Tracking {
    order_no: String,
    status: String,
    status_title: &'static str,
    created_at: String,
    delivery_method: String,
    city: Option<String>,
    total: String,
    payment_status: String,
    items: Vec<TrackItem>,
    history: Vec<TrackStep>,
}

/// Покупатель видит этап заказа по номеру и телефону, без входа (ФТ-ЛК-03).
/// На неверный телефон ответ тот же, что на неверный номер: перебором
/// чужие заказы не найти.
async fn track_order(State(db): State<Db>, Query(q): Query<TrackQuery>) -> ApiResult<Json<Tracking>> {
    let norm = phone_norm(&q.phone).ok_or(ApiError::NotFound)?;
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!("SELECT {ORDER_COLS} FROM {ORDER_FROM} WHERE o.order_no = ?1"),
            libsql::params![q.order_no.trim()],
        )
        .await?;
    let o: OrderRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    if o.client_phone.as_deref().and_then(phone_norm) != Some(norm) {
        return Err(ApiError::NotFound);
    }
    let items = load_items(&conn, &o.id)
        .await?
        .into_iter()
        .map(|i| TrackItem { name: i.product_name, quantity: milli_to_string(i.quantity_milli), unit: i.unit })
        .collect();
    let mut history = vec![TrackStep { status: "new".into(), title: status_title("new"), at: o.created_at.clone() }];
    let mut rows = conn
        .query(
            "SELECT message, created_at FROM order_events WHERE order_id = ?1 AND kind = 'status' ORDER BY created_at, rowid",
            libsql::params![o.id.clone()],
        )
        .await?;
    while let Some(r) = rows.next().await? {
        let (msg, at): (String, String) = (r.get(0)?, r.get(1)?);
        if let Some(s) = STATUSES.iter().find(|s| msg.starts_with(&format!("Статус: {}", status_title(s)))) {
            history.push(TrackStep { status: s.to_string(), title: status_title(s), at });
        }
    }
    Ok(Json(Tracking {
        order_no: o.order_no.unwrap_or_default(),
        status_title: status_title(&o.status),
        status: o.status,
        created_at: o.created_at,
        delivery_method: o.delivery_method,
        city: o.city,
        total: tiyn_to_string(o.total_tiyn),
        payment_status: o.payment_status,
        items,
        history,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn нельзя_перескочить_статус() {
        assert!(!next_statuses("new").contains(&"completed"));
        assert!(!next_statuses("handed_over").contains(&"cancelled"));
        assert!(next_statuses("handed_over").contains(&"completed"));
        assert!(next_statuses("completed").is_empty());
    }

    #[test]
    fn скидка_по_строке() {
        // 3 шт по 1 000 ₸ со скидкой 10 % = 2 700 ₸
        assert_eq!(line_subtotal(100_000, 3_000, 10), 270_000);
        assert_eq!(line_subtotal(100_000, 3_000, 0), 300_000);
    }
}
