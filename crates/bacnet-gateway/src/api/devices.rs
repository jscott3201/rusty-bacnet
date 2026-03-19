//! Device discovery and info endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::state::GatewayState;

use super::types::{decode_raw_property_to_json, property_name, ApiError};

/// POST /api/v1/devices/discover
#[derive(Debug, Deserialize)]
pub struct DiscoverRequest {
    pub low_instance: Option<u32>,
    pub high_instance: Option<u32>,
    pub timeout_seconds: Option<u64>,
}

pub async fn discover_devices(
    State(state): State<GatewayState>,
    body: Option<Json<DiscoverRequest>>,
) -> impl IntoResponse {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::internal(msg)),
            )
                .into_response()
        }
    };

    let (low, high) = match &body {
        Some(Json(req)) => (req.low_instance, req.high_instance),
        None => (None, None),
    };
    const MAX_DISCOVER_TIMEOUT_SECS: u64 = 30;
    let timeout = body
        .as_ref()
        .and_then(|b| b.timeout_seconds)
        .unwrap_or(3)
        .min(MAX_DISCOVER_TIMEOUT_SECS);

    if let Err(e) = client.who_is(low, high).await {
        let (status, err) = ApiError::from_bacnet_error(&e);
        return (status, Json(err)).into_response();
    }

    tokio::time::sleep(std::time::Duration::from_secs(timeout)).await;

    list_known_devices_inner(&state).await.into_response()
}

/// GET /api/v1/devices
pub async fn list_devices(State(state): State<GatewayState>) -> impl IntoResponse {
    list_known_devices_inner(&state).await
}

async fn list_known_devices_inner(state: &GatewayState) -> impl IntoResponse {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(_) => {
            return Json(serde_json::json!({ "devices": [] })).into_response();
        }
    };

    let devices = client.discovered_devices().await;
    let entries: Vec<serde_json::Value> = devices
        .iter()
        .map(|dev| {
            serde_json::json!({
                "instance": dev.object_identifier.instance_number(),
                "mac": format!("{:02x?}", dev.mac_address.as_slice()),
                "vendor_id": dev.vendor_id,
                "max_apdu_length": dev.max_apdu_length,
                "network": dev.source_network,
            })
        })
        .collect();

    Json(serde_json::json!({ "devices": entries })).into_response()
}

/// GET /api/v1/devices/{instance}
pub async fn get_device_info(
    State(state): State<GatewayState>,
    Path(instance): Path<u32>,
) -> impl IntoResponse {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::internal(msg)),
            )
                .into_response()
        }
    };

    let entry = match state.resolve_device(instance).await {
        Ok(e) => e,
        Err(msg) => return ApiError::not_found(msg).into_response(),
    };

    let device_oid = match ObjectIdentifier::new(ObjectType::DEVICE, instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let props = [
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::VENDOR_NAME,
        PropertyIdentifier::VENDOR_IDENTIFIER,
        PropertyIdentifier::MODEL_NAME,
        PropertyIdentifier::FIRMWARE_REVISION,
        PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
        PropertyIdentifier::PROTOCOL_VERSION,
        PropertyIdentifier::PROTOCOL_REVISION,
    ];

    let mut info = serde_json::json!({
        "instance": instance,
        "mac": format!("{:02x?}", entry.mac_address.as_slice()),
        "vendor_id": entry.vendor_id,
    });

    for prop in props {
        if let Ok(ack) = client
            .read_property(&entry.mac_address, device_oid, prop, None)
            .await
        {
            let prop_name = property_name(prop);
            info[&prop_name] = decode_raw_property_to_json(&ack.property_value);
        }
    }

    Json(info).into_response()
}
