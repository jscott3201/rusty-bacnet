//! What the receive loop does with one datagram, once the node's [`Link`]
//! has decided: hand an NPDU up, answer with a NAK, match a response to this
//! node's own requests, hand a BBMD its share, or count the drop.
//!
//! [`Link`]: bacnet_bip_core::node::Link

use std::net::SocketAddrV4;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{Datagram, Discard, Received};
use bacnet_types::MacAddr;

use crate::bbmd::BbmdState;
use crate::bvll::{encode_bip_mac, encode_bvll};
use crate::npdu_channel::{self, Refused};
use crate::port::ReceivedNpdu;

use super::BipSocket;
use super::fanout::FanoutDispatcher;
use super::groups::GroupDrops;
use super::node::{HostPolicy, NodeHandle};
use super::rate_limit::ManagementRateLimiter;
use super::receive_counters::{DropReason, ReceiveDrops};

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
pub(super) struct RecvContext {
    /// The node: its link decides each datagram, and it matches responses
    /// to its own requests.
    pub(super) node: Arc<NodeHandle>,
    pub(super) npdu_tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) bbmd: Option<Arc<std::sync::Mutex<BbmdState>>>,
    pub(super) management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    pub(super) fanout: Option<FanoutDispatcher>,
    /// Where a group UDP source (#1504) or Forwarded-NPDU origin (#1493) is
    /// counted.
    pub(super) group_drops: GroupDrops,
    /// Where the datagrams this loop drops are counted (#1624), shared with
    /// the transport.
    pub(super) drops: Arc<ReceiveDrops>,
    #[cfg(test)]
    pub(super) force_dbtn_forward_failure: bool,
}

impl RecvContext {
    /// The socket every send leaves from.
    pub(super) fn socket(&self) -> &BipSocket {
        self.node.socket()
    }

    /// This subnet's B/IP broadcast address, where a BBMD rebroadcasts.
    pub(super) fn local_broadcast(&self) -> SocketAddrV4 {
        self.node.link().subnet.broadcast
    }
}

/// Hand an NPDU this transport received up to the network layer, or count
/// it dropped when the channel is full or closed. `frame` names its kind for
/// the log. The channel's slot is reserved before the NPDU is copied, so a
/// drop costs no allocation.
pub(super) fn hand_up(
    ctx: &RecvContext,
    npdu: &[u8],
    source: SocketAddrV4,
    link_layer_group: bool,
    frame: &'static str,
) {
    let npdu = || Bytes::copy_from_slice(npdu);
    let source_mac = MacAddr::from(encode_bip_mac(source.ip().octets(), source.port()));
    match npdu_channel::hand_up(&ctx.npdu_tx, npdu, source_mac, link_layer_group) {
        Ok(()) => {}
        Err(Refused::Full) => {
            ctx.drops.npdu_channel_full(frame);
        }
        Err(Refused::Closed) => {
            ctx.drops.count(DropReason::NpduChannelClosed);
            debug!(
                frame,
                "B/IP: NPDU channel closed, dropping an incoming frame"
            );
        }
    }
}

/// The kind of frame that carried a delivered NPDU, for the log.
fn frame_kind(datagram: &Datagram<'_>) -> &'static str {
    match datagram.payload.get(1).copied().map(BvlcFunction::from_raw) {
        Some(BvlcFunction::ORIGINAL_UNICAST_NPDU) => "unicast",
        Some(BvlcFunction::ORIGINAL_BROADCAST_NPDU) => "broadcast",
        Some(BvlcFunction::FORWARDED_NPDU) => "forwarded",
        _ => "distributed broadcast",
    }
}

/// Decide what to do with one datagram, and do it. A discarded datagram is
/// counted, and its reason returned for the caller's own log.
pub(super) async fn receive(datagram: &Datagram<'_>, ctx: &RecvContext) -> Option<Discard> {
    let mut policy = HostPolicy::new(&ctx.management_limiter);
    let now = ctx.node.clock().now();
    match ctx.node.link().receive(datagram, now, &mut policy) {
        Received::Deliver(delivery) => {
            let kind = frame_kind(datagram);
            hand_up(ctx, delivery.npdu, delivery.source, delivery.group, kind);
        }
        Received::Nak {
            to,
            code,
            malformed,
        } => {
            if malformed {
                ctx.drops.count(DropReason::MalformedPayload);
                debug!("Refusing a malformed BVLL management request");
            }
            send_bvlc_result(ctx.socket(), to, code).await;
        }
        Received::Response(response) => {
            // A malformed response is counted here as well as against the
            // request it may answer.
            if let Err(error) = &response.message {
                ctx.drops.count(DropReason::of_decode_error(error));
                debug!(%error, "Received malformed BVLC response");
            }
            ctx.node
                .resolve(response, datagram.payload, policy.into_refusal());
        }
        Received::Bbmd(share) => super::bbmd_receive::handle(share, ctx).await,
        Received::Discarded(discard) => {
            discarded(discard, datagram, ctx);
            return Some(discard);
        }
    }
    None
}

/// Count a datagram the node discarded.
fn discarded(discard: Discard, datagram: &Datagram<'_>, ctx: &RecvContext) {
    let source = datagram.source;
    match discard {
        Discard::GroupSource => {
            ctx.group_drops.source();
            debug!(%source, "Dropping a datagram whose UDP source is a group address");
        }
        Discard::GroupOrigin => {
            ctx.group_drops.origin();
            debug!("Dropping Forwarded-NPDU whose origin is a group address");
        }
        other => {
            if let Some(reason) = DropReason::of_discard(other) {
                ctx.drops.count(reason);
            }
            // Per datagram, so at debug: a flood of them must not flood the
            // log at warn. The counters carry the signal (#1624).
            debug!(%source, reason = ?other, "Dropping a B/IP datagram");
        }
    }
}

/// Send a BVLC-Result to a destination.
pub(super) async fn send_bvlc_result(socket: &BipSocket, dest: SocketAddrV4, code: BvlcResultCode) {
    let mut buf = BytesMut::with_capacity(6);
    if let Err(e) = encode_bvll(&mut buf, &Bvll::BvlcResult(code)) {
        warn!(error = %e, "Failed to encode BVLC-Result");
        return;
    }
    let _ = socket.send_to(&buf, dest).await;
}
