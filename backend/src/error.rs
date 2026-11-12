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
