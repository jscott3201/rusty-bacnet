//! Pluggable authentication middleware.

pub mod bearer;

use axum::http::{HeaderMap, StatusCode};

/// Error returned when authentication fails.
#[derive(Debug, Clone)]
pub struct AuthError {
    pub status: StatusCode,
    pub message: String,
}

/// Trait for pluggable authentication.
///
/// Implementors validate incoming requests by inspecting headers.
/// The gateway ships with [`bearer::BearerTokenAuth`] as the default.
pub trait Authenticator: Send + Sync + 'static {
    /// Validate request headers. Returns `Ok(())` to allow, `Err` to reject.
    fn authenticate(&self, headers: &HeaderMap) -> Result<(), AuthError>;
}
