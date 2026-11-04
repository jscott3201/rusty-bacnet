//! Routed source Audit routes (#2371): a recipient Device behind a router,
//! named by its network number and its MAC there, and the routers the
//! session learns for those networks.
//!
//! The session finds a network's router the way Clause 6.5.3 leaves to the
//! node: the first records to a network go with DNET/DADR and the link's
//! broadcast MAC, and the immediate MAC of any traffic that comes back from
//! that network (an answer the session admits, or a request from a station
//! there) is that network's router from then on, as is the sender of an
//! I-Am-Router-To-Network that lists it. A delivery through a learned
//! router that is lost (no answer in time, or a failed send) forgets it, so
//! the next record goes by broadcast again and the router is learned anew;
//! a peer's Error, Reject or Abort came back through the router, so it
//! stays.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination;
use bacnet_network::layer::{
    NetworkReport, ReceivedApdu, ReceivedNetworkControl, ReceivedNetworkReport,
};
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_types::MacAddr;

/// Where one source Audit record goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceRoute {
    /// A MAC on the session's own link.
    Direct(MacAddr),
    /// A station on another network: its number and its MAC there.
    Routed {
        /// The station's network, 1 to 65534 and not the session's own.
        network: u16,
        /// The station's MAC on that network.
        mac: MacAddr,
    },
}

impl SourceRoute {
    /// The peer a confirmed record's answer comes from: the station itself,
    /// as the SNET/SADR a router adds names it (#1465).
    pub(crate) fn peer(&self) -> CanonicalPeer {
        match self {
            Self::Direct(mac) => CanonicalPeer::direct(mac),
            Self::Routed { network, mac } => CanonicalPeer::routed(*network, mac),
        }
    }
}

/// Whether `network` and `mac` name one station behind a router, as a
/// routed confirmed request must (#1278): a network of 1 to 65534 (0 is
/// how an address names the local network, 65535 every network) and a MAC
/// of 1 to 18 octets (an empty one is that network's broadcast). The MAC's
/// length isn't checked against this node's link, since the station's
/// network may be another kind of link; 18 octets is the most an NPDU's
/// DADR and a BACnetAddress carry.
pub(crate) fn routed_unicast(network: u16, mac: &[u8]) -> bool {
    (1..=0xFFFE).contains(&network) && (1..=bacnet_npdu::MAX_MAC_LEN).contains(&mac.len())
}

/// A send's next hop through a learned router, which a lost delivery
/// forgets.
pub(crate) struct Hop(Option<(Arc<SourceRouters>, u16, MacAddr)>);

impl Hop {
    /// A send with no learned router: direct, by broadcast or global.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// The delivery through this hop was lost: forget its router, unless
    /// another has been learned since.
    pub(crate) fn lost(self) {
        if let Some((routers, network, router)) = self.0 {
            let mut known = routers.routers.lock().unwrap();
            if let Some(slot) = known.get_mut(&network)
                && slot.as_ref() == Some(&router)
            {
                *slot = None;
            }
        }
    }
}

/// The router learned for each network a routed binding names. It holds
/// only those networks, so it is bounded by the bindings.
#[derive(Debug, Default)]
pub(crate) struct SourceRouters {
    routers: Mutex<HashMap<u16, Option<MacAddr>>>,
    /// The session's own network number, once the started link's slot is
    /// handed over: a record queued for a network that has since become
    /// this one goes direct.
    local_network: OnceLock<LocalNetworkNumber>,
}

impl SourceRouters {
    /// Track `network`, with no router learned yet.
    pub(crate) fn track(&self, network: u16) {
        self.routers.lock().unwrap().entry(network).or_default();
    }

    /// Read the session's own network number from `slot` from now on.
    pub(super) fn set_local_network(&self, slot: LocalNetworkNumber) {
        let _ = self.local_network.set(slot);
    }

    fn local_network(&self) -> Option<u16> {
        self.local_network.get().and_then(LocalNetworkNumber::get)
    }

    /// Whether any network is tracked: the session has routed bindings.
    pub(crate) fn is_empty(&self) -> bool {
        self.routers.lock().unwrap().is_empty()
    }

    /// `router` reaches `network`, if a binding names that network and it
    /// isn't this one.
    fn learn(&self, network: u16, router: &MacAddr) {
        if router.is_empty() || Some(network) == self.local_network() {
            return;
        }
        if let Some(slot) = self.routers.lock().unwrap().get_mut(&network) {
            *slot = Some(router.clone());
        }
    }

    /// Learn from traffic that came from another network: its immediate MAC
    /// is the router to that network. Traffic from this network, from a
    /// network no binding names, or naming no station teaches nothing.
    pub(crate) fn observe(&self, received: &ReceivedApdu) {
        if let Some(source) = received
            .source_network
            .as_ref()
            .filter(|source| !source.mac_address.is_empty())
        {
            self.learn(source.network, &received.source_mac);
        }
    }

    /// Learn from an I-Am-Router-To-Network: its sender reaches each
    /// network it lists (Clause 6.4.2).
    pub(crate) fn observe_control(&self, control: &ReceivedNetworkControl) {
        if let Some(ReceivedNetworkReport {
            source_mac,
            report: NetworkReport::IAmRouterToNetwork(networks),
            ..
        }) = ReceivedNetworkReport::parse(control)
        {
            for network in networks {
                self.learn(network, &source_mac);
            }
        }
    }

    /// Where a record for `route` goes: a direct one to its MAC; a routed
    /// one through the router learned for its network, or with the link's
    /// broadcast MAC while none is (Clause 6.5.3). A routed one for a
    /// network that has become this one, queued or in flight when the
    /// number changed, goes to its MAC with no DNET, since a non-routing
    /// node drops an NPDU whose DNET names a network (Clause 6.5.2.1).
    pub(crate) fn destination(
        self: &Arc<Self>,
        route: &SourceRoute,
    ) -> (EndpointApduDestination, Hop) {
        match route {
            SourceRoute::Routed { network, mac } if Some(*network) != self.local_network() => {
                let router = self.routers.lock().unwrap().get(network).cloned().flatten();
                match router {
                    Some(router) => (
                        EndpointApduDestination::Routed {
                            destination_network: *network,
                            destination_mac: mac.clone(),
                            router_mac: router.clone(),
                        },
                        Hop(Some((Arc::clone(self), *network, router))),
                    ),
                    None => (
                        EndpointApduDestination::RoutedViaLocalBroadcast {
                            destination_network: *network,
                            destination_mac: mac.clone(),
                        },
                        Hop::none(),
                    ),
                }
            }
            SourceRoute::Direct(mac) | SourceRoute::Routed { mac, .. } => (
                EndpointApduDestination::Direct {
                    destination_mac: mac.clone(),
                },
                Hop::none(),
            ),
        }
    }

    /// The router learned for `network`, if any.
    #[cfg(test)]
    pub(crate) fn router(&self, network: u16) -> Option<MacAddr> {
        self.routers
            .lock()
            .unwrap()
            .get(&network)
            .cloned()
            .flatten()
    }

    /// Teach `router` for `network`, as traffic from there would.
    #[cfg(test)]
    pub(crate) fn learn_for_test(&self, network: u16, router: &[u8]) {
        self.learn(network, &MacAddr::from_slice(router));
    }
}
