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
//!   a local address. One on the client's own network, once the client
//!   knows that network's number, is the local station. A remote one the
//!   discovery table holds no router for is asked with a Who-Is addressed
//!   to the station first, then, with no answer, one Who-Is to its network,
//!   where only the station's own address answers (an MS/TP slave behind a
//!   slave proxy is found only so); a station found that way is asked on
//!   its network first from then on. Its instance, when a call needs it and the table doesn't say,
//!   comes from one read of the Device object by the wildcard instance,
//!   remembered by the client.
//! - **From an instance** (`1234`, or `2:1234` on one network): one
//!   address claiming it in the discovery table gives a handle that
//!   follows the instance; none sends one Who-Is for it and waits
//!   ([`Client::with_resolve_wait`]), to the network the target names, or
//!   for a bare instance to the client's resolve scope, this network alone
//!   by default ([`Client::with_resolve_scope`]), and a miss fails as
//!   [`OpsError::UnknownDevice`] saying where it asked; two or more, before
//!   or after that wait, fail with [`OpsError::Ambiguous`], listing them.
//! - **From a discovery result** ([`Discovered`]): pinned, knowing the
//!   instance.
//!
//! A pinned handle never moves: [`Device::moved_to`] says when its
//! instance is heard at one other address while its own has gone quiet
//! ([`QUIET`]). A following handle's place moves only on its device's
//! answers (#2795: the discovery table is filled by I-Ams nobody asked
//! for). A write or relinquish never moves it: it goes only to the place
//! the handle already holds while the device there is taken for the
//! instance: its discovery-table row is absent or names the instance
//! (neither is the device's own answer, and neither has an age limit), or
//! the device's last own answer there named it; else one identity read is
//! sent within the call, and an answer as the instance, an unclear one (a
//! Reject, an Abort, another error) or the client's own failure keeps the
//! write there. Otherwise nothing is sent ([`OpsError::NotConfirmed`]). A
//! read moves it to the one other address on its network claiming its
//! instance, telling [`Client::events`] ([`ClientEvent::Moved`]), once the
//! device at its own address was asked which it is and isn't the instance
//! (silent, by a timeout alone, or another device) and the device at the
//! new address names the instance, when:
//!
//! - before the read, another device has announced itself at its address
//!   (still the instance when asked, or no word of which it is, the handle
//!   stays; with no other claim, or one that doesn't answer as it, the
//!   read fails as [`OpsError::UnknownDevice`], with two or more claims as
//!   ambiguous, and the read isn't sent);
//! - after the read timed out, or a router refused it, no row at its
//!   address claims the instance any more;
//! - after the read timed out while its address still claims the
//!   instance, only the other address answered the one Who-Is for the
//!   instance an earlier timeout sent, on the network where the device was
//!   last reached, never on every network ([`QUIET`] says how often it
//!   asks, and how a clone that sends no Who-Is decides instead);
//! - after the read found no route to a routed address, a Who-Is for the
//!   instance addressed to the station, then one to its network, drew no
//!   answer as the instance (one sends the read there again).
//!
//! Two or more other claims fail a timed-out request as ambiguous, and
//! otherwise its error stays as it was. Handles following one instance on
//! one network share where it is, so one moving takes the others with it;
//! a handle made later for the instance joins them where they are, whatever
//! the table says, and a watch hub following the instance moves them when
//! its own reads find the device elsewhere after asking their address
//! ([`crate::watch`]).
//!
//! What this leaves: a read moves a following handle once its old address
//! misses one identity question (about 24 s at the default timers; about 3
//! to 6 s for a routed address with no row) while another address names
//! the instance, and later writes go to the new place. A routed device's
//! path through its router is #2823, and a new handle's first resolution
//! of an instance is #2824.

/// Evaluate `$body` with `$at` where the device is reached now
/// ([`Device::target`]), and for a following handle whose request timed
/// out or found no route once more where it moved to ([`Device::retry`]).
/// `$body` gives a `Result<_, Error>`, the macro a `Result<_, OpsError>`
/// whose error names the address the request went to, for a function
/// returning `Result<_, OpsError>`.
macro_rules! sent {
    ($device:expr, |$at:ident| $body:expr) => {{
        let address = $device
            .target()
            .await
            .map_err($crate::error::OpsError::from)?;
        let result = {
            let $at = &address;
            $body
        };
        match $device
            .retry(&address, &result)
            .await
            .map_err($crate::error::OpsError::from)?
        {
            Some(to) => {
                let $at = &to;
                ($body).map_err(|error| $crate::error::OpsError::at(&to, error))
            }
            None => result.map_err(|error| $crate::error::OpsError::at(&address, error)),
        }
    }};
}
pub(crate) use sent;

mod address;
pub(crate) mod confirm;
mod follow;
mod misses;
mod reads;
mod resolve;
mod route;
mod watching;
mod writes;

use std::time::Duration;

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::port::TransportPort;

pub use self::address::DeviceAddress;
pub(crate) use self::confirm::Answer;
pub(crate) use self::follow::{FollowKey, Followed};
pub(crate) use self::follow::{Follows, keep_room};
pub use self::misses::MISS_MEMORY;
pub(crate) use self::misses::StationMisses;
pub(crate) use self::route::{path_to, router_for};
use crate::client::{Client, lock};
use crate::error::OpsError;

/// How long an address must have been quiet before another announcing its
/// instance counts as the device moving there, where only the discovery
/// table can say: for [`Device::moved_to`], which sends nothing, and for a
/// following handle of a clone that sends no Who-Is
/// ([`Client::with_resolve_wait`] `None`), after a request times out. Two
/// devices sharing an instance both answer one Who-Is within this of each
/// other. A duplicate that boots and announces itself longer than this
/// after the last Who-Is the first answered reads as a move, until a
/// Who-Is hears them both again. A row registered by hand, never
/// announced, is never quiet by this alone.
///
/// A following handle of a clone that may send asks instead. When a
/// request times out while one other address claims its instance, or
/// none does, it sends one Who-Is for the instance on its network, at
/// most one every `QUIET` for the handles following it: a device gone for
/// good costs one Who-Is every `QUIET`, not one every read. A later
/// timeout, once the Who-Is has had the clone's resolve wait for its
/// answers, moves the handle only if the other address answered and its
/// own has not been heard from since, by an I-Am or an answer; a device
/// that misses a read but answers keeps its handles.
pub const QUIET: Duration = Duration::from_secs(30);

/// What names a device for [`Client::device`].
#[derive(Debug, Clone)]
pub enum DeviceRef<'a> {
    /// A target in the grammar
    /// ([`parse_target_on`](crate::target::parse_target_on)): an address,
    /// an instance or a routed instance.
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
    /// The handles following `instance` found it gone from `from` (another
    /// device answers there, or it stopped answering there while `to`
    /// answered for it) and moved to `to`, the one other address claiming
    /// it, once the device there named the instance to a read of the
    /// handle's or of a watch hub following it (#2795).
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
#[derive(Debug, Clone)]
enum Pin {
    /// Pinned to an address.
    At(DeviceAddress),
    /// Following `instance`, on `network` when it was named with one.
    Follows {
        /// The instance followed.
        instance: u32,
        /// The network its claims are looked for on; any for `None`.
        network: Option<u16>,
        /// Where it is reached now, shared with the other handles
        /// following it on that network.
        at: Followed,
    },
}

/// One device, as a [`Client`] reaches it: by address, pinned, or by
/// instance, following it. Cheap to clone; a handle carries its client.
/// The transport type defaults to the connect builders' [`AnyTransport`].
pub struct Device<T: TransportPort + 'static = AnyTransport> {
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
    /// following handle last reached its instance. An address on the
    /// client's own network is the local station's, once the client knows
    /// the network's number, however the handle was named.
    pub fn address(&self) -> DeviceAddress {
        let address = match &self.pin {
            Pin::At(address) => address.clone(),
            Pin::Follows { at, .. } => lock(at).at.clone(),
        };
        self.client.localized(address)
    }

    /// The device instance, when known: from discovery, the discovery
    /// table, the instance the handle follows, or a read by the wildcard
    /// instance made earlier ([`resolve_instance`](Self::resolve_instance)).
    pub fn instance(&self) -> Option<u32> {
        self.instance.or_else(|| match &self.pin {
            Pin::At(_) => self.client.remembered(&self.address()),
            Pin::Follows { instance, .. } => Some(*instance),
        })
    }

    /// True when the handle follows its instance rather than an address.
    pub fn follows(&self) -> bool {
        matches!(self.pin, Pin::Follows { .. })
    }

    /// The client the handle reaches its device through.
    pub fn client(&self) -> &Client<T> {
        &self.client
    }

    /// The device instance: as known ([`instance`](Self::instance)), or
    /// from one ReadProperty of the Device object's Object_Identifier by
    /// the wildcard instance, which the client remembers for the address,
    /// so it is read once: until the discovery table loses a row at that
    /// address, or a device there announces another instance, which may
    /// mean another device has it now.
    pub async fn resolve_instance(&self) -> Result<u32, OpsError> {
        if let Some(instance) = self.instance() {
            return Ok(instance);
        }
        let address = self.address();
        let mut reader = self.client.reader(&address, None);
        let instance = reader
            .instance()
            .await
            .map_err(|halt| OpsError::at(&address, crate::client::device_error(halt)))?;
        self.client.remember(address, instance);
        Ok(instance)
    }

    /// For a pinned handle whose instance is known, the one other address
    /// the discovery table holds a claim to that instance at, while this
    /// handle's own address has gone quiet: its row is gone, or claims
    /// another instance now, or last announced itself at least [`QUIET`]
    /// before the other did (a row registered by hand, never announced,
    /// isn't quiet by that). Answers to requests don't count as
    /// announcements, so another device answering at a reused address
    /// doesn't hide a move. `None` otherwise, and always for a following
    /// handle, which moves itself. Sends nothing.
    pub async fn moved_to(&self) -> Option<DeviceAddress> {
        let (Pin::At(_), Some(instance)) = (&self.pin, self.instance()) else {
            return None;
        };
        let here = self.address();
        self.client
            .elsewhere(instance, None, &here)
            .await
            .by_announcements()
    }
}

#[cfg(test)]
mod ask_tests;

#[cfg(test)]
mod confirm_tests;

#[cfg(test)]
mod discover_tests;

#[cfg(test)]
mod find_tests;

#[cfg(test)]
mod follow_tests;

#[cfg(test)]
mod one_station_tests;

#[cfg(test)]
mod proxy_tests;

#[cfg(test)]
mod resolve_scope_tests;

#[cfg(test)]
mod routed_tests;

#[cfg(test)]
mod router_app_tests;

#[cfg(test)]
pub(crate) mod test_bus;

#[cfg(test)]
mod write_follow_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
