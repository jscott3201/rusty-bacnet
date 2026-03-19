//! Bearer token authentication.

use axum::http::{HeaderMap, StatusCode};
use subtle::ConstantTimeEq;

use super::{AuthError, Authenticator};

/// Bearer token authenticator.
///
/// Validates `Authorization: Bearer <token>` headers using constant-time
/// comparison to prevent timing attacks.
pub struct BearerTokenAuth {
    token_bytes: Vec<u8>,
}

impl BearerTokenAuth {
    /// Create a new bearer token authenticator.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token_bytes: token.into().into_bytes(),
        }
    }
}

impl Authenticator for BearerTokenAuth {
    fn authenticate(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        let header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AuthError {
                status: StatusCode::UNAUTHORIZED,
                message: "missing Authorization header".to_string(),
            })?;

        let token = header.strip_prefix("Bearer ").ok_or_else(|| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid Authorization header format, expected: Bearer <token>".to_string(),
        })?;

        let token_bytes = token.as_bytes();

        // subtle::ct_eq returns false for different-length slices without
        // constant-time guarantees on the length check itself. Token length
        // is observable via timing, which is acceptable for bearer tokens
        // where the token space is large enough that length alone is not
        // exploitable.
        let is_valid: bool = token_bytes.ct_eq(&self.token_bytes).into();

        if is_valid {
            Ok(())
        } else {
            Err(AuthError {
                status: StatusCode::UNAUTHORIZED,
                message: "invalid token".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_auth(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", value.parse().unwrap());
        headers
    }

    #[test]
    fn valid_token() {
        let auth = BearerTokenAuth::new("secret-key-123");
        let headers = headers_with_auth("Bearer secret-key-123");
        assert!(auth.authenticate(&headers).is_ok());
    }

    #[test]
    fn invalid_token() {
        let auth = BearerTokenAuth::new("secret-key-123");
        let headers = headers_with_auth("Bearer wrong-key");
        let err = auth.authenticate(&headers).unwrap_err();
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert!(err.message.contains("invalid token"));
    }

    #[test]
    fn missing_header() {
        let auth = BearerTokenAuth::new("secret-key-123");
        let headers = HeaderMap::new();
        let err = auth.authenticate(&headers).unwrap_err();
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert!(err.message.contains("missing"));
    }

    #[test]
    fn wrong_scheme() {
        let auth = BearerTokenAuth::new("secret-key-123");
        let headers = headers_with_auth("Basic dXNlcjpwYXNz");
        let err = auth.authenticate(&headers).unwrap_err();
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert!(err.message.contains("format"));
    }

    #[test]
    fn empty_bearer_token() {
        let auth = BearerTokenAuth::new("secret-key-123");
        let headers = headers_with_auth("Bearer ");
        let err = auth.authenticate(&headers).unwrap_err();
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    }
}
