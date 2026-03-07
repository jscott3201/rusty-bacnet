//! Discovery commands: WhoIs and WhoHas.

use bacnet_client::client::BACnetClient;
use bacnet_services::who_has::WhoHasObject;
use bacnet_transport::port::TransportPort;

use crate::output::{self, device_info, DeviceInfo, OutputFormat};

/// Collect device infos from the client's discovered device table,
/// optionally filtering by instance range.
async fn collect_devices<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    low: Option<u32>,
    high: Option<u32>,
) -> Vec<DeviceInfo> {
    let devices = client.discovered_devices().await;
    devices
        .iter()
        .filter(|d| {
            let inst = d.object_identifier.instance_number();
            match (low, high) {
                (Some(lo), Some(hi)) => inst >= lo && inst <= hi,
                _ => true,
            }
        })
        .map(device_info)
        .collect()
}

/// Send a WhoIs broadcast and display discovered devices after waiting.
pub async fn discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    low: Option<u32>,
    high: Option<u32>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client.who_is(low, high).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, low, high).await, format);
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
    client.who_is_directed(target_mac, low, high).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, low, high).await, format);
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
    client.who_is_network(dnet, low, high).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, low, high).await, format);
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
