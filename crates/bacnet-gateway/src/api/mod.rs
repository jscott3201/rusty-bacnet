//! REST API routes under /api/v1/.

pub mod devices;
pub mod diagnostics;
pub mod objects;
pub mod properties;
pub mod types;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::auth::Authenticator;
use crate::state::GatewayState;

/// Build the complete API router.
///
/// If an authenticator is provided, all routes except /health are protected.
pub fn api_router(state: GatewayState, auth: Option<Box<dyn Authenticator>>) -> Router {
    // Health endpoint — always unprotected.
    let health_route = Router::new().route("/health", get(diagnostics::health));

    // Protected routes.
    let protected = Router::new()
        // Local objects.
        .route(
            "/objects",
            get(objects::list_objects).post(objects::create_object),
        )
        .route(
            "/objects/{specifier}",
            get(objects::get_object).delete(objects::delete_object),
        )
        .route(
            "/objects/{specifier}/properties/{property}",
            get(objects::get_object_property).put(objects::put_object_property),
        )
        // Device discovery.
        .route("/devices", get(devices::list_devices))
        .route("/devices/discover", post(devices::discover_devices))
        .route("/devices/{instance}", get(devices::get_device_info))
        // Remote property access.
        .route(
            "/devices/{instance}/objects/{specifier}/properties/{property}",
            get(properties::read_remote_property).put(properties::write_remote_property),
        );

    // Apply auth middleware if configured.
    let protected = if let Some(authenticator) = auth {
        let authenticator = std::sync::Arc::new(authenticator);
        protected.layer(middleware::from_fn(move |req: Request, next: Next| {
            let auth = authenticator.clone();
            async move {
                let result = auth.authenticate(req.headers());
                match result {
                    Ok(()) => next.run(req).await,
                    Err(e) => {
                        (e.status, Json(serde_json::json!({ "error": e.message }))).into_response()
                    }
                }
            }
        }))
    } else {
        protected
    };

    Router::new()
        .nest("/api/v1", health_route.merge(protected))
        .with_state(state)
}
