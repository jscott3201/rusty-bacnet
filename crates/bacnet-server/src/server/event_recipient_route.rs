use super::device_bindings::DeviceResolution;
use super::*;
use bacnet_types::constructed::BACnetAddress;

const GLOBAL_BROADCAST_NETWORK: u16 = 0xFFFF;

/// The transport action selected for one matched Notification Class recipient.
pub(super) enum RecipientRoute {
    LocalUnicast(MacAddr),
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
    },
    ContradictoryGlobal,
    UnknownDevice,
    StaleDevice,
    InvalidDevice,
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
            DeviceResolution::ResolvedLocal { peer_mac } => Self::LocalUnicast(peer_mac),
            DeviceResolution::ResolvedRouted {
                network,
                final_mac,
                router_mac,
            } => Self::BoundRoutedUnicast {
                network,
                mac: final_mac,
                router: router_mac,
            },
            DeviceResolution::Unknown => Self::UnknownDevice,
            DeviceResolution::Stale => Self::StaleDevice,
            DeviceResolution::Invalid => Self::InvalidDevice,
        }
    }

    pub(super) fn permits_confirmed(&self) -> bool {
        matches!(
            self,
            Self::LocalUnicast(_) | Self::RemoteUnicast { .. } | Self::BoundRoutedUnicast { .. }
        )
    }

    /// Log only bounded classification data for unusable recipients.
    pub(super) fn is_deliverable(&self, notification_class: u32) -> bool {
        match self {
            Self::LocalUnicast(_)
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
