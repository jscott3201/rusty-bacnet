//! The read operations every front end shares, over one client and one set
//! of request limits.
//!
//! [`Reader`] is the reading strategy for one device at its address: each
//! request waits for its turn at the device's gate and the global rate cap
//! ([`RequestLimits`]), a property list goes by ReadPropertyMultiple with a
//! ReadProperty fallback, and the Object_List falls back to element-by-element
//! reads. The TUI's browser, watch list and write previews use it directly,
//! with their own stop flags.
//!
//! [`Client`] is the facade the MCP server builds on: it owns the
//! low-level client, the limits and the per-address memory of RPM
//! refusals, and hands out [`Device`] handles, whose methods are the read
//! operations as plain async calls. Cancelling a call means dropping its
//! future: a turn's wait is cancel-safe, and a request already sent is
//! abandoned to the client's transaction state machine. A `Client` is
//! cheap to clone: clones share one client, one set of limits and one
//! memory.

mod discover;
mod log;
mod read;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEvent};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast;

pub use self::discover::WhoIsScope;
pub use self::log::Naming;
pub(crate) use self::log::PagedLog;
pub use self::read::{
    Halt, ListRead, MAX_OBJECTS, ReadVia, Reader, RpmFailure, Wanted, rpm_failure, rpm_rows,
    timed_out, unreachable,
};
use crate::device::{ClientEvent, DeviceAddress, Follows};
use crate::format::{RawRow, object_name};
use crate::limits::RequestLimits;

#[cfg(doc)]
use crate::device::Device;

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

/// How long a handle made from an instance no address claims waits for
/// the device's I-Am, unless [`Client::with_resolve_wait`] says otherwise.
pub const RESOLVE_WAIT: Duration = Duration::from_secs(3);

/// The stop flag of the client's readers. Nothing sets it: a call is
/// cancelled by dropping its future.
pub(crate) static NEVER_STOPPED: AtomicBool = AtomicBool::new(false);

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

/// The high-level BACnet client: the read operations over one low-level
/// [`BACnetClient`], shared by every caller.
///
/// A device is reached through a [`Device`] handle ([`device`](Self::device)),
/// made from its address, its instance or a discovery result. Calls to one
/// device queue at its gate, keyed by its address, one request outstanding
/// at a time, and every request takes a slot under the global rate cap, so
/// many callers can share a client without overrunning a device. Cheap to
/// clone: clones share everything but how long a handle by instance waits
/// for its device's I-Am ([`with_resolve_wait`](Self::with_resolve_wait)).
pub struct Client<T: TransportPort + 'static> {
    pub(crate) shared: Arc<Shared<T>>,
    /// How long a handle by instance that no address claims waits for an
    /// I-Am; `None` sends no Who-Is at all. This clone's and its handles'.
    resolve_wait: Option<Duration>,
}

/// What a [`Client`]'s clones share. Each std mutex is held only to look up
/// or record an entry, never across an await.
pub(crate) struct Shared<T: TransportPort + 'static> {
    pub(crate) client: Arc<BACnetClient<T>>,
    limits: Arc<RequestLimits>,
    /// Addresses whose device refused ReadPropertyMultiple, read by
    /// ReadProperty from then on.
    no_rpm: Mutex<HashSet<DeviceAddress>>,
    /// Watch hubs made on this client, so each takes its own range of
    /// subscriber process identifiers (#2185).
    hubs: AtomicU32,
    /// Where each instance a handle follows is reached now, and when its
    /// handles last asked where it went, while one does (at most
    /// `device::follow::REMEMBERED`).
    pub(crate) follows: Mutex<Follows>,
    /// The instances devices named for their address, from the one read by
    /// the wildcard instance a handle made from an address needed (at most
    /// `device::follow::REMEMBERED`); one goes when the discovery table
    /// loses a row at its address, or a device there announces another
    /// instance.
    pub(crate) instances: Mutex<HashMap<DeviceAddress, u32>>,
    /// The discovery table's changes, for the rows it loses and the
    /// instances announced at each address.
    pub(crate) changes: Mutex<broadcast::Receiver<DeviceEvent>>,
    pub(crate) events: broadcast::Sender<ClientEvent>,
}

impl<T: TransportPort + 'static> Clone for Client<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            resolve_wait: self.resolve_wait,
        }
    }
}

/// Lock `mutex`, past a panic elsewhere.
pub(crate) fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T: TransportPort + 'static> Client<T> {
    /// A client over the low-level `client`, under `limits` (shared with
    /// any other worker on the same low-level client).
    pub fn new(client: Arc<BACnetClient<T>>, limits: Arc<RequestLimits>) -> Self {
        let changes = Mutex::new(client.device_events());
        Self {
            shared: Arc::new(Shared {
                client,
                limits,
                no_rpm: Mutex::new(HashSet::new()),
                hubs: AtomicU32::new(0),
                follows: Mutex::new(HashMap::new()),
                instances: Mutex::new(HashMap::new()),
                changes,
                events: broadcast::channel(64).0,
            }),
            resolve_wait: Some(RESOLVE_WAIT),
        }
    }

    /// The low-level client, once this is its last clone, for a test that
    /// shuts it down.
    #[cfg(test)]
    pub(crate) fn into_low_level(self) -> Option<Arc<BACnetClient<T>>> {
        Arc::try_unwrap(self.shared)
            .ok()
            .map(|shared| shared.client)
    }

    /// How many watch hubs were made on this client before this one: a
    /// client's hubs share it, so their subscriptions can't clash.
    pub(crate) fn next_hub(&self) -> u32 {
        self.shared.hubs.fetch_add(1, Ordering::Relaxed)
    }

    /// The limits every request waits for.
    pub fn limits(&self) -> &RequestLimits {
        &self.shared.limits
    }

    /// The low-level client, for the watch hub's subscriptions, the write
    /// desk and any service this client doesn't offer.
    pub fn low_level(&self) -> &Arc<BACnetClient<T>> {
        &self.shared.client
    }

    /// A clone of this client whose handles made from an instance that no
    /// address claims wait `wait` for the device's I-Am after one Who-Is
    /// ([`RESOLVE_WAIT`] for [`new`](Self::new)'s), and whose following
    /// handles, after a request times out, ask once where the device went
    /// and give the answers that long (see [`QUIET`](crate::device::QUIET)).
    /// `None` sends no Who-Is: such a handle fails at once with
    /// [`Error::UnknownDevice`], as does a routed address the discovery
    /// table holds no router for, and a following one goes by the
    /// discovery table's I-Am times alone. The setting is the clone's
    /// alone, and its clones' and handles': the other clones keep theirs.
    pub fn with_resolve_wait(&self, wait: Option<Duration>) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            resolve_wait: wait,
        }
    }

    /// How long this clone's handles wait for an I-Am; see
    /// [`with_resolve_wait`](Self::with_resolve_wait).
    pub fn resolve_wait(&self) -> Option<Duration> {
        self.resolve_wait
    }

    /// A receiver of this client's own events: a handle that follows an
    /// instance moving to another address. Lossy under load, like every
    /// client channel.
    pub fn events(&self) -> broadcast::Receiver<ClientEvent> {
        self.shared.events.subscribe()
    }

    /// True once the device at `device` has refused ReadPropertyMultiple.
    pub fn refused_rpm(&self, device: &DeviceAddress) -> bool {
        lock(&self.shared.no_rpm).contains(device)
    }

    /// A reader for the device at `device`, by RPM unless it refused it
    /// before.
    pub(crate) fn reader(&self, device: &DeviceAddress, instance: Option<u32>) -> Reader<'_, T> {
        Reader {
            client: &self.shared.client,
            device: device.clone(),
            instance,
            rate: &self.shared.limits,
            cancel: &NEVER_STOPPED,
            rpm: !self.refused_rpm(device),
        }
    }

    /// Remember a refusal `reader` met.
    pub(crate) fn settle(&self, reader: &Reader<'_, T>) {
        if !reader.rpm {
            lock(&self.shared.no_rpm).insert(reader.device.clone());
        }
    }

    /// Remember that the device at `address` refused ReadPropertyMultiple.
    pub(crate) fn refuse_rpm(&self, address: &DeviceAddress) {
        lock(&self.shared.no_rpm).insert(address.clone());
    }
}

/// The device's error behind a halted read.
pub(crate) fn device_error(halt: Halt) -> Error {
    match halt {
        Halt::Device(error) => error,
        // The client's readers share NEVER_STOPPED, which nothing sets, and
        // a reader halts as cancelled only when its flag is set.
        Halt::Cancelled => unreachable!("a client's read is never stopped by its flag"),
    }
}

#[cfg(test)]
mod tests;
