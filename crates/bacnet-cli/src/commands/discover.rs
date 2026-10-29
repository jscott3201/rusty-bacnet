//! Discovery commands: WhoIs and WhoHas.

use bacnet_client::client::{BACnetClient, ReceivedIHave};
use bacnet_ops::target::AddressStyle;
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use serde::Serialize;

use crate::output::{self, DeviceInfo, OutputFormat, device_info};

/// Collect device infos from the client's discovered device table,
/// optionally filtering by instance range.
async fn collect_devices<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    range: Option<DeviceInstanceRange>,
) -> Vec<DeviceInfo> {
    let devices = client.discovered_devices().await;
    devices
        .iter()
        .filter(|d| {
            let inst = d.object_identifier.instance_number();
            range.is_none_or(|range| range.contains(inst))
        })
        .map(|device| device_info(device, AddressStyle::of(client.transport())))
        .collect()
}

/// Send a WhoIs broadcast and display discovered devices after waiting.
pub async fn discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    range: Option<DeviceInstanceRange>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client.who_is(range).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, range).await, format);
    Ok(())
}

/// Send a directed (unicast) WhoIs to a specific device and display responses.
pub async fn discover_directed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_mac: &[u8],
    range: Option<DeviceInstanceRange>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client.who_is_directed(target_mac, range).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, range).await, format);
    Ok(())
}

/// Send a WhoIs broadcast targeting a specific remote network.
pub async fn discover_network<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    dnet: u16,
    range: Option<DeviceInstanceRange>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client.who_is_network(dnet, range).await?;
    output::print_waiting(wait_secs);
    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    output::print_devices(&collect_devices(client, range).await, format);
    Ok(())
}

/// One object an I-Have reported, as `find` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoundObject {
    /// The instance of the device that has the object.
    pub device: u32,
    /// The object, as `TYPE:instance`.
    pub object: String,
    /// The object's name.
    pub name: String,
}

impl From<&ReceivedIHave> for FoundObject {
    fn from(reply: &ReceivedIHave) -> Self {
        let object = reply.i_have.object_identifier;
        Self {
            device: reply.i_have.device_identifier.instance_number(),
            object: format!("{}:{}", object.object_type(), object.instance_number()),
            name: reply.i_have.object_name.clone(),
        }
    }
}

/// Send a Who-Has for `name` and print the objects the I-Haves that answer
/// it within `wait_secs` report: a JSON array of `{device, object, name}`,
/// empty when nothing answered.
pub async fn find_by_name<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    name: &str,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    output::print_waiting(wait_secs);
    let replies = client
        .who_has_replies(
            WhoHasObject::Name(name.to_string()),
            None,
            std::time::Duration::from_secs(wait_secs),
        )
        .await?;
    let found: Vec<FoundObject> = replies.iter().map(FoundObject::from).collect();
    print_found(&found, format);
    Ok(())
}

fn print_found(found: &[FoundObject], format: OutputFormat) {
    match format {
        OutputFormat::Table if found.is_empty() => println!("No objects found."),
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Device", "Object", "Name"]);
            for f in found {
                table.add_row(vec![
                    f.device.to_string(),
                    f.object.clone(),
                    output::screen(&f.name),
                ]);
            }
            println!("{table}");
        }
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(found).expect("serialize found objects")
        ),
    }
}
