//! Turn a [`Target`] into something a confirmed request can go to.
//!
//! A local MAC is used as written. A device instance, or a device behind a
//! router, goes through the client's discovery table, so the `*_from_device`
//! client calls route it: they unicast to the router with the device's
//! network and MAC in the NPDU (Clause 6.2.2). Only ReadProperty,
//! ReadPropertyMultiple, WriteProperty, WritePropertyMultiple and
//! SubscribeCOV have routed client calls today; every other command takes
//! [`resolve_mac`], which refuses a routed target.

use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

use super::target::{bip6_vmac, parse_target_on, Target};

/// How long a routed target waits for its device to answer a Who-Is sent
/// to its network, as long as `discover` waits by default.
const ROUTED_DISCOVERY_WAIT: Duration = Duration::from_secs(3);

/// Undocumented: milliseconds to wait instead, so tests of a device that
/// never answers don't wait the full three seconds.
const ROUTED_WAIT_VAR: &str = "BACNET_CLI_ROUTED_WAIT_MS";

/// How long a routed target waits for its device: [`ROUTED_DISCOVERY_WAIT`],
/// unless the environment sets another wait for a test.
pub(crate) fn routed_wait() -> Duration {
    std::env::var(ROUTED_WAIT_VAR)
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(ROUTED_DISCOVERY_WAIT, Duration::from_millis)
}

/// Where a confirmed request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    /// A MAC on this network.
    Mac(Vec<u8>),
    /// A device in the client's discovery table, local or routed.
    Device(u32),
}

/// The client's error as the CLI prints it.
fn text(error: Error) -> String {
    error.to_string()
}

fn not_found(instance: u32) -> String {
    format!("Device {instance} not found. Run 'discover' first or use an address.")
}

/// Resolve `text` to a [`Destination`]. A routed target asks its network
/// with a Who-Is when the discovery table doesn't hold it yet, and waits up
/// to `wait` for the answer.
pub(crate) async fn resolve<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    text: &str,
    wait: Duration,
) -> Result<Destination, String> {
    match parse_target_on(client.transport(), text)? {
        Target::Mac(mac) => Ok(Destination::Mac(mac)),
        Target::Bip6(peer) => Ok(Destination::Mac(bip6_vmac(client.transport(), peer).await?)),
        Target::Instance(instance) => match client.get_device(instance).await {
            Some(_) => Ok(Destination::Device(instance)),
            None => Err(not_found(instance)),
        },
        Target::Routed { dnet, instance } => {
            find_routed(client, dnet, &Station::Instance(instance), wait).await
        }
        Target::RoutedMac { dnet, mac } => {
            find_routed(client, dnet, &Station::Mac(mac), wait).await
        }
    }
}

/// Resolve `text` to a MAC on this network, for the commands that have no
/// routed client call. A device instance is looked up in the discovery
/// table; a routed target is refused.
pub(crate) async fn resolve_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    text: &str,
) -> Result<Vec<u8>, String> {
    match parse_target_on(client.transport(), text)? {
        Target::Mac(mac) => Ok(mac),
        Target::Bip6(peer) => bip6_vmac(client.transport(), peer).await,
        Target::Instance(instance) => match client.get_device(instance).await {
            Some(device) if device.source_network.is_some() => Err(format!(
                "Device {instance} is behind a router; only read, readm, write, \
                 writem and subscribe reach routed devices"
            )),
            Some(device) => Ok(device.mac_address.to_vec()),
            None => Err(not_found(instance)),
        },
        Target::Routed { .. } | Target::RoutedMac { .. } => Err(format!(
            "'{text}' is a routed target; only read, readm, write, writem and \
             subscribe reach routed devices"
        )),
    }
}

/// How a routed target names its device.
enum Station {
    Instance(u32),
    Mac(Vec<u8>),
}

impl Station {
    fn matches(&self, dnet: u16, device: &DiscoveredDevice) -> bool {
        device.source_network == Some(dnet)
            && match self {
                Self::Instance(instance) => device.object_identifier.instance_number() == *instance,
                Self::Mac(mac) => device
                    .source_address
                    .as_ref()
                    .is_some_and(|address| address.as_slice() == mac.as_slice()),
            }
    }
}

/// The instance of the device `station` names on network `dnet`, from the
/// discovery table or, failing that, from its answer to a Who-Is sent to
/// that network (to its one instance when the target names it).
async fn find_routed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    dnet: u16,
    station: &Station,
    wait: Duration,
) -> Result<Destination, String> {
    let known = |devices: Vec<DiscoveredDevice>| {
        devices
            .iter()
            .find(|device| station.matches(dnet, device))
            .map(|device| Destination::Device(device.object_identifier.instance_number()))
    };
    if let Some(found) = known(client.discovered_devices().await) {
        return Ok(found);
    }
    // Subscribe before asking, so the answer can't slip in between.
    let mut events = client.device_events();
    let range = match station {
        Station::Instance(instance) => {
            Some(DeviceInstanceRange::new(*instance, *instance).map_err(text)?)
        }
        Station::Mac(_) => None,
    };
    client.who_is_network(dnet, range).await.map_err(text)?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => {
                let name = match station {
                    Station::Instance(instance) => format!("{dnet}:{instance}"),
                    // The target parser takes no empty MAC, so this is
                    // always a station; the fallback is never printed.
                    Station::Mac(mac) => super::target::routed_station(dnet, mac)
                        .unwrap_or_else(|| format!("{dnet}:*")),
                };
                return Err(format!(
                    "device {name} did not answer a Who-Is on network {dnet} within {} ms",
                    wait.as_millis()
                ));
            }
            Ok(Ok(event)) => {
                if event.kind != DeviceEventKind::Lost && station.matches(dnet, &event.device) {
                    return Ok(Destination::Device(
                        event.device.object_identifier.instance_number(),
                    ));
                }
            }
            // Missed events may include the answer; the table holds it.
            Ok(Err(RecvError::Lagged(_))) => {
                if let Some(found) = known(client.discovered_devices().await) {
                    return Ok(found);
                }
            }
            Ok(Err(RecvError::Closed)) => return Err("the client stopped".into()),
        }
    }
}

impl Destination {
    /// ReadProperty.
    pub(crate) async fn read_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        match self {
            Self::Mac(mac) => client.read_property(mac, object, property, index).await,
            Self::Device(instance) => {
                client
                    .read_property_from_device(*instance, object, property, index)
                    .await
            }
        }
    }

    /// ReadPropertyMultiple.
    pub(crate) async fn read_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, Error> {
        match self {
            Self::Mac(mac) => client.read_property_multiple(mac, specs).await,
            Self::Device(instance) => {
                client
                    .read_property_multiple_from_device(*instance, specs)
                    .await
            }
        }
    }

    /// WriteProperty of an encoded value.
    pub(crate) async fn write_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => {
                client
                    .write_property(mac, object, property, index, value, priority)
                    .await
            }
            Self::Device(instance) => {
                client
                    .write_property_to_device(*instance, object, property, index, value, priority)
                    .await
            }
        }
    }

    /// WritePropertyMultiple.
    pub(crate) async fn write_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<WriteAccessSpecification>,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => client.write_property_multiple(mac, specs).await,
            Self::Device(instance) => {
                client
                    .write_property_multiple_to_device(*instance, specs)
                    .await
            }
        }
    }

    /// SubscribeCOV for `object`, from process `process`.
    pub(crate) async fn subscribe_cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        process: u32,
        object: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => {
                client
                    .subscribe_cov(mac, process, object, confirmed, lifetime)
                    .await
            }
            Self::Device(instance) => {
                client
                    .subscribe_cov_to_device(*instance, process, object, confirmed, lifetime)
                    .await
            }
        }
    }

    /// Cancel process `process`'s subscription to `object`.
    pub(crate) async fn unsubscribe_cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        process: u32,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => client.unsubscribe_cov(mac, process, object).await,
            Self::Device(instance) => {
                client
                    .unsubscribe_cov_to_device(*instance, process, object)
                    .await
            }
        }
    }
}
