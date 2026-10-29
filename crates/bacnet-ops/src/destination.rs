//! Turn a [`Target`] into a service destination.
//!
//! A local MAC is used as written. A device instance, or a device behind a
//! router, goes through the client's discovery table, so the `*_from_device`
//! client calls route it: they unicast to the router with the device's
//! network and MAC in the NPDU (Clause 6.2.2). ReadProperty,
//! ReadPropertyMultiple, WriteProperty, WritePropertyMultiple, SubscribeCOV,
//! GetEventInformation, GetAlarmSummary, GetEnrollmentSummary, ReadRange
//! (including log paging), AtomicReadFile and AtomicWriteFile,
//! DeviceCommunicationControl, ReinitializeDevice and both
//! time synchronization services, CreateObject, DeleteObject and canonical
//! AcknowledgeAlarm use routed calls.
//! Commands without a routed call take [`resolve_mac`], which refuses one.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_client::log_reader::{LogCursor, LogPage};
use bacnet_services::alarm_event::GetEventInformationAck;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

use super::target::{Target, bip6_vmac, parse_target_on};
use crate::engine::{RpmFailure, rpm_failure, rpm_rows, unreachable};
use crate::format::{LogColumns, RawRow, RowError, StateTexts};

/// Where a service request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
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
pub async fn resolve<T: TransportPort + 'static>(
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
pub async fn resolve_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    text: &str,
) -> Result<Vec<u8>, String> {
    match parse_target_on(client.transport(), text)? {
        Target::Mac(mac) => Ok(mac),
        Target::Bip6(peer) => bip6_vmac(client.transport(), peer).await,
        Target::Instance(instance) => match client.get_device(instance).await {
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
    /// One checked stream or record file window.
    pub async fn atomic_read_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileAccessMethod,
    ) -> Result<AtomicReadFileAck, Error> {
        match self {
            Self::Mac(mac) => client.atomic_read_file_decoded(mac, file, access).await,
            Self::Device(instance) => {
                client
                    .atomic_read_file_decoded_from_device(*instance, file, access)
                    .await
            }
        }
    }

    /// AtomicWriteFile, retaining its raw service acknowledgement.
    pub async fn atomic_write_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileWriteAccessMethod,
    ) -> Result<bytes::Bytes, Error> {
        match self {
            Self::Mac(mac) => client.atomic_write_file(mac, file, access).await,
            Self::Device(instance) => {
                client
                    .atomic_write_file_from_device(*instance, file, access)
                    .await
            }
        }
    }
    /// Create an object, returning the identifier its acknowledgement names.
    pub async fn create_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<bacnet_types::primitives::ObjectIdentifier, Error> {
        match self {
            Self::Mac(mac) => client.create_object(mac, specifier, initial_values).await,
            Self::Device(instance) => {
                client
                    .create_object_from_device(*instance, specifier, initial_values)
                    .await
            }
        }
    }

    /// Delete the identified object.
    pub async fn delete_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => client.delete_object(mac, object).await,
            Self::Device(instance) => client.delete_object_from_device(*instance, object).await,
        }
    }

    /// Acknowledge an alarm with the caller's exact request.
    pub async fn acknowledge_alarm_request<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &bacnet_services::alarm_event::AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => client.acknowledge_alarm_request(mac, request).await,
            Self::Device(instance) => {
                client
                    .acknowledge_alarm_request_from_device(*instance, request)
                    .await
            }
        }
    }

    /// Dispatch device_communication_control through the resolved device or local MAC.
    pub async fn device_communication_control<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: bacnet_types::enums::EnableDisable,
        duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => {
                client
                    .device_communication_control(mac, state, duration, password)
                    .await
            }
            Self::Device(instance) => {
                client
                    .device_communication_control_from_device(*instance, state, duration, password)
                    .await
            }
        }
    }

    /// Dispatch reinitialize_device through the resolved device or local MAC.
    pub async fn reinitialize_device<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: bacnet_types::enums::ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        match self {
            Self::Mac(mac) => client.reinitialize_device(mac, state, password).await,
            Self::Device(instance) => {
                client
                    .reinitialize_device_from_device(*instance, state, password)
                    .await
            }
        }
    }

    /// Send the selected unconfirmed time service once.
    pub async fn time_synchronization<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
        utc: bool,
    ) -> Result<(), Error> {
        match (self, utc) {
            (Self::Mac(mac), false) => client.time_synchronization(mac, date, time).await,
            (Self::Mac(mac), true) => client.utc_time_synchronization(mac, date, time).await,
            (Self::Device(instance), false) => {
                client
                    .time_synchronization_from_device(*instance, date, time)
                    .await
            }
            (Self::Device(instance), true) => {
                client
                    .utc_time_synchronization_from_device(*instance, date, time)
                    .await
            }
        }
    }

    /// GetEventInformation from after `last`, decoded.
    pub async fn get_event_information<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        last: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        match self {
            Self::Mac(mac) => client.get_event_information(mac, last).await,
            Self::Device(instance) => {
                client
                    .get_event_information_from_device(*instance, last)
                    .await
            }
        }
    }

    /// GetAlarmSummary, decoded.
    pub async fn get_alarm_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<GetAlarmSummaryAck, Error> {
        match self {
            Self::Mac(mac) => client.get_alarm_summary(mac).await,
            Self::Device(instance) => client.get_alarm_summary_from_device(*instance).await,
        }
    }

    /// GetEnrollmentSummary with the caller's filters, decoded.
    pub async fn get_enrollment_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        match self {
            Self::Mac(mac) => client.get_enrollment_summary(mac, request).await,
            Self::Device(instance) => {
                client
                    .get_enrollment_summary_from_device(*instance, request)
                    .await
            }
        }
    }

    /// ReadRange with the selected acknowledgement validation.
    pub async fn read_range<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        match self {
            Self::Mac(mac) => client.read_range_with(mac, request, validation).await,
            Self::Device(instance) => {
                client
                    .read_range_from_device_with(*instance, request, validation)
                    .await
            }
        }
    }

    /// One log page, including all metadata and range subrequests.
    pub async fn read_log_page<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        match self {
            Self::Mac(mac) => client.read_log_page(mac, log, cursor, page_size).await,
            Self::Device(instance) => {
                client
                    .read_log_page_from_device(*instance, log, cursor, page_size)
                    .await
            }
        }
    }

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
    /// as [`Engine::log_state_texts`](crate::engine::Engine::log_state_texts)
    /// does: one request an object, each object once, at most
    /// [`MAX_STATE_TEXT_OBJECTS`](crate::format::MAX_STATE_TEXT_OBJECTS).
    /// A reference that names a device is read only when it names this
    /// destination's device: one addressed by instance, or a MAC the
    /// client's discovery table holds for exactly one device. A refused
    /// read leaves its columns named as before, a device that refuses
    /// ReadPropertyMultiple is read by ReadProperty for the rest of the
    /// objects, and a device that stops answering ends the reading (#2122).
    pub async fn log_state_texts<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        columns: &mut LogColumns,
    ) {
        let log_device = match self {
            Self::Device(instance) => Some(*instance),
            Self::Mac(mac) => {
                let devices = client.discovered_devices().await;
                let mut at_mac = devices.iter().filter(|device| {
                    device.source_network.is_none()
                        && device.mac_address.as_slice() == mac.as_slice()
                });
                // Two instances at one MAC: which one the log is on is unknown.
                match (at_mac.next(), at_mac.next()) {
                    (Some(device), None) => Some(device.object_identifier.instance_number()),
                    _ => None,
                }
            }
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
                    Err(error) => rows.push((*property, Err(RowError::from(error)))),
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

    /// ReadProperty.
    pub async fn read_property<T: TransportPort + 'static>(
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
    pub async fn read_property_multiple<T: TransportPort + 'static>(
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
    pub async fn write_property<T: TransportPort + 'static>(
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
    pub async fn write_property_multiple<T: TransportPort + 'static>(
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
    pub async fn subscribe_cov<T: TransportPort + 'static>(
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
    pub async fn unsubscribe_cov<T: TransportPort + 'static>(
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
