//! Device identity and handles (#2560).
//!
//! A device is identified by where it answers, its [`DeviceAddress`]: the
//! network (0 for the local one) and its MAC there. Two devices sharing an
//! instance are two addresses, so each stays reachable, and the request
//! gates, the no-RPM memory and the handles key on the address.
//!
//! A [`Device`] handle comes from [`Client::device`]:
//!
//! - **From an address** (`192.168.1.10`, `mstp:5`, `2:0x05`, a
//!   [`DeviceAddress`]): pinned to it and made at once, sending nothing for
//!   a local address. A remote one the discovery table holds no router for
//!   asks its network with a Who-Is first. Its instance, when a call needs
//!   it and the table doesn't say, comes from one read of the Device object
//!   by the wildcard instance, remembered by the client.
//! - **From an instance** (`1234`, `2:1234`): one address claiming it in
//!   the discovery table gives a handle that follows the instance; none
//!   sends one Who-Is for it and waits ([`Client::set_resolve_wait`]); two
//!   or more fail with [`Error::AmbiguousDevice`], listing them.
//! - **From a discovery result** ([`Discovered`]): pinned, knowing the
//!   instance.
//!
//! A pinned handle never moves: [`Device::moved_to`] says when its
//! instance is heard at one other address while its own has gone quiet. A
//! following handle whose request finds the device unreachable looks again,
//! once: when exactly one other address claims the instance it moves
//! there, tells [`Client::events`] ([`ClientEvent::Moved`]) and retries;
//! two or more fail as ambiguous, and none leaves the error as it was.

/// Evaluate `$body` with `$at` the device's address, and for a following
/// handle whose request found the device unreachable once more where it
/// moved to ([`Device::retry`]). For a function returning `Result<_, Error>`.
macro_rules! sent {
    ($device:expr, |$at:ident| $body:expr) => {{
        let address = $device.address();
        let result = {
            let $at = &address;
            $body
        };
        match $device.retry(&address, &result).await? {
            Some(to) => {
                let $at = &to;
                $body
            }
            None => result,
        }
    }};
}
pub(crate) use sent;

mod address;
mod follow;
mod reads;
mod resolve;
mod route;

use std::time::{Duration, Instant};

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

pub use self::address::DeviceAddress;
pub(crate) use self::route::Path;
use crate::client::{Client, lock};

/// How long a pinned handle's own address must have been quiet before
/// another address announcing its instance counts as a move: two devices
/// sharing an instance both answer one Who-Is within this of each other.
pub const QUIET: Duration = Duration::from_secs(30);

/// What names a device for [`Client::device`].
#[derive(Debug, Clone)]
pub enum DeviceRef<'a> {
    /// A target in the grammar ([`parse_target_on`]): an address, an
    /// instance or a routed instance.
    Text(&'a str),
    /// A device instance: the handle follows it.
    Instance(u32),
    /// An address: the handle is pinned to it.
    Address(DeviceAddress),
    /// A discovery result: pinned to its address, knowing its instance.
    Found(&'a Discovered),
    /// A row of the low-level client's discovery table, as a discovery
    /// result.
    Entry(&'a DiscoveredDevice),
}

impl<'a> From<&'a str> for DeviceRef<'a> {
    fn from(text: &'a str) -> Self {
        Self::Text(text)
    }
}

impl<'a> From<&'a String> for DeviceRef<'a> {
    fn from(text: &'a String) -> Self {
        Self::Text(text)
    }
}

impl From<u32> for DeviceRef<'_> {
    fn from(instance: u32) -> Self {
        Self::Instance(instance)
    }
}

impl From<DeviceAddress> for DeviceRef<'_> {
    fn from(address: DeviceAddress) -> Self {
        Self::Address(address)
    }
}

impl<'a> From<&'a Discovered> for DeviceRef<'a> {
    fn from(found: &'a Discovered) -> Self {
        Self::Found(found)
    }
}

impl<'a> From<&'a DiscoveredDevice> for DeviceRef<'a> {
    fn from(entry: &'a DiscoveredDevice) -> Self {
        Self::Entry(entry)
    }
}

/// One device that answered a [`Client::discover`], by its address.
#[derive(Debug, Clone)]
pub struct Discovered {
    /// Where it answers.
    pub address: DeviceAddress,
    /// Its Device instance.
    pub instance: u32,
    /// Another address claims the same instance: name this one by its
    /// address.
    pub duplicate: bool,
    /// Its row in the low-level client's discovery table.
    pub entry: DiscoveredDevice,
}

/// An instance two or more addresses claim ([`Client::duplicates`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// The instance.
    pub instance: u32,
    /// Every address claiming it, ordered by network and then MAC.
    pub addresses: Vec<DeviceAddress>,
}

/// What a [`Client`] tells its [`events`](Client::events) receivers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientEvent {
    /// A handle following `instance` found it unreachable at `from` and
    /// moved to `to`, the one other address claiming it.
    Moved {
        /// The device instance.
        instance: u32,
        /// Where it stopped answering.
        from: DeviceAddress,
        /// Where it is claimed now.
        to: DeviceAddress,
    },
}

/// How a handle names its device.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pin {
    /// Pinned to an address.
    At(DeviceAddress),
    /// Following an instance, to the address the client's registry holds.
    Follows(u32),
}

/// One device, as a [`Client`] reaches it: by address, pinned, or by
/// instance, following it. Cheap to clone; a handle carries its client.
pub struct Device<T: TransportPort + 'static> {
    client: Client<T>,
    pin: Pin,
    /// The instance when known: a following handle's own, or a pinned
    /// handle's from discovery or the discovery table.
    instance: Option<u32>,
}

impl<T: TransportPort + 'static> Clone for Device<T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            pin: self.pin.clone(),
            instance: self.instance,
        }
    }
}

impl<T: TransportPort + 'static> std::fmt::Debug for Device<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("address", &self.address())
            .field("instance", &self.instance())
            .field("follows", &self.follows())
            .finish()
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// Where the device answers now: a pinned handle's address, or where a
    /// following handle last reached its instance.
    pub fn address(&self) -> DeviceAddress {
        match &self.pin {
            Pin::At(address) => address.clone(),
            Pin::Follows(instance) => lock(&self.client.shared.follows)
                .get(instance)
                .cloned()
                .expect("a following handle's instance is registered when it is made"),
        }
    }

    /// The device instance, when known: from discovery, the discovery
    /// table, the instance the handle follows, or a read by the wildcard
    /// instance made earlier ([`resolve_instance`](Self::resolve_instance)).
    pub fn instance(&self) -> Option<u32> {
        self.instance.or_else(|| match &self.pin {
            Pin::At(address) => lock(&self.client.shared.instances).get(address).copied(),
            Pin::Follows(instance) => Some(*instance),
        })
    }

    /// True when the handle follows its instance rather than an address.
    pub fn follows(&self) -> bool {
        matches!(self.pin, Pin::Follows(_))
    }

    /// The client the handle reaches its device through.
    pub fn client(&self) -> &Client<T> {
        &self.client
    }

    /// The device instance: as known ([`instance`](Self::instance)), or
    /// from one ReadProperty of the Device object's Object_Identifier by
    /// the wildcard instance, which the client remembers for the address,
    /// so it is read once.
    pub async fn resolve_instance(&self) -> Result<u32, Error> {
        if let Some(instance) = self.instance() {
            return Ok(instance);
        }
        let address = self.address();
        let mut reader = self.client.reader(&address, None);
        let instance = reader
            .instance()
            .await
            .map_err(crate::client::device_error)?;
        lock(&self.client.shared.instances).insert(address, instance);
        Ok(instance)
    }

    /// For a pinned handle whose instance is known, the one other address
    /// the discovery table holds a claim to that instance at, while this
    /// handle's own address has gone quiet: its row is gone, or claims
    /// another instance now, or last announced itself at least [`QUIET`]
    /// before the other did. Answers to requests don't count as
    /// announcements, so another device answering at a reused address
    /// doesn't hide a move. `None` otherwise, and always for a following
    /// handle, which moves itself. Sends nothing.
    pub async fn moved_to(&self) -> Option<DeviceAddress> {
        let (Pin::At(here), Some(instance)) = (&self.pin, self.instance()) else {
            return None;
        };
        let table = self.client.low_level().discovered_devices().await;
        let rows = table
            .iter()
            .filter_map(|row| Some((DeviceAddress::of(row)?, row)));
        let claims = |row: &&DiscoveredDevice| row.object_identifier.instance_number() == instance;
        let mut others = rows.clone().filter(|(at, row)| at != here && claims(row));
        let (Some((other, row)), None) = (others.next(), others.next()) else {
            return None;
        };
        let own = rows.clone().find(|(at, _)| at == here).map(|(_, own)| own);
        let quiet = match own.filter(claims) {
            None => true,
            Some(own) => announced_before(own.last_announced, row.last_announced),
        };
        quiet.then_some(other)
    }
}

/// Whether an announcement at `own` came at least [`QUIET`] before one at
/// `other`; a row never announced counts as quiet.
fn announced_before(own: Option<Instant>, other: Option<Instant>) -> bool {
    match (own, other) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(own), Some(other)) => other.checked_duration_since(own) >= Some(QUIET),
    }
}

#[cfg(test)]
mod routed_tests;

#[cfg(test)]
mod test_bus;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
