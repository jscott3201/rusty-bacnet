//! [`CovTarget`]: the device a managed COV subscription goes to (#1790).

use super::*;

/// The device a managed COV subscription goes to: on this network at a MAC,
/// or on another network behind a router.
///
/// A device keeps a subscription under the address the request came from
/// (Clause 13.14.2), and its notifications carry that subscription's process
/// identifier back from the device's own address (Clauses 13.6.1 and
/// 13.7.1). So a notification belongs to this target when it comes from the
/// same address: the MAC with no source network for a local device, or the
/// source network and station address for a routed one. The router a
/// routed notification came through is not part of the device's address and
/// isn't compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CovTarget {
    /// A device on this network.
    Local {
        /// Its MAC address.
        mac: MacAddr,
    },
    /// A device on another network, reached through a router on this one.
    Routed {
        /// The MAC of the router the requests go to.
        router_mac: MacAddr,
        /// The device's network number, 1 to 65534.
        network: u16,
        /// The device's MAC address on that network.
        address: MacAddr,
    },
}

impl CovTarget {
    /// A device on this network at `mac`.
    pub fn local(mac: &[u8]) -> Self {
        Self::Local {
            mac: MacAddr::from_slice(mac),
        }
    }

    /// A device at `address` on `network`, reached through the router at
    /// `router_mac`.
    pub fn routed(router_mac: &[u8], network: u16, address: &[u8]) -> Self {
        Self::Routed {
            router_mac: MacAddr::from_slice(router_mac),
            network,
            address: MacAddr::from_slice(address),
        }
    }

    /// Where a discovered device is: routed when its I-Am came with a source
    /// network and address (through the router it came from), local
    /// otherwise.
    pub fn of_device(device: &DiscoveredDevice) -> Self {
        match (device.source_network, &device.source_address) {
            (Some(network), Some(address)) => Self::Routed {
                router_mac: device.mac_address.clone(),
                network,
                address: address.clone(),
            },
            _ => Self::Local {
                mac: device.mac_address.clone(),
            },
        }
    }

    pub(super) fn confirmed(&self) -> ConfirmedTarget<'_> {
        match self {
            Self::Local { mac } => ConfirmedTarget::Local { mac },
            Self::Routed {
                router_mac,
                network,
                address,
            } => ConfirmedTarget::Routed {
                router_mac,
                dest_network: *network,
                dest_mac: address,
            },
        }
    }

    /// True when `received` came from this device's address. A routed
    /// target on the client's own network (`local_network`, once learned)
    /// is sent to as a local device (#1358), so its notifications may come
    /// with no source network, from its address as the MAC.
    pub(super) fn sent(
        &self,
        received: &ReceivedCOVNotification,
        local_network: Option<u16>,
    ) -> bool {
        let local_from = |mac: &MacAddr| {
            received.source_network.is_none()
                && received.source_address.is_none()
                && received.source_mac == *mac
        };
        match self {
            Self::Local { mac } => local_from(mac),
            Self::Routed {
                network, address, ..
            } => {
                (received.source_network == Some(*network)
                    && received.source_address.as_ref() == Some(address))
                    || (local_network == Some(*network) && local_from(address))
            }
        }
    }
}
