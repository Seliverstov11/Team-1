use axum::{Json, Router, extract::State, routing::get};

use super::{collect, first, new_id, now};
use crate::{
    db::Db,
    error::{ApiError, ApiResult},
    models::{CreateMovement, MovementRow, MovementView, StockRow, StockView},
    num::parse_milli,
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/stock", get(list_stock))
        .route("/movements", get(list_movements).post(add_movement))
}

const MOVEMENT_COLS: &str =
    "id, product_id, movement_type, quantity_milli, reason, order_id, employee_id, created_at";

async fn list_stock(State(db): State<Db>) -> ApiResult<Json<Vec<StockView>>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            "SELECT id, name, sku, unit, stock_milli, reserved_milli, min_stock_milli
             FROM products WHERE is_archived = 0 ORDER BY name",
            (),
        )
        .await?;
    let items: Vec<StockRow> = collect(rows).await?;
    Ok(Json(items.into_iter().map(StockView::from).collect()))
}

async fn list_movements(State(db): State<Db>) -> ApiResult<Json<Vec<MovementView>>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!(
                "SELECT {MOVEMENT_COLS} FROM stock_movements
                 ORDER BY created_at DESC LIMIT 200"
            ),
            (),
        )
        .await?;
    let items: Vec<MovementRow> = collect(rows).await?;
    Ok(Json(items.into_iter().map(MovementView::from).collect()))
}

/// Приход и корректировка. Резервом отсюда не управляем — он живёт
/// по заказам, иначе остаток и резерв разъедутся.
async fn add_movement(
    State(db): State<Db>,
    Json(body): Json<CreateMovement>,
) -> ApiResult<Json<MovementView>> {
    if !["in", "out", "adjustment"].contains(&body.movement_type.as_str()) {
        return Err(ApiError::BadRequest(
            "movement_type: in, out или adjustment".into(),
        ));
    }

    let raw = body.quantity.trim();
    let (negative, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let magnitude =
        parse_milli(digits).map_err(|e| ApiError::BadRequest(format!("количество: {e}")))?;
    if magnitude == 0 {
        return Err(ApiError::BadRequest("количество не может быть нулём".into()));
    }

    let delta = match body.movement_type.as_str() {
        "in" => {
            if negative {
                return Err(ApiError::BadRequest("приход не может быть отрицательным".into()));
            }
            magnitude
        }
        "out" => -magnitude,
        _ => {
            if negative {
                -magnitude
            } else {
                magnitude
            }
        }
    };

    let conn = db.connect()?;
    let tx = conn
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await?;

    let rows = tx
        .query(
            "SELECT stock_milli, reserved_milli, name FROM products WHERE id = ?1",
            libsql::params![body.product_id.clone()],
        )
        .await?;
    let Some(row) = ({
        let mut rows = rows;
        rows.next().await?
    }) else {
        return Err(ApiError::NotFound);
    };
    let stock: i64 = row.get(0)?;
    let reserved: i64 = row.get(1)?;
    let name: String = row.get(2)?;

    let new_stock = stock + delta;
    if new_stock < 0 {
        return Err(ApiError::BadRequest(format!(
            "«{name}»: на складе {}, списать больше нельзя",
            crate::num::milli_to_string(stock)
        )));
    }
    if new_stock < reserved {
        return Err(ApiError::BadRequest(format!(
            "«{name}»: {} уже зарезервировано под заказы, остаток нельзя опустить ниже",
            crate::num::milli_to_string(reserved)
        )));
    }

    let ts = now();
    tx.execute(
        "UPDATE products SET stock_milli = ?1, updated_at = ?2 WHERE id = ?3",
        libsql::params![new_stock, ts.clone(), body.product_id.clone()],
    )
    .await?;

    let mov_id = new_id();
    tx.execute(
        "INSERT INTO stock_movements
         (id, product_id, movement_type, quantity_milli, reason, employee_id, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        libsql::params![
            mov_id.clone(),
            body.product_id.clone(),
            body.movement_type.clone(),
            delta,
            body.reason.clone(),
            body.employee_id.clone(),
            ts
        ],
    )
    .await?;

    tx.commit().await?;

    let rows = conn
        .query(
            &format!("SELECT {MOVEMENT_COLS} FROM stock_movements WHERE id = ?1"),
            libsql::params![mov_id],
        )
        .await?;
    let mov: MovementRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(MovementView::from(mov)))
}
