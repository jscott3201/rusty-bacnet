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
    /// Target address for unicast discovery (e.g., "192.168.1.100:47808").
    /// If omitted, sends a broadcast WhoIs.
    #[schemars(
        description = "Target address for unicast WhoIs (e.g., '192.168.1.100:47808'). Omit for broadcast."
    )]
    pub target: Option<String>,
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

/// Parameters for register_device tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterDeviceParams {
    /// Device instance number.
    #[schemars(description = "Device instance number")]
    pub device_instance: u32,
    /// Device address as ip:port (e.g., "192.168.1.100:47808").
    #[schemars(description = "Device address as ip:port (e.g., '192.168.1.100:47808')")]
    pub address: String,
}

pub async fn register_device_impl(
    state: &GatewayState,
    params: RegisterDeviceParams,
) -> Result<String, String> {
    state
        .add_device_manual(params.device_instance, &params.address)
        .await?;
    Ok(format!(
        "Registered device {} at {}",
        params.device_instance, params.address
    ))
}

pub async fn discover_devices_impl(
    state: &GatewayState,
    params: DiscoverParams,
) -> Result<String, String> {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return Err(msg),
    };

    const MAX_DISCOVER_TIMEOUT_SECS: u64 = 30;
    let timeout = params
        .timeout_seconds
        .unwrap_or(3)
        .min(MAX_DISCOVER_TIMEOUT_SECS);

    match &params.target {
        Some(target) => {
            let addr: std::net::SocketAddrV4 = target
                .parse()
                .map_err(|e| format!("invalid target address '{target}': {e}"))?;
            let mac = crate::parse::socket_addr_to_mac(addr);
            if let Err(e) = client
                .who_is_directed(&mac, params.low_instance, params.high_instance)
                .await
            {
                return Err(format!("Error sending directed WhoIs: {e}"));
            }
        }
        None => {
            if let Err(e) = client
                .who_is(params.low_instance, params.high_instance)
                .await
            {
                return Err(format!("Error sending WhoIs: {e}"));
            }
        }
    }

    tokio::time::sleep(std::time::Duration::from_secs(timeout)).await;

    let devices = client.discovered_devices().await;
    if devices.is_empty() {
        return Ok("No devices discovered.".to_string());
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
    Ok(result)
}

pub async fn list_known_devices_impl(state: &GatewayState) -> Result<String, String> {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return Err(msg),
    };

    let devices = client.discovered_devices().await;
    if devices.is_empty() {
        return Ok("No devices in the device table. Use discover_devices first.".to_string());
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
    Ok(result)
}

pub async fn get_device_info_impl(
    state: &GatewayState,
    params: DeviceInfoParams,
) -> Result<String, String> {
    let client = match state.require_client() {
        Ok(c) => c,
        Err(msg) => return Err(msg),
    };

    let entry = match state.resolve_device(params.device_instance).await {
        Ok(e) => e,
        Err(msg) => return Err(msg),
    };

    let device_oid = match ObjectIdentifier::new(ObjectType::DEVICE, params.device_instance) {
        Ok(oid) => oid,
        Err(e) => return Err(format!("Invalid device instance: {e}")),
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

    Ok(result)
}
