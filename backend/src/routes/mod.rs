pub mod catalog;
pub mod orders;
pub mod warehouse;

use axum::Router;
use serde::de::DeserializeOwned;

use crate::{db::Db, error::ApiResult};

pub fn router() -> Router<Db> {
    Router::new()
        .nest("/catalog", catalog::router())
        .nest("/orders", orders::router())
        .nest("/warehouse", warehouse::router())
}

fn decode<T: DeserializeOwned>(row: &libsql::Row) -> ApiResult<T> {
    libsql::de::from_row::<T>(row)
        .map_err(|e| crate::error::ApiError::Other(anyhow::anyhow!("разбор строки БД: {e}")))
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
