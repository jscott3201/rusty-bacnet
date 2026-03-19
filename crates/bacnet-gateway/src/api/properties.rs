//! Remote device property read/write endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use bacnet_types::primitives::ObjectIdentifier;

use crate::state::GatewayState;

use super::types::{
    decode_raw_property_to_json, json_to_property_value, object_type_name, parse_object_specifier,
    parse_property_name, property_name, ApiError, WritePropertyRequest,
};

/// GET /api/v1/devices/{instance}/objects/{specifier}/properties/{property}
pub async fn read_remote_property(
    State(state): State<GatewayState>,
    Path((instance, specifier, prop_str)): Path<(u32, String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
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

    let (obj_type, obj_instance) = match parse_object_specifier(&specifier) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let property = match parse_property_name(&prop_str) {
        Ok(p) => p,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let array_index = params.get("index").and_then(|s| s.parse::<u32>().ok());

    let oid = match ObjectIdentifier::new(obj_type, obj_instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let entry = match state.resolve_device(instance).await {
        Ok(e) => e,
        Err(msg) => return ApiError::not_found(msg).into_response(),
    };

    match client
        .read_property(&entry.mac_address, oid, property, array_index)
        .await
    {
        Ok(ack) => Json(serde_json::json!({
            "device": instance,
            "object": format!("{}:{}", object_type_name(obj_type), obj_instance),
            "property": property_name(property),
            "value": decode_raw_property_to_json(&ack.property_value),
        }))
        .into_response(),
        Err(e) => {
            let (status, err) = ApiError::from_bacnet_error(&e);
            (status, Json(err)).into_response()
        }
    }
}

/// PUT /api/v1/devices/{instance}/objects/{specifier}/properties/{property}
pub async fn write_remote_property(
    State(state): State<GatewayState>,
    Path((instance, specifier, prop_str)): Path<(u32, String, String)>,
    Json(body): Json<WritePropertyRequest>,
) -> impl IntoResponse {
    if let Err(msg) = state.require_writable() {
        return ApiError::bad_request(msg).into_response();
    }
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

    let (obj_type, obj_instance) = match parse_object_specifier(&specifier) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let property = match parse_property_name(&prop_str) {
        Ok(p) => p,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let value = match json_to_property_value(&body.value) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let oid = match ObjectIdentifier::new(obj_type, obj_instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let entry = match state.resolve_device(instance).await {
        Ok(e) => e,
        Err(msg) => return ApiError::not_found(msg).into_response(),
    };

    // Encode the PropertyValue to bytes for the client's write_property.
    let mut encoded = bytes::BytesMut::new();
    if let Err(e) = bacnet_encoding::primitives::encode_property_value(&mut encoded, &value) {
        return ApiError::bad_request(format!("failed to encode value: {e}")).into_response();
    }

    match client
        .write_property(
            &entry.mac_address,
            oid,
            property,
            body.index,
            encoded.to_vec(),
            body.priority,
        )
        .await
    {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => {
            let (status, err) = ApiError::from_bacnet_error(&e);
            (status, Json(err)).into_response()
        }
    }
}
