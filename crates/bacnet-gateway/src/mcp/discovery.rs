//! MCP discovery tools: discover_devices, list_known_devices, get_device_info.

use schemars::JsonSchema;
use serde::Deserialize;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::parse::{decode_raw_property_to_json, property_name};
use crate::state::GatewayState;

/// Parameters for discover_devices tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverParams {
    /// Minimum device instance number to discover (optional).
    #[schemars(description = "Minimum device instance number to search for (optional, 0-4194302)")]
    pub low_instance: Option<u32>,
    /// Maximum device instance number to discover (optional).
    #[schemars(description = "Maximum device instance number to search for (optional, 0-4194302)")]
    pub high_instance: Option<u32>,
    /// How long to wait for responses in seconds (default: 3, max: 30).
    #[schemars(description = "Seconds to wait for IAm responses (default: 3, max: 30)")]
    pub timeout_seconds: Option<u64>,
}

/// Parameters for get_device_info tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeviceInfoParams {
    /// The device instance number to query.
    #[schemars(
        description = "Device instance number (must be in the device table from a prior discover)"
    )]
    pub device_instance: u32,
}

pub async fn discover_devices_impl(state: &GatewayState, params: DiscoverParams) -> String {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return format!("Error: {msg}"),
    };

    const MAX_DISCOVER_TIMEOUT_SECS: u64 = 30;
    let timeout = params
        .timeout_seconds
        .unwrap_or(3)
        .min(MAX_DISCOVER_TIMEOUT_SECS);

    if let Err(e) = client
        .who_is(params.low_instance, params.high_instance)
        .await
    {
        return format!("Error sending WhoIs: {e}");
    }

    tokio::time::sleep(std::time::Duration::from_secs(timeout)).await;

    let devices = client.discovered_devices().await;
    if devices.is_empty() {
        return "No devices discovered.".to_string();
    }

    let mut result = format!("Discovered {} device(s):\n", devices.len());
    for dev in &devices {
        result.push_str(&format!(
            "  - Instance {}, vendor ID {}, max APDU {}, MAC {:02x?}",
            dev.object_identifier.instance_number(),
            dev.vendor_id,
            dev.max_apdu_length,
            dev.mac_address.as_slice(),
        ));
        if let Some(net) = dev.source_network {
            result.push_str(&format!(", network {net}"));
        }
        result.push('\n');
    }
    result
}

pub async fn list_known_devices_impl(state: &GatewayState) -> String {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return format!("No devices ({msg})."),
    };

    let devices = client.discovered_devices().await;
    if devices.is_empty() {
        return "No devices in the device table. Use discover_devices first.".to_string();
    }

    let mut result = format!("{} known device(s):\n", devices.len());
    for dev in &devices {
        result.push_str(&format!(
            "  - Instance {}, vendor ID {}, MAC {:02x?}",
            dev.object_identifier.instance_number(),
            dev.vendor_id,
            dev.mac_address.as_slice(),
        ));
        if let Some(net) = dev.source_network {
            result.push_str(&format!(", network {net}"));
        }
        result.push('\n');
    }
    result
}

pub async fn get_device_info_impl(state: &GatewayState, params: DeviceInfoParams) -> String {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return format!("Error: {msg}"),
    };

    let entry = match state.resolve_device(params.device_instance).await {
        Ok(e) => e,
        Err(msg) => return msg,
    };

    let device_oid = match ObjectIdentifier::new(ObjectType::DEVICE, params.device_instance) {
        Ok(oid) => oid,
        Err(e) => return format!("Invalid device instance: {e}"),
    };

    let mut result = format!("Device {} info:\n", params.device_instance);
    result.push_str(&format!("  MAC: {:02x?}\n", entry.mac_address.as_slice()));
    result.push_str(&format!("  Vendor ID: {}\n", entry.vendor_id));
    result.push_str(&format!("  Max APDU: {}\n", entry.max_apdu_length));

    let props = [
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::VENDOR_NAME,
        PropertyIdentifier::MODEL_NAME,
        PropertyIdentifier::FIRMWARE_REVISION,
        PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
        PropertyIdentifier::PROTOCOL_VERSION,
        PropertyIdentifier::PROTOCOL_REVISION,
        PropertyIdentifier::DESCRIPTION,
    ];

    for prop in props {
        if let Ok(ack) = client
            .read_property(&entry.mac_address, device_oid, prop, None)
            .await
        {
            let val = decode_raw_property_to_json(&ack.property_value);
            let display = match val.get("value") {
                Some(v) => format!("{v}"),
                None => format!("{val}"),
            };
            result.push_str(&format!("  {}: {}\n", property_name(prop), display));
        }
    }

    result
}
