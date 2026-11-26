//! The read operations every front end shares, over one client and one set
//! of request limits.
//!
//! [`Reader`] is the reading strategy for one device
//! at its address: each request waits for its turn at the device's gate
//! and the global rate cap ([`RequestLimits`]), a property list goes by
//! ReadPropertyMultiple with a ReadProperty fallback, and the Object_List
//! falls back to element-by-element reads. The TUI's browser, watch list
//! and write previews use it directly ([`raw`](crate::raw)), with their own
//! stop flags.
//!
//! [`Client`] is the facade the MCP server builds on: it owns the
//! low-level client, the limits and the per-address memory of RPM
//! refusals, and hands out [`Device`] handles, whose methods are the read
//! operations as plain async calls. Cancelling a call means dropping its
//! future: a turn's wait is cancel-safe, and a request already sent is
//! abandoned to the client's transaction state machine. A `Client` is
//! cheap to clone: clones share one client, one set of limits and one
//! memory.
//!
//! An application connects with a builder that names its link
//! ([`Client::bip`], [`Client::bip6`], `Client::sc` with `sc-tls`,
//! `Client::mstp` with `serial`) and gets a client over the type-erased
//! [`AnyTransport`], or wraps a low-level client it built itself
//! ([`Client::over`]). [`Client::discover`] finds devices and
//! [`Client::find`] the devices that hold an object; each one's Who-Is or
//! Who-Has is a local broadcast unless the [`Discover`] or [`Find`] names
//! another [`Scope`].
//!
//! [`Client::routed_paths`] shows the size limit of each path through a
//! router, and [`Client::set_routed_path_limit`] sets one (#2667).

pub(crate) mod batch;
mod connect;
mod discover;
pub(crate) mod find;
mod log;
pub(crate) mod read;
pub(crate) mod routed_paths;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEvent};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::sync::MutexExt;
use tokio::sync::broadcast;

#[cfg(feature = "serial")]
#[cfg_attr(docsrs, doc(cfg(feature = "serial")))]
pub use self::connect::MstpConnect;
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub use self::connect::ScConnect;
pub use self::connect::{Bip6Connect, BipConnect};
pub(crate) use self::discover::Ask;
pub use self::discover::{DISCOVER_WAIT, Discover, ResolveScope, RouterDiscovery, Scope};
pub use self::find::{FIND_WAIT, Find, FoundObject};
pub use self::log::Naming;
pub(crate) use self::log::PagedLog;
pub(crate) use self::read::{
    Halt, Reader, RpmFailure, failed_locally, rpm_failure, timed_out, unreachable,
};
pub use self::read::{ListRead, MAX_OBJECTS, ReadVia, Wanted};
pub use self::routed_paths::{
    DEFAULT_ROUTED_PATH_MAX_NPDU, RoutedPath, RoutedPathStatus, RoutedPaths,
};
use crate::device::{ClientEvent, DeviceAddress, Follows};
use crate::format::Value;
use crate::limits::RequestLimits;
use crate::reading::Reading;
use crate::watch::WatchSettings;
use crate::write::WritePolicy;

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
/// It is the wait per Who-Is: a handle made from a remote address the
/// discovery table holds no row for may send two, one addressed to the
/// station and one to its network, so a silent station costs up to twice
/// this (6 s by default), plus any wait for the Who-Is rate limit.
pub const RESOLVE_WAIT: Duration = Duration::from_secs(3);

/// The stop flag of the client's readers. Nothing sets it: a call is
/// cancelled by dropping its future.
pub(crate) static NEVER_STOPPED: AtomicBool = AtomicBool::new(false);

/// A device's [`DEVICE_PROPERTIES`] as read.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceSummary {
    /// The device instance.
    pub device: u32,
    /// Each property's value, or why the device gave none, in the order
    /// of [`DEVICE_PROPERTIES`].
    pub readings: Vec<Reading>,
    /// Whether the read went by ReadPropertyMultiple or ReadProperty.
    pub via: ReadVia,
}

impl DeviceSummary {
    /// The device's Object_Name, cleaned for display
    /// ([`clean_name`](crate::format::clean_name)); `None` when the device
    /// gave none. Device-supplied text: a front end shows it as data.
    pub fn name(&self) -> Option<String> {
        self.value(PropertyIdentifier::OBJECT_NAME)
            .and_then(Value::as_str)
            .map(crate::format::clean_name)
    }

    /// The value of `property`, when the device gave one.
    pub fn value(&self, property: PropertyIdentifier) -> Option<&Value> {
        self.readings
            .iter()
            .find(|reading| reading.property == property)
            .and_then(|reading| reading.value.as_ref().ok())
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
/// clone: clones share everything but how a handle made from an instance
/// looks for its device: how long it waits for an I-Am
/// ([`with_resolve_wait`](Self::with_resolve_wait)) and where its Who-Is
/// goes ([`with_resolve_scope`](Self::with_resolve_scope)); the
/// priorities its handles may write at
/// ([`with_write_policy`](Self::with_write_policy)); and the settings its
/// watches take ([`with_watch_settings`](Self::with_watch_settings)), whose
/// hubs every clone shares by settings ([`watch_with`](Self::watch_with)).
///
/// What the client learns of a device it keeps by the device's address,
/// for every handle and clone: that it refused ReadPropertyMultiple
/// ([`refused_rpm`](Self::refused_rpm)), which it then reads by
/// ReadProperty for the client's life; the instance the device last named
/// itself, read by a handle made from the address or in answer to a watch
/// hub's own reads (#2603); the most properties one ReadPropertyMultiple of
/// [`Device::point_records`] can ask it for, once it couldn't send a larger
/// batch's answer ([`rpm_limit`](Self::rpm_limit)); and what a watch hub
/// learned there: that the device refused a COV service, or a read by the
/// wildcard instance. All but the first go when the discovery table loses
/// its row at that address or a device there announces another instance,
/// since another device may answer there now, and when more of the table's
/// changes came at once than the client's channel holds, as it then can't
/// tell which addresses changed: each device's next point records cost one
/// refused request to learn the limit again. Each is kept for at most 4,096
/// addresses.
///
/// The transport type defaults to [`AnyTransport`], which the connect
/// builders ([`bip`](Client::bip) and the others) make, so an application
/// that connects with one never names it.
pub struct Client<T: TransportPort + 'static = AnyTransport> {
    pub(crate) shared: Arc<Shared<T>>,
    /// How long a handle by instance that no address claims waits for an
    /// I-Am; `None` sends no Who-Is at all. This clone's and its handles'.
    resolve_wait: Option<Duration>,
    /// Where such a handle's Who-Is goes when its target names no
    /// network. This clone's and its handles'.
    resolve_scope: ResolveScope,
    /// The priorities this clone's handles may write and relinquish at.
    write_policy: WritePolicy,
    /// What this clone's watches take unless told otherwise.
    pub(crate) watch_settings: WatchSettings,
}

/// What a [`Client`]'s clones share. Each std mutex is held only to look up
/// or record an entry, never across an await.
pub(crate) struct Shared<T: TransportPort + 'static> {
    pub(crate) client: Arc<BACnetClient<T>>,
    limits: Arc<RequestLimits>,
    /// Addresses whose device refused ReadPropertyMultiple, read by
    /// ReadProperty from then on.
    no_rpm: Mutex<HashSet<DeviceAddress>>,
    /// The most properties one ReadPropertyMultiple of point records asks
    /// for at each address whose device couldn't send a larger batch's
    /// answer ([`Client::rpm_limit`]), with the instance that device had:
    /// it only ever shrinks, and goes with the address's other learned
    /// facts (`Client::sweep`); at most `device::follow::REMEMBERED`.
    pub(crate) rpm_limits: Mutex<HashMap<DeviceAddress, (u32, usize)>>,
    /// The ranges of subscriber process identifiers the client's live hubs
    /// hold, so no two alive at once share one (#2185).
    pub(crate) ranges: Mutex<crate::watch::hub::ranges::Ranges>,
    /// The hubs the client's [`watch`](Client::watch)es run on, one for
    /// each set of watch settings in use. Taken before `ranges`.
    pub(crate) watch_pool: Mutex<crate::watch::Pool<T>>,
    /// What each address's device declared and did of COV
    /// ([`CovSupport`](crate::watch::CovSupport)).
    pub(crate) cov: Mutex<HashMap<DeviceAddress, crate::watch::CovMemory>>,
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
    /// The stations on other networks a lookup by address found answering
    /// only a Who-Is to their network, or nothing
    /// ([`StationMisses`](crate::device::StationMisses)).
    pub(crate) station_misses: Mutex<crate::device::StationMisses>,
    /// The discovery table's changes, for the rows it loses and the
    /// instances announced at each address.
    pub(crate) changes: Mutex<broadcast::Receiver<DeviceEvent>>,
    pub(crate) events: broadcast::Sender<ClientEvent>,
    /// One confirmed request's time with every retry on the low-level
    /// client (its APDU timeout times one more than its retries), which a
    /// watch's renewals leave room for.
    pub(crate) request_budget: Duration,
}

impl<T: TransportPort + 'static> Clone for Client<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            resolve_wait: self.resolve_wait,
            resolve_scope: self.resolve_scope,
            write_policy: self.write_policy,
            watch_settings: self.watch_settings.clone(),
        }
    }
}

/// Lock `mutex`, past a panic elsewhere.
pub(crate) fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.locked()
}

impl<T: TransportPort + 'static> Client<T> {
    /// A client over the low-level `client`, under `limits` (shared with
    /// any other worker on the same low-level client).
    pub fn new(client: Arc<BACnetClient<T>>, limits: Arc<RequestLimits>) -> Self {
        let changes = Mutex::new(client.device_events());
        let request_budget = client.apdu_timeout() * (u32::from(client.apdu_retries()) + 1);
        Self {
            shared: Arc::new(Shared {
                client,
                limits,
                no_rpm: Mutex::new(HashSet::new()),
                rpm_limits: Mutex::new(HashMap::new()),
                ranges: Mutex::default(),
                watch_pool: Mutex::new(crate::watch::Pool::new()),
                cov: Mutex::new(HashMap::new()),
                follows: Mutex::new(HashMap::new()),
                instances: Mutex::new(HashMap::new()),
                station_misses: Mutex::default(),
                changes,
                events: broadcast::channel(64).0,
                request_budget,
            }),
            resolve_wait: Some(RESOLVE_WAIT),
            resolve_scope: ResolveScope::default(),
            write_policy: WritePolicy::default(),
            watch_settings: WatchSettings::default(),
        }
    }

    /// A clone with the settings [`new`](Self::new) gives, whichever clone
    /// this is: what the client's watch hubs work with (they never write,
    /// and their own settings are theirs).
    pub(crate) fn base(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            resolve_wait: Some(RESOLVE_WAIT),
            resolve_scope: ResolveScope::default(),
            write_policy: WritePolicy::default(),
            watch_settings: WatchSettings::default(),
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
    /// ([`RESOLVE_WAIT`] for [`new`](Self::new)'s), whose handles made from
    /// a remote address with no row wait `wait` after each of up to two
    /// (addressed to the station, then to its network: up to `2 × wait`
    /// for a silent station, plus any wait for the Who-Is rate limit), and
    /// whose following
    /// handles, after a request times out, ask once where the device went
    /// and give the answers that long (see [`QUIET`](crate::device::QUIET)).
    /// `None` sends no Who-Is: such a handle fails at once with
    /// [`OpsError::UnknownDevice`](crate::OpsError::UnknownDevice), as does a routed address the discovery
    /// table holds no router for, and a following one goes by the
    /// discovery table's I-Am times alone. The setting is the clone's
    /// alone, and its clones' and handles': the other clones keep theirs.
    pub fn with_resolve_wait(&self, wait: Option<Duration>) -> Self {
        Self {
            resolve_wait: wait,
            ..self.clone()
        }
    }

    /// How long this clone's handles wait for an I-Am; see
    /// [`with_resolve_wait`](Self::with_resolve_wait).
    pub fn resolve_wait(&self) -> Option<Duration> {
        self.resolve_wait
    }

    /// A clone of this client whose handles made from a bare instance
    /// (`1234`) that no address claims send their one Who-Is for it to
    /// `scope`: this network alone for [`new`](Self::new)'s
    /// ([`ResolveScope::Local`]), so a handle never makes every device on a
    /// routed site answer unasked. A target that names its network
    /// (`5:1234`) asks that network whatever this says, and a handle that
    /// stops answering asks again on the network where its device was last
    /// heard. A miss fails as
    /// [`OpsError::UnknownDevice`](crate::OpsError::UnknownDevice), saying
    /// where it asked. The setting is the clone's alone, as
    /// [`with_resolve_wait`](Self::with_resolve_wait)'s is.
    ///
    /// `ResolveScope::Network(0)` is taken as [`ResolveScope::Local`] and
    /// `ResolveScope::Network(65535)` as [`ResolveScope::Global`], the
    /// scopes those numbers mean ([`ResolveScope::normalized`]), so every
    /// setting is one a Who-Is can be sent to.
    pub fn with_resolve_scope(&self, scope: ResolveScope) -> Self {
        Self {
            resolve_scope: scope.normalized(),
            ..self.clone()
        }
    }

    /// Where this clone's handles made from a bare instance ask for it;
    /// see [`with_resolve_scope`](Self::with_resolve_scope).
    pub fn resolve_scope(&self) -> ResolveScope {
        self.resolve_scope
    }

    /// A clone of this client whose handles write and relinquish under
    /// `policy` ([`WritePolicy::default`] for [`new`](Self::new)'s, which
    /// refuses priorities 1, 2, 5 and 6). The setting is the clone's alone,
    /// and its clones' and handles': the other clones keep theirs.
    ///
    /// ```
    /// # use bacnet_ops::{Client, Priority, WritePolicy};
    /// # use bacnet_transport::port::TransportPort;
    /// # fn life_safety<T: TransportPort + 'static>(client: &Client<T>) -> Client<T> {
    /// let policy = WritePolicy::default().allow_priorities([Priority::MANUAL_LIFE_SAFETY]);
    /// client.with_write_policy(policy)
    /// # }
    /// ```
    pub fn with_write_policy(&self, policy: WritePolicy) -> Self {
        Self {
            write_policy: policy,
            ..self.clone()
        }
    }

    /// The priorities this clone's handles may write at; see
    /// [`with_write_policy`](Self::with_write_policy).
    pub fn write_policy(&self) -> WritePolicy {
        self.write_policy
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

    /// The most properties one ReadPropertyMultiple of
    /// [`Device::point_records`] asks the device at `device` for, once it
    /// couldn't send the answer to a larger batch of several objects (an
    /// abort or reject for the answer's size, or a request too long for
    /// the link or a router's path): half that batch's properties, so it
    /// holds for any [`InventoryOptions`](crate::InventoryOptions), whose
    /// objects ask for more or fewer each. `None` while no batch was
    /// refused for its size, and 32 properties apply.
    ///
    /// It only shrinks: a device whose buffer was full once keeps the
    /// smaller batches, which cost more requests but are never refused for
    /// their size again. It goes, with the instance read for the address,
    /// when the discovery table loses its row at that address or a device
    /// there announces another instance, and then starts again from 32;
    /// every device's goes when a burst of the table's changes overran the
    /// client's channel, and its next point records cost one refused
    /// request to learn it again. A timeout, a silent device or any other
    /// error leaves it as it was. An
    /// [`inventory`](crate::Device::inventory) keeps its own count, which
    /// starts at 16 objects for each inventory and never reads or sets
    /// this.
    pub fn rpm_limit(&self, device: &DeviceAddress) -> Option<usize> {
        self.sweep();
        lock(&self.shared.rpm_limits)
            .get(device)
            .map(|&(_, properties)| properties)
    }

    /// Remember that the device at `device`, instance `instance`, takes
    /// at most `properties` properties a ReadPropertyMultiple of point
    /// records, when that is fewer than before.
    pub(crate) fn settle_rpm_limit(
        &self,
        device: &DeviceAddress,
        instance: u32,
        properties: usize,
    ) {
        if properties >= batch::MAX_BATCH_PROPERTIES {
            return;
        }
        self.sweep();
        let mut limits = lock(&self.shared.rpm_limits);
        match limits.get_mut(device) {
            Some((known, limit)) if *known == instance => *limit = (*limit).min(properties),
            _ => {
                crate::device::keep_room(&mut limits, |_| true);
                limits.insert(device.clone(), (instance, properties));
            }
        }
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

#[cfg(test)]
mod read_many_tests;

#[cfg(test)]
mod read_many_more_tests;

#[cfg(test)]
mod write_tests;
