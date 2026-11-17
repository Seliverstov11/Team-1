pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod photo_services;
pub mod num;
pub mod pricing;
pub mod routes;

use axum::{Json, Router, routing::get};
use serde_json::json;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

/// Всё приложение целиком. Отдельно от main, чтобы тесты поднимали
/// тот же роутер на временной БД.
pub fn app(db: db::Db) -> Router {
    Router::new()
        .route("/health", get(health))
        .nest("/api", routes::router())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(db)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}
