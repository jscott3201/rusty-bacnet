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

use bacnet_client::client::{BACnetClient, Broadcast, DeviceEvent, DeviceEventKind};
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
use bacnet_ops::raw::{RpmFailure, failed_locally, rpm_failure, rpm_rows, unreachable};
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
        Target::RoutedMac {
            dnet,
            mac,
            one_station,
        } => {
            let station = Station::Mac { mac, one_station };
            station.check(dnet, client.network_number())?;
            find_routed(client, dnet, &station, wait).await
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
        let address =
            DeviceAddress::of(device).ok_or_else(|| {
                match DeviceAddress::one_station_form(device) {
                    Some(form) => format!(
                        "device {instance} answered from {form}, an address that needs the \
                     one-station form: name it by that address"
                    ),
                    None => format!("device {instance} has no address the CLI can write"),
                }
            })?;
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
    /// A station by its MAC, marked one station by the target's
    /// `:one-station` suffix (#2738).
    Mac {
        mac: Vec<u8>,
        one_station: bool,
    },
}

impl Station {
    /// Refuse a station the address rules refuse before anything is sent
    /// or waited for: a MAC that is a broadcast or group form without the
    /// mark (the refusal names it, #2738), and a marked station on the
    /// client's own network, where it is on this link and the mark lifts
    /// nothing.
    fn check(&self, dnet: u16, own: Option<u16>) -> Result<(), String> {
        let Self::Mac { mac, one_station } = self else {
            return Ok(());
        };
        if !one_station {
            return DeviceAddress::new(dnet, mac).map(drop).map_err(|why| {
                // On the client's own network the MAC is a local one, and
                // the mark lifts nothing there: give the link's reason.
                let why = match own {
                    Some(own) if own == dnet => DeviceAddress::new(0, mac).err().unwrap_or(why),
                    _ => why,
                };
                format!("'{}': {why}", self.name(dnet))
            });
        }
        if own == Some(dnet) {
            return Err(format!(
                "'{}': network {dnet} is this client's own network, so the station is on this \
                 link and ':{}' lifts nothing there; its MAC is judged by the link",
                self.name(dnet),
                bacnet_ops::target::ONE_STATION_SUFFIX
            ));
        }
        Ok(())
    }

    /// The station's address, when `device` is it. A row at a MAC the
    /// one-station mark lifts has no `DeviceAddress` of its own (an I-Am
    /// can't mark itself), so a marked target takes the row by its raw
    /// address.
    fn address_of(&self, dnet: u16, device: &DiscoveredDevice) -> Option<DeviceAddress> {
        match self {
            Self::Mac {
                mac,
                one_station: true,
            } => {
                let at = device.address()?;
                (at.network_number == dnet && at.mac_address.as_slice() == mac.as_slice())
                    .then(|| DeviceAddress::marked(dnet, mac).ok())
                    .flatten()
            }
            _ => DeviceAddress::of(device),
        }
    }

    /// Match a received or cached row with one snapshot of the local number.
    fn destination(
        &self,
        dnet: u16,
        own: Option<u16>,
        device: &DiscoveredDevice,
    ) -> Option<Destination> {
        let address = self.address_of(dnet, device)?.localized(own);
        let network = if Some(dnet) == own { 0 } else { dnet };
        let instance = device.object_identifier.instance_number();
        let selected = match self {
            Self::Instance(wanted) => *wanted == instance,
            Self::Mac { mac, .. } => mac.as_slice() == address.mac(),
        };
        (selected && address.network() == network).then_some(Destination {
            address,
            instance: Some(instance),
        })
    }

    /// Resolve against the client's canonical one-row-per-station snapshot.
    fn known(
        &self,
        dnet: u16,
        own: Option<u16>,
        devices: &[DiscoveredDevice],
    ) -> Option<Result<Destination, String>> {
        let found: Vec<_> = devices
            .iter()
            .filter_map(|row| self.destination(dnet, own, row))
            .collect();
        match found.as_slice() {
            [] => {
                // An instance whose only claimant answered from an address
                // the mark lifts has no `DeviceAddress`: say so, and how to
                // name it, instead of waiting for an answer (#2738).
                let Self::Instance(wanted) = self else {
                    return None;
                };
                let form = devices
                    .iter()
                    .filter(|row| {
                        row.object_identifier.instance_number() == *wanted
                            && row.source_network == Some(dnet)
                    })
                    .find_map(DeviceAddress::one_station_form)?;
                Some(Err(format!(
                    "device {} answered from {form}, an address that needs the one-station \
                     form: name it by that address",
                    self.name(dnet)
                )))
            }
            [station] => Some(Ok(station.clone())),
            several => {
                let at: Vec<_> = several
                    .iter()
                    .map(|station| station.address.to_string())
                    .collect();
                Some(Err(format!(
                    "device {} is claimed by {}; name one by its address",
                    self.name(dnet),
                    at.join(" and ")
                )))
            }
        }
    }

    fn name(&self, dnet: u16) -> String {
        match self {
            Self::Instance(instance) => format!("{dnet}:{instance}"),
            // The target parser takes no empty MAC, so this is always a
            // station; the fallback is never printed.
            Self::Mac { mac, one_station } => {
                bacnet_ops::target::routed_target(dnet, mac, *one_station)
                    .unwrap_or_else(|| format!("{dnet}:*"))
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
    let (own, cached) = client.localized_device_snapshot();
    if let Some(found) = station.known(dnet, own, &cached) {
        return found;
    }
    if let Station::Mac { mac, .. } = station
        && own == Some(dnet)
    {
        return Destination::local(mac);
    }
    // Subscribe before asking, so the answer can't slip in between.
    let mut events = client.device_events();
    // How many Who-Is went out, each with its own wait, for the message.
    let (asked, sent) = match station {
        Station::Instance(instance) => {
            let range = DeviceInstanceRange::new(*instance, *instance).map_err(text)?;
            client
                .who_is(Broadcast::Network(dnet), Some(range))
                .await
                .map_err(text)?;
            (format!("a Who-Is on network {dnet}"), 1)
        }
        Station::Mac { mac, one_station } => {
            let to = bacnet_client::client::Station::remote(dnet, mac);
            let to = if *one_station { to.one_station() } else { to };
            client.who_is_directed(&to, None).await.map_err(text)?;
            if let Some(found) = answer(client, &mut events, dnet, station, wait).await {
                return found;
            }
            // No answer to the Who-Is addressed to the station: one to its
            // network, for a station a slave proxy answers for (Clause
            // 16.10.2). Only the station's own address answers it, and
            // nothing goes further. A one-shot command keeps no memory of
            // the miss; bacnet-ops' clients do.
            client
                .who_is(Broadcast::Network(dnet), None)
                .await
                .map_err(text)?;
            (
                format!("neither a Who-Is addressed to it nor one on network {dnet}"),
                2,
            )
        }
    };
    match answer(client, &mut events, dnet, station, wait).await {
        Some(found) => found,
        None if sent == 1 => Err(format!(
            "device {} did not answer {asked} within {} ms",
            station.name(dnet),
            wait.as_millis()
        )),
        None => Err(format!(
            "device {} answered {asked}, waiting {} ms for each ({} ms in all)",
            station.name(dnet),
            wait.as_millis(),
            wait.as_millis() * sent
        )),
    }
}

/// The destination `station` names, once an answer for it lands within
/// `wait`; `None` when none did.
async fn answer<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    events: &mut tokio::sync::broadcast::Receiver<DeviceEvent>,
    dnet: u16,
    station: &Station,
    wait: Duration,
) -> Option<Result<Destination, String>> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => return None,
            Ok(Ok(event)) => {
                // The queued copy can name a previous instance of the local
                // station. Match its link identity first and consult the
                // current snapshot for its network, instance and route.
                let relevant = match station {
                    Station::Mac { mac, .. } => station
                        .address_of(dnet, &event.device)
                        .is_some_and(|at| at.mac() == mac),
                    Station::Instance(instance) => {
                        event.device.object_identifier.instance_number() == *instance
                            || event.device.source_network == Some(dnet)
                    }
                };
                if event.kind != DeviceEventKind::Lost && relevant {
                    let (own, cached) = client.localized_device_snapshot();
                    if let Some(found) = station.known(dnet, own, &cached) {
                        return Some(found);
                    }
                }
            }
            // Missed events may include the answer; the table holds it.
            Ok(Err(RecvError::Lagged(_))) => {
                let (own, cached) = client.localized_device_snapshot();
                if let Some(found) = station.known(dnet, own, &cached) {
                    return Some(found);
                }
            }
            Ok(Err(RecvError::Closed)) => return Some(Err("the client stopped".into())),
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
                    Err(error) if unreachable(&error) || failed_locally(&error) => {
                        ControlFlow::Break(())
                    }
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
    /// answering ([`unreachable()`]), or the client's own link failed
    /// ([`failed_locally()`], a stopped transport say, #1833), so a caller
    /// reading several objects stops asking. Clean the texts before showing them
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
            Err(error) if unreachable(&error) || failed_locally(&error) => Err(error),
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
                    Err(error) if unreachable(&error) || failed_locally(&error) => {
                        return Err(error);
                    }
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

#[cfg(test)]
#[path = "destination_one_station_tests.rs"]
mod one_station_tests;

#[cfg(test)]
#[path = "destination_proxy_tests.rs"]
mod proxy_tests;
