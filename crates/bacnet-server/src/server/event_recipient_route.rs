use super::device_bindings::{BindingFreshness, DeviceResolution};
use super::event_suppression::EventSuppression;
use super::*;
use bacnet_npdu::RemoteBroadcast;
use bacnet_objects::notification_class::local_day_and_time;
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::primitives::Time;
use std::fmt;

const GLOBAL_BROADCAST_NETWORK: u16 = 0xFFFF;

/// A station on another network whose MAC there is one of the forms that
/// name every station of that network, or a group of them, rather than one
/// ([`RemoteBroadcast`], #2709). A router puts the MAC on that network as
/// the link's destination, so a confirmed request sent to it would reach
/// them all, and a confirmed request goes to one station (Clause 6.3): no
/// confirmed request the server starts goes to one (#2732), while an
/// unconfirmed one still may.
///
/// The forms are read from the MAC's length and octets alone, since the
/// far network's data link is unknown here. A recipient, a device binding
/// or a COV subscriber can't say its network is ARCNET, as a client's
/// target can (`Station::arcnet`), so the one octet `0xFF` is always taken
/// as the MS/TP broadcast: ARCNET station 255 gets no confirmed request
/// from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BroadcastStation {
    network: u16,
    mac: MacAddr,
    form: RemoteBroadcast,
}

impl BroadcastStation {
    /// The station at `mac` on `network`, a remote network, when that MAC
    /// is a broadcast or group form; `None` for one station. A few
    /// comparisons on the slice.
    pub(super) fn of(network: u16, mac: &[u8]) -> Option<Self> {
        RemoteBroadcast::of(mac, false).map(|form| Self {
            network,
            mac: MacAddr::from_slice(mac),
            form,
        })
    }
}

impl fmt::Display for BroadcastStation {
    /// The address as the CLI writes it (`5:0xFF`), then what the MAC is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:0x", self.network)?;
        for octet in &self.mac {
            write!(f, "{octet:02X}")?;
        }
        match self.form {
            // The form's own text tells a client how to mark an ARCNET
            // station, which a recipient can't do.
            RemoteBroadcast::MstpBroadcast => f.write_str(
                ", station 255, the broadcast of an MS/TP network (ARCNET station 255 can't \
                 be named here)",
            ),
            form => write!(f, ", {form}"),
        }
    }
}

pub(super) fn system_utc_recipient_filter_time(now: Duration) -> (DaysOfWeek, Time) {
    let (today, mut current_time) = local_day_and_time(now.as_secs(), 0);
    current_time.hundredths = (now.subsec_millis() / 10) as u8;
    (today, current_time)
}

/// The transport action selected for one matched Notification Class recipient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RecipientRoute {
    LocalUnicast(MacAddr),
    BoundLocalUnicast {
        mac: MacAddr,
        freshness: BindingFreshness,
    },
    LocalBroadcast,
    RemoteBroadcast(u16),
    GlobalBroadcast,
    /// An address recipient whose next hop is selected by existing route policy.
    RemoteUnicast {
        network: u16,
        mac: MacAddr,
    },
    /// A Device binding with a fixed local next-hop router.
    BoundRoutedUnicast {
        network: u16,
        mac: MacAddr,
        router: MacAddr,
        freshness: BindingFreshness,
    },
    ContradictoryGlobal,
    UnknownDevice,
    StaleDevice,
    InvalidDevice,
}

/// Why [`RecipientRoute::into_unicast`] or
/// [`RecipientRoute::into_confirmed`] gives no route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConfirmedRouteRefusal {
    /// The route names no single device: a broadcast, or a Device recipient
    /// with no usable binding.
    NotOneDevice,
    /// Its local next hop, the destination's own MAC or the binding's router,
    /// reaches a group of nodes (#1493).
    GroupNextHop,
    /// Its station on another network is at a broadcast or group form there
    /// (#2732): refused for a confirmed request only.
    RemoteBroadcast(BroadcastStation),
}

#[derive(PartialEq, Eq)]
pub(super) struct ConfirmedRecipientRoute {
    pub(super) canonical_peer: bacnet_endpoint_core::coordinator::CanonicalPeer,
    pub(super) local_target: Option<MacAddr>,
    pub(super) remote: Option<(u16, MacAddr, Option<MacAddr>)>,
    pub(super) freshness: Option<BindingFreshness>,
}

impl ConfirmedRecipientRoute {
    /// The route's station on another network, when its MAC there is a
    /// broadcast or group form ([`BroadcastStation`], #2732): a confirmed
    /// request may not take this route, an unconfirmed one may. A route on
    /// this network has none; its local MAC passed the link's own group
    /// check ([`RecipientRoute::into_unicast`]).
    pub(super) fn broadcast_station(&self) -> Option<BroadcastStation> {
        let (network, mac, _) = self.remote.as_ref()?;
        BroadcastStation::of(*network, mac)
    }

    /// Whether a notification, `confirmed` or not, may take this route: an
    /// unconfirmed one always, a confirmed one unless its station is at a
    /// broadcast or group form ([`Self::broadcast_station`]).
    pub(super) fn serves(&self, confirmed: bool) -> bool {
        !confirmed || self.broadcast_station().is_none()
    }
}

impl RecipientRoute {
    /// Preserve the pre-existing address-recipient route distinctions.
    pub(super) fn resolve_address(
        address: &BACnetAddress,
        is_link_broadcast: impl Fn(&[u8]) -> bool,
    ) -> Self {
        match (address.network_number, address.mac_address.is_empty()) {
            (0, true) => Self::LocalBroadcast,
            (0, false) if is_link_broadcast(&address.mac_address) => Self::LocalBroadcast,
            (0, false) => Self::LocalUnicast(address.mac_address.clone()),
            (GLOBAL_BROADCAST_NETWORK, true) => Self::GlobalBroadcast,
            (network, true) => Self::RemoteBroadcast(network),
            (GLOBAL_BROADCAST_NETWORK, false) => Self::ContradictoryGlobal,
            (network, false) => Self::RemoteUnicast {
                network,
                mac: address.mac_address.clone(),
            },
        }
    }

    /// Take a route that names the network numbered `local_network`, the one
    /// this device's port is attached to, as the local route it is: the
    /// destination is on this network, so the NPDU goes with no DNET, as a
    /// local broadcast or a unicast to the MAC (Clause 6.5.1). A non-routing
    /// node drops an NPDU whose DNET names a network (Clause 6.5.2.1), so a
    /// routed form might never arrive. An address at the link's broadcast MAC
    /// (`is_link_broadcast`) is a local broadcast. A Device binding at any
    /// group address (`is_group`, which takes in the link broadcast) names no
    /// single device, so the binding is unusable ([`Self::InvalidDevice`],
    /// skipped as unroutable), as a local binding at such a MAC is (#1493).
    /// With the number unknown every route stays as it is.
    pub(super) fn localize(
        self,
        local_network: Option<u16>,
        is_link_broadcast: impl Fn(&[u8]) -> bool,
        is_group: impl Fn(&[u8]) -> bool,
    ) -> Self {
        let here = |network: u16| Some(network) == local_network;
        match self {
            Self::RemoteBroadcast(network) if here(network) => Self::LocalBroadcast,
            Self::RemoteUnicast { network, mac } if here(network) => {
                if is_link_broadcast(&mac) {
                    Self::LocalBroadcast
                } else {
                    Self::LocalUnicast(mac)
                }
            }
            Self::BoundRoutedUnicast {
                network,
                mac,
                freshness,
                ..
            } if here(network) => {
                if is_group(&mac) {
                    Self::InvalidDevice
                } else {
                    Self::BoundLocalUnicast { mac, freshness }
                }
            }
            route => route,
        }
    }

    pub(super) fn from_device_resolution(resolution: DeviceResolution) -> Self {
        match resolution {
            DeviceResolution::ResolvedLocal {
                peer_mac,
                freshness,
            } => Self::BoundLocalUnicast {
                mac: peer_mac,
                freshness,
            },
            DeviceResolution::ResolvedRouted {
                network,
                final_mac,
                router_mac,
                freshness,
            } => Self::BoundRoutedUnicast {
                network,
                mac: final_mac,
                router: router_mac,
                freshness,
            },
            DeviceResolution::Unknown => Self::UnknownDevice,
            DeviceResolution::Stale => Self::StaleDevice,
            DeviceResolution::Invalid => Self::InvalidDevice,
        }
    }

    /// The route a confirmed request to this recipient takes: each confirmed
    /// request the server starts toward a configured recipient or a bound
    /// device (event notifications, a Notification Forwarder's copies, a
    /// Channel's or Command's requests to another device, and Audit Log
    /// forwarding) gets its route here. Replies and COV notifications go to
    /// the source a request came from and don't come through here; a COV
    /// subscriber takes the same remote check ([`BroadcastStation`]).
    ///
    /// It is [`Self::into_unicast`]'s route, also refused when its station
    /// on another network is at a broadcast or group form there
    /// ([`ConfirmedRecipientRoute::broadcast_station`], #2732): the router
    /// that delivers it would hand it to every station there.
    pub(super) fn into_confirmed(
        self,
        is_group: impl Fn(&[u8]) -> bool,
    ) -> Result<ConfirmedRecipientRoute, ConfirmedRouteRefusal> {
        let route = self.into_unicast(is_group)?;
        match route.broadcast_station() {
            Some(station) => Err(ConfirmedRouteRefusal::RemoteBroadcast(station)),
            None => Ok(route),
        }
    }

    /// The route to the one device this recipient names, for a confirmed
    /// request or an unconfirmed one: audit notifications take it for both,
    /// and check a confirmed one's station at delivery (#2732). A route that
    /// names no device is refused, and so is one whose local next hop, the
    /// destination's MAC or the binding's router, reaches a group of nodes
    /// (`is_group`, [`TransportPort::is_group_destination`]): with no DNET
    /// that send is a local broadcast, which carries only unconfirmed
    /// requests (Clause 6.3), and a binding to a router there names no
    /// single router (#1493).
    pub(super) fn into_unicast(
        self,
        is_group: impl Fn(&[u8]) -> bool,
    ) -> Result<ConfirmedRecipientRoute, ConfirmedRouteRefusal> {
        let (canonical_peer, local_target, remote, freshness) = match self {
            Self::LocalUnicast(mac) => (canonical_direct_peer(&mac), Some(mac), None, None),
            Self::BoundLocalUnicast { mac, freshness } => (
                canonical_direct_peer(&mac),
                Some(mac),
                None,
                Some(freshness),
            ),
            Self::RemoteUnicast { network, mac } => (
                canonical_routed_peer(network, &mac),
                None,
                Some((network, mac, None)),
                None,
            ),
            Self::BoundRoutedUnicast {
                network,
                mac,
                router,
                freshness,
            } => (
                canonical_routed_peer(network, &mac),
                None,
                Some((network, mac, Some(router))),
                Some(freshness),
            ),
            _ => return Err(ConfirmedRouteRefusal::NotOneDevice),
        };
        let next_hop = local_target
            .as_ref()
            .or_else(|| remote.as_ref().and_then(|(_, _, router)| router.as_ref()));
        if next_hop.is_some_and(|mac| is_group(mac)) {
            return Err(ConfirmedRouteRefusal::GroupNextHop);
        }
        Ok(ConfirmedRecipientRoute {
            canonical_peer,
            local_target,
            remote,
            freshness,
        })
    }

    /// The counter a matched destination on this route moves when it is
    /// skipped, or `None` when the route can carry the notification. Logs only
    /// bounded classification data. Each route shape has exactly one outcome,
    /// so a skipped destination counts once.
    pub(super) fn skip(
        &self,
        confirmed: bool,
        notification_class: u32,
    ) -> Option<EventSuppression> {
        let (reason, suppression) = match self {
            Self::LocalUnicast(_)
            | Self::BoundLocalUnicast { .. }
            | Self::RemoteUnicast { .. }
            | Self::BoundRoutedUnicast { .. } => return None,
            Self::LocalBroadcast | Self::RemoteBroadcast(_) | Self::GlobalBroadcast
                if !confirmed =>
            {
                return None;
            }
            // Clause 6.3 restricts broadcast to Unconfirmed-Request-PDUs, and
            // downgrading would drop the acknowledgment the recipient was
            // configured to require, so both are skips.
            Self::LocalBroadcast | Self::RemoteBroadcast(_) | Self::GlobalBroadcast => {
                warn!(
                    notification_class,
                    "Recipient requests confirmed notifications at a broadcast address; \
                     Clause 6.3 permits only unconfirmed PDUs there, skipping"
                );
                return Some(EventSuppression::ConfirmedBroadcastRecipient);
            }
            Self::ContradictoryGlobal => {
                warn!(
                    notification_class,
                    "Skipping recipient: global broadcast network has a unicast address"
                );
                return Some(EventSuppression::RecipientUnroutable);
            }
            Self::UnknownDevice => ("unknown", EventSuppression::DeviceRecipientUnbound),
            Self::StaleDevice => ("stale", EventSuppression::DeviceRecipientUnbound),
            Self::InvalidDevice => ("invalid", EventSuppression::RecipientUnroutable),
        };
        warn!(
            notification_class,
            reason, "Skipping Device recipient: binding is unusable"
        );
        Some(suppression)
    }
}

#[cfg(test)]
#[path = "broadcast_station_tests.rs"]
pub(super) mod broadcast_station_tests;
