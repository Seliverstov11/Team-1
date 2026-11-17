pub mod catalog;
pub mod clients;
pub mod dashboard;
pub mod employees;
pub mod invoice;
pub mod me;
pub mod orders;
pub mod photos;
pub mod warehouse;

use axum::Router;
use serde::de::DeserializeOwned;

use crate::{
    db::Db,
    error::{ApiError, ApiResult, bad},
};

pub fn router() -> Router<Db> {
    Router::new()
        .nest("/auth", crate::auth::router())
        .nest("/catalog", catalog::router().merge(photos::router()))
        .nest("/orders", orders::router())
        .nest("/warehouse", warehouse::router())
        .nest("/clients", clients::router())
        .nest("/employees", employees::router())
        .nest("/me", me::router())
        .merge(dashboard::router())
}

fn decode<T: DeserializeOwned>(row: &libsql::Row) -> ApiResult<T> {
    libsql::de::from_row::<T>(row)
        .map_err(|e| ApiError::Other(anyhow::anyhow!("разбор строки БД: {e}")))
}

pub async fn collect<T: DeserializeOwned>(mut rows: libsql::Rows) -> ApiResult<Vec<T>> {
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(decode(&row)?);
    }
    Ok(out)
}

pub async fn first<T: DeserializeOwned>(mut rows: libsql::Rows) -> ApiResult<Option<T>> {
    match rows.next().await? {
        Some(row) => Ok(Some(decode(&row)?)),
        None => Ok(None),
    }
}

pub fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Последние 10 цифр телефона: «+7 (701) 123-45-67» и «87011234567»
/// — один и тот же клиент.
pub fn phone_norm(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 10 {
        return None;
    }
    Some(digits[digits.len() - 10..].to_string())
}

/// «?2, ?3, ?4» для IN (...) начиная с номера `from`.
pub fn placeholders(from: usize, n: usize) -> String {
    (from..from + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

/// Магазин в Казахстане, единое время UTC+5. «Сегодня» и «неделя»
/// считаются по местному времени, а в базе время хранится в UTC.
const LOCAL_OFFSET_HOURS: i64 = 5;

pub struct Period {
    pub key: &'static str,
    /// Начало периода в UTC, None — за всё время.
    pub from: Option<String>,
}

pub fn period(p: Option<&str>) -> ApiResult<Period> {
    use chrono::{Datelike, Duration, NaiveTime};
    let local_now = chrono::Utc::now().naive_utc() + Duration::hours(LOCAL_OFFSET_HOURS);
    let today = local_now.date();
    let (key, start) = match p.unwrap_or("week") {
        "today" => ("today", Some(today)),
        "week" => (
            "week",
            Some(today - Duration::days(today.weekday().num_days_from_monday() as i64)),
        ),
        "month" => ("month", today.with_day(1)),
        "all" => ("all", None),
        other => return Err(bad(format!("период: today, week, month или all, а не «{other}»"))),
    };
    let from = start.map(|d| {
        (d.and_time(NaiveTime::MIN) - Duration::hours(LOCAL_OFFSET_HOURS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    });
    Ok(Period { key, from })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn телефон_нормализуется() {
        assert_eq!(phone_norm("+7 (701) 123-45-67").as_deref(), Some("7011234567"));
        assert_eq!(phone_norm("87011234567").as_deref(), Some("7011234567"));
        assert_eq!(phone_norm("12345"), None);
    }

    #[test]
    fn периоды() {
        assert!(period(Some("today")).unwrap().from.is_some());
        assert!(period(Some("all")).unwrap().from.is_none());
        assert!(period(Some("год")).is_err());
    }
}
