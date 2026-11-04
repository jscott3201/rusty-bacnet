//! Where one endpoint APDU send goes: the [`EndpointApduDestination`] the
//! ingress's egress takes, split from `endpoint_ingress.rs` (#2357).

use bacnet_types::MacAddr;

/// Network-layer destination for one hidden endpoint APDU send.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointApduDestination {
    /// Direct unicast on the local data link. A MAC that reaches a group of
    /// nodes here, such as the link's broadcast MAC or a multicast address,
    /// is a local broadcast, so it carries only an Unconfirmed-Request APDU
    /// (Clause 6.3, #1479).
    Direct {
        /// Destination MAC on the local data link.
        destination_mac: MacAddr,
    },
    /// Routed unicast through a known next-hop router. An empty
    /// `destination_mac` asks that router to broadcast on the network, so it
    /// carries only an Unconfirmed-Request APDU (Clause 6.3, #1479).
    Routed {
        /// Ultimate BACnet network number.
        destination_network: u16,
        /// Ultimate BACnet MAC address.
        destination_mac: MacAddr,
        /// Immediate router MAC on the local data link.
        router_mac: MacAddr,
    },
    /// Routed unicast using a local broadcast because the router MAC is unknown.
    /// An empty `destination_mac` is refused: [`Self::RemoteBroadcast`]
    /// sends that network's broadcast (#1479).
    RoutedViaLocalBroadcast {
        /// Ultimate BACnet network number.
        destination_network: u16,
        /// Ultimate BACnet MAC address.
        destination_mac: MacAddr,
    },
    /// Broadcast on only the local BACnet network.
    LocalBroadcast,
    /// Broadcast to one remote BACnet network.
    RemoteBroadcast {
        /// Ultimate BACnet network number.
        destination_network: u16,
    },
    /// Broadcast to all reachable BACnet networks.
    GlobalBroadcast,
}
