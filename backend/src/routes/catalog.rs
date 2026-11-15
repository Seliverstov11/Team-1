//! Каталог: публичная выдача для сайта и управление для владельца.
//!
//! Категории, товары и характеристики динамические, без хардкода: владелец
//! добавляет их сам (ФТ-К-02, ФТ-Т-03, ФТ-КТ-01). Товар с историей не
//! удаляется, а снимается с продажи (ФТ-КТ-03).

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post, put},
};
use serde::Serialize;

use super::{collect, first, new_id, now, placeholders};
use crate::{
    auth::{ADMIN, ANY_STAFF, MaybeActor, Role, Staff},
    db::Db,
    error::{ApiError, ApiResult, bad},
    models::{
        Attribute, AttributeRow, Category, CategoryInput, PRODUCT_COLS, ProductInput,
        ProductQuery, ProductRow, ProductView,
    },
    num::{parse_milli, parse_tiyn, tiyn_to_string},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/categories", get(list_categories).post(create_category))
        .route("/categories/{id}", axum::routing::patch(update_category).delete(delete_category))
        .route("/products", get(list_products).post(create_product))
        .route(
            "/products/{id}",
            get(get_product).patch(update_product).delete(delete_product),
        )
        .route("/products/{id}/archive", post(archive_product))
        .route("/products/{id}/restore", post(restore_product))
        .route("/products/{id}/attributes", put(replace_attributes))
        .route("/attributes", get(attribute_names))
}

// ─── Категории ───────────────────────────────────────────────────────────────

async fn list_categories(State(db): State<Db>) -> ApiResult<Json<Vec<Category>>> {
    let conn = db.connect()?;
    let rows = conn
        .query(
            "SELECT c.id, c.name, c.slug, c.parent_id, c.sort_order, c.created_at,
                    (SELECT COUNT(*) FROM products p
                      WHERE p.category_id = c.id AND p.is_archived = 0) AS products_count
             FROM categories c ORDER BY c.sort_order, c.name",
            (),
        )
        .await?;
    Ok(Json(collect::<Category>(rows).await?))
}

async fn get_category(conn: &libsql::Connection, id: &str) -> ApiResult<Category> {
    let rows = conn
        .query(
            "SELECT c.id, c.name, c.slug, c.parent_id, c.sort_order, c.created_at,
                    (SELECT COUNT(*) FROM products p
                      WHERE p.category_id = c.id AND p.is_archived = 0) AS products_count
             FROM categories c WHERE c.id = ?1",
            libsql::params![id],
        )
        .await?;
    first(rows).await?.ok_or(ApiError::NotFound)
}

fn slugify(s: &str) -> String {
    const TR: &[(char, &str)] = &[
        ('а', "a"), ('б', "b"), ('в', "v"), ('г', "g"), ('д', "d"), ('е', "e"), ('ё', "e"),
        ('ж', "zh"), ('з', "z"), ('и', "i"), ('й', "y"), ('к', "k"), ('л', "l"), ('м', "m"),
        ('н', "n"), ('о', "o"), ('п', "p"), ('р', "r"), ('с', "s"), ('т', "t"), ('у', "u"),
        ('ф', "f"), ('х', "h"), ('ц', "c"), ('ч', "ch"), ('ш', "sh"), ('щ', "sch"), ('ъ', ""),
        ('ы', "y"), ('ь', ""), ('э', "e"), ('ю', "yu"), ('я', "ya"),
    ];
    let mut out = String::new();
    for ch in s.to_lowercase().chars() {
        if let Some((_, t)) = TR.iter().find(|(c, _)| *c == ch) {
            out.push_str(t);
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() { "category".into() } else { s }
}

async fn create_category(
    staff: Staff,
    State(db): State<Db>,
    Json(b): Json<CategoryInput>,
) -> ApiResult<Json<Category>> {
    staff.require(ADMIN)?;
    let name = b.name.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad("название категории обязательно"));
    }
    let slug = b.slug.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| slugify(name));
    let conn = db.connect()?;
    let mut rows = conn
        .query("SELECT 1 FROM categories WHERE slug = ?1", libsql::params![slug.clone()])
        .await?;
    if rows.next().await?.is_some() {
        return Err(ApiError::Conflict(format!("категория с адресом «{slug}» уже есть")));
    }
    let sort = match b.sort_order {
        Some(s) => s,
        None => {
            let mut r = conn.query("SELECT IFNULL(MAX(sort_order), 0) + 1 FROM categories", ()).await?;
            r.next().await?.map(|r| r.get::<i64>(0)).transpose()?.unwrap_or(1)
        }
    };
    let id = format!("c-{slug}");
    conn.execute(
        "INSERT INTO categories (id, name, slug, parent_id, sort_order, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params![id.clone(), name, slug, b.parent_id.clone(), sort, now()],
    )
    .await?;
    Ok(Json(get_category(&conn, &id).await?))
}

async fn update_category(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(b): Json<CategoryInput>,
) -> ApiResult<Json<Category>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    get_category(&conn, &id).await?;
    if let Some(n) = &b.name
        && n.trim().is_empty()
    {
        return Err(bad("название категории не может быть пустым"));
    }
    if b.parent_id.as_deref() == Some(id.as_str()) {
        return Err(bad("категория не может быть вложена сама в себя"));
    }
    conn.execute(
        "UPDATE categories SET name = IFNULL(?1, name), slug = IFNULL(?2, slug),
                parent_id = IFNULL(?3, parent_id), sort_order = IFNULL(?4, sort_order)
         WHERE id = ?5",
        libsql::params![
            b.name.as_deref().map(str::trim),
            b.slug.clone(),
            b.parent_id.clone(),
            b.sort_order,
            id.clone()
        ],
    )
    .await?;
    Ok(Json(get_category(&conn, &id).await?))
}

async fn delete_category(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    get_category(&conn, &id).await?;
    let mut rows = conn
        .query(
            "SELECT (SELECT COUNT(*) FROM products WHERE category_id = ?1),
                    (SELECT COUNT(*) FROM categories WHERE parent_id = ?1)",
            libsql::params![id.clone()],
        )
        .await?;
    let r = rows.next().await?.ok_or(ApiError::NotFound)?;
    let (products, children): (i64, i64) = (r.get(0)?, r.get(1)?);
    if products > 0 || children > 0 {
        return Err(ApiError::Conflict(format!(
            "в категории {products} товаров и {children} подкатегорий — сначала перенесите их"
        )));
    }
    conn.execute("DELETE FROM categories WHERE id = ?1", libsql::params![id]).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ─── Выдача товаров ──────────────────────────────────────────────────────────

/// Характеристики одним запросом на всю выдачу, а не по запросу на товар.
async fn attributes_for(
    conn: &libsql::Connection,
    ids: &[String],
) -> ApiResult<HashMap<String, Vec<Attribute>>> {
    let mut grouped: HashMap<String, Vec<Attribute>> = HashMap::new();
    if ids.is_empty() {
        return Ok(grouped);
    }
    let sql = format!(
        "SELECT product_id, name, value FROM product_attributes
         WHERE product_id IN ({}) ORDER BY sort_order, name",
        placeholders(1, ids.len())
    );
    let params: Vec<libsql::Value> = ids.iter().map(|s| libsql::Value::from(s.clone())).collect();
    let rows = conn.query(&sql, params).await?;
    for r in collect::<AttributeRow>(rows).await? {
        grouped.entry(r.product_id).or_default().push(Attribute { name: r.name, value: r.value });
    }
    Ok(grouped)
}

async fn personal_prices_for(
    conn: &libsql::Connection,
    client_id: Option<&str>,
    ids: &[String],
) -> ApiResult<HashMap<String, i64>> {
    let mut out = HashMap::new();
    let Some(client_id) = client_id else { return Ok(out) };
    if ids.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT product_id, price_tiyn FROM client_prices
         WHERE client_id = ?1 AND product_id IN ({})",
        placeholders(2, ids.len())
    );
    let mut params: Vec<libsql::Value> = vec![libsql::Value::from(client_id.to_string())];
    params.extend(ids.iter().map(|s| libsql::Value::from(s.clone())));
    let mut rows = conn.query(&sql, params).await?;
    while let Some(row) = rows.next().await? {
        out.insert(row.get::<String>(0)?, row.get::<i64>(1)?);
    }
    Ok(out)
}

/// Чьи персональные цены показывать. Покупатель — свои, по входу.
/// Сотрудник может посмотреть каталог глазами клиента через client_id.
/// Гостю параметр не помогает: иначе любой увидел бы чужие договорные цены.
fn price_client(actor: &MaybeActor, requested: Option<&String>) -> Option<String> {
    if let Some(c) = actor.customer() {
        return Some(c.id.clone());
    }
    if actor.staff().is_some() {
        return requested.cloned();
    }
    None
}

async fn to_views(
    conn: &libsql::Connection,
    products: Vec<ProductRow>,
    client: Option<&str>,
) -> ApiResult<Vec<ProductView>> {
    let ids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let mut attrs = attributes_for(conn, &ids).await?;
    let personal = personal_prices_for(conn, client, &ids).await?;
    Ok(products
        .into_iter()
        .map(|p| {
            let a = attrs.remove(&p.id).unwrap_or_default();
            let own = personal.get(&p.id).copied();
            let mut view = ProductView::from_row(p, a);
            view.personal_price = own.map(tiyn_to_string);
            view
        })
        .collect())
}

async fn list_products(
    actor: MaybeActor,
    State(db): State<Db>,
    Query(q): Query<ProductQuery>,
) -> ApiResult<Json<Vec<ProductView>>> {
    let conn = db.connect()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    let in_stock = q.in_stock.filter(|v| *v).map(|_| 1i64);
    let archived = actor.staff().is_some_and(|s| s.is(Role::Admin)) && q.include_archived == Some(true);
    let order = match q.sort.as_deref().unwrap_or("name") {
        "price_asc" => "price_tiyn, name",
        "price_desc" => "price_tiyn DESC, name",
        "new" => "created_at DESC, name",
        "name" => "name, IFNULL(variant_name, '')",
        other => return Err(bad(format!("сортировка: name, price_asc, price_desc или new, а не «{other}»"))),
    };

    let sql = format!(
        "SELECT {PRODUCT_COLS}
         FROM products
         WHERE (?7 = 1 OR is_archived = 0)
           AND (?1 IS NULL OR category_id = ?1)
           AND (?2 IS NULL OR name LIKE ?2 OR IFNULL(sku,'') LIKE ?2)
           AND (?3 IS NULL OR stock_milli - reserved_milli > 0)
           AND (?6 IS NULL OR group_id = ?6)
         ORDER BY {order}
         LIMIT ?4 OFFSET ?5"
    );
    let rows = conn
        .query(
            &sql,
            libsql::params![
                q.category_id.clone(),
                search,
                in_stock,
                limit,
                offset,
                q.group_id.clone(),
                archived as i64
            ],
        )
        .await?;
    let products: Vec<ProductRow> = collect(rows).await?;
    let client = price_client(&actor, q.client_id.as_ref());
    Ok(Json(to_views(&conn, products, client.as_deref()).await?))
}

async fn load_product(conn: &libsql::Connection, id: &str) -> ApiResult<ProductRow> {
    let rows = conn
        .query(&format!("SELECT {PRODUCT_COLS} FROM products WHERE id = ?1"), libsql::params![id])
        .await?;
    first(rows).await?.ok_or(ApiError::NotFound)
}

async fn get_product(
    actor: MaybeActor,
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(q): Query<ProductQuery>,
) -> ApiResult<Json<ProductView>> {
    let conn = db.connect()?;
    let row = load_product(&conn, &id).await?;
    if row.is_archived != 0 && actor.staff().is_none() {
        return Err(ApiError::NotFound);
    }
    let client = price_client(&actor, q.client_id.as_ref());
    let mut v = to_views(&conn, vec![row], client.as_deref()).await?;
    Ok(Json(v.remove(0)))
}

// ─── Управление товарами ─────────────────────────────────────────────────────

fn opt_tiyn(v: &Option<String>, what: &str) -> ApiResult<Option<i64>> {
    v.as_deref()
        .map(|s| parse_tiyn(s).map_err(|e| bad(format!("{what}: {e}"))))
        .transpose()
}

fn opt_milli(v: &Option<String>, what: &str) -> ApiResult<Option<i64>> {
    v.as_deref()
        .map(|s| parse_milli(s).map_err(|e| bad(format!("{what}: {e}"))))
        .transpose()
}

fn clean_attributes(list: &[Attribute]) -> ApiResult<Vec<Attribute>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for a in list {
        let (name, value) = (a.name.trim(), a.value.trim());
        if name.is_empty() && value.is_empty() {
            continue;
        }
        if name.is_empty() || value.is_empty() {
            return Err(bad("у характеристики нужны и название, и значение"));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(bad(format!("характеристика «{name}» указана дважды")));
        }
        out.push(Attribute { name: name.into(), value: value.into() });
    }
    Ok(out)
}

async fn write_attributes(conn: &libsql::Connection, product_id: &str, list: &[Attribute]) -> ApiResult<()> {
    conn.execute("DELETE FROM product_attributes WHERE product_id = ?1", libsql::params![product_id])
        .await?;
    for (i, a) in list.iter().enumerate() {
        conn.execute(
            "INSERT INTO product_attributes (id, product_id, name, value, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            libsql::params![new_id(), product_id, a.name.clone(), a.value.clone(), (i + 1) as i64],
        )
        .await?;
    }
    Ok(())
}

async fn ensure_category(conn: &libsql::Connection, id: &str) -> ApiResult<()> {
    let mut rows = conn.query("SELECT 1 FROM categories WHERE id = ?1", libsql::params![id]).await?;
    if rows.next().await?.is_none() {
        return Err(bad(format!("категория {id} не найдена")));
    }
    Ok(())
}

async fn ensure_sku_free(conn: &libsql::Connection, sku: &str, except: Option<&str>) -> ApiResult<()> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM products WHERE sku = ?1 AND (?2 IS NULL OR id <> ?2)",
            libsql::params![sku, except],
        )
        .await?;
    if rows.next().await?.is_some() {
        return Err(ApiError::Conflict(format!("артикул «{sku}» уже занят")));
    }
    Ok(())
}

fn check_order_limits(min: i64, step: i64) -> ApiResult<()> {
    if min <= 0 {
        return Err(bad("минимальный заказ должен быть больше нуля"));
    }
    if step <= 0 {
        return Err(bad("шаг заказа должен быть больше нуля"));
    }
    Ok(())
}

async fn create_product(
    staff: Staff,
    State(db): State<Db>,
    Json(b): Json<ProductInput>,
) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    let name = b.name.as_deref().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad("название товара обязательно"));
    }
    let category_id = b.category_id.clone().ok_or_else(|| bad("категория обязательна"))?;
    let price = opt_tiyn(&b.price, "цена")?.ok_or_else(|| bad("цена обязательна"))?;
    let min_order = opt_milli(&b.min_order, "минимальный заказ")?.unwrap_or(1000);
    let step = opt_milli(&b.order_step, "шаг заказа")?.unwrap_or(1000);
    check_order_limits(min_order, step)?;
    let min_stock = opt_milli(&b.min_stock, "минимальный остаток")?;
    let wholesale = opt_tiyn(&b.wholesale_price, "оптовая цена")?;
    let initial = opt_milli(&b.stock, "остаток")?.unwrap_or(0);
    let attrs = clean_attributes(b.attributes.as_deref().unwrap_or(&[]))?;
    let sku = b.sku.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    let conn = db.connect()?;
    ensure_category(&conn, &category_id).await?;
    if let Some(s) = &sku {
        ensure_sku_free(&conn, s, None).await?;
    }

    let id = new_id();
    let group = b.group_id.clone().filter(|g| !g.trim().is_empty()).unwrap_or_else(|| format!("g-{id}"));
    let ts = now();
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    tx.execute(
        "INSERT INTO products (id, category_id, name, sku, description, price_tiyn, unit, is_weight,
                               stock_milli, min_stock_milli, min_order_milli, order_step_milli,
                               group_id, variant_name, wholesale_price_tiyn, wholesale_min_qty,
                               storage_location, image_url, image_name, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?20)",
        libsql::params![
            id.clone(),
            category_id,
            name,
            sku,
            b.description.clone(),
            price,
            b.unit.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| "шт".into()),
            b.is_weight.unwrap_or(false) as i64,
            initial,
            min_stock,
            min_order,
            step,
            group,
            b.variant_name.clone(),
            wholesale,
            b.wholesale_min_qty,
            b.storage_location.clone(),
            b.image_url.clone(),
            b.image_name.clone(),
            ts.clone()
        ],
    )
    .await?;
    write_attributes(&tx, &id, &attrs).await?;
    if initial > 0 {
        tx.execute(
            "INSERT INTO stock_movements (id, product_id, movement_type, quantity_milli, reason,
                                          employee_id, storage_location, created_at)
             VALUES (?1, ?2, 'in', ?3, 'начальный остаток при добавлении товара', ?4, ?5, ?6)",
            libsql::params![new_id(), id.clone(), initial, staff.id.clone(), b.storage_location.clone(), ts],
        )
        .await?;
    }
    tx.commit().await?;

    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}

/// Правка товара. Цена меняется только для новых заказов: в старых
/// строках цена сохранена снимком (ФТ-Ц-05). Остаток здесь не меняется —
/// только через приход и корректировку, чтобы был след в журнале.
async fn update_product(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(b): Json<ProductInput>,
) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    if b.stock.is_some() {
        return Err(bad("остаток меняется приходом или корректировкой на складе, а не правкой товара"));
    }
    let conn = db.connect()?;
    let cur = load_product(&conn, &id).await?;
    if let Some(n) = &b.name
        && n.trim().is_empty()
    {
        return Err(bad("название не может быть пустым"));
    }
    if let Some(c) = &b.category_id {
        ensure_category(&conn, c).await?;
    }
    let sku = b.sku.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    if let Some(s) = &sku {
        ensure_sku_free(&conn, s, Some(&id)).await?;
    }
    let price = opt_tiyn(&b.price, "цена")?;
    let min_order = opt_milli(&b.min_order, "минимальный заказ")?.unwrap_or(cur.min_order_milli);
    let step = opt_milli(&b.order_step, "шаг заказа")?.unwrap_or(cur.order_step_milli);
    check_order_limits(min_order, step)?;
    let min_stock = opt_milli(&b.min_stock, "минимальный остаток")?;
    let wholesale = opt_tiyn(&b.wholesale_price, "оптовая цена")?;
    let attrs = b.attributes.as_deref().map(clean_attributes).transpose()?;

    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    tx.execute(
        "UPDATE products SET
            category_id = IFNULL(?1, category_id), name = IFNULL(?2, name), sku = IFNULL(?3, sku),
            description = IFNULL(?4, description), price_tiyn = IFNULL(?5, price_tiyn),
            unit = IFNULL(?6, unit), is_weight = IFNULL(?7, is_weight),
            min_order_milli = ?8, order_step_milli = ?9,
            min_stock_milli = IFNULL(?10, min_stock_milli), group_id = IFNULL(?11, group_id),
            variant_name = IFNULL(?12, variant_name),
            wholesale_price_tiyn = IFNULL(?13, wholesale_price_tiyn),
            wholesale_min_qty = IFNULL(?14, wholesale_min_qty),
            storage_location = IFNULL(?15, storage_location), image_url = IFNULL(?16, image_url),
            image_name = IFNULL(?17, image_name), updated_at = ?18
         WHERE id = ?19",
        libsql::params![
            b.category_id.clone(),
            b.name.as_deref().map(str::trim),
            sku,
            b.description.clone(),
            price,
            b.unit.clone(),
            b.is_weight.map(i64::from),
            min_order,
            step,
            min_stock,
            b.group_id.clone(),
            b.variant_name.clone(),
            wholesale,
            b.wholesale_min_qty,
            b.storage_location.clone(),
            b.image_url.clone(),
            b.image_name.clone(),
            now(),
            id.clone()
        ],
    )
    .await?;
    if let Some(a) = &attrs {
        write_attributes(&tx, &id, a).await?;
    }
    tx.commit().await?;

    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}

async fn set_archived(staff: Staff, db: Db, id: String, archived: bool) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    load_product(&conn, &id).await?;
    conn.execute(
        "UPDATE products SET is_archived = ?1, updated_at = ?2 WHERE id = ?3",
        libsql::params![archived as i64, now(), id.clone()],
    )
    .await?;
    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}

/// Снять с продажи: товар пропадает из каталога, старые заказы не меняются.
async fn archive_product(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<ProductView>> {
    set_archived(staff, db, id, true).await
}

async fn restore_product(staff: Staff, State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<ProductView>> {
    set_archived(staff, db, id, false).await
}

/// Удаление — только для ошибочно созданного товара без заказов и движений.
async fn delete_product(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    staff.require(ADMIN)?;
    let conn = db.connect()?;
    load_product(&conn, &id).await?;
    let mut rows = conn
        .query(
            "SELECT (SELECT COUNT(*) FROM order_items WHERE product_id = ?1)
                  + (SELECT COUNT(*) FROM stock_movements WHERE product_id = ?1
                       AND reason <> 'начальный остаток при добавлении товара')",
            libsql::params![id.clone()],
        )
        .await?;
    let history: i64 = rows.next().await?.map(|r| r.get(0)).transpose()?.unwrap_or(0);
    if history > 0 {
        return Err(ApiError::Conflict(
            "у товара есть заказы или движения по складу — снимите его с продажи".into(),
        ));
    }
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    tx.execute("DELETE FROM stock_movements WHERE product_id = ?1", libsql::params![id.clone()]).await?;
    tx.execute("DELETE FROM client_prices WHERE product_id = ?1", libsql::params![id.clone()]).await?;
    tx.execute("DELETE FROM product_attributes WHERE product_id = ?1", libsql::params![id.clone()]).await?;
    tx.execute("DELETE FROM products WHERE id = ?1", libsql::params![id]).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn replace_attributes(
    staff: Staff,
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(list): Json<Vec<Attribute>>,
) -> ApiResult<Json<ProductView>> {
    staff.require(ADMIN)?;
    let attrs = clean_attributes(&list)?;
    let conn = db.connect()?;
    load_product(&conn, &id).await?;
    let tx = conn.transaction_with_behavior(libsql::TransactionBehavior::Immediate).await?;
    write_attributes(&tx, &id, &attrs).await?;
    tx.execute("UPDATE products SET updated_at = ?1 WHERE id = ?2", libsql::params![now(), id.clone()])
        .await?;
    tx.commit().await?;
    let row = load_product(&conn, &id).await?;
    Ok(Json(to_views(&conn, vec![row], None).await?.remove(0)))
}

#[derive(Serialize, serde::Deserialize)]
struct AttributeName {
    name: String,
    products: i64,
}

/// Уже заведённые названия характеристик — подсказки в форме товара,
/// чтобы «Цвет» и «цвет» не стали двумя разными.
async fn attribute_names(staff: Staff, State(db): State<Db>) -> ApiResult<Json<Vec<AttributeName>>> {
    staff.require(ANY_STAFF)?;
    let conn = db.connect()?;
    let rows = conn
        .query(
            "SELECT name, COUNT(DISTINCT product_id) AS products FROM product_attributes
             GROUP BY name ORDER BY products DESC, name",
            (),
        )
        .await?;
    Ok(Json(collect(rows).await?))
}
