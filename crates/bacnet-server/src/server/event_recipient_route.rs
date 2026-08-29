use super::device_bindings::{BindingFreshness, DeviceResolution};
use super::*;
use bacnet_objects::notification_class::local_day_and_time;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::primitives::Time;

const GLOBAL_BROADCAST_NETWORK: u16 = 0xFFFF;

pub(super) fn network_priority_for_event(priority: u8) -> NetworkPriority {
    match priority {
        0..=63 => NetworkPriority::LIFE_SAFETY,
        64..=127 => NetworkPriority::CRITICAL_EQUIPMENT,
        128..=191 => NetworkPriority::URGENT,
        192..=255 => NetworkPriority::NORMAL,
    }
}

pub(super) fn system_utc_recipient_filter_time(now: Duration) -> (u8, Time) {
    let (today_bit, mut current_time) = local_day_and_time(now.as_secs(), 0);
    current_time.hundredths = (now.subsec_millis() / 10) as u8;
    (today_bit, current_time)
}

/// The transport action selected for one matched Notification Class recipient.
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

pub(super) struct ConfirmedRecipientRoute {
    pub(super) canonical_peer: bacnet_endpoint_core::coordinator::CanonicalPeer,
    pub(super) local_target: Option<MacAddr>,
    pub(super) remote: Option<(u16, MacAddr, Option<MacAddr>)>,
    pub(super) freshness: Option<BindingFreshness>,
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

    pub(super) fn permits_confirmed(&self) -> bool {
        matches!(
            self,
            Self::LocalUnicast(_)
                | Self::BoundLocalUnicast { .. }
                | Self::RemoteUnicast { .. }
                | Self::BoundRoutedUnicast { .. }
        )
    }

    pub(super) fn into_confirmed(self) -> Option<ConfirmedRecipientRoute> {
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
            _ => return None,
        };
        Some(ConfirmedRecipientRoute {
            canonical_peer,
            local_target,
            remote,
            freshness,
        })
    }

    /// Log only bounded classification data for unusable recipients.
    pub(super) fn is_deliverable(&self, notification_class: u32) -> bool {
        match self {
            Self::LocalUnicast(_)
            | Self::BoundLocalUnicast { .. }
            | Self::LocalBroadcast
            | Self::RemoteBroadcast(_)
            | Self::GlobalBroadcast
            | Self::RemoteUnicast { .. }
            | Self::BoundRoutedUnicast { .. } => true,
            Self::ContradictoryGlobal => {
                warn!(
                    notification_class,
                    "Skipping recipient: global broadcast network has a unicast address"
                );
                false
            }
            Self::UnknownDevice => {
                warn!(
                    notification_class,
                    reason = "unknown",
                    "Skipping Device recipient: binding is unusable"
                );
                false
            }
            Self::StaleDevice => {
                warn!(
                    notification_class,
                    reason = "stale",
                    "Skipping Device recipient: binding is unusable"
                );
                false
            }
            Self::InvalidDevice => {
                warn!(
                    notification_class,
                    reason = "invalid",
                    "Skipping Device recipient: binding is unusable"
                );
                false
            }
        }
    }
}
