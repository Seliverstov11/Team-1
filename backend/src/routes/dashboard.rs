//! Дашборд и аналитика для владельца (ФТ-В-01…04).
//!
//! Дашборд — главный экран CRM: что происходит сегодня, за неделю, за
//! месяц. Аналитика — отдельный раздел: откуда заказы, где теряются
//! клиенты, что популярно, с каких устройств заходят на сайт.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use super::{new_id, now, period};
use crate::{
    auth::{ADMIN, MaybeActor, OFFICE, Staff},
    db::Db,
    error::ApiResult,
    num::{milli_to_string, tiyn_to_string},
};

pub fn router() -> Router<Db> {
    Router::new()
        .route("/dashboard", get(dashboard))
        .route("/analytics", get(analytics))
        .route("/analytics/visit", post(visit))
}

#[derive(Deserialize)]
struct PeriodQuery {
    /// today | week | month | all
    period: Option<String>,
}

#[derive(Serialize)]
struct CountSum {
    count: i64,
    sum: String,
}

#[derive(Serialize)]
struct Named {
    key: String,
    count: i64,
    sum: String,
}

#[derive(Serialize)]
struct LowItem {
    product_id: String,
    name: String,
    available: String,
    min_stock: Option<String>,
    unit: String,
}

#[derive(Serialize)]
struct TopItem {
    name: String,
    quantity: String,
    orders: i64,
    sum: String,
}

#[derive(Serialize)]
struct Dashboard {
    period: &'static str,
    from: Option<String>,
    /// Заказы за период без отменённых.
    orders: CountSum,
    /// Сейчас ждут подтверждения.
    new_orders: i64,
    /// Действующие заказы с долгом: сколько и на какую сумму, доля от заказов.
    awaiting_payment: CountSum,
    awaiting_payment_share: f64,
    /// Получено денег за период по отметкам менеджеров (минус возвраты).
    payments_received: String,
    urgent: i64,
    problem: i64,
    /// Позиции на исходе — с переходом в список закупки (ФТ-В-01).
    low_stock_count: i64,
    out_of_stock_count: i64,
    low_stock: Vec<LowItem>,
    by_status: Vec<Named>,
    by_channel: Vec<Named>,
    top_products: Vec<TopItem>,
}

/// ?1 — начало периода; запросы без периода параметров не получают.
fn bind(sql: &str, from: &Option<String>) -> Vec<libsql::Value> {
    if sql.contains("?1") {
        vec![from.clone().map(libsql::Value::from).unwrap_or(libsql::Value::Null)]
    } else {
        vec![]
    }
}

async fn count_sum(conn: &libsql::Connection, sql: &str, from: &Option<String>) -> ApiResult<CountSum> {
    let mut r = conn.query(sql, bind(sql, from)).await?;
    let row = r.next().await?;
    Ok(match row {
        Some(row) => CountSum { count: row.get(0)?, sum: tiyn_to_string(row.get::<Option<i64>>(1)?.unwrap_or(0)) },
        None => CountSum { count: 0, sum: tiyn_to_string(0) },
    })
}

async fn grouped(conn: &libsql::Connection, sql: &str, from: &Option<String>) -> ApiResult<Vec<Named>> {
    let mut r = conn.query(sql, bind(sql, from)).await?;
    let mut out = Vec::new();
    while let Some(row) = r.next().await? {
        out.push(Named {
            key: row.get::<Option<String>>(0)?.unwrap_or_else(|| "—".into()),
            count: row.get(1)?,
            sum: tiyn_to_string(row.get::<Option<i64>>(2)?.unwrap_or(0)),
        });
    }
    Ok(out)
}

async fn scalar(conn: &libsql::Connection, sql: &str, from: &Option<String>) -> ApiResult<i64> {
    let mut r = conn.query(sql, bind(sql, from)).await?;
    Ok(r.next().await?.map(|r| r.get::<Option<i64>>(0)).transpose()?.flatten().unwrap_or(0))
}

async fn top_products(conn: &libsql::Connection, from: &Option<String>, limit: i64) -> ApiResult<Vec<TopItem>> {
    let mut r = conn
        .query(
            "SELECT i.product_name, SUM(i.quantity_milli), COUNT(DISTINCT i.order_id), SUM(i.subtotal_tiyn)
             FROM order_items i JOIN orders o ON o.id = i.order_id
             WHERE o.status <> 'cancelled' AND (?1 IS NULL OR o.created_at >= ?1)
             GROUP BY i.product_name ORDER BY SUM(i.subtotal_tiyn) DESC LIMIT ?2",
            libsql::params![from.clone(), limit],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = r.next().await? {
        out.push(TopItem {
            name: row.get(0)?,
            quantity: milli_to_string(row.get(1)?),
            orders: row.get(2)?,
            sum: tiyn_to_string(row.get(3)?),
        });
    }
    Ok(out)
}

async fn dashboard(staff: Staff, State(db): State<Db>, Query(q): Query<PeriodQuery>) -> ApiResult<Json<Dashboard>> {
    staff.require(OFFICE)?;
    let p = period(q.period.as_deref())?;
    let from = p.from.clone();
    let conn = db.connect()?;

    let orders = count_sum(
        &conn,
        "SELECT COUNT(*), SUM(total_tiyn) FROM orders WHERE status <> 'cancelled' AND (?1 IS NULL OR created_at >= ?1)",
        &from,
    )
    .await?;
    let awaiting = count_sum(
        &conn,
        "SELECT COUNT(*), SUM(total_tiyn - paid_tiyn) FROM orders
         WHERE status <> 'cancelled' AND payment_status IN ('unpaid', 'partial')",
        &None,
    )
    .await?;
    let active = scalar(&conn, "SELECT COUNT(*) FROM orders WHERE status <> 'cancelled'", &None).await?;
    let received = scalar(
        &conn,
        "SELECT SUM(amount_tiyn) FROM order_events WHERE kind = 'payment' AND (?1 IS NULL OR created_at >= ?1)",
        &from,
    )
    .await?;

    let mut low = Vec::new();
    let mut r = conn
        .query(
            "SELECT id, name || IFNULL(', ' || variant_name, ''), stock_milli - reserved_milli, min_stock_milli, unit
             FROM products WHERE is_archived = 0 AND min_stock_milli IS NOT NULL
               AND stock_milli - reserved_milli < min_stock_milli
             ORDER BY (stock_milli - reserved_milli) * 1.0 / min_stock_milli LIMIT 10",
            (),
        )
        .await?;
    while let Some(row) = r.next().await? {
        low.push(LowItem {
            product_id: row.get(0)?,
            name: row.get(1)?,
            available: milli_to_string(row.get::<i64>(2)?.max(0)),
            min_stock: row.get::<Option<i64>>(3)?.map(milli_to_string),
            unit: row.get(4)?,
        });
    }

    Ok(Json(Dashboard {
        period: p.key,
        from: from.clone(),
        new_orders: scalar(&conn, "SELECT COUNT(*) FROM orders WHERE status = 'new'", &None).await?,
        awaiting_payment_share: if active > 0 { (awaiting.count as f64 / active as f64 * 100.0).round() / 100.0 } else { 0.0 },
        awaiting_payment: awaiting,
        payments_received: tiyn_to_string(received),
        urgent: scalar(
            &conn,
            "SELECT COUNT(*) FROM orders WHERE is_urgent = 1 AND status NOT IN ('completed', 'cancelled')",
            &None,
        )
        .await?,
        problem: scalar(
            &conn,
            "SELECT COUNT(*) FROM orders WHERE is_problem = 1 AND status NOT IN ('completed', 'cancelled')",
            &None,
        )
        .await?,
        low_stock_count: scalar(
            &conn,
            "SELECT COUNT(*) FROM products WHERE is_archived = 0 AND min_stock_milli IS NOT NULL
               AND stock_milli - reserved_milli < min_stock_milli",
            &None,
        )
        .await?,
        out_of_stock_count: scalar(
            &conn,
            "SELECT COUNT(*) FROM products WHERE is_archived = 0 AND stock_milli - reserved_milli <= 0",
            &None,
        )
        .await?,
        low_stock: low,
        by_status: grouped(
            &conn,
            "SELECT status, COUNT(*), SUM(total_tiyn) FROM orders WHERE (?1 IS NULL OR created_at >= ?1) GROUP BY status",
            &from,
        )
        .await?,
        by_channel: grouped(
            &conn,
            "SELECT channel, COUNT(*), SUM(total_tiyn) FROM orders
             WHERE status <> 'cancelled' AND (?1 IS NULL OR created_at >= ?1) GROUP BY channel ORDER BY COUNT(*) DESC",
            &from,
        )
        .await?,
        top_products: top_products(&conn, &from, 5).await?,
        orders,
    }))
}

// ─── Аналитика ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ClientLogin {
    id: String,
    name: String,
    last_login_at: String,
}

#[derive(Serialize)]
struct Analytics {
    period: &'static str,
    from: Option<String>,
    /// Откуда приходят заказы (ФТ-В-04).
    orders_by_channel: Vec<Named>,
    orders_by_city: Vec<Named>,
    orders_by_client_type: Vec<Named>,
    /// Где теряются клиенты: отменённые заказы и их причины.
    cancelled: CountSum,
    cancel_reasons: Vec<Named>,
    /// Популярные товары и категории.
    top_products: Vec<TopItem>,
    top_categories: Vec<Named>,
    /// Посещения сайта: устройства, браузеры, системы.
    visits: i64,
    visitors: i64,
    by_device: Vec<Named>,
    by_browser: Vec<Named>,
    by_os: Vec<Named>,
    /// Заказы с сайта на 100 посетителей.
    site_conversion: f64,
    new_clients: i64,
    repeat_clients: i64,
    /// Последний вход покупателей.
    last_logins: Vec<ClientLogin>,
}

async fn analytics(staff: Staff, State(db): State<Db>, Query(q): Query<PeriodQuery>) -> ApiResult<Json<Analytics>> {
    staff.require(ADMIN)?;
    let p = period(q.period.as_deref())?;
    let from = p.from.clone();
    let conn = db.connect()?;
    const IN_PERIOD: &str = "(?1 IS NULL OR o.created_at >= ?1)";

    let visits = scalar(&conn, "SELECT COUNT(*) FROM visits WHERE (?1 IS NULL OR created_at >= ?1)", &from).await?;
    let visitors = scalar(
        &conn,
        "SELECT COUNT(DISTINCT IFNULL(session_id, id)) FROM visits WHERE (?1 IS NULL OR created_at >= ?1)",
        &from,
    )
    .await?;
    let site_orders = scalar(
        &conn,
        &format!("SELECT COUNT(*) FROM orders o WHERE o.channel = 'site' AND {IN_PERIOD}"),
        &from,
    )
    .await?;

    let mut last = Vec::new();
    let mut r = conn
        .query(
            "SELECT id, name, last_login_at FROM clients WHERE last_login_at IS NOT NULL ORDER BY last_login_at DESC LIMIT 20",
            (),
        )
        .await?;
    while let Some(row) = r.next().await? {
        last.push(ClientLogin { id: row.get(0)?, name: row.get(1)?, last_login_at: row.get(2)? });
    }

    let visits_by = |col: &'static str| {
        format!("SELECT {col}, COUNT(*), 0 FROM visits WHERE (?1 IS NULL OR created_at >= ?1) GROUP BY {col} ORDER BY COUNT(*) DESC")
    };

    Ok(Json(Analytics {
        period: p.key,
        from: from.clone(),
        orders_by_channel: grouped(
            &conn,
            &format!("SELECT o.channel, COUNT(*), SUM(o.total_tiyn) FROM orders o WHERE o.status <> 'cancelled' AND {IN_PERIOD} GROUP BY o.channel ORDER BY COUNT(*) DESC"),
            &from,
        )
        .await?,
        orders_by_city: grouped(
            &conn,
            &format!("SELECT o.city, COUNT(*), SUM(o.total_tiyn) FROM orders o WHERE o.status <> 'cancelled' AND {IN_PERIOD} GROUP BY o.city ORDER BY COUNT(*) DESC"),
            &from,
        )
        .await?,
        orders_by_client_type: grouped(
            &conn,
            &format!("SELECT c.client_type, COUNT(*), SUM(o.total_tiyn) FROM orders o LEFT JOIN clients c ON c.id = o.client_id
                      WHERE o.status <> 'cancelled' AND {IN_PERIOD} GROUP BY c.client_type ORDER BY COUNT(*) DESC"),
            &from,
        )
        .await?,
        cancelled: count_sum(&conn, &format!("SELECT COUNT(*), SUM(o.total_tiyn) FROM orders o WHERE o.status = 'cancelled' AND {IN_PERIOD}"), &from).await?,
        cancel_reasons: grouped(
            &conn,
            &format!("SELECT IFNULL(o.cancelled_reason, 'причина не указана'), COUNT(*), SUM(o.total_tiyn) FROM orders o
                      WHERE o.status = 'cancelled' AND {IN_PERIOD} GROUP BY 1 ORDER BY COUNT(*) DESC"),
            &from,
        )
        .await?,
        top_products: top_products(&conn, &from, 10).await?,
        top_categories: grouped(
            &conn,
            &format!("SELECT c.name, COUNT(DISTINCT o.id), SUM(i.subtotal_tiyn)
                      FROM order_items i JOIN orders o ON o.id = i.order_id
                      JOIN products p ON p.id = i.product_id JOIN categories c ON c.id = p.category_id
                      WHERE o.status <> 'cancelled' AND {IN_PERIOD} GROUP BY c.name ORDER BY SUM(i.subtotal_tiyn) DESC LIMIT 10"),
            &from,
        )
        .await?,
        visits,
        visitors,
        by_device: grouped(&conn, &visits_by("device"), &from).await?,
        by_browser: grouped(&conn, &visits_by("browser"), &from).await?,
        by_os: grouped(&conn, &visits_by("os"), &from).await?,
        site_conversion: if visitors > 0 { (site_orders as f64 / visitors as f64 * 10000.0).round() / 100.0 } else { 0.0 },
        new_clients: scalar(&conn, "SELECT COUNT(*) FROM clients WHERE (?1 IS NULL OR created_at >= ?1)", &from).await?,
        repeat_clients: scalar(
            &conn,
            "SELECT COUNT(*) FROM (SELECT client_id FROM orders WHERE status <> 'cancelled' AND client_id IS NOT NULL
               GROUP BY client_id HAVING COUNT(*) > 1)",
            &None,
        )
        .await?,
        last_logins: last,
    }))
}

// ─── Посещения ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Visit {
    path: Option<String>,
    referrer: Option<String>,
    /// Случайный идентификатор вкладки с сайта — чтобы считать посетителей.
    session_id: Option<String>,
}

/// Грубое определение по User-Agent: точности до модели телефона не нужно,
/// нужно понять, куда делать акцент — мобильный или desktop (В3 02:15).
pub fn parse_user_agent(ua: &str) -> (&'static str, &'static str, &'static str) {
    let l = ua.to_lowercase();
    let device = if l.contains("ipad") || l.contains("tablet") || (l.contains("android") && !l.contains("mobile")) {
        "tablet"
    } else if l.contains("mobi") || l.contains("iphone") || l.contains("android") {
        "mobile"
    } else {
        "desktop"
    };
    let browser = if l.contains("yabrowser") {
        "Яндекс Браузер"
    } else if l.contains("edg/") {
        "Edge"
    } else if l.contains("opr/") || l.contains("opera") {
        "Opera"
    } else if l.contains("firefox") || l.contains("fxios") {
        "Firefox"
    } else if l.contains("chrome") || l.contains("crios") {
        "Chrome"
    } else if l.contains("safari") {
        "Safari"
    } else {
        "другой"
    };
    let os = if l.contains("android") {
        "Android"
    } else if l.contains("iphone") || l.contains("ipad") || l.contains("ios") {
        "iOS"
    } else if l.contains("windows") {
        "Windows"
    } else if l.contains("mac os") || l.contains("macintosh") {
        "macOS"
    } else if l.contains("linux") {
        "Linux"
    } else {
        "другая"
    };
    (device, browser, os)
}

/// Сайт сообщает о просмотре страницы. Без IP и без cookies:
/// хранятся только тип устройства, браузер и система.
async fn visit(actor: MaybeActor, State(db): State<Db>, headers: HeaderMap, Json(v): Json<Visit>) -> ApiResult<Json<serde_json::Value>> {
    let ua = headers.get(axum::http::header::USER_AGENT).and_then(|h| h.to_str().ok()).unwrap_or("");
    let (device, browser, os) = parse_user_agent(ua);
    let clip = |s: Option<String>, n: usize| s.map(|s| s.chars().take(n).collect::<String>());
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO visits (id, created_at, path, referrer, device, browser, os, session_id, client_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            new_id(),
            now(),
            clip(v.path, 200),
            clip(v.referrer, 200),
            device,
            browser,
            os,
            clip(v.session_id, 64),
            actor.customer().map(|c| c.id.clone())
        ],
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::parse_user_agent;

    #[test]
    fn устройства_и_браузеры() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1";
        assert_eq!(parse_user_agent(iphone), ("mobile", "Safari", "iOS"));
        let android = "Mozilla/5.0 (Linux; Android 14; SM-A546E) AppleWebKit/537.36 Chrome/120.0 Mobile Safari/537.36";
        assert_eq!(parse_user_agent(android), ("mobile", "Chrome", "Android"));
        let win = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36 Edg/120.0";
        assert_eq!(parse_user_agent(win), ("desktop", "Edge", "Windows"));
    }
}
