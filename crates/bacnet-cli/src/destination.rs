//! Turn a [`Target`] into a service destination: the address of one
//! station (#2560).
//!
//! A local address is used as written. A device instance goes to the
//! address that claims it in the client's discovery table, and a device
//! behind a router (`dnet:instance`, `dnet:0xMAC`) to its station there,
//! asking for the named instance or station with a Who-Is when the table doesn't hold it yet.
//! Every service then goes to that address through the client's address
//! entry points: to a local station at its MAC, and to a routed one
//! through the router its row names, with its network and MAC in the NPDU
//! (Clause 6.2.2). So a routed station is reached even when another
//! device claims its instance, and an address needs no instance at all.
//! The BVLL commands, which only go to this network, take [`resolve_mac`].

use std::cell::Cell;
use std::ops::ControlFlow;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

use bacnet_ops::DeviceAddress;
use bacnet_ops::format::{LogColumns, PropertyError, RawRow, StateTexts};
use bacnet_ops::raw::{RpmFailure, rpm_failure, rpm_rows, unreachable};
use bacnet_ops::target::{Target, bip6_vmac, parse_target_on};

mod services;

/// Where a service request goes: one station, by its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// Where the device answers.
    pub address: DeviceAddress,
    /// Its instance, when the target named it or the discovery table held
    /// it at the address; `None` for an address the table doesn't know.
    pub instance: Option<u32>,
}

/// The client's error as the CLI prints it.
fn text(error: Error) -> String {
    error.to_string()
}

fn not_found(instance: u32) -> String {
    format!("Device {instance} not found. Run 'discover' first or use an address.")
}

/// A target's refusal, naming the flag that picks the link it needs when
/// it says it needs another: the operations core's text names no front
/// end's flags (#2561).
pub fn with_link_flag(reason: String) -> String {
    use bacnet_ops::target::{NEEDS_BIP6, NEEDS_MSTP};
    let flag = match () {
        () if reason.ends_with(NEEDS_BIP6) => "--ipv6",
        () if reason.ends_with(NEEDS_MSTP) => "--mstp",
        () => return reason,
    };
    format!("{reason} ({flag})")
}

/// Resolve `text` to a [`Destination`]. An address names its station, and
/// nothing is sent to resolve a local one or a routed one the discovery
/// table holds. A routed target asks for its instance or station when the
/// table doesn't hold it yet, and waits up to `wait` for the answer.
pub async fn resolve<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    text: &str,
    wait: Duration,
) -> Result<Destination, String> {
    match parse_target_on(client.transport(), text).map_err(with_link_flag)? {
        Target::Mac(mac) => Destination::local(&mac),
        Target::Bip6(peer) => Destination::local(
            &bip6_vmac(client.transport(), peer)
                .await
                .map_err(|e| with_link_flag(e.to_string()))?,
        ),
        Target::Instance(instance) => match client
            .get_device(instance)
            .await
            .map_err(|e| e.to_string())?
        {
            Some(device) => Destination::of(&device),
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

impl Destination {
    /// The station at `mac` on this network.
    fn local(mac: &[u8]) -> Result<Self, String> {
        Ok(Self {
            address: DeviceAddress::local(mac)?,
            instance: None,
        })
    }

    /// Where `device`, a row of the discovery table, answers.
    fn of(device: &DiscoveredDevice) -> Result<Self, String> {
        let instance = device.object_identifier.instance_number();
        let address = DeviceAddress::of(device)
            .ok_or_else(|| format!("device {instance} has no address the CLI can write"))?;
        Ok(Self {
            address,
            instance: Some(instance),
        })
    }
}

/// Resolve `text` to a MAC on this network, for the commands that only go
/// to this network (the BVLL ones). A device instance is looked up in the
/// discovery table; a routed target is refused.
pub async fn resolve_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    text: &str,
) -> Result<Vec<u8>, String> {
    match parse_target_on(client.transport(), text).map_err(with_link_flag)? {
        Target::Mac(mac) => Ok(mac),
        Target::Bip6(peer) => bip6_vmac(client.transport(), peer)
            .await
            .map_err(|e| with_link_flag(e.to_string())),
        Target::Instance(instance) => match client
            .get_device(instance)
            .await
            .map_err(|e| e.to_string())?
        {
            Some(device) if device.source_network.is_some() => Err(format!(
                "Device {instance} is behind a router; only read, readm, read-range, alarms, \
                 alarm-summary, enrollment-summary, file-read, file-write, write, writem, subscribe, control, reinit, time-sync, create-object, delete-object and ack-alarm reach routed devices"
            )),
            Some(device) => Ok(device.mac_address.to_vec()),
            None => Err(not_found(instance)),
        },
        Target::Routed { .. } | Target::RoutedMac { .. } => Err(format!(
            "'{text}' is a routed target; only read, readm, read-range, alarms, \
             alarm-summary, enrollment-summary, file-read, file-write, write, writem, subscribe, control, reinit, time-sync, create-object, delete-object and ack-alarm reach routed devices"
        )),
    }
}

/// How a routed target names its device.
enum Station {
    Instance(u32),
    Mac(Vec<u8>),
}

impl Station {
    /// True when `device` is this station on network `dnet`, which is the
    /// local network (0) when it is this client's own.
    fn matches(&self, dnet: u16, device: &DiscoveredDevice) -> bool {
        DeviceAddress::of(device).is_some_and(|at| {
            at.network() == dnet
                && match self {
                    Self::Instance(instance) => {
                        device.object_identifier.instance_number() == *instance
                    }
                    Self::Mac(mac) => at.mac() == mac.as_slice(),
                }
        })
    }

    fn name(&self, dnet: u16) -> String {
        match self {
            Self::Instance(instance) => format!("{dnet}:{instance}"),
            // The target parser takes no empty MAC, so this is always a
            // station; the fallback is never printed.
            Self::Mac(mac) => {
                bacnet_ops::target::routed_station(dnet, mac).unwrap_or_else(|| format!("{dnet}:*"))
            }
        }
    }
}

/// The device `station` names on network `dnet`: its address, from the
/// discovery table or, failing that, from its answer to a Who-Is sent to
/// that instance on its network or directly to the named MAC. A routed
/// MAC the table holds needs nothing sent, and nor does one on this
/// client's own network, once it knows the number: that is the local
/// station. `dnet:instance` claimed twice on that network is ambiguous.
async fn find_routed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    dnet: u16,
    station: &Station,
    wait: Duration,
) -> Result<Destination, String> {
    let own = client.network_number() == Some(dnet);
    let scope = || {
        if client.network_number() == Some(dnet) {
            0
        } else {
            dnet
        }
    };
    if let (true, Station::Mac(mac)) = (own, station) {
        let mut found = Destination::local(mac)?;
        found.instance = client
            .get_device_at(0, mac)
            .await
            .map(|row| row.object_identifier.instance_number());
        return Ok(found);
    }
    let known = |devices: Vec<DiscoveredDevice>| {
        let found: Vec<DiscoveredDevice> = devices
            .into_iter()
            .filter(|device| station.matches(scope(), device))
            .collect();
        match found.as_slice() {
            [] => None,
            [device] => Some(Destination::of(device)),
            several => {
                let at: Vec<String> = several
                    .iter()
                    .filter_map(DeviceAddress::of)
                    .map(|at| at.to_string())
                    .collect();
                Some(Err(format!(
                    "device {} is claimed by {}; name one by its address",
                    station.name(dnet),
                    at.join(" and ")
                )))
            }
        }
    };
    if let Some(found) = known(client.discovered_devices().await) {
        return found;
    }
    // Subscribe before asking, so the answer can't slip in between.
    let mut events = client.device_events();
    let asked = match station {
        Station::Instance(instance) => {
            let range = DeviceInstanceRange::new(*instance, *instance).map_err(text)?;
            client
                .who_is(Broadcast::Network(dnet), Some(range))
                .await
                .map_err(text)?;
            format!("a Who-Is on network {dnet}")
        }
        Station::Mac(mac) => {
            let to = bacnet_client::client::Station::remote(dnet, mac);
            client.who_is_directed(&to, None).await.map_err(text)?;
            "a station-directed Who-Is".into()
        }
    };
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => {
                return Err(format!(
                    "device {} did not answer {asked} within {} ms",
                    station.name(dnet),
                    wait.as_millis()
                ));
            }
            Ok(Ok(event)) => {
                if event.kind != DeviceEventKind::Lost && station.matches(scope(), &event.device) {
                    return Destination::of(&event.device);
                }
            }
            // Missed events may include the answer; the table holds it.
            Ok(Err(RecvError::Lagged(_))) => {
                if let Some(found) = known(client.discovered_devices().await) {
                    return found;
                }
            }
            Ok(Err(RecvError::Closed)) => return Err("the client stopped".into()),
        }
    }
}

impl Destination {
    /// What each column of `log` samples, from its
    /// Log_DeviceObjectProperty: one ReadProperty for a Trend Log or Trend
    /// Log Multiple, none for another log. A refused, failed or empty read
    /// names no column, so the values print as the log gives them (#2091).
    pub async fn log_columns<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        log: ObjectIdentifier,
    ) -> LogColumns {
        if !LogColumns::applies(log.object_type()) {
            return LogColumns::default();
        }
        self.read_property(
            client,
            log,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
            None,
        )
        .await
        .map(|ack| LogColumns::decode(&ack.property_value))
        .unwrap_or_default()
    }

    /// Name the states `columns` sample with their objects' state texts,
    /// as [`Device::log_state_texts`](bacnet_ops::Device::log_state_texts)
    /// does: one request an object, each object once, at most
    /// [`MAX_STATE_TEXT_OBJECTS`](bacnet_ops::format::MAX_STATE_TEXT_OBJECTS).
    /// A reference that names a device is read only when it names this
    /// destination's device: its instance as the target or the discovery
    /// table at its address gave it. A refused
    /// read leaves its columns named as before, a device that refuses
    /// ReadPropertyMultiple is read by ReadProperty for the rest of the
    /// objects, and a device that stops answering ends the reading (#2122).
    pub async fn log_state_texts<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        columns: &mut LogColumns,
    ) {
        let log_device = match self.instance {
            Some(instance) => Some(instance),
            None => client
                .get_device_at(self.address.network(), self.address.mac())
                .await
                .map(|device| device.object_identifier.instance_number()),
        };
        let no_rpm = Cell::new(false);
        let no_rpm = &no_rpm;
        columns
            .read_state_texts(log_device, |object| async move {
                match self.state_text_rows(client, object, no_rpm).await {
                    Ok(rows) => ControlFlow::Continue(StateTexts::from_rows(&rows)),
                    Err(error) if unreachable(&error) => ControlFlow::Break(()),
                    Err(_) => ControlFlow::Continue(StateTexts::default()),
                }
            })
            .await;
    }

    /// `object`'s state texts as the device sent them, for naming the
    /// states a one-shot read prints (#2132): one ReadProperty of a
    /// multi-state object's State_Text, or one ReadPropertyMultiple of a
    /// binary object's Active_Text and Inactive_Text, and nothing for
    /// another object type. `no_rpm` is the caller's memory of the device
    /// refusing ReadPropertyMultiple: once set, by the first refusal, the
    /// pair goes by ReadProperty each, so reading several objects costs
    /// one refused RPM in all, not one an object.
    /// Best effort: a refused or failed read gives no texts. The error
    /// comes back only when the device, or the way to it, stopped
    /// answering ([`unreachable()`]), so a caller reading several objects
    /// stops asking. Clean the texts before showing them
    /// ([`StateTexts::cleaned`]).
    pub async fn state_texts<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        no_rpm: &Cell<bool>,
    ) -> Result<StateTexts, Error> {
        if StateTexts::properties(object.object_type()).is_empty() {
            return Ok(StateTexts::default());
        }
        match self.state_text_rows(client, object, no_rpm).await {
            Ok(rows) => Ok(StateTexts::from_rows(&rows)),
            Err(error) if unreachable(&error) => Err(error),
            Err(_) => Ok(StateTexts::default()),
        }
    }

    /// `object`'s state text properties: one ReadProperty for a single
    /// one, else one ReadPropertyMultiple, or ReadProperty each when the
    /// device refuses it. `no_rpm` is set once the device has refused
    /// RPM, and skips it from then on.
    async fn state_text_rows<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        no_rpm: &Cell<bool>,
    ) -> Result<Vec<RawRow>, Error> {
        let properties = StateTexts::properties(object.object_type());
        let read_each = async || {
            let mut rows = Vec::with_capacity(properties.len());
            for property in properties {
                match self.read_property(client, object, *property, None).await {
                    Ok(ack) => rows.push((*property, Ok(ack.property_value))),
                    Err(error) if unreachable(&error) => return Err(error),
                    Err(error) => rows.push((*property, Err(PropertyError::from(error)))),
                }
            }
            Ok(rows)
        };
        if no_rpm.get() {
            return read_each().await;
        }
        if let [property] = properties {
            let ack = self.read_property(client, object, *property, None).await?;
            return Ok(vec![(*property, Ok(ack.property_value))]);
        }
        let spec = ReadAccessSpecification {
            object_identifier: object,
            list_of_property_references: properties
                .iter()
                .map(|property| PropertyReference {
                    property_identifier: *property,
                    property_array_index: None,
                })
                .collect(),
        };
        match self.read_property_multiple(client, vec![spec]).await {
            Ok(ack) => Ok(rpm_rows(ack)),
            Err(error) => match rpm_failure(&error) {
                RpmFailure::Other => Err(error),
                RpmFailure::Refused => {
                    no_rpm.set(true);
                    read_each().await
                }
                RpmFailure::TooLarge => read_each().await,
            },
        }
    }
}

#[cfg(test)]
#[path = "destination_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "destination_routed_tests.rs"]
mod routed_tests;
