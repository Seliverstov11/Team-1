use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("не найдено")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    /// Нет входа или сессия истекла.
    #[error("{0}")]
    Unauthorized(String),
    /// Вход есть, но роли не хватает прав.
    #[error("{0}")]
    Forbidden(String),
    /// Действие противоречит состоянию: отмена оплаченного заказа,
    /// удаление товара с историей, занятый логин.
    #[error("{0}")]
    Conflict(String),
    /// Слишком много неудачных попыток входа подряд.
    #[error("{0}")]
    TooManyRequests(String),
    #[error(transparent)]
    Db(#[from] libsql::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            ApiError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m.clone()),
            ApiError::Db(e) => {
                tracing::error!("ошибка БД: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "ошибка базы данных".into())
            }
            ApiError::Other(e) => {
                tracing::error!("внутренняя ошибка: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "внутренняя ошибка".into())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::BadRequest(msg.into())
}
