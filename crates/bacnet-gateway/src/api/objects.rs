//! Local object database CRUD endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use bacnet_types::primitives::ObjectIdentifier;

use crate::state::GatewayState;

use super::types::{
    json_to_property_value, object_type_name, parse_object_specifier, parse_object_type,
    parse_property_name, property_name, property_value_to_json_with_context, ApiError,
    WritePropertyRequest,
};

#[derive(Debug, Deserialize)]
pub struct ListObjectsQuery {
    #[serde(rename = "type")]
    pub object_type: Option<String>,
}

/// GET /api/v1/objects
pub async fn list_objects(
    State(state): State<GatewayState>,
    Query(query): Query<ListObjectsQuery>,
) -> impl IntoResponse {
    let db = state.db.read().await;

    let filter_type = if let Some(type_str) = &query.object_type {
        match parse_object_type(type_str) {
            Ok(ot) => Some(ot),
            Err(e) => return ApiError::bad_request(e).into_response(),
        }
    } else {
        None
    };

    let objects: Vec<serde_json::Value> = db
        .iter_objects()
        .filter(|(oid, _)| {
            filter_type
                .map(|ft| oid.object_type() == ft)
                .unwrap_or(true)
        })
        .map(|(oid, obj)| {
            serde_json::json!({
                "identifier": format!("{}:{}", object_type_name(oid.object_type()), oid.instance_number()),
                "type": object_type_name(oid.object_type()),
                "instance": oid.instance_number(),
                "name": obj.object_name(),
            })
        })
        .collect();

    Json(serde_json::json!({ "objects": objects })).into_response()
}

/// GET /api/v1/objects/:specifier
pub async fn get_object(
    State(state): State<GatewayState>,
    Path(specifier): Path<String>,
) -> impl IntoResponse {
    let (obj_type, instance) = match parse_object_specifier(&specifier) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let oid = match ObjectIdentifier::new(obj_type, instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let db = state.db.read().await;
    let obj = match db.get(&oid) {
        Some(obj) => obj,
        None => {
            return ApiError::not_found(format!("object {specifier} not found")).into_response()
        }
    };

    let props: Vec<serde_json::Value> = obj
        .property_list()
        .iter()
        .filter_map(|&prop| {
            obj.read_property(prop, None).ok().map(|val| {
                serde_json::json!({
                    "property": property_name(prop),
                    "value": property_value_to_json_with_context(&val, prop),
                })
            })
        })
        .collect();

    Json(serde_json::json!({
        "identifier": specifier,
        "name": obj.object_name(),
        "properties": props,
    }))
    .into_response()
}

/// GET /api/v1/objects/:specifier/properties/:property
pub async fn get_object_property(
    State(state): State<GatewayState>,
    Path((specifier, prop_str)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let (obj_type, instance) = match parse_object_specifier(&specifier) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let property = match parse_property_name(&prop_str) {
        Ok(p) => p,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let array_index = params.get("index").and_then(|s| s.parse::<u32>().ok());

    let oid = match ObjectIdentifier::new(obj_type, instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let db = state.db.read().await;
    let obj = match db.get(&oid) {
        Some(obj) => obj,
        None => {
            return ApiError::not_found(format!("object {specifier} not found")).into_response()
        }
    };

    match obj.read_property(property, array_index) {
        Ok(val) => Json(serde_json::json!({
            "property": property_name(property),
            "value": property_value_to_json_with_context(&val, property),
        }))
        .into_response(),
        Err(e) => {
            let (status, err) = ApiError::from_bacnet_error(&e);
            (status, Json(err)).into_response()
        }
    }
}

/// PUT /api/v1/objects/:specifier/properties/:property
pub async fn put_object_property(
    State(state): State<GatewayState>,
    Path((specifier, prop_str)): Path<(String, String)>,
    Json(body): Json<WritePropertyRequest>,
) -> impl IntoResponse {
    if let Err(msg) = state.require_writable() {
        return ApiError::bad_request(msg).into_response();
    }
    let (obj_type, instance) = match parse_object_specifier(&specifier) {
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

    let oid = match ObjectIdentifier::new(obj_type, instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let mut db = state.db.write().await;
    let obj = match db.get_mut(&oid) {
        Some(obj) => obj,
        None => {
            return ApiError::not_found(format!("object {specifier} not found")).into_response()
        }
    };

    match obj.write_property(property, body.index, value, body.priority) {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => {
            let (status, err) = ApiError::from_bacnet_error(&e);
            (status, Json(err)).into_response()
        }
    }
}

/// DELETE /api/v1/objects/:specifier
pub async fn delete_object(
    State(state): State<GatewayState>,
    Path(specifier): Path<String>,
) -> impl IntoResponse {
    if let Err(msg) = state.require_writable() {
        return ApiError::bad_request(msg).into_response();
    }
    let (obj_type, instance) = match parse_object_specifier(&specifier) {
        Ok(v) => v,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let oid = match ObjectIdentifier::new(obj_type, instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    // Don't allow deleting the Device object.
    if oid.object_type() == bacnet_types::enums::ObjectType::DEVICE {
        return ApiError::bad_request("cannot delete the Device object").into_response();
    }

    let mut db = state.db.write().await;
    match db.remove(&oid) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "deleted" })),
        )
            .into_response(),
        None => ApiError::not_found(format!("object {specifier} not found")).into_response(),
    }
}

/// Request body for creating a local object.
#[derive(Debug, Deserialize)]
pub struct CreateObjectRequest {
    #[serde(rename = "type")]
    pub object_type: String,
    pub instance: u32,
    pub name: String,
    /// Number of states for multi-state objects (default: 2).
    pub number_of_states: Option<u32>,
}

/// POST /api/v1/objects
///
/// Creates a new object in the local database. Supports common BACnet object types.
pub async fn create_object(
    State(state): State<GatewayState>,
    Json(body): Json<CreateObjectRequest>,
) -> impl IntoResponse {
    if let Err(msg) = state.require_writable() {
        return ApiError::bad_request(msg).into_response();
    }
    let obj_type = match parse_object_type(&body.object_type) {
        Ok(ot) => ot,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let oid = match ObjectIdentifier::new(obj_type, body.instance) {
        Ok(oid) => oid,
        Err(e) => return ApiError::bad_request(format!("{e}")).into_response(),
    };

    let obj = match crate::parse::construct_object(
        obj_type,
        body.instance,
        &body.name,
        body.number_of_states,
    ) {
        Ok(o) => o,
        Err(e) => return ApiError::bad_request(e).into_response(),
    };

    let mut db = state.db.write().await;
    match db.add(obj) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "identifier": format!("{}:{}", object_type_name(oid.object_type()), oid.instance_number()),
                "status": "created",
            })),
        )
            .into_response(),
        Err(e) => {
            let (status, err) = ApiError::from_bacnet_error(&e);
            (status, Json(err)).into_response()
        }
    }
}
