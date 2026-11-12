use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};

use super::{collect, first};
use crate::{
    db::Db,
    error::{ApiError, ApiResult},
    models::{Category, ProductQuery, ProductRow, ProductView},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/categories", get(list_categories))
        .route("/products", get(list_products))
        .route("/products/{id}", get(get_product))
}

const PRODUCT_COLS: &str = "id, category_id, name, sku, description, price_tiyn, unit,
     is_weight, stock_milli, reserved_milli, image_url, is_archived";

async fn list_categories(State(db): State<Db>) -> ApiResult<Json<Vec<Category>>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            "SELECT id, name, slug, parent_id, sort_order, created_at
             FROM categories ORDER BY sort_order, name",
            (),
        )
        .await?;
    Ok(Json(collect::<Category>(rows).await?))
}

async fn list_products(
    State(db): State<Db>,
    Query(q): Query<ProductQuery>,
) -> ApiResult<Json<Vec<ProductView>>> {
    let conn = db.connect()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = q.search.as_deref().map(|s| format!("%{s}%"));
    let in_stock = q.in_stock.map(|v| i64::from(v));

    let sql = format!(
        "SELECT {PRODUCT_COLS}
         FROM products
         WHERE is_archived = 0
           AND (?1 IS NULL OR category_id = ?1)
           AND (?2 IS NULL OR name LIKE ?2 OR IFNULL(sku,'') LIKE ?2)
           AND (?3 IS NULL OR (?3 = 1 AND stock_milli - reserved_milli > 0))
         ORDER BY name
         LIMIT ?4 OFFSET ?5"
    );

    let rows = conn
        .query(&sql, libsql::params![q.category_id, search, in_stock, limit, offset])
        .await?;

    let items: Vec<ProductRow> = collect(rows).await?;
    Ok(Json(items.into_iter().map(ProductView::from).collect()))
}

async fn get_product(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<ProductView>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!("SELECT {PRODUCT_COLS} FROM products WHERE id = ?1"),
            libsql::params![id],
        )
        .await?;

    let row: ProductRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(ProductView::from(row)))
}
