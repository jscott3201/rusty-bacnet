//! Making handles: from a target, an address, an instance or a discovery
//! result, and discovery by address.

use std::time::Duration;

use bacnet_client::client::DeviceEventKind;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::RecvError;

use super::{Device, DeviceAddress, DeviceRef, Discovered, Duplicate, Pin};
use crate::client::{Client, WhoIsScope};
use crate::target::{Target, bip6_vmac, check_local_mac, parse_target_on};

impl<T: TransportPort + 'static> Client<T> {
    /// A handle on the device `target` names ([`DeviceRef`]); see the
    /// [module docs](crate::device) for the rules.
    ///
    /// # Errors
    ///
    /// A target the grammar refuses on this transport, or a local address
    /// its link can't reach ([`Error::OutOfRange`]), an instance two or more
    /// addresses claim ([`Error::AmbiguousDevice`]), and an instance no
    /// address answered for, or a remote address with no router known
    /// ([`Error::UnknownDevice`]). A B/IPv6 socket address fails as
    /// [`bip6_vmac`] does when its node doesn't say its VMAC.
    pub async fn device<'a>(&self, target: impl Into<DeviceRef<'a>>) -> Result<Device<T>, Error> {
        self.sweep();
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
    /// holds for it. An address on this client's own network is the local
    /// station at its MAC, which the link must be able to reach. A remote
    /// address without a row asks its network first.
    async fn at(&self, address: DeviceAddress) -> Result<Device<T>, Error> {
        let address = self.localized(address);
        if address.is_local() {
            let shown = address.styled(self.address_style());
            check_local_mac(self.low_level().transport(), address.mac(), &shown)
                .map_err(Error::OutOfRange)?;
        }
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

    /// `address` as the discovery table and the gates key it: one on this
    /// client's own network is the local station at its MAC, once the
    /// client knows the network's number. Until then the two stay apart.
    fn localized(&self, address: DeviceAddress) -> DeviceAddress {
        match self.low_level().network_number() {
            Some(own) if own == address.network() => {
                DeviceAddress::local(address.mac()).unwrap_or(address)
            }
            _ => address,
        }
    }

    /// A handle following `instance`, from the one address claiming it
    /// (on `network` when one is named, network 0 for this client's own),
    /// or the one that answers a Who-Is, so long as no other answered it
    /// too.
    async fn following(&self, instance: u32, network: Option<u16>) -> Result<Device<T>, Error> {
        let network = network.map(|n| match self.low_level().network_number() {
            Some(own) if own == n => 0,
            _ => n,
        });
        let claims = self.claims(instance, network).await;
        let address = match claims.as_slice() {
            [] => {
                let row = self.find_instance(instance, network).await?;
                let address = DeviceAddress::of(&row).ok_or_else(|| unknown(instance, network))?;
                // Two devices answering the one Who-Is are both in the
                // table once the first answer is.
                let claims = self.claims(instance, network).await;
                if claims.len() > 1 {
                    return Err(ambiguous(instance, &claims));
                }
                address
            }
            [address] => address.clone(),
            _ => return Err(ambiguous(instance, &claims)),
        };
        let at = self.follow((network, instance), address);
        Ok(Device {
            client: self.clone(),
            pin: Pin::Follows {
                instance,
                network,
                at,
            },
            instance: Some(instance),
        })
    }

    /// The addresses whose rows claim `instance`, on `network` when one is
    /// named, ordered by address.
    pub(super) async fn claims(&self, instance: u32, network: Option<u16>) -> Vec<DeviceAddress> {
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
        if let Some(network) = network {
            claims.retain(|address| address.network() == network);
        }
        claims
    }

    /// The device claiming `instance` that answers one Who-Is for it, to
    /// `network` when named (this one for 0) and globally otherwise.
    async fn find_instance(
        &self,
        instance: u32,
        network: Option<u16>,
    ) -> Result<DiscoveredDevice, Error> {
        let range = DeviceInstanceRange::single(instance)?;
        let found = self
            .wait_for(network, Some(range), |row| {
                row.object_identifier.instance_number() == instance
                    && network.is_none_or(|n| {
                        DeviceAddress::of(row).is_some_and(|address| address.network() == n)
                    })
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
            Some(0) => WhoIsScope::Local,
            Some(network) => WhoIsScope::Network(network),
            None => WhoIsScope::Global,
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
        scope: WhoIsScope<'_>,
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
pub(super) fn ambiguous(instance: u32, claims: &[DeviceAddress]) -> Error {
    Error::AmbiguousDevice {
        instance,
        candidates: claims.iter().map(BACnetAddress::from).collect(),
    }
}

/// The unknown-device error for `instance`, named as the caller did (on
/// this client's own network, by the instance alone).
fn unknown(instance: u32, network: Option<u16>) -> Error {
    Error::UnknownDevice {
        device: match network {
            Some(network) if network != 0 => format!("{network}:{instance}"),
            _ => instance.to_string(),
        },
    }
}
