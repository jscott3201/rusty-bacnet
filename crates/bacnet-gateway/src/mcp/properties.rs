//! MCP property tools: read_property, write_property.

use schemars::JsonSchema;
use serde::Deserialize;

use bacnet_types::primitives::ObjectIdentifier;

use crate::parse::{
    decode_raw_property_to_json, object_type_name, parse_object_type, parse_property_name,
    property_name,
};
use crate::state::GatewayState;

/// Parameters for read_property tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPropertyParams {
    /// Device instance number of the target device.
    #[schemars(description = "Device instance number (e.g., 1234)")]
    pub device_instance: u32,
    /// Object type name (e.g., "analog-input", "binary-value").
    #[schemars(description = "Object type name (e.g., 'analog-input', 'binary-value', 'device')")]
    pub object_type: String,
    /// Object instance number.
    #[schemars(description = "Object instance number (e.g., 1)")]
    pub object_instance: u32,
    /// Property name (e.g., "present-value", "object-name").
    #[schemars(
        description = "Property name (e.g., 'present-value', 'object-name', 'status-flags')"
    )]
    pub property: String,
    /// Array index for array properties (optional).
    #[schemars(description = "Array index for array properties (optional)")]
    pub array_index: Option<u32>,
}

/// Parameters for write_property tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WritePropertyParams {
    /// Device instance number of the target device.
    #[schemars(description = "Device instance number (e.g., 1234)")]
    pub device_instance: u32,
    /// Object type name (e.g., "analog-output", "binary-value").
    #[schemars(description = "Object type name (e.g., 'analog-output', 'binary-value')")]
    pub object_type: String,
    /// Object instance number.
    #[schemars(description = "Object instance number (e.g., 1)")]
    pub object_instance: u32,
    /// Property name (e.g., "present-value").
    #[schemars(description = "Property name (e.g., 'present-value')")]
    pub property: String,
    /// Value to write (number, boolean, string, or null).
    #[schemars(
        description = "Value to write: number (72.5), boolean (true/false), string, or null"
    )]
    pub value: serde_json::Value,
    /// Command priority 1-16 (optional, for commandable properties).
    #[schemars(
        description = "Command priority 1-16 (optional, for commandable properties like present-value on outputs)"
    )]
    pub priority: Option<u8>,
}

pub async fn read_property_impl(state: &GatewayState, params: ReadPropertyParams) -> String {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return format!("Error: {msg}"),
    };

    let obj_type = match parse_object_type(&params.object_type) {
        Ok(t) => t,
        Err(e) => return format!("Error: {e}"),
    };

    let property = match parse_property_name(&params.property) {
        Ok(p) => p,
        Err(e) => return format!("Error: {e}"),
    };

    let oid = match ObjectIdentifier::new(obj_type, params.object_instance) {
        Ok(o) => o,
        Err(e) => return format!("Error: {e}"),
    };

    let entry = match state.resolve_device(params.device_instance).await {
        Ok(e) => e,
        Err(msg) => return msg,
    };

    match client
        .read_property(&entry.mac_address, oid, property, params.array_index)
        .await
    {
        Ok(ack) => {
            let val = decode_raw_property_to_json(&ack.property_value);
            let display = match val.get("value") {
                Some(v) => format!("{v}"),
                None => format!("{val}"),
            };
            format!(
                "{}:{} {} = {}",
                object_type_name(obj_type),
                params.object_instance,
                property_name(property),
                display,
            )
        }
        Err(e) => format!("Error reading property: {e}"),
    }
}

pub async fn write_property_impl(state: &GatewayState, params: WritePropertyParams) -> String {
    if let Err(msg) = state.require_writable() {
        return format!("Error: {msg}");
    }
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return format!("Error: {msg}"),
    };

    let obj_type = match parse_object_type(&params.object_type) {
        Ok(t) => t,
        Err(e) => return format!("Error: {e}"),
    };

    let property = match parse_property_name(&params.property) {
        Ok(p) => p,
        Err(e) => return format!("Error: {e}"),
    };

    let oid = match ObjectIdentifier::new(obj_type, params.object_instance) {
        Ok(o) => o,
        Err(e) => return format!("Error: {e}"),
    };

    let value = match crate::parse::json_to_property_value(&params.value) {
        Ok(v) => v,
        Err(e) => return format!("Error parsing value: {e}"),
    };

    let entry = match state.resolve_device(params.device_instance).await {
        Ok(e) => e,
        Err(msg) => return msg,
    };

    // Encode PropertyValue to bytes.
    let mut buf = bytes::BytesMut::new();
    if let Err(e) = bacnet_encoding::primitives::encode_property_value(&mut buf, &value) {
        return format!("Error encoding value: {e}");
    }

    match client
        .write_property(
            &entry.mac_address,
            oid,
            property,
            None,
            buf.to_vec(),
            params.priority,
        )
        .await
    {
        Ok(()) => format!(
            "Successfully wrote {} to {}:{} {}",
            params.value,
            object_type_name(obj_type),
            params.object_instance,
            property_name(property),
        ),
        Err(e) => format!("Error writing property: {e}"),
    }
}
