use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};

use super::{collect, first};
use crate::{
    db::Db,
    error::{ApiError, ApiResult},
    models::{Attribute, AttributeRow, Category, ProductQuery, ProductRow, ProductView},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/categories", get(list_categories))
        .route("/products", get(list_products))
        .route("/products/{id}", get(get_product))
}

const PRODUCT_COLS: &str = "id, category_id, name, sku, description, price_tiyn, unit,
     is_weight, stock_milli, reserved_milli, min_order_milli, order_step_milli,
     image_url, is_archived, group_id, variant_name, wholesale_price_tiyn,
     wholesale_min_qty, storage_location, image_name";

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

/// Характеристики тянутся одним запросом на всю выдачу, а не по запросу
/// на товар — иначе на странице из 50 товаров будет 51 обращение к БД.
async fn attributes_for(
    conn: &libsql::Connection,
    ids: &[String],
) -> ApiResult<HashMap<String, Vec<Attribute>>> {
    let mut grouped: HashMap<String, Vec<Attribute>> = HashMap::new();
    if ids.is_empty() {
        return Ok(grouped);
    }

    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT product_id, name, value FROM product_attributes
         WHERE product_id IN ({placeholders}) ORDER BY sort_order, name"
    );

    let params: Vec<libsql::Value> = ids.iter().map(|s| libsql::Value::from(s.clone())).collect();
    let rows = conn.query(&sql, params).await?;

    for r in collect::<AttributeRow>(rows).await? {
        grouped.entry(r.product_id).or_default().push(Attribute {
            name: r.name,
            value: r.value,
        });
    }
    Ok(grouped)
}

async fn list_products(
    State(db): State<Db>,
    Query(q): Query<ProductQuery>,
) -> ApiResult<Json<Vec<ProductView>>> {
    let conn = db.connect()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = q.search.as_deref().map(|s| format!("%{s}%"));
    let in_stock = q.in_stock.map(i64::from);

    let sql = format!(
        "SELECT {PRODUCT_COLS}
         FROM products
         WHERE is_archived = 0
           AND (?1 IS NULL OR category_id = ?1)
           AND (?2 IS NULL OR name LIKE ?2 OR IFNULL(sku,'') LIKE ?2)
           AND (?3 IS NULL OR (?3 = 1 AND stock_milli - reserved_milli > 0))
           AND (?6 IS NULL OR group_id = ?6)
         ORDER BY name, IFNULL(variant_name, '')
         LIMIT ?4 OFFSET ?5"
    );

    let rows = conn
        .query(
            &sql,
            libsql::params![q.category_id, search, in_stock, limit, offset, q.group_id],
        )
        .await?;

    let products: Vec<ProductRow> = collect(rows).await?;
    let ids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let mut attrs = attributes_for(&conn, &ids).await?;

    Ok(Json(
        products
            .into_iter()
            .map(|p| {
                let a = attrs.remove(&p.id).unwrap_or_default();
                ProductView::from_row(p, a)
            })
            .collect(),
    ))
}

async fn get_product(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<ProductView>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            &format!("SELECT {PRODUCT_COLS} FROM products WHERE id = ?1"),
            libsql::params![id.clone()],
        )
        .await?;

    let row: ProductRow = first(rows).await?.ok_or(ApiError::NotFound)?;
    let mut attrs = attributes_for(&conn, std::slice::from_ref(&id)).await?;
    let a = attrs.remove(&id).unwrap_or_default();

    Ok(Json(ProductView::from_row(row, a)))
}
