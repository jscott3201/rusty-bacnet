//! Diagnostics endpoints: health check, transport stats.

use axum::response::IntoResponse;
use axum::Json;

/// GET /api/v1/health
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}
