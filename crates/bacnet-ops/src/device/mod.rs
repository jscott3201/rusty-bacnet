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
mod reads;
mod route;

use std::time::{Duration, Instant};

use bacnet_client::client::DeviceEventKind;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::RecvError;

pub use self::address::DeviceAddress;
pub(crate) use self::route::Path;
use crate::client::{Client, lock, unreachable};
use crate::target::{Target, bip6_vmac, parse_target_on};

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

    /// Where to send a request once more after `result` at `at`: for a
    /// following handle whose request found the device unreachable, the
    /// one other address claiming its instance ([`Client::relocate`]);
    /// `None` to keep `result`. A pinned handle never moves.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn retry<R>(
        &self,
        at: &DeviceAddress,
        result: &Result<R, Error>,
    ) -> Result<Option<DeviceAddress>, Error> {
        match (&self.pin, result) {
            (Pin::Follows(instance), Err(error)) if unreachable(error) => {
                self.client.relocate(*instance, at).await
            }
            _ => Ok(None),
        }
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

impl<T: TransportPort + 'static> Client<T> {
    /// A handle on the device `target` names ([`DeviceRef`]); see the
    /// [module docs](crate::device) for the rules.
    ///
    /// # Errors
    ///
    /// A target the grammar refuses on this transport
    /// ([`Error::OutOfRange`]), an instance two or more addresses claim
    /// ([`Error::AmbiguousDevice`]), and an instance no address answered
    /// for, or a remote address with no router known
    /// ([`Error::UnknownDevice`]). A B/IPv6 socket address fails as
    /// [`bip6_vmac`] does when its node doesn't say its VMAC.
    pub async fn device<'a>(&self, target: impl Into<DeviceRef<'a>>) -> Result<Device<T>, Error> {
        match target.into() {
            DeviceRef::Text(text) => {
                let transport = self.low_level().transport();
                match parse_target_on(transport, text).map_err(Error::OutOfRange)? {
                    Target::Mac(mac) => self.at(local(&mac)?).await,
                    Target::RoutedMac { dnet, mac } => {
                        self.at(DeviceAddress::new(dnet, &mac).map_err(Error::OutOfRange)?)
                            .await
                    }
                    Target::Bip6(peer) => {
                        let vmac = bip6_vmac(transport, peer)
                            .await
                            .map_err(Error::OutOfRange)?;
                        self.at(local(&vmac)?).await
                    }
                    Target::Instance(instance) => self.following(instance, None).await,
                    Target::Routed { dnet, instance } => self.following(instance, Some(dnet)).await,
                }
            }
            DeviceRef::Instance(instance) => self.following(instance, None).await,
            DeviceRef::Address(address) => self.at(address).await,
            DeviceRef::Found(found) => Ok(self.pinned(found.address.clone(), found.instance)),
            DeviceRef::Entry(entry) => {
                let address = DeviceAddress::of(entry).ok_or_else(|| Error::UnknownDevice {
                    device: entry.object_identifier.instance_number().to_string(),
                })?;
                Ok(self.pinned(address, entry.object_identifier.instance_number()))
            }
        }
    }

    /// A handle pinned to `address`, knowing `instance`.
    fn pinned(&self, address: DeviceAddress, instance: u32) -> Device<T> {
        Device {
            client: self.clone(),
            pin: Pin::At(address),
            instance: Some(instance),
        }
    }

    /// A handle pinned to `address`, with the instance the discovery table
    /// holds for it. A remote address without a row asks its network first.
    async fn at(&self, address: DeviceAddress) -> Result<Device<T>, Error> {
        let mut row = self
            .low_level()
            .get_device_at(address.network(), address.mac())
            .await;
        if row.is_none() && !address.is_local() {
            row = Some(self.find(&address, None).await?);
        }
        Ok(Device {
            client: self.clone(),
            instance: row.map(|row| row.object_identifier.instance_number()),
            pin: Pin::At(address),
        })
    }

    /// A handle following `instance`, from the one address claiming it
    /// (on `network` when one is named), or the one that answers a Who-Is.
    async fn following(&self, instance: u32, network: Option<u16>) -> Result<Device<T>, Error> {
        let mut claims = self.claims(instance).await;
        if let Some(network) = network {
            claims.retain(|address| address.network() == network);
        }
        let address = match claims.as_slice() {
            [] => {
                let row = self.find_instance(instance, network).await?;
                DeviceAddress::of(&row).ok_or_else(|| unknown(instance, network))?
            }
            [address] => address.clone(),
            _ => return Err(ambiguous(instance, &claims)),
        };
        self.follow(instance, address);
        Ok(Device {
            client: self.clone(),
            pin: Pin::Follows(instance),
            instance: Some(instance),
        })
    }

    /// Record that `instance` is followed at `address`, telling the events
    /// receivers when it was followed elsewhere before.
    fn follow(&self, instance: u32, address: DeviceAddress) {
        let before = lock(&self.shared.follows).insert(instance, address.clone());
        if let Some(from) = before.filter(|from| *from != address) {
            let _ = self.shared.events.send(ClientEvent::Moved {
                instance,
                from,
                to: address,
            });
        }
    }

    /// Where `instance` is now, for a following handle that found it
    /// unreachable at `from`: the one other address claiming it, followed
    /// from now on; `None` when no other does.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn relocate(
        &self,
        instance: u32,
        from: &DeviceAddress,
    ) -> Result<Option<DeviceAddress>, Error> {
        let claims = self.claims(instance).await;
        let others: Vec<&DeviceAddress> = claims.iter().filter(|a| *a != from).collect();
        match others.as_slice() {
            [] => Ok(None),
            [to] => {
                let to = (*to).clone();
                self.follow(instance, to.clone());
                Ok(Some(to))
            }
            _ => Err(ambiguous(instance, &claims)),
        }
    }

    /// The addresses whose rows claim `instance`, ordered by address.
    async fn claims(&self, instance: u32) -> Vec<DeviceAddress> {
        match self.low_level().get_device(instance).await {
            Ok(found) => found
                .as_ref()
                .and_then(DeviceAddress::of)
                .into_iter()
                .collect(),
            Err(Error::AmbiguousDevice { candidates, .. }) => candidates
                .iter()
                .filter_map(|c| DeviceAddress::new(c.network_number, &c.mac_address).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The device claiming `instance` that answers one Who-Is for it, to
    /// `network` when named and globally otherwise.
    async fn find_instance(
        &self,
        instance: u32,
        network: Option<u16>,
    ) -> Result<DiscoveredDevice, Error> {
        let range = DeviceInstanceRange::single(instance)?;
        let found = self
            .wait_for(network, Some(range), |row| {
                row.object_identifier.instance_number() == instance
                    && network.is_none_or(|n| row.source_network == Some(n))
            })
            .await?;
        found.ok_or_else(|| unknown(instance, network))
    }

    /// The row of the station at the remote `address`, once it answers a
    /// Who-Is to its network (for `range`, or every device).
    async fn find(
        &self,
        address: &DeviceAddress,
        range: Option<DeviceInstanceRange>,
    ) -> Result<DiscoveredDevice, Error> {
        let found = self
            .wait_for(Some(address.network()), range, |row| {
                DeviceAddress::of(row).as_ref() == Some(address)
            })
            .await?;
        found.ok_or_else(|| Error::UnknownDevice {
            device: address.to_string(),
        })
    }

    /// Send one Who-Is (to `network`, or globally) for `range`, and wait
    /// up to [`resolve_wait`](Self::resolve_wait) for a row `wanted`
    /// takes; `None` when none came, or the client sends no Who-Is.
    async fn wait_for(
        &self,
        network: Option<u16>,
        range: Option<DeviceInstanceRange>,
        wanted: impl Fn(&DiscoveredDevice) -> bool,
    ) -> Result<Option<DiscoveredDevice>, Error> {
        let Some(wait) = self.resolve_wait() else {
            return Ok(None);
        };
        // Subscribed before asking, so the answer can't slip in between.
        let mut events = self.device_events();
        let scope = match network {
            Some(network) => crate::client::WhoIsScope::Network(network),
            None => crate::client::WhoIsScope::Global,
        };
        self.who_is(scope, range).await?;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) | Ok(Err(RecvError::Closed)) => return Ok(None),
                Ok(Ok(event)) if event.kind != DeviceEventKind::Lost && wanted(&event.device) => {
                    return Ok(Some(event.device));
                }
                Ok(Ok(_)) => {}
                // A missed answer is in the table.
                Ok(Err(RecvError::Lagged(_))) => {
                    let table = self.low_level().discovered_devices().await;
                    if let Some(row) = table.into_iter().find(|row| wanted(row)) {
                        return Ok(Some(row));
                    }
                }
            }
        }
    }

    /// Send one Who-Is to `scope` for `range` and collect, for `wait`, the
    /// devices that answer: one [`Discovered`] per address, flagged when
    /// another address claims its instance too.
    pub async fn discover(
        &self,
        scope: crate::client::WhoIsScope<'_>,
        range: Option<DeviceInstanceRange>,
        wait: Duration,
    ) -> Result<Vec<Discovered>, Error> {
        let mut events = self.device_events();
        self.who_is(scope, range).await?;
        let in_range = |row: &DiscoveredDevice| {
            range.is_none_or(|r| r.contains(row.object_identifier.instance_number()))
        };
        let mut heard = std::collections::BTreeSet::new();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) | Ok(Err(RecvError::Closed)) => break,
                Ok(Ok(event)) if event.kind != DeviceEventKind::Lost && in_range(&event.device) => {
                    heard.extend(DeviceAddress::of(&event.device));
                }
                Ok(Ok(_)) => {}
                // Missed answers are in the table: every row in range counts.
                Ok(Err(RecvError::Lagged(_))) => {
                    let table = self.low_level().discovered_devices().await;
                    heard.extend(
                        table
                            .iter()
                            .filter(|r| in_range(r))
                            .filter_map(DeviceAddress::of),
                    );
                }
            }
        }
        let table = self.low_level().discovered_devices().await;
        let claimed = |instance| {
            table
                .iter()
                .filter(|row| row.object_identifier.instance_number() == instance)
                .count()
        };
        Ok(table
            .iter()
            .filter_map(|row| {
                let address = DeviceAddress::of(row).filter(|a| heard.contains(a))?;
                let instance = row.object_identifier.instance_number();
                Some(Discovered {
                    address,
                    instance,
                    duplicate: claimed(instance) > 1,
                    entry: row.clone(),
                })
            })
            .collect())
    }

    /// Every instance two or more addresses claim in the discovery table,
    /// in instance order. Sends nothing.
    pub async fn duplicates(&self) -> Vec<Duplicate> {
        let mut by_instance = std::collections::BTreeMap::<u32, Vec<DeviceAddress>>::new();
        for row in self.low_level().discovered_devices().await {
            if let Some(address) = DeviceAddress::of(&row) {
                let instance = row.object_identifier.instance_number();
                by_instance.entry(instance).or_default().push(address);
            }
        }
        by_instance
            .into_iter()
            .filter(|(_, addresses)| addresses.len() > 1)
            .map(|(instance, mut addresses)| {
                addresses.sort();
                Duplicate {
                    instance,
                    addresses,
                }
            })
            .collect()
    }
}

/// The local station at `mac`.
fn local(mac: &[u8]) -> Result<DeviceAddress, Error> {
    DeviceAddress::local(mac).map_err(Error::OutOfRange)
}

/// The ambiguity error for `instance`, naming every claimant.
fn ambiguous(instance: u32, claims: &[DeviceAddress]) -> Error {
    Error::AmbiguousDevice {
        instance,
        candidates: claims.iter().map(BACnetAddress::from).collect(),
    }
}

/// The unknown-device error for `instance`, named as the caller did.
fn unknown(instance: u32, network: Option<u16>) -> Error {
    Error::UnknownDevice {
        device: match network {
            Some(network) => format!("{network}:{instance}"),
            None => instance.to_string(),
        },
    }
}

#[cfg(test)]
mod routed_tests;

#[cfg(test)]
mod test_bus;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
