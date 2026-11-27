//! Routed source Audit routes (#2371): a recipient Device behind a router,
//! named by its network number and its MAC there, and the routers the
//! session's records to those networks go through.
//!
//! Those routers are a [`RouterLearning`]'s, by the rule the server's own
//! requests follow (#2831): the first records to a network go with DNET/DADR
//! and the link's broadcast MAC (Clause 6.5.3), and the session learns the
//! network's router only from its own router query (Who-Is-Router-To-Network,
//! Clause 6.4.1), when the same one router alone answers two of its queries
//! a minute apart. No answer to a record,
//! no request from the network and no I-Am-Router-To-Network it did not ask
//! for teaches one. A delivery through the router that is lost forgets
//! nothing: it starts a check of the router, which moves the records to
//! another router only after two checks a minute apart find it silent and
//! the same other router alone answering.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};

use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination;
use bacnet_network::layer::ReceivedApdu;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_network::router_learning::RouterLearning;
use bacnet_types::MacAddr;
use bacnet_types::sync::MutexExt;

use super::recipient::SourceLink;

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
        /// The binding marks the address one station (#2738).
        one_station: bool,
    },
}

impl SourceRoute {
    /// The peer a confirmed record's answer is matched to
    /// ([`CanonicalPeer::of_answer`]): the station's MAC for a direct route,
    /// the network and MAC a router adds as SNET/SADR for a routed one.
    pub(crate) fn peer(&self) -> CanonicalPeer {
        match self {
            Self::Direct(mac) => CanonicalPeer::direct(mac),
            Self::Routed { network, mac, .. } => CanonicalPeer::routed(*network, mac),
        }
    }

    /// The broadcast or group form a routed station's MAC is on its network
    /// ([`bacnet_npdu::RemoteBroadcast`], #2732), as the binding's
    /// one-station mark has it (#2738), if it is one: the router there would
    /// hand a record sent to it to every station. Unmarked, the one octet
    /// `0xFF` is the MS/TP broadcast and two octets starting `0x00` or `0xFF`
    /// a LonTalk broadcast or group; a binding marked one station reaches
    /// ARCNET's station 255 and a gateway's two-octet virtual station. A MAC
    /// on the session's own link has none; the link's unicast rule judges
    /// it.
    pub(crate) fn broadcast_form(&self) -> Option<bacnet_npdu::RemoteBroadcast> {
        match self {
            Self::Routed {
                mac, one_station, ..
            } => bacnet_npdu::RemoteBroadcast::of(mac, *one_station),
            Self::Direct(_) => None,
        }
    }

    /// Whether a record, `confirmed` or not, may take this route: an
    /// unconfirmed one always, a confirmed one unless the station is at a
    /// broadcast or group form ([`Self::broadcast_form`]), since a
    /// confirmed request goes to one station.
    pub(crate) fn serves(&self, confirmed: bool) -> bool {
        !confirmed || self.broadcast_form().is_none()
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

/// A send's next hop through a known router, whose lost delivery starts a
/// check of that router.
pub(crate) struct Hop(Option<(Arc<SourceRouters>, u16, MacAddr, MacAddr)>);

impl Hop {
    /// A send with no known router: direct, by broadcast or global.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// The delivery through this hop was lost: check its router, while it
    /// is still the network's (#2831). Nothing is forgotten.
    pub(crate) fn lost(self) {
        if let Some((routers, network, router, station)) = self.0 {
            routers.learning.silent(network, &router, &station);
        }
    }
}

/// The networks routed bindings name, and the routers the session's
/// records to them go through. It tracks only those networks, so it is
/// bounded by the bindings.
#[derive(Debug, Default)]
pub(crate) struct SourceRouters {
    networks: Mutex<BTreeSet<u16>>,
    /// The routers, learned and replaced only through the session's own
    /// router query; the session runs its checks ([`Self::learning`]).
    learning: Arc<RouterLearning>,
    /// The session's own network number, once the started link's slot is
    /// handed over: a record queued for a network that has since become
    /// this one goes direct.
    local_network: OnceLock<LocalNetworkNumber>,
    /// The started link's unicast rule, which such a record must meet, as
    /// a direct binding does.
    link: OnceLock<SourceLink>,
}

impl SourceRouters {
    /// Track `network`, with no router known yet.
    pub(crate) fn track(&self, network: u16) {
        self.networks.locked().insert(network);
    }

    /// Take the started link's unicast rule, and read the session's own
    /// network number from `slot` from now on.
    pub(super) fn finalize(&self, link: SourceLink, slot: LocalNetworkNumber) {
        let _ = self.link.set(link);
        let _ = self.local_network.set(slot);
    }

    fn local_network(&self) -> Option<u16> {
        self.local_network.get().and_then(LocalNetworkNumber::get)
    }

    /// Whether any network is tracked: the session has routed bindings.
    pub(crate) fn is_empty(&self) -> bool {
        self.networks.locked().is_empty()
    }

    /// The learner the session runs the checks of.
    pub(crate) fn learning(&self) -> &Arc<RouterLearning> {
        &self.learning
    }

    /// Traffic from another network arrived from link station
    /// `received.source_mac`: when that is the known router of a network a
    /// binding names, it still carries the network's traffic. It teaches
    /// nothing (#2831).
    pub(crate) fn observe(&self, received: &ReceivedApdu) {
        if let Some(source) = received.source_network.as_ref()
            && self.networks.locked().contains(&source.network)
        {
            self.learning.heard(source.network, &received.source_mac);
        }
    }

    /// Where a record for `route` goes: a direct one to its MAC; a routed
    /// one through the router known for its network, or with the link's
    /// broadcast MAC while none is (Clause 6.5.3), which has the session
    /// ask which router reaches it. A routed one for a
    /// network that has become this one, queued or in flight when the
    /// number changed, goes to its MAC with no DNET, since a non-routing
    /// node drops an NPDU whose DNET names a network (Clause 6.5.2.1); it
    /// must then be a direct unicast MAC by the link's rule, as
    /// [`SourceRoutes::resolve`](super::recipient::SourceRoutes::resolve)
    /// requires of new records, or it has no destination (`None`).
    pub(crate) fn destination(
        self: &Arc<Self>,
        route: &SourceRoute,
    ) -> Option<(EndpointApduDestination, Hop)> {
        Some(match route {
            SourceRoute::Routed { network, mac, .. } if Some(*network) != self.local_network() => {
                match self.learning.router(*network) {
                    Some(router) => (
                        EndpointApduDestination::Routed {
                            destination_network: *network,
                            destination_mac: mac.clone(),
                            router_mac: router.clone(),
                        },
                        Hop(Some((Arc::clone(self), *network, router, mac.clone()))),
                    ),
                    None => {
                        self.learning.unrouted(*network);
                        (
                            EndpointApduDestination::RoutedViaLocalBroadcast {
                                destination_network: *network,
                                destination_mac: mac.clone(),
                            },
                            Hop::none(),
                        )
                    }
                }
            }
            SourceRoute::Direct(mac) => (
                EndpointApduDestination::Direct {
                    destination_mac: mac.clone(),
                },
                Hop::none(),
            ),
            SourceRoute::Routed { mac, .. } => {
                if !self.link.get().is_some_and(|link| link.unicast(mac)) {
                    return None;
                }
                (
                    EndpointApduDestination::Direct {
                        destination_mac: mac.clone(),
                    },
                    Hop::none(),
                )
            }
        })
    }

    /// The router known for `network`, if any.
    #[cfg(test)]
    pub(crate) fn router(&self, network: u16) -> Option<MacAddr> {
        self.learning.router(network)
    }

    /// Know `router` for `network` at once, as the session's router query
    /// would have it.
    #[cfg(test)]
    pub(crate) fn learn_for_test(&self, network: u16, router: &[u8]) {
        self.learning.learn_for_test(network, router);
    }
}
