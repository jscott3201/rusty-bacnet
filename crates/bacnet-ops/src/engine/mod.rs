//! The read operations every front end shares, over one client and one set
//! of request limits.
//!
//! [`Reader`] is the reading strategy for one device: each request waits
//! for its turn at the device's gate and the global rate cap
//! ([`RequestLimits`]), a property list goes by ReadPropertyMultiple with a
//! ReadProperty fallback, and the Object_List falls back to element-by-element
//! reads. The TUI's browser, watch list and write previews use it directly,
//! with their own stop flags.
//!
//! [`Engine`] is the facade the MCP server builds on: it owns the client,
//! the limits and the per-device memory of RPM refusals, and offers the
//! read operations as plain async calls on a device instance. A device is
//! looked up in the client's discovery table, so routed devices work as
//! the `*_from_device` client calls route them. Cancelling a call means
//! dropping its future: a turn's wait is cancel-safe, and a request
//! already sent is abandoned to the client's transaction state machine.

mod bbmd;
mod discover;
mod log;
mod read;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogCursor, LogPage, read_log_page};
use bacnet_services::alarm_event::GetEventInformationAck;
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

pub use self::discover::WhoIsScope;
pub use self::log::Naming;
pub(crate) use self::read::rpm_rows;
pub use self::read::{
    Halt, ListRead, MAX_OBJECTS, ReadVia, Reader, RpmFailure, Wanted, network_of, rpm_failure,
    timed_out, unreachable,
};
use crate::format::{RawRow, object_name};
use crate::limits::{RequestLimits, Turn};

/// The Device properties a device summary reads (Clause 12.11): who made
/// it, what it runs, and what it can do. The TUI's device card shows them.
pub const DEVICE_PROPERTIES: [PropertyIdentifier; 13] = [
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::VENDOR_NAME,
    PropertyIdentifier::VENDOR_IDENTIFIER,
    PropertyIdentifier::MODEL_NAME,
    PropertyIdentifier::FIRMWARE_REVISION,
    PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
    PropertyIdentifier::PROTOCOL_VERSION,
    PropertyIdentifier::PROTOCOL_REVISION,
    PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
    PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
    PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
    PropertyIdentifier::SEGMENTATION_SUPPORTED,
    PropertyIdentifier::DATABASE_REVISION,
];

/// The stop flag of the engine's readers. Nothing sets it: an engine call
/// is cancelled by dropping its future.
static NEVER_STOPPED: AtomicBool = AtomicBool::new(false);

/// One object's properties as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyRead {
    /// The object read.
    pub object: ObjectIdentifier,
    /// Each property's raw application-tagged value, or why the device
    /// gave none.
    pub rows: Vec<RawRow>,
    /// Whether the read went by ReadPropertyMultiple or ReadProperty.
    pub via: ReadVia,
}

/// A device's [`DEVICE_PROPERTIES`] as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    /// The device instance.
    pub device: u32,
    /// Each property's raw value, or why the device gave none.
    pub rows: Vec<RawRow>,
    /// Whether the read went by ReadPropertyMultiple or ReadProperty.
    pub via: ReadVia,
}

impl DeviceSummary {
    /// The device's Object_Name, cleaned for display
    /// ([`clean_name`](crate::format::clean_name)); `None` when the device
    /// gave none. Device-supplied text: a front end shows it as data.
    pub fn name(&self) -> Option<String> {
        object_name(&self.rows)
    }
}

/// The read operations over one client, shared by every caller.
///
/// Calls to one device queue at its gate, one request outstanding at a
/// time, and every request takes a slot under the global rate cap, so many
/// callers can share an engine without overrunning a device.
pub struct Engine<T: TransportPort + 'static> {
    pub(crate) client: Arc<BACnetClient<T>>,
    limits: Arc<RequestLimits>,
    /// Devices that refused ReadPropertyMultiple, read by ReadProperty
    /// from then on. The lock is held only to look up or record a device,
    /// never across an await.
    no_rpm: Mutex<HashSet<u32>>,
    /// Watch hubs made on this engine, so each takes its own range of
    /// subscriber process identifiers (#2185).
    hubs: AtomicU32,
}

impl<T: TransportPort + 'static> Engine<T> {
    /// An engine over `client`, under `limits` (shared with any other
    /// worker on the same client).
    pub fn new(client: Arc<BACnetClient<T>>, limits: Arc<RequestLimits>) -> Self {
        Self {
            client,
            limits,
            no_rpm: Mutex::new(HashSet::new()),
            hubs: AtomicU32::new(0),
        }
    }

    /// How many watch hubs were made on this engine before this one: a
    /// client's hubs share its engine, so their subscriptions can't clash.
    pub(crate) fn next_hub(&self) -> u32 {
        self.hubs.fetch_add(1, Ordering::Relaxed)
    }

    /// The limits every request waits for.
    pub fn limits(&self) -> &RequestLimits {
        &self.limits
    }

    /// The client, for the watch hub's subscriptions and the write desk.
    pub fn client(&self) -> &Arc<BACnetClient<T>> {
        &self.client
    }

    /// True once `device` has refused ReadPropertyMultiple.
    pub fn refused_rpm(&self, device: u32) -> bool {
        self.no_rpm().contains(&device)
    }

    fn no_rpm(&self) -> std::sync::MutexGuard<'_, HashSet<u32>> {
        self.no_rpm.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A reader for `device`, by RPM unless the device refused it before.
    pub(crate) fn reader(&self, device: u32) -> Reader<'_, T> {
        Reader {
            client: &self.client,
            device,
            rate: &self.limits,
            cancel: &NEVER_STOPPED,
            rpm: !self.refused_rpm(device),
        }
    }

    /// Remember a refusal `reader` met.
    pub(crate) fn settle(&self, reader: &Reader<'_, T>) {
        if !reader.rpm {
            self.no_rpm().insert(reader.device);
        }
    }

    /// The properties `wanted` of `object` on `device`: one
    /// ReadPropertyMultiple, or ReadProperty each once the device has
    /// refused RPM. A property the device can't give is an error row,
    /// not a failed read.
    pub async fn read(
        &self,
        device: u32,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<PropertyRead, Error> {
        let mut reader = self.reader(device);
        let read = reader.properties(object, wanted).await;
        self.settle(&reader);
        let (rows, via) = read.map_err(device_error)?;
        Ok(PropertyRead { object, rows, via })
    }

    /// One property of `object` on `device`, or one element of it
    /// (`index` 0 is an array's length), as its raw application-tagged
    /// value.
    pub async fn read_property(
        &self,
        device: u32,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, Error> {
        self.reader(device)
            .rp(object, property, index)
            .await
            .map_err(device_error)
    }

    /// `device`'s Object_List: whole, or element by element when the
    /// device can't send it in one answer (at most [`MAX_OBJECTS`]).
    pub async fn list_objects(&self, device: u32) -> Result<ListRead, Error> {
        self.reader(device)
            .object_list(|_, _| {})
            .await
            .map_err(device_error)
    }

    /// `device`'s [`DEVICE_PROPERTIES`].
    pub async fn describe_device(&self, device: u32) -> Result<DeviceSummary, Error> {
        let object = ObjectIdentifier::new(ObjectType::DEVICE, device)?;
        let PropertyRead { rows, via, .. } = self
            .read(device, object, Wanted::List(&DEVICE_PROPERTIES))
            .await?;
        Ok(DeviceSummary { device, rows, via })
    }

    /// One page of at most `page_size` records of `log` (a Trend Log,
    /// Trend Log Multiple, Event Log or Audit Log) on `device`, from
    /// `cursor`; the page says where to read next.
    ///
    /// A page can take more than one request (the log's record counts,
    /// then the ReadRange). It holds the device's gate throughout, so they
    /// still go one at a time, and each takes a rate slot of its own.
    pub async fn read_trend(
        &self,
        device: u32,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        let network = network_of(&self.client, &self.limits, device).await;
        let _turn = self.limits.turn_on(device, network).await;
        let requester = log::PagedLog {
            client: &self.client,
            limits: &self.limits,
            device,
            network,
            first: AtomicBool::new(true),
        };
        // The requester's calls route by device, so the link MAC is unused.
        read_log_page(&requester, &[], log, cursor, page_size).await
    }

    /// `device`'s turn under the limits, on its network.
    async fn turn(&self, device: u32) -> Turn {
        let network = network_of(&self.client, &self.limits, device).await;
        self.limits.turn_on(device, network).await
    }

    /// One GetEventInformation answer from `device`: the objects with an
    /// event state other than normal or with transitions not yet
    /// acknowledged. `after` continues from the last object of the
    /// previous answer while that answer said more events remain.
    pub async fn alarms(
        &self,
        device: u32,
        after: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let _turn = self.turn(device).await;
        self.client
            .get_event_information_from_device(device, after)
            .await
    }

    /// `device`'s GetAlarmSummary answer: the objects whose alarms it
    /// reports this way.
    pub async fn alarm_summary(&self, device: u32) -> Result<GetAlarmSummaryAck, Error> {
        let _turn = self.turn(device).await;
        self.client.get_alarm_summary_from_device(device).await
    }

    /// `device`'s GetEnrollmentSummary answer for `request`'s filters.
    pub async fn enrollment_summary(
        &self,
        device: u32,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let _turn = self.turn(device).await;
        self.client
            .get_enrollment_summary_from_device(device, request)
            .await
    }
}

/// The device's error behind a halted read.
fn device_error(halt: Halt) -> Error {
    match halt {
        Halt::Device(error) => error,
        // The engine's readers share NEVER_STOPPED, which nothing sets, and
        // a reader halts as cancelled only when its flag is set.
        Halt::Cancelled => unreachable!("an engine read is never stopped by its flag"),
    }
}

#[cfg(test)]
mod tests;
