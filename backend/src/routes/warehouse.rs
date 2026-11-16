//! Склад: остатки и полки, приход и корректировка, очередь сборки.
//! Сканера штрихкодов нет, всё вручную; место хранения указывают при
//! приёмке (ФТ-СК-01, ФТ-СК-02).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch},
};
use serde::{Deserialize, Serialize};

use super::{collect, first, new_id, now, orders::load_items};
use crate::{
    auth::{ANY_STAFF, STOREKEEPING, Staff},
    db::Db,
    error::{ApiError, ApiResult, bad},
    models::{
        CreateMovement, MovementRow, MovementView, ORDER_COLS, ORDER_FROM, OrderItemView, OrderRow,
        OrderView, StockRow, StockView,
    },
    num::{milli_to_string, parse_milli},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/stock", get(list_stock))
        .route("/stock/{product_id}", patch(update_stock))
        .route("/movements", get(list_movements).post(add_movement))
        .route("/assembly", get(assembly_queue))
}

const STOCK_COLS: &str = "id, name, sku, variant_name, category_id, unit, stock_milli, reserved_milli,
     min_stock_milli, storage_location, updated_at";

const MOVEMENT_SQL: &str = "SELECT m.id, m.product_id, p.name AS product_name, m.movement_type,
     m.quantity_milli, m.reason, m.order_id, m.employee_id, e.name AS employee_name,
     m.storage_location, m.created_at
     FROM stock_movements m
     LEFT JOIN products p ON p.id = m.product_id
     LEFT JOIN employees e ON e.id = m.employee_id";

#[derive(Deserialize)]
struct StockQuery {
    /// in_stock | low | out — «Ниже минимума — закупить» и «Нет в наличии» (ФТ-СК-04)
    status: Option<String>,
    search: Option<String>,
    category_id: Option<String>,
}

async fn list_stock(staff: Staff, State(db): State<Db>, Query(q): Query<StockQuery>) -> ApiResult<Json<Vec<StockView>>> {
    staff.require(ANY_STAFF)?;
    if let Some(s) = &q.status
        && !["in_stock", "low", "out"].contains(&s.as_str())
    {
        return Err(bad("status: in_stock, low или out"));
    }
    // Слова ищутся в названии, фасовке, артикуле и полке без учёта регистра
    // в кириллице (#36). Позиций на складе немного — фильтруем здесь, а не в SQLite.
    let terms = crate::db::search_terms(q.search.as_deref().unwrap_or(""));
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "SELECT {STOCK_COLS} FROM products
                 WHERE is_archived = 0 AND (?1 IS NULL OR category_id = ?1)
                 ORDER BY IFNULL(storage_location, 'я'), name"
            ),
            libsql::params![q.category_id.clone()],
        )
        .await?;
    let items: Vec<StockRow> = collect(rows).await?;
    Ok(Json(
        items
            .into_iter()
            .filter(|r| {
                let hay = crate::db::normalize_search(&format!(
                    "{} {} {} {}",
                    r.name,
                    r.variant_name.as_deref().unwrap_or(""),
                    r.sku.as_deref().unwrap_or(""),
                    r.storage_location.as_deref().unwrap_or("")
                ));
                terms.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(StockView::from)
            .filter(|v| q.status.as_deref().is_none_or(|s| v.status == s))
            .collect(),
    ))
}

#[derive(Deserialize)]
struct StockPatch {
    /// Пустая строка снимает минимум.
    min_stock: Option<String>,
    storage_location: Option<String>,
}

/// Минимальный остаток и полку меняют владелец или кладовщик (ФТ-СК-09).
async fn update_stock(
    staff: Staff,
    State(db): State<Db>,
    Path(product_id): Path<String>,
    Json(b): Json<StockPatch>,
) -> ApiResult<Json<StockView>> {
    staff.require(STOREKEEPING)?;
    let conn = db.connect()?;
    let rows = conn
        .query(&format!("SELECT {STOCK_COLS} FROM products WHERE id = ?1"), libsql::params![product_id.clone()])
        .await?;
    let cur: StockRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    let min = match b.min_stock.as_deref().map(str::trim) {
        None => cur.min_stock_milli,
        Some("") => None,
        Some(v) => Some(parse_milli(v).map_err(|e| bad(format!("минимальный остаток: {e}")))?),
    };
    let location = match b.storage_location.as_deref().map(str::trim) {
        None => cur.storage_location.clone(),
        Some("") => None,
        Some(v) => Some(v.to_string()),
    };
    conn.execute(
        "UPDATE products SET min_stock_milli = ?1, storage_location = ?2, updated_at = ?3 WHERE id = ?4",
        libsql::params![min, location, now(), product_id.clone()],
    )
    .await?;
    let rows = conn
        .query(&format!("SELECT {STOCK_COLS} FROM products WHERE id = ?1"), libsql::params![product_id])
        .await?;
    Ok(Json(StockView::from(first::<StockRow>(rows).await?.ok_or(ApiError::NotFound)?)))
}

#[derive(Deserialize)]
struct MovementQuery {
    product_id: Option<String>,
    order_id: Option<String>,
    movement_type: Option<String>,
    limit: Option<i64>,
}

async fn list_movements(staff: Staff, State(db): State<Db>, Query(q): Query<MovementQuery>) -> ApiResult<Json<Vec<MovementView>>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "{MOVEMENT_SQL}
                 WHERE (?1 IS NULL OR m.product_id = ?1) AND (?2 IS NULL OR m.order_id = ?2)
                   AND (?3 IS NULL OR m.movement_type = ?3)
                 ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?4"
            ),
            libsql::params![q.product_id.clone(), q.order_id.clone(), q.movement_type.clone(), q.limit.unwrap_or(200).clamp(1, 1000)],
        )
        .await?;
    let items: Vec<MovementRow> = collect(rows).await?;
    Ok(Json(items.into_iter().map(MovementView::from).collect()))
}

/// Приход и корректировка (ФТ-СК-05, ФТ-СК-06). Резервом отсюда не
/// управляем — он живёт по заказам, иначе остаток и резерв разъедутся.
async fn add_movement(staff: Staff, State(db): State<Db>, Json(body): Json<CreateMovement>) -> ApiResult<Json<MovementView>> {
    staff.require(STOREKEEPING)?;
    if !["in", "out", "adjustment"].contains(&body.movement_type.as_str()) {
        return Err(bad("movement_type: in, out или adjustment"));
    }
    let reason = body.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    if body.movement_type != "in" && reason.is_none() {
        return Err(bad("для расхода и корректировки укажите причину: просрочка, пересчёт, брак"));
    }
    let raw = body.quantity.trim();
    let (negative, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let magnitude = parse_milli(digits).map_err(|e| bad(format!("количество: {e}")))?;
    if magnitude == 0 {
        return Err(bad("количество не может быть нулём"));
    }
    let delta = match body.movement_type.as_str() {
        "in" if negative => return Err(bad("приход не может быть отрицательным")),
        "in" => magnitude,
        "out" => -magnitude,
        _ if negative => -magnitude,
        _ => magnitude,
    };

    let conn = db.connect()?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    let mut rows = tx
        .query(
            "SELECT stock_milli, reserved_milli, name, storage_location FROM products WHERE id = ?1",
            libsql::params![body.product_id.clone()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::NotFound);
    };
    let stock: i64 = row.get(0)?;
    let reserved: i64 = row.get(1)?;
    let name: String = row.get(2)?;
    let current_location: Option<String> = row.get(3)?;
    let location = body.storage_location.clone().filter(|s| !s.trim().is_empty()).or(current_location);

    let new_stock = stock + delta;
    if new_stock < 0 {
        return Err(bad(format!("«{name}»: на складе {}, списать больше нельзя", milli_to_string(stock))));
    }
    if new_stock < reserved {
        return Err(bad(format!(
            "«{name}»: {} уже зарезервировано под заказы — сначала поправьте заказы",
            milli_to_string(reserved)
        )));
    }

    let ts = now();
    tx.execute(
        "UPDATE products SET stock_milli = ?1, storage_location = ?2, updated_at = ?3 WHERE id = ?4",
        libsql::params![new_stock, location.clone(), ts.clone(), body.product_id.clone()],
    )
    .await?;
    let mov_id = new_id();
    tx.execute(
        "INSERT INTO stock_movements (id, product_id, movement_type, quantity_milli, reason, employee_id,
                                      storage_location, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        libsql::params![mov_id.clone(), body.product_id.clone(), body.movement_type.clone(), delta, reason, staff.id.clone(), location, ts],
    )
    .await?;
    tx.commit().await?;

    let rows = conn.query(&format!("{MOVEMENT_SQL} WHERE m.id = ?1"), libsql::params![mov_id]).await?;
    let mov: MovementRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(MovementView::from(mov)))
}

#[derive(Serialize)]
struct AssemblyOrder {
    #[serde(flatten)]
    order: OrderView,
    /// Позиции в порядке обхода склада — по полкам.
    items: Vec<OrderItemView>,
    picked: usize,
    total: usize,
}

/// Очередь сборки: подтверждённые заказы, срочные сверху, позиции
/// отсортированы по полкам (ФТ-СК-07). Без сумм — кладовщику они не нужны.
async fn assembly_queue(staff: Staff, State(db): State<Db>) -> ApiResult<Json<Vec<AssemblyOrder>>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "SELECT {ORDER_COLS} FROM {ORDER_FROM}
                 WHERE o.status IN ('confirmed', 'assembled')
                 ORDER BY o.status = 'assembled', o.is_urgent DESC, o.created_at"
            ),
            (),
        )
        .await?;
    let orders: Vec<OrderRow> = collect(rows).await?;
    let mut out = Vec::new();
    for o in orders {
        let mut items = load_items(&conn, &o.id).await?;
        items.sort_by(|a, b| {
            a.storage_location.as_deref().unwrap_or("я").cmp(b.storage_location.as_deref().unwrap_or("я"))
        });
        let total = items.len();
        let picked = items.iter().filter(|i| i.picked != 0).count();
        out.push(AssemblyOrder {
            order: OrderView::from(o).without_payment(),
            items: items.into_iter().map(OrderItemView::from).collect(),
            picked,
            total,
        });
    }
    Ok(Json(out))
}
