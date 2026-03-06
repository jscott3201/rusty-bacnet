//! Discovery commands: WhoIs and WhoHas.

use std::collections::HashSet;
use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_has::WhoHasObject;
use bacnet_transport::port::TransportPort;

use crate::output::{self, DeviceInfo, OutputFormat};

/// Format a BIP MAC address (6 bytes: 4 IP + 2 port) as `ip:port`.
/// Falls back to hex display for non-BIP MACs.
fn format_mac(mac: &[u8]) -> String {
    if mac.len() == 6 {
        let ip = Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        format!("{ip}:{port}")
    } else {
        mac.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

fn device_info(d: &DiscoveredDevice) -> DeviceInfo {
    DeviceInfo {
        instance: d.object_identifier.instance_number(),
        address: format_mac(d.mac_address.as_slice()),
        vendor_id: d.vendor_id,
        max_apdu: d.max_apdu_length,
        segmentation: format!("{}", d.segmentation_supported),
    }
}

/// Send a WhoIs broadcast and display discovered devices after waiting.
pub async fn discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    low: Option<u32>,
    high: Option<u32>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    // Snapshot existing devices before the scan.
    let before: HashSet<u32> = client
        .discovered_devices()
        .await
        .iter()
        .map(|d| d.object_identifier.instance_number())
        .collect();

    client.who_is(low, high).await?;
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

    let devices = client.discovered_devices().await;
    let infos: Vec<DeviceInfo> = devices
        .iter()
        .filter(|d| !before.contains(&d.object_identifier.instance_number()))
        .map(device_info)
        .collect();
    output::print_devices(&infos, format);
    Ok(())
}

/// Send a directed (unicast) WhoIs to a specific device and display responses.
pub async fn discover_directed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_mac: &[u8],
    low: Option<u32>,
    high: Option<u32>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let before: HashSet<u32> = client
        .discovered_devices()
        .await
        .iter()
        .map(|d| d.object_identifier.instance_number())
        .collect();

    client.who_is_directed(target_mac, low, high).await?;
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

    let devices = client.discovered_devices().await;
    let infos: Vec<DeviceInfo> = devices
        .iter()
        .filter(|d| !before.contains(&d.object_identifier.instance_number()))
        .map(device_info)
        .collect();
    output::print_devices(&infos, format);
    Ok(())
}

/// Send a WhoIs broadcast targeting a specific remote network.
pub async fn discover_network<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    dnet: u16,
    low: Option<u32>,
    high: Option<u32>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let before: HashSet<u32> = client
        .discovered_devices()
        .await
        .iter()
        .map(|d| d.object_identifier.instance_number())
        .collect();

    client.who_is_network(dnet, low, high).await?;
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

    let devices = client.discovered_devices().await;
    let infos: Vec<DeviceInfo> = devices
        .iter()
        .filter(|d| !before.contains(&d.object_identifier.instance_number()))
        .map(device_info)
        .collect();
    output::print_devices(&infos, format);
    Ok(())
}

/// Send a WhoHas-by-name broadcast and display discovered devices.
pub async fn find_by_name<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    name: &str,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .who_has(WhoHasObject::Name(name.to_string()), None, None)
        .await?;
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

    // IHave response capture is not yet implemented in the client.
    // Show a clear message rather than misleading device list.
    output::print_success(
        &format!("WhoHas broadcast sent for '{name}'. IHave response capture not yet implemented."),
        format,
    );
    Ok(())
}
