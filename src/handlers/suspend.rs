use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use crate::suspend::suspend_system;

pub async fn suspend_handler() -> (StatusCode, Json<Value>) {
    match suspend_system().await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "message": "System suspend requested"
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "status": "error",
                "error": format!("Failed to suspend: {error}")
            })),
        ),
    }
}
