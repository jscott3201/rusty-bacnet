//! Making handles: from a target, an address, an instance or a discovery
//! result, and discovery by address.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bacnet_client::client::DeviceEventKind;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::RecvError;

use super::{Device, DeviceAddress, DeviceRef, Discovered, Duplicate, Pin};
use crate::client::{Client, Discover, ResolveScope, Scope};
use crate::error::OpsError;
use crate::target::{Target, VmacLookup, bip6_vmac, check_local_mac, parse_target_on};

/// How long the one Who-Is for an instance no address claims listens on
/// past the first answer, for a second device claiming it too (within the
/// resolve wait). Two devices answer one Who-Is moments apart, not
/// together: a few milliseconds apart on an IP link, but on an MS/TP trunk
/// each answers only when the token reaches it, which on a slow or busy
/// trunk is hundreds of milliseconds. Half a second keeps a handle's first
/// lookup quick while hearing a duplicate on all but the slowest trunks;
/// one heard later still shows in the table, as a duplicate.
const SETTLE: Duration = Duration::from_millis(500);

impl<T: TransportPort + 'static> Client<T> {
    /// A handle on the device `target` names ([`DeviceRef`]); see the
    /// [module docs](crate::device) for the rules.
    ///
    /// # Errors
    ///
    /// A target the grammar refuses on this transport, a local address its
    /// link can't reach, or a B/IPv6 socket address on another link
    /// ([`OpsError::Parse`]); an instance two or more addresses claim
    /// ([`OpsError::Ambiguous`]); and an instance no address answered for,
    /// a remote address with no router known, or a B/IPv6 socket address
    /// whose node doesn't say its VMAC ([`OpsError::UnknownDevice`]).
    pub async fn device<'a>(
        &self,
        target: impl Into<DeviceRef<'a>>,
    ) -> Result<Device<T>, OpsError> {
        self.sweep();
        match target.into() {
            DeviceRef::Text(text) => {
                let transport = self.low_level().transport();
                let parse = |reason| OpsError::parse("target", text, reason);
                match parse_target_on(transport, text).map_err(parse)? {
                    Target::Mac(mac) => self.at(local(&mac).map_err(parse)?).await,
                    Target::RoutedMac { dnet, mac } => {
                        self.at(DeviceAddress::new(dnet, &mac).map_err(parse)?)
                            .await
                    }
                    Target::Bip6(peer) => {
                        let vmac = bip6_vmac(transport, peer).await.map_err(|why| match why {
                            VmacLookup::NoAnswer { .. } => OpsError::UnknownDevice {
                                target: text.to_string(),
                                asked: None,
                            },
                            why => parse(why.to_string()),
                        })?;
                        self.at(local(&vmac).map_err(parse)?).await
                    }
                    Target::Instance(instance) => self.following(instance, None).await,
                    Target::Routed { dnet, instance } => self.following(instance, Some(dnet)).await,
                }
            }
            DeviceRef::Instance(instance) => self.following(instance, None).await,
            DeviceRef::Address(address) => self.at(address).await,
            DeviceRef::Found(found) => Ok(self.pinned(found.address.clone(), found.instance)),
            DeviceRef::Entry(entry) => {
                let address = DeviceAddress::of(entry).ok_or_else(|| OpsError::UnknownDevice {
                    target: entry.object_identifier.instance_number().to_string(),
                    asked: None,
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
    /// holds for it. An address on this client's own network is the local
    /// station at its MAC, which the link must be able to reach. A remote
    /// address without a row gets a station-directed Who-Is first.
    async fn at(&self, address: DeviceAddress) -> Result<Device<T>, OpsError> {
        let address = self.localized(address);
        if address.is_local() {
            let shown = address.styled(self.address_style());
            check_local_mac(self.low_level().transport(), address.mac(), &shown)
                .map_err(|reason| OpsError::parse("address", &shown, reason))?;
        }
        let mut row = address.row(self.low_level()).await;
        if row.is_none() && !address.is_local() {
            row = Some(self.find_station(&address, None).await?);
        }
        Ok(Device {
            client: self.clone(),
            instance: row.map(|row| row.object_identifier.instance_number()),
            pin: Pin::At(address),
        })
    }

    /// `address` as the discovery table and the gates key it: one on this
    /// client's own network is the local station at its MAC, once the
    /// client knows the network's number. Until then the two stay apart.
    /// Applied wherever an address is used, not only when a handle is
    /// made, so a handle made before the number was learned keys as the
    /// local station once it is.
    pub(super) fn localized(&self, address: DeviceAddress) -> DeviceAddress {
        address.localized(self.low_level().network_number())
    }

    /// `network` as the discovery table numbers it: this client's own
    /// network, once known, is the local one (0).
    pub(super) fn scope(&self, network: Option<u16>) -> Option<u16> {
        let own = self.low_level().network_number();
        network.map(|network| if Some(network) == own { 0 } else { network })
    }

    /// A handle following `instance`, from the one address claiming it
    /// (on `named` when the target names a network, network 0 for this
    /// client's own), or the one that answers a Who-Is, so long as no other
    /// answered it too.
    async fn following(&self, instance: u32, named: Option<u16>) -> Result<Device<T>, OpsError> {
        let network = self.scope(named);
        let claims = self.claims(instance, network).await;
        let address = match claims.as_slice() {
            [] => {
                let row = self.find_instance(instance, named).await?;
                let address = DeviceAddress::of(&row)
                    .map(|address| self.localized(address))
                    .ok_or_else(|| OpsError::from(unknown(instance, named)))?;
                // A second device answers the one Who-Is a little after
                // the first, not with it: `find_instance` listened on for
                // SETTLE past the first answer, so one in that time is in
                // the table now.
                let claims = self.claims(instance, network).await;
                if claims.len() > 1 {
                    return Err(ambiguous(instance, &claims).into());
                }
                address
            }
            [address] => address.clone(),
            _ => return Err(ambiguous(instance, &claims).into()),
        };
        let at = self.follow((network, instance), address);
        Ok(Device {
            client: self.clone(),
            // As named: the follow key localizes it where it is used.
            pin: Pin::Follows {
                instance,
                network: named,
                at,
            },
            instance: Some(instance),
        })
    }

    /// The addresses whose rows claim `instance`, on `network` when one is
    /// named, ordered by address. A station on this client's own network
    /// is the local one, once the number is known: a routed row for it the
    /// table hasn't folded yet is no second claim.
    pub(super) async fn claims(&self, instance: u32, network: Option<u16>) -> Vec<DeviceAddress> {
        let own = self.low_level().network_number();
        let mut claims: Vec<DeviceAddress> = match self.low_level().get_device(instance).await {
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
        };
        for claim in &mut claims {
            *claim = claim.clone().localized(own);
        }
        claims.sort();
        claims.dedup();
        if let Some(network) = self.scope(network) {
            claims.retain(|address| address.network() == network);
        }
        claims
    }

    /// The device claiming `instance` that answers one Who-Is for it, to
    /// the network the target `named` (this client's own as a local
    /// broadcast) and to this clone's [`resolve_scope`](Self::resolve_scope)
    /// otherwise, listening [`SETTLE`] past its answer for a second. A miss
    /// names the target as written and says where it asked: the network
    /// named, or the resolve scope.
    async fn find_instance(
        &self,
        instance: u32,
        named: Option<u16>,
    ) -> Result<DiscoveredDevice, OpsError> {
        let range = DeviceInstanceRange::single(instance)?;
        let own = self.low_level().network_number();
        let network = self.scope(named);
        let sent_to = network.map_or(self.resolve_scope(), ResolveScope::on);
        let asked = named.map_or(sent_to, ResolveScope::Network);
        let found = self
            .wait_for(&sent_to.into(), Some(range), SETTLE, |row| {
                row.object_identifier.instance_number() == instance
                    && network.is_none_or(|n| {
                        DeviceAddress::of(row)
                            .is_some_and(|address| address.localized(own).network() == n)
                    })
            })
            .await?;
        found.ok_or_else(|| {
            let sent = self.resolve_wait().is_some();
            match OpsError::from(unknown(instance, named)) {
                OpsError::UnknownDevice { target, .. } => OpsError::UnknownDevice {
                    target,
                    asked: sent.then(|| asked.into()),
                },
                other => other,
            }
        })
    }

    /// The row of the station at the remote `address`, once it answers a
    /// Who-Is to that station, optionally limited to `range`.
    async fn find_station(
        &self,
        address: &DeviceAddress,
        range: Option<DeviceInstanceRange>,
    ) -> Result<DiscoveredDevice, OpsError> {
        let asked = Scope::Directed(address.clone().into());
        let found = self
            .wait_for(&asked, range, Duration::ZERO, |row| {
                DeviceAddress::of(row)
                    .is_some_and(|at| asked.covers(&at, self.low_level().network_number()))
            })
            .await?;
        found.ok_or_else(|| OpsError::UnknownDevice {
            target: address.to_string(),
            asked: self.resolve_wait().map(|_| asked),
        })
    }

    /// Send one Who-Is to `asked` for `range`, and wait
    /// up to [`resolve_wait`](Self::resolve_wait) for a row `wanted` takes,
    /// then up to `settle` more, within that wait, for one at another
    /// address, which ends the wait at once. The first such row; `None`
    /// when none came, or the client sends no Who-Is. A caller that wants
    /// every answer reads the table after.
    async fn wait_for(
        &self,
        asked: &Scope,
        range: Option<DeviceInstanceRange>,
        settle: Duration,
        wanted: impl Fn(&DiscoveredDevice) -> bool,
    ) -> Result<Option<DiscoveredDevice>, OpsError> {
        let Some(wait) = self.resolve_wait() else {
            return Ok(None);
        };
        // Subscribed before asking, so the answer can't slip in between.
        let mut events = self.device_events();
        self.who_is(asked, range).await?;
        let mut deadline = tokio::time::Instant::now() + wait;
        let mut first: Option<DiscoveredDevice> = None;
        loop {
            let row = match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) | Ok(Err(RecvError::Closed)) => return Ok(first),
                Ok(Ok(event)) if event.kind != DeviceEventKind::Lost && wanted(&event.device) => {
                    event.device
                }
                Ok(Ok(_)) => continue,
                // Missed answers are in the table: the first, or a second
                // for the caller to read there.
                Ok(Err(RecvError::Lagged(_))) => {
                    if first.is_some() {
                        return Ok(first);
                    }
                    let table = self.low_level().discovered_devices().await;
                    match table.into_iter().find(|row| wanted(row)) {
                        Some(row) => row,
                        None => continue,
                    }
                }
            };
            match &first {
                None if settle.is_zero() => return Ok(Some(row)),
                None => {
                    deadline = deadline.min(tokio::time::Instant::now() + settle);
                    first = Some(row);
                }
                Some(seen) if DeviceAddress::of(seen) != DeviceAddress::of(&row) => {
                    return Ok(first);
                }
                Some(_) => {}
            }
        }
    }

    /// Send the one Who-Is `request` describes, under the rate cap, and
    /// collect for its wait the devices that answer: one [`Discovered`]
    /// per address, flagged when another address claims its instance too.
    /// With no scope named ([`Discover::default`], [`Discover::local`]) the
    /// Who-Is is a local broadcast: no router passes it on.
    ///
    /// ```no_run
    /// use std::net::Ipv4Addr;
    /// use bacnet_ops::{Client, Discover};
    /// # async fn example() -> Result<(), bacnet_ops::OpsError> {
    /// let client = Client::bip().interface(Ipv4Addr::new(192, 168, 1, 50)).connect().await?;
    /// for found in client.discover(Discover::local().range(1000..=2000)).await? {
    ///     println!("{} device {}", found.address, found.instance);
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// Before anything is sent, [`OpsError::Parse`] for an instance range
    /// that is empty or past instance 4194303, or a scope that can't be
    /// sent (see [`Scope`]); then [`OpsError::Client`] for a send the link
    /// refused, when nothing is collected.
    pub async fn discover(&self, request: Discover) -> Result<Vec<Discovered>, OpsError> {
        let range = request.instances()?;
        let mut events = self.device_events();
        self.who_is(request.scope(), range).await?;
        let answered = |row: &DiscoveredDevice| {
            let instance = row.object_identifier.instance_number();
            let address = DeviceAddress::of(row)?;
            let own = self.low_level().network_number();
            (range.is_none_or(|r| r.contains(instance)) && request.scope().covers(&address, own))
                .then_some((address, instance))
        };
        let mut heard = BTreeSet::new();
        let deadline = tokio::time::Instant::now() + request.wait;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) | Ok(Err(RecvError::Closed)) => break,
                Ok(Ok(event)) if event.kind != DeviceEventKind::Lost => {
                    heard.extend(answered(&event.device));
                }
                Ok(Ok(_)) => {}
                // Missed answers are in the table: every row in range counts.
                Ok(Err(RecvError::Lagged(_))) => {
                    let table = self.low_level().discovered_devices().await;
                    heard.extend(table.iter().filter_map(answered));
                }
            }
        }
        let (own, table) = self.low_level().localized_device_snapshot();
        Ok(discovery_results(table, heard, own))
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

/// Join this call's answers to the final table with one local-number snapshot.
pub(super) fn discovery_results(
    table: Vec<DiscoveredDevice>,
    heard: BTreeSet<(DeviceAddress, u32)>,
    own: Option<u16>,
) -> Vec<Discovered> {
    let heard: BTreeSet<_> = heard
        .into_iter()
        .map(|(address, instance)| (address.localized(own), instance))
        .collect();
    let mut claims = BTreeMap::new();
    let stations: Vec<_> = table
        .into_iter()
        .filter_map(|entry| {
            let address = DeviceAddress::of(&entry)?;
            let instance = entry.object_identifier.instance_number();
            *claims.entry(instance).or_insert(0usize) += 1;
            Some((address, instance, entry))
        })
        .collect();
    stations
        .into_iter()
        .filter_map(|(address, instance, entry)| {
            heard
                .contains(&(address.clone(), instance))
                .then(|| Discovered {
                    address,
                    instance,
                    duplicate: claims[&instance] > 1,
                    entry,
                })
        })
        .collect()
}

/// The local station at `mac`.
fn local(mac: &[u8]) -> Result<DeviceAddress, String> {
    DeviceAddress::local(mac)
}

/// The ambiguity error for `instance`, naming every claimant.
pub(super) fn ambiguous(instance: u32, claims: &[DeviceAddress]) -> Error {
    Error::AmbiguousDevice {
        instance,
        candidates: claims.iter().map(BACnetAddress::from).collect(),
    }
}

/// The unknown-device error for `instance`, named as the caller did: with
/// the network the target named, or by the instance alone.
pub(super) fn unknown(instance: u32, network: Option<u16>) -> Error {
    Error::UnknownDevice {
        device: match network {
            Some(network) if network != 0 => format!("{network}:{instance}"),
            _ => instance.to_string(),
        },
    }
}
