//! A BBMD's share of a received message, once every node rule has passed
//! ([`ForBbmd`]): the BBMD state decides, under its own lock, and this sends
//! what it decided (Annex J.4, J.5).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Instant;

use bytes::BytesMut;
use tracing::{debug, warn};

use bacnet_bip_codec::{BdtEntries, BvlcFunction, BvlcResultCode, Bvll, FdtEntries};
use bacnet_bip_core::bbmd::ForwardRefusal;
use bacnet_bip_core::node::ForBbmd;
use bacnet_bip_core::Arrival;

use crate::bbmd::{self, Forward};
use crate::bvll::encode_bvll;

use super::io::{hand_up, send_bvlc_result, RecvContext};
use super::rate_limit::ManagementRateLimiter;
use super::receive_counters::DropReason;

/// Act on a BBMD's share of a message.
pub(super) async fn handle(share: ForBbmd<'_>, ctx: &RecvContext) {
    let Some(bbmd) = &ctx.bbmd else {
        debug!("Dropping a BBMD message: this transport has no BBMD state");
        return;
    };
    match share {
        ForBbmd::OriginalBroadcast(delivery) => {
            hand_up(ctx, delivery.npdu, delivery.source, true, "broadcast");
            // Forward as Forwarded-NPDU to BDT peers + FDT entries
            let forward = bbmd::lock(bbmd).forward_original_broadcast(
                delivery.source,
                delivery.npdu,
                Instant::now(),
            );
            let _ = forward_remote(ctx, delivery.npdu, forward).await;
        }

        ForBbmd::Forwarded {
            sender,
            arrival,
            origin,
            npdu,
        } => {
            // Only a BDT peer's is delivered and forwarded: to the foreign
            // devices (the peers had it from its first BBMD), and to the
            // local subnet when it came to this BBMD alone, which this BBMD's
            // own two-hop row and a unicast arrival tell (Annex J.4.5). One
            // that came by broadcast, or from a sender on this subnet at this
            // port, already reached it; never rebroadcasting one that came by
            // broadcast also ends any loop through a BDT row that is this
            // BBMD under another address.
            let decision = bbmd::lock(bbmd).forward_forwarded_npdu(
                sender,
                arrival,
                origin,
                ctx.local_broadcast(),
                Instant::now(),
            );
            let forward = match decision {
                Ok(forward) => forward,
                Err(refusal) => {
                    // The node's own-echo rule makes OwnEcho unreachable. A
                    // refusal the core adds later has no counter here yet,
                    // so it is logged, not counted under a reason it isn't.
                    let counted = match refusal {
                        ForwardRefusal::OwnEcho => Some(DropReason::OwnEcho),
                        ForwardRefusal::NotFromPeer => Some(DropReason::ForwardedNotFromPeer),
                        _ => None,
                    };
                    if let Some(reason) = counted {
                        ctx.drops.count(reason);
                    }
                    debug!(%sender, ?refusal, "Rejecting FORWARDED_NPDU");
                    return;
                }
            };

            hand_up(ctx, npdu, origin, true, "forwarded");

            match forward.local_broadcast {
                Some(local_dest) => {
                    let _ = send_forwarded_npdu(ctx, local_dest, origin, npdu).await;
                }
                None if arrival == Arrival::Broadcast => {
                    debug!("Forwarded-NPDU arrived by broadcast; no local rebroadcast");
                }
                None => {}
            }
            let _ = forward_remote(ctx, npdu, forward).await;
        }

        ForBbmd::Distribute { sender, npdu } => distribute_broadcast(npdu, sender, ctx).await,

        ForBbmd::Register { sender, ttl } => {
            let result =
                bbmd::lock(bbmd).register_foreign_device(sender.ip().octets(), sender.port(), ttl);
            debug!(%sender, ttl, "Foreign device registered");
            send_bvlc_result(ctx.socket(), sender, result).await;
        }

        ForBbmd::ReadBdt { sender } => {
            let rows = bbmd::lock(bbmd).bdt_wire_entries();
            let ack = Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&rows));
            send_table_ack(ack, sender, ctx).await;
        }

        ForBbmd::ReadFdt { sender } => {
            let rows = bbmd::lock(bbmd).fdt_wire_entries();
            let ack = Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&rows));
            send_table_ack(ack, sender, ctx).await;
        }

        ForBbmd::DeleteFdtEntry { sender, entry } => {
            let result = {
                // Check management ACL before accepting Delete-FDT-Entry
                let mut state = bbmd::lock(bbmd);
                if state.is_management_allowed(&sender.ip().octets()) {
                    state.delete_foreign_device(entry.ip().octets(), entry.port())
                } else {
                    debug!(%sender, "Rejecting Delete-FDT-Entry from non-ACL sender");
                    BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
                }
            };
            send_bvlc_result(ctx.socket(), sender, result).await;
        }
    }
}

/// A Distribute-Broadcast-To-Network (Annex J.4.5): a BBMD forwards it for a
/// registered foreign device, and refuses it from anyone else.
async fn distribute_broadcast(npdu: &[u8], sender: SocketAddrV4, ctx: &RecvContext) {
    let nak = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;
    let Some(bbmd) = &ctx.bbmd else {
        return;
    };

    // If BBMD, only a registered foreign device may distribute
    let decision = bbmd::lock(bbmd).forward_distribute_broadcast(
        sender,
        npdu,
        ctx.local_broadcast(),
        Instant::now(),
    );
    let forward = match decision {
        Ok(forward) => forward,
        Err(refusal) => {
            debug!(
                %sender,
                "Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender"
            );
            send_bvlc_result(ctx.socket(), sender, refusal.result_code()).await;
            return;
        }
    };

    hand_up(ctx, npdu, sender, true, "distributed broadcast");

    let local_ok = if should_force_dbtn_forward_failure(ctx) {
        warn!("Forced DBTN forwarding failure");
        false
    } else {
        match forward.local_broadcast {
            Some(local_dest) => send_forwarded_npdu(ctx, local_dest, forward.origin, npdu).await,
            None => true,
        }
    };

    let remote_ok = forward_remote(ctx, npdu, forward).await;

    if !(local_ok && remote_ok) {
        send_bvlc_result(ctx.socket(), sender, nak).await;
    }
}

/// Send a Read-BDT-Ack or Read-FDT-Ack, unless it would spend more of the
/// requester's response-amplification budget than is left.
async fn send_table_ack(ack: Bvll<'_>, sender: SocketAddrV4, ctx: &RecvContext) {
    let resp_len = match ack.encoded_len() {
        Ok(length) => length,
        Err(error) => {
            warn!(error = %error, function = %ack.function(), "Failed to encode table ACK");
            return;
        }
    };
    let bdt = ack.function() == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK;
    let ip = sender.ip().octets();
    let check = |limiter: &mut ManagementRateLimiter| {
        if bdt {
            limiter.check_and_record_bdt_response(ip, resp_len)
        } else {
            limiter.check_and_record_fdt_response(ip, resp_len)
        }
    };
    let allowed = match ctx.management_limiter.lock() {
        Ok(mut limiter) => check(&mut limiter),
        Err(poison) => check(&mut poison.into_inner()),
    };
    if !allowed {
        debug!(
            ip = %Ipv4Addr::from(ip),
            bytes = resp_len,
            function = %ack.function(),
            "Throttling table ACK due to amplification budget"
        );
        return;
    }
    let mut buf = BytesMut::with_capacity(resp_len);
    match encode_bvll(&mut buf, &ack) {
        Ok(()) => {
            let _ = ctx.socket().send_to(&buf, sender).await;
        }
        Err(e) => warn!(error = %e, "Failed to encode table ACK"),
    }
}

#[cfg(test)]
fn should_force_dbtn_forward_failure(ctx: &RecvContext) -> bool {
    ctx.force_dbtn_forward_failure
}

#[cfg(not(test))]
fn should_force_dbtn_forward_failure(_ctx: &RecvContext) -> bool {
    false
}

/// Send `forward`'s peer and foreign-device destinations a Forwarded-NPDU
/// carrying `npdu`: through the fan-out worker's queue and budgets, or, in a
/// receive context without one, directly. Whether it was queued or sent.
async fn forward_remote(ctx: &RecvContext, npdu: &[u8], forward: Forward) -> bool {
    let origin = forward.origin;
    match &ctx.fanout {
        Some(fanout) => fanout.dispatch_forwarded_npdu(
            origin.ip().octets(),
            origin.port(),
            npdu,
            forward.remote,
            forward.deduplicated,
        ),
        None => forward_npdu(ctx, npdu, origin, &forward.remote).await,
    }
}

/// Encode a Forwarded-NPDU carrying `npdu` from `origin`.
fn forwarded_npdu(origin: SocketAddrV4, npdu: &[u8]) -> Option<BytesMut> {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    let message = Bvll::ForwardedNpdu { origin, npdu };
    match encode_bvll(&mut buf, &message) {
        Ok(()) => Some(buf),
        Err(e) => {
            warn!(error = %e, "Failed to encode Forwarded-NPDU");
            None
        }
    }
}

/// Send `npdu` from `origin` to `dest` as a Forwarded-NPDU. A failed send
/// is logged through the throttle in
/// [`ReceiveDrops::forward_failed`](super::receive_counters::ReceiveDrops::forward_failed).
async fn send_forwarded_npdu(
    ctx: &RecvContext,
    dest: SocketAddrV4,
    origin: SocketAddrV4,
    npdu: &[u8],
) -> bool {
    let Some(buf) = forwarded_npdu(origin, npdu) else {
        return false;
    };
    match ctx.socket().send_to(&buf, dest).await {
        Ok(_) => true,
        Err(e) => {
            ctx.drops.forward_failed(&e, dest);
            false
        }
    }
}

/// Forward an NPDU as Forwarded-NPDU to a list of targets, logging a failed
/// send as [`send_forwarded_npdu`] does. Only a receive context without a
/// fan-out worker, as in some tests, forwards this way.
///
/// Yields between sends for large target lists to avoid starving the recv loop
/// when there are many targets (up to 128 BDT rows and 128 foreign devices).
async fn forward_npdu(
    ctx: &RecvContext,
    npdu: &[u8],
    origin: SocketAddrV4,
    targets: &[SocketAddrV4],
) -> bool {
    if targets.is_empty() {
        return true;
    }
    let Some(buf) = forwarded_npdu(origin, npdu) else {
        return false;
    };
    let frame = buf.freeze();
    let mut all_sent = true;

    for (i, &dest) in targets.iter().enumerate() {
        if let Err(e) = ctx.socket().send_to(&frame, dest).await {
            all_sent = false;
            ctx.drops.forward_failed(&e, dest);
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
        }
    }
    all_sent
}
