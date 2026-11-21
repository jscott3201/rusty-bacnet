//! Where a typed call goes (#2588): one station, named in any of three
//! forms, and how a request reaches it.
//!
//! A station is a MAC on this link, or a network and the MAC there (the
//! address the discovery table keys a device by, #2560). Two devices that
//! share an instance are two addresses, so each is reachable this way
//! while a lookup by the instance is refused.
//!
//! A local station is sent to at its MAC. A remote one goes through a
//! router (Clause 6.2.2): the one the caller names, or else the one the
//! discovery table's row at its address names. The table is the client's
//! only record of a route, so a remote address with no named router and no
//! row fails before sending, except for a Who-Is directed to that station.
//! Every form goes out through the same request path as every other call, so the row at the address, when there
//! is one, sizes the request and its answer keeps the row alive.

use std::future::Future;

use super::*;
use bacnet_types::constructed::BACnetAddress;

/// One station a typed call goes to, and how it is reached: the one address
/// type every typed call takes in its [`ToStation`] form.
///
/// A [`BACnetAddress`] and a MAC are stations too, and a call takes them as
/// they are; this type adds a router named by the caller. The three forms
/// are all there are, so a caller can match every one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Station {
    /// A station on this link, at its MAC.
    Local {
        /// Its MAC.
        mac: MacAddr,
    },
    /// A station on another network, through the router the discovery
    /// table's row at its address names. A network this client has learned
    /// is its own is this link.
    Remote {
        /// Its network, 1 to 65534.
        network: u16,
        /// Its MAC on that network.
        mac: MacAddr,
    },
    /// A station on another network, through the router at `router` on this
    /// link, with no table lookup. A network this client has learned is its
    /// own is this link, and the request goes to `mac` there.
    Routed {
        /// The MAC of the router on this link.
        router: MacAddr,
        /// The station's network, 1 to 65534.
        network: u16,
        /// Its MAC on that network.
        mac: MacAddr,
    },
}

impl Station {
    /// The address that identifies the station, independent of its next hop:
    /// network zero for a local station, or its network number and MAC there.
    pub fn address(&self) -> (u16, &[u8]) {
        match self {
            Self::Local { mac } => (0, mac),
            Self::Remote { network, mac } | Self::Routed { network, mac, .. } => (*network, mac),
        }
    }

    /// The station at `mac` on this link.
    pub fn local(mac: &[u8]) -> Self {
        Self::Local {
            mac: MacAddr::from_slice(mac),
        }
    }

    /// The station at `mac` on `network`, through the router its row in
    /// the discovery table names.
    pub fn remote(network: u16, mac: &[u8]) -> Self {
        Self::Remote {
            network,
            mac: MacAddr::from_slice(mac),
        }
    }

    /// The station at `mac` on `network`, through the router at `router`.
    pub fn routed(router: &[u8], network: u16, mac: &[u8]) -> Self {
        Self::Routed {
            router: MacAddr::from_slice(router),
            network,
            mac: MacAddr::from_slice(mac),
        }
    }
}

impl From<BACnetAddress> for Station {
    /// Network 0 is this link; any other is a remote station.
    fn from(address: BACnetAddress) -> Self {
        match address.network_number {
            0 => Self::Local {
                mac: address.mac_address,
            },
            network => Self::Remote {
                network,
                mac: address.mac_address,
            },
        }
    }
}

impl From<&DiscoveredDevice> for Station {
    /// Where a discovered device answers: through the router its I-Am came
    /// through when that I-Am carried a source network and address, else
    /// at its MAC on this link.
    fn from(device: &DiscoveredDevice) -> Self {
        match (device.source_network, &device.source_address) {
            (Some(network), Some(mac)) => Self::Routed {
                router: device.mac_address.clone(),
                network,
                mac: mac.clone(),
            },
            _ => Self::Local {
                mac: device.mac_address.clone(),
            },
        }
    }
}

impl Station {
    /// True when `received` came from this station. A device keeps a
    /// subscription under the address the request came from (Clause
    /// 13.14.2) and sends its notifications from its own (Clauses 13.6.1
    /// and 13.7.1), so a notification belongs when it comes from the same
    /// address: the MAC with no source network for a local station, the
    /// source network and address for a remote one. The router it came
    /// through is no part of that address and isn't compared. A remote
    /// station on this client's own network (`local_network`, once
    /// learned, as [`BACnetClient::network_number`] gives it) is sent to as
    /// a local one, so its notifications may come with no source network,
    /// from its address as the MAC.
    ///
    /// A station named on network 0 is on this link, so its notifications
    /// come with no source network too.
    ///
    /// Both kinds of notification are matched alike, by the source fields
    /// they share. The managed COV helpers match notifications by this rule.
    pub fn is_source_of(&self, received: &COVEvent, local_network: Option<u16>) -> bool {
        let (source_network, source_address) =
            (received.source_network(), received.source_address());
        let local_from = |mac: &MacAddr| {
            source_network.is_none() && source_address.is_none() && received.source_mac() == mac
        };
        match self {
            Self::Local { mac } => local_from(mac),
            Self::Remote { network, mac } | Self::Routed { network, mac, .. } => {
                (source_network == Some(*network) && source_address == Some(mac))
                    || ((*network == 0 || local_network == Some(*network)) && local_from(mac))
            }
        }
    }
}

/// Where an unconfirmed broadcast goes: the scope of a typed broadcast
/// call such as [`BACnetClient::who_is`] (#2588). A call to one station
/// takes a [`ToStation`] instead (`who_is_directed` and the like).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Broadcast {
    /// Every station on this network.
    Local,
    /// Every station on one remote network, 1 to 65534, through the router
    /// to it. Once the client knows its own network's number, that number
    /// is this network, and the broadcast goes as a local one. Network 0
    /// and 65535 name no one remote network: [`Broadcast::Local`] and
    /// [`Broadcast::Global`] are those, and a call refuses them here before
    /// anything is sent.
    Network(u16),
    /// Every station on every network (DNET 65535).
    Global,
}

impl Broadcast {
    /// Whether an answer heard from `source_network`, the SNET it carried
    /// (`None` when it came from this network with none), is one this
    /// scope's broadcast asked for, `own` being the client's network number
    /// once known: every answer for [`Broadcast::Global`]; for
    /// [`Broadcast::Local`], one from this network; for
    /// [`Broadcast::Network`], one from that network. An answer a router
    /// relays back from `own` is from this network, so a broadcast to
    /// `own` takes the local answers too. An answer from elsewhere during
    /// the wait, unasked or another's, is no answer to this one.
    pub fn answered_from(&self, source_network: Option<u16>, own: Option<u16>) -> bool {
        let from = source_network.filter(|network| Some(*network) != own);
        match *self {
            Self::Global => true,
            Self::Local => from.is_none(),
            Self::Network(network) => match from {
                None => own == Some(network),
                Some(from) => from == network,
            },
        }
    }
}

/// A station a typed call takes: a [`Station`], a [`BACnetAddress`] (network
/// 0 is this link, any other goes through the router its discovery-table
/// row names), or a MAC on this link (`[u8]`, `[u8; N]`, `Vec<u8>`,
/// [`MacAddr`]), and a reference to any of them.
///
/// The trait is sealed: these are the forms there are. A MAC costs nothing
/// to take: the request goes to it as it always has, with no table lookup.
pub trait ToStation: sealed::Sealed + Sync {}

impl<A: sealed::Sealed + Sync + ?Sized> ToStation for A {}

mod sealed {
    use super::*;

    /// How a request reaches a station in one of its forms.
    pub trait Sealed {
        /// One confirmed request to the station, its answer's service data.
        fn confirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: ConfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<Bytes, Error>> + Send + 'a;

        /// One unconfirmed request to the station.
        fn unconfirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: UnconfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<(), Error>> + Send + 'a;

        /// The station as an owned [`Station`], for one that outlives the
        /// call, such as a managed COV subscription's.
        fn station(&self) -> Station;
    }

    impl Sealed for [u8] {
        fn confirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: ConfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<Bytes, Error>> + Send + 'a {
            client.confirmed_request_inner(
                ConfirmedTarget::Local { mac: self },
                service_choice,
                service_data,
            )
        }

        fn unconfirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: UnconfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<(), Error>> + Send + 'a {
            client.unconfirmed_local(self, service_choice, service_data)
        }

        fn station(&self) -> Station {
            Station::local(self)
        }
    }

    /// A MAC by another name: as `[u8]`.
    macro_rules! mac_station {
        ($([$($param:tt)*] $ty:ty;)*) => {$(
            impl<$($param)*> Sealed for $ty {
                fn confirmed<'a, T: TransportPort + 'static>(
                    &'a self,
                    client: &'a BACnetClient<T>,
                    service_choice: ConfirmedServiceChoice,
                    service_data: &'a [u8],
                ) -> impl Future<Output = Result<Bytes, Error>> + Send + 'a {
                    self[..].confirmed(client, service_choice, service_data)
                }

                fn unconfirmed<'a, T: TransportPort + 'static>(
                    &'a self,
                    client: &'a BACnetClient<T>,
                    service_choice: UnconfirmedServiceChoice,
                    service_data: &'a [u8],
                ) -> impl Future<Output = Result<(), Error>> + Send + 'a {
                    self[..].unconfirmed(client, service_choice, service_data)
                }

                fn station(&self) -> Station {
                    self[..].station()
                }
            }
        )*};
    }

    mac_station! {
        [const N: usize] [u8; N];
        [] Vec<u8>;
        [] MacAddr;
    }

    impl<A: Sealed + ?Sized> Sealed for &A {
        fn confirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: ConfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<Bytes, Error>> + Send + 'a {
            (**self).confirmed(client, service_choice, service_data)
        }

        fn unconfirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: UnconfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<(), Error>> + Send + 'a {
            (**self).unconfirmed(client, service_choice, service_data)
        }

        fn station(&self) -> Station {
            (**self).station()
        }
    }

    impl Sealed for BACnetAddress {
        fn confirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: ConfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<Bytes, Error>> + Send + 'a {
            client.confirmed_at(
                self.network_number,
                &self.mac_address,
                service_choice,
                service_data,
            )
        }

        fn unconfirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: UnconfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> impl Future<Output = Result<(), Error>> + Send + 'a {
            client.unconfirmed_at(
                self.network_number,
                &self.mac_address,
                service_choice,
                service_data,
            )
        }

        fn station(&self) -> Station {
            Station::from(self.clone())
        }
    }

    impl Sealed for Station {
        async fn confirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: ConfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> Result<Bytes, Error> {
            match self {
                Station::Local { mac } => mac.confirmed(client, service_choice, service_data).await,
                Station::Remote { network, mac } => {
                    client
                        .confirmed_at(*network, mac, service_choice, service_data)
                        .await
                }
                Station::Routed {
                    router,
                    network,
                    mac,
                } => {
                    let target = ConfirmedTarget::Routed {
                        router_mac: router,
                        dest_network: *network,
                        dest_mac: mac,
                    };
                    client
                        .confirmed_request_inner(target, service_choice, service_data)
                        .await
                }
            }
        }

        async fn unconfirmed<'a, T: TransportPort + 'static>(
            &'a self,
            client: &'a BACnetClient<T>,
            service_choice: UnconfirmedServiceChoice,
            service_data: &'a [u8],
        ) -> Result<(), Error> {
            match self {
                Station::Local { mac } => {
                    mac.unconfirmed(client, service_choice, service_data).await
                }
                Station::Remote { network, mac } => {
                    client
                        .unconfirmed_at(*network, mac, service_choice, service_data)
                        .await
                }
                Station::Routed {
                    router,
                    network,
                    mac,
                } => {
                    client
                        .unconfirmed_via(router, *network, mac, service_choice, service_data)
                        .await
                }
            }
        }

        fn station(&self) -> Station {
            self.clone()
        }
    }
}

/// Routing target for confirmed requests.
#[derive(Clone, Copy)]
pub(super) enum ConfirmedTarget<'a> {
    Local {
        mac: &'a [u8],
    },
    Routed {
        router_mac: &'a [u8],
        dest_network: u16,
        dest_mac: &'a [u8],
    },
}

impl<'a> ConfirmedTarget<'a> {
    /// The device's address as the discovery table keys it: network 0 and
    /// the MAC for a local target.
    pub(super) fn station(&self) -> (u16, &'a [u8]) {
        match *self {
            Self::Local { mac } => (0, mac),
            Self::Routed {
                dest_network,
                dest_mac,
                ..
            } => (dest_network, dest_mac),
        }
    }

    pub(super) fn additional_npdu_header_len(&self) -> u16 {
        match self {
            Self::Local { .. } => 0,
            Self::Routed { dest_mac, .. } => u16::try_from(dest_mac.len())
                .unwrap_or(u16::MAX)
                .saturating_add(4),
        }
    }
}

/// How a request reaches a station by its address.
enum Route {
    /// On this network, at the station's MAC.
    Local,
    /// Through the router at this MAC on this network.
    Via(MacAddr),
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// A confirmed request to the station at `mac` on `network` (0 for this
    /// link): at its MAC on this link or this client's own network, else
    /// through the router the discovery table's row at the address names.
    async fn confirmed_at(
        &self,
        network: u16,
        mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        match self.route_to(network, mac)? {
            Route::Local => {
                let target = ConfirmedTarget::Local { mac };
                self.confirmed_request_inner(target, service_choice, service_data)
                    .await
            }
            Route::Via(router) => {
                let target = ConfirmedTarget::Routed {
                    router_mac: &router,
                    dest_network: network,
                    dest_mac: mac,
                };
                self.confirmed_request_inner(target, service_choice, service_data)
                    .await
            }
        }
    }

    /// An unconfirmed request to the station at `mac` on `network`, routed
    /// as [`Self::confirmed_at`] routes. On this client's own network the
    /// MAC must be one station, not a broadcast or group address.
    async fn unconfirmed_at(
        &self,
        network: u16,
        mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let route = match self.route_to(network, mac) {
            Ok(route) => route,
            Err(Error::UnknownDevice { .. })
                if service_choice == UnconfirmedServiceChoice::WHO_IS =>
            {
                return self
                    .unconfirmed_request_routed(None, network, mac, service_choice, service_data)
                    .await;
            }
            Err(error) => return Err(error),
        };
        match route {
            Route::Local if network != 0 => {
                self.unconfirmed_own_network(network, mac, service_choice, service_data)
                    .await
            }
            Route::Local => {
                self.unconfirmed_local(mac, service_choice, service_data)
                    .await
            }
            Route::Via(router) => {
                self.unconfirmed_request_routed(
                    Some(&router),
                    network,
                    mac,
                    service_choice,
                    service_data,
                )
                .await
            }
        }
    }

    /// An unconfirmed request through the router at `router` to the station
    /// at `mac` on `network`, or to `mac` on this link when `network` is
    /// this client's own.
    async fn unconfirmed_via(
        &self,
        router: &[u8],
        network: u16,
        mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        Self::station_of(network, mac)?;
        if self.network.local_network_number().get() == Some(network) {
            return self
                .unconfirmed_own_network(network, mac, service_choice, service_data)
                .await;
        }
        self.unconfirmed_request_routed(Some(router), network, mac, service_choice, service_data)
            .await
    }

    /// An unconfirmed request to `mac` on this client's own network, which
    /// must name one station.
    async fn unconfirmed_own_network(
        &self,
        network: u16,
        mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        if self.network.transport().is_group_destination(mac) {
            return Err(Error::Encoding(format!(
                "{} names this network's broadcast or a group, not one station",
                station_text(network, mac)
            )));
        }
        self.unconfirmed_local(mac, service_choice, service_data)
            .await
    }

    /// Send to one remote station. A Who-Is for a station with no known
    /// next hop uses the link broadcast, keeping its DNET and DADR.
    async fn unconfirmed_request_routed(
        &self,
        router: Option<&[u8]>,
        network: u16,
        mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let apdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });
        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &apdu)?;
        if service_choice == UnconfirmedServiceChoice::WHO_IS
            && let Some(router) = router
            && !self.network.is_local_unicast_station(router).await?
        {
            return Err(Error::Encoding(
                "a directed Who-Is needs a unicast router on this link".into(),
            ));
        }
        self.ask_before(service_choice).await?;
        let priority = NetworkPriority::Normal;
        match router {
            Some(router) => {
                self.network
                    .send_apdu_routed(&buf, network, mac, router, false, priority)
                    .await
            }
            None => {
                self.network
                    .send_apdu_routed_via_local_broadcast(&buf, network, mac, false, priority)
                    .await
            }
        }
    }

    /// The checks every address takes before any request state: a MAC that
    /// names one station, and for a remote network one a router can carry.
    pub(super) fn station_of(network: u16, mac: &[u8]) -> Result<(), Error> {
        if mac.is_empty() {
            return Err(Error::Encoding(format!(
                "network {network} with an empty MAC is a broadcast, not one station"
            )));
        }
        if network != 0 {
            check_routed_unicast(network, mac.len())?;
        }
        Ok(())
    }

    /// `to` with its route fixed now, as [`Station::Local`] or
    /// [`Station::Routed`]: a station named by its address
    /// ([`Station::Remote`]) is local on network 0 or this client's own
    /// network, and otherwise goes through the router its discovery-table
    /// row names; any other station is `to` as it is. With no row,
    /// [`Error::UnknownDevice`], as a request to it would fail, before
    /// anything is sent.
    pub(super) fn with_fixed_route(&self, to: Station) -> Result<Station, Error> {
        match to {
            Station::Remote { network, mac } => match self.route_to(network, &mac)? {
                Route::Local => Ok(Station::Local { mac }),
                Route::Via(router) => Ok(Station::Routed {
                    router,
                    network,
                    mac,
                }),
            },
            other => Ok(other),
        }
    }

    /// How a request reaches the station at `mac` on `network`: locally on
    /// network 0 or this client's own network, else through the router its
    /// discovery-table row names. Checked before any request state is
    /// taken.
    fn route_to(&self, network: u16, mac: &[u8]) -> Result<Route, Error> {
        Self::station_of(network, mac)?;
        if network == 0 || self.network.local_network_number().get() == Some(network) {
            return Ok(Route::Local);
        }
        match self.device_table.lock().get(network, mac) {
            Some(row) => Ok(Route::Via(MacAddr::from_slice(&row.mac_address))),
            None => Err(Error::UnknownDevice {
                device: station_text(network, mac),
            }),
        }
    }
}

/// `2:0x05`: a station as the CLI's target grammar writes it.
fn station_text(network: u16, mac: &[u8]) -> String {
    let hex: String = mac.iter().map(|octet| format!("{octet:02X}")).collect();
    format!("{network}:0x{hex}")
}

#[cfg(test)]
#[path = "station_tests.rs"]
mod tests;
