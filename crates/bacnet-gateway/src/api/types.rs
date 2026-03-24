//! HTTP-specific JSON request/response types for the REST API.
//!
//! Parsing and formatting utilities live in [`crate::parse`] (always available).
//! This module adds HTTP status codes and Axum response types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

// Re-export parsing utilities so existing API handlers don't need to change imports.
pub use crate::parse::*;

/// Standard API error response.
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub class: String,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                class: "object".to_string(),
                code: "unknown-object".to_string(),
                message: message.into(),
            },
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                class: "services".to_string(),
                code: "invalid-parameter".to_string(),
                message: message.into(),
            },
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                class: "device".to_string(),
                code: "internal-error".to_string(),
                message: message.into(),
            },
        }
    }

    pub fn from_bacnet_error(err: &bacnet_types::error::Error) -> (StatusCode, Self) {
        let msg = format!("{err}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Self {
                error: ApiErrorBody {
                    class: "protocol".to_string(),
                    code: "error".to_string(),
                    message: msg,
                },
            },
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.error.code.as_str() {
            "unknown-object" => StatusCode::NOT_FOUND,
            "invalid-parameter" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, axum::Json(self)).into_response()
    }
}

/// Request body for writing a property value.
#[derive(Debug, Deserialize)]
pub struct WritePropertyRequest {
    pub value: serde_json::Value,
    pub priority: Option<u8>,
    pub index: Option<u32>,
}
