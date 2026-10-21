//! What [`NetworkLayer`](super::NetworkLayer)'s dispatch does with each
//! frame: `bacnet-router-core`'s [`Node`] decides, by the same admission
//! checks the router's engine runs (#1795), and its drained counts are
//! added to the layer's totals ([`Counters`]).

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_router_core::node::{self, Node};
use bacnet_router_core::router::Discard;
use bacnet_transport::warn_throttle::WarnThrottle;
use bytes::{Buf, Bytes};
use tracing::{debug, warn};

use crate::totals::totals;

/// What a [`NetworkLayer`](super::NetworkLayer) has dropped on receipt
/// since it was created, by reason. Each total saturates at `u64::MAX`.
/// A dropped NPDU reaches neither the APDU nor the control receiver, so
/// it never shows up in their admission counters. The totals never change
/// a decision, and name no peer.
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `Counters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counters {
    /// NPDUs with an address past [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN):
    /// their DLEN or SLEN (#1141), or the link-layer source MAC the
    /// transport reported (#1198).
    pub address_length_drops: u64,
    /// NPDUs whose DNET 0xFFFF also carried a DADR (#1379).
    pub global_broadcast_dadr_drops: u64,
    /// Broadcasts, global or remote, whose APDU wasn't an
    /// Unconfirmed-Request (#1491).
    pub broadcast_pdu_type_drops: u64,
    /// Frames that didn't parse as an NPDU for any other reason.
    pub malformed: u64,
    /// NPDUs whose DNET named one network: for a device elsewhere, which a
    /// node that doesn't route drops (Clauses 6.5.2.1, 6.5.2.2 and 6.5.4).
    pub routed_drops: u64,
}

totals!(
    /// A layer's totals since it was created, one atomic each. Its dispatch
    /// task adds a frame's counts when the node dropped the frame.
    NodeTotals: node::Counters => Counters {
        address_length_drops,
        global_broadcast_dadr_drops,
        broadcast_pdu_type_drops,
        malformed,
        routed_drops,
    }
);

/// What one frame is for, owned, so the frame it was read from can move.
pub(crate) enum Inbound {
    /// An APDU for the application.
    Apdu {
        /// The APDU: the frame, advanced past the NPCI, not a copy.
        apdu: Bytes,
        /// SNET and SADR, when a router relayed it.
        source_network: Option<NpduAddress>,
        /// Whether it was sent to a group.
        is_group: bool,
        /// Whether it was a global broadcast.
        global_broadcast: bool,
    },
    /// A network message, for the control receiver: `None` when the layer
    /// has none, so nothing was copied.
    Control(Option<Npdu>),
    /// Dropped, and counted.
    Dropped,
}

/// Run `frame`, from link source `source_mac`, through `node`, adding what
/// it counted to `totals`. `controls` says whether a control receiver
/// wants network messages. A drop warns at most once per `drop_warning`
/// interval, with the drops since the last warning; the rest log at debug,
/// so a flood of bad frames doesn't flood the log.
pub(crate) fn admit(
    node: &mut Node,
    totals: &NodeTotals,
    drop_warning: &WarnThrottle,
    source_mac: &[u8],
    link_group: bool,
    frame: Bytes,
    controls: bool,
) -> Inbound {
    let (offset, inbound) = match node.receive(source_mac, link_group, &frame) {
        node::Received::Apdu(apdu) => (
            apdu.npdu.payload_offset,
            Inbound::Apdu {
                apdu: Bytes::new(),
                source_network: apdu.npdu.source.map(NpduAddress::from),
                is_group: apdu.group,
                global_broadcast: apdu.global,
            },
        ),
        node::Received::Message(npdu) if controls => (
            npdu.payload_offset,
            Inbound::Control(Some(Npdu::header_from(&npdu))),
        ),
        node::Received::Message(npdu) => {
            debug!(
                message_type = ?npdu.message_type,
                "Ignoring network layer message (non-router mode)"
            );
            return Inbound::Control(None);
        }
        node::Received::Discarded(discard) => {
            totals.add(&node.take_counters());
            if discard == Discard::NotRouted {
                debug!("Discarding routed message (non-router)");
            } else if let Some(dropped) = drop_warning.note() {
                warn!(
                    ?discard,
                    dropped, "Dropping an inbound NPDU (drops since the last warning)"
                );
            } else {
                debug!(?discard, "Dropping an inbound NPDU");
            }
            return Inbound::Dropped;
        }
    };
    // Advanced, not sliced: a slice is a clone, and the first clone of a
    // frame received into a plain buffer allocates its shared header (#1713).
    let mut payload = frame;
    payload.advance(offset);
    match inbound {
        Inbound::Apdu {
            source_network,
            is_group,
            global_broadcast,
            ..
        } => Inbound::Apdu {
            apdu: payload,
            source_network,
            is_group,
            global_broadcast,
        },
        Inbound::Control(Some(mut npdu)) => {
            npdu.payload = payload;
            Inbound::Control(Some(npdu))
        }
        other => other,
    }
}
