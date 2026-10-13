//! Hand an APDU the engine delivers locally to the router's application,
//! with the original ingress metadata.

use bacnet_encoding::npdu::NpduAddress;
use bacnet_router_core::router::Local;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;

use crate::layer::ReceivedApdu;

/// What a [`Local`] result says, owned, so the frame it borrowed can move.
#[derive(Debug, Clone)]
pub(super) struct Delivery {
    /// Where the APDU starts in the frame's NPDU.
    offset: usize,
    /// SNET and SADR, when a router relayed it.
    source: Option<NpduAddress>,
    /// Whether it was sent to a group.
    group: bool,
    /// Whether it was a global broadcast.
    global: bool,
}

impl Delivery {
    /// The delivery `local` describes.
    pub(super) fn of(local: &Local<'_>) -> Self {
        Self {
            offset: local.apdu_offset,
            source: local.source.map(|source| NpduAddress {
                network: source.network,
                mac_address: MacAddr::from_slice(source.mac),
            }),
            group: local.group,
            global: local.global,
        }
    }

    /// The APDU `received` carries, as the application gets it, from the
    /// port on `port_network`. The APDU is a slice of the frame, not a copy.
    ///
    /// A broadcast on the arrival network, a DLEN 0 one or a group DADR
    /// sent as one, is also passed back out that port, and its delivery
    /// keeps no MS/TP reply channel, as before the engine: it is a copy of
    /// the frame the router relays. Every other delivery keeps it.
    pub(super) fn application(self, received: ReceivedNpdu, port_network: u16) -> ReceivedApdu {
        let relayed_too = self.group && !self.global && has_destination(&received.npdu);
        ReceivedApdu {
            apdu: received.npdu.slice(self.offset..),
            source_mac: received.source_mac,
            ingress_network: Some(port_network),
            source_network: self.source,
            link_layer_group: received.link_layer_group,
            is_group: self.group,
            global_broadcast: self.global,
            data_attributes: received.data_attributes,
            provenance: received.provenance,
            direct_response: received.direct_response,
            reply_tx: if relayed_too { None } else { received.reply_tx },
        }
    }
}

/// Whether an NPDU the engine parsed names a DNET: its control octet's
/// destination bit.
fn has_destination(npdu: &[u8]) -> bool {
    npdu.get(1).is_some_and(|control| control & 0x20 != 0)
}
