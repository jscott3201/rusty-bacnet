use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::MacAddr;

use crate::bbmd::{self, BbmdState};
use crate::bvll::{self, encode_bip_mac, encode_bvll, encode_bvll_forwarded};
use crate::port::{ReceivedNpdu, TransportProvenance};

use super::decode_bvlc_result_code;
use super::fanout::FanoutDispatcher;
use super::ingress::Delivery;
use super::rate_limit::{is_covered_management_request, ManagementRateLimiter};

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
pub(super) struct RecvContext {
    pub(super) local_mac: [u8; 6],
    pub(super) socket: Arc<super::BipSocket>,
    pub(super) npdu_tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) bbmd: Option<Arc<std::sync::Mutex<BbmdState>>>,
    pub(super) broadcast_addr: Ipv4Addr,
    pub(super) broadcast_port: u16,
    pub(super) client_management: Arc<super::client_management::ManagementClient>,
    pub(super) management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    pub(super) fanout: Option<FanoutDispatcher>,
    /// Refuses a UDP source (#1504) or a Forwarded-NPDU origin (#1493)
    /// that is a group address.
    pub(super) group_sources: super::groups::GroupSources,
    #[cfg(test)]
    pub(super) force_dbtn_forward_failure: bool,
}

fn complete_pending_bvlc_response(
    msg: &bvll::BvllMessage,
    sender: ([u8; 4], u16),
    ctx: &RecvContext,
) -> bool {
    ctx.client_management.complete(msg, sender)
}

/// Handle a decoded BVLL message in the recv loop. `delivery` says how its
/// datagram arrived.
pub(super) async fn handle_bvll_message(
    msg: &bvll::BvllMessage,
    sender: ([u8; 4], u16),
    delivery: Delivery,
    ctx: &RecvContext,
) {
    // Bounded inbound management quota. Excess covered requests are
    // silently discarded before payload validation, table access, ACL
    // evaluation, or any response/NAK. Write-BDT and data-plane functions
    // are excluded and fall through below.
    if is_covered_management_request(msg.function) {
        let allowed = match ctx.management_limiter.lock() {
            Ok(mut limiter) => limiter.check_now(sender.0),
            Err(poison) => poison.into_inner().check_now(sender.0),
        };
        if !allowed {
            debug!(
                function = msg.function.to_raw(),
                ip = %Ipv4Addr::from(sender.0),
                "Discarding over-limit BBMD management request"
            );
            return;
        }
    }
    match msg.function {
        f if f == BvlcFunction::ORIGINAL_UNICAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }
            if ctx
                .npdu_tx
                .try_send(ReceivedNpdu {
                    direct_response: None,
                    npdu: msg.payload.clone(),
                    source_mac,
                    // From the datagram's address, not its BVLC function, so
                    // a group-addressed confirmed request is never answered as
                    // a directed one (Clause 5.4.5.1).
                    link_layer_group: delivery == Delivery::Broadcast,
                    data_attributes: Vec::new(),
                    provenance: TransportProvenance::unverified(),
                    reply_tx: None,
                })
                .is_err()
            {
                warn!("BIP: NPDU channel full, dropping incoming unicast frame");
            }
        }

        f if f == BvlcFunction::ORIGINAL_BROADCAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            if ctx
                .npdu_tx
                .try_send(ReceivedNpdu {
                    direct_response: None,
                    npdu: msg.payload.clone(),
                    source_mac,
                    link_layer_group: true,
                    data_attributes: Vec::new(),
                    provenance: TransportProvenance::unverified(),
                    reply_tx: None,
                })
                .is_err()
            {
                warn!("BIP: NPDU channel full, dropping incoming broadcast frame");
            }

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = &ctx.bbmd {
                let (targets, dedup_count) = {
                    let mut state = bbmd::lock(bbmd);
                    let before = state.fdt_counters().destinations_deduplicated;
                    let targets = state.forwarding_targets(sender.0, sender.1);
                    let after = state.fdt_counters().destinations_deduplicated;
                    (targets, after.saturating_sub(before))
                };
                if let Some(fanout) = &ctx.fanout {
                    let addrs = targets
                        .into_iter()
                        .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
                        .collect();
                    fanout.dispatch_forwarded_npdu(
                        sender.0,
                        sender.1,
                        &msg.payload,
                        addrs,
                        dedup_count,
                    );
                } else {
                    let _ =
                        forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await;
                }
            }
        }

        f if f == BvlcFunction::FORWARDED_NPDU => {
            // BBMD mode: use originating_ip as source_mac (same subnet, directly reachable).
            // Non-BBMD mode: use actual UDP sender as source_mac (originator may be behind NAT).
            let source_mac =
                if let (Some(ip), Some(port)) = (msg.originating_ip, msg.originating_port) {
                    MacAddr::from(encode_bip_mac(ip, port))
                } else {
                    return;
                };
            if *source_mac == ctx.local_mac[..] || !ctx.group_sources.admits_origin(&source_mac) {
                return;
            }

            // BBMD mode: only accept FORWARDED_NPDU from BDT peers
            if let Some(bbmd) = &ctx.bbmd {
                // A local rebroadcast retains the remote embedded origin. The
                // self BDT row must not turn its UDP echo into peer traffic.
                if encode_bip_mac(sender.0, sender.1) == ctx.local_mac {
                    return;
                }
                let (is_bdt_peer, needs_local_broadcast) = {
                    let state = bbmd::lock(bbmd);
                    (
                        state.is_bdt_peer(sender.0, sender.1),
                        state.forwarded_npdu_needs_local_broadcast(sender.0, sender.1),
                    )
                };
                if !is_bdt_peer {
                    debug!(
                        "Rejecting FORWARDED_NPDU from non-BDT sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    return;
                }

                if ctx
                    .npdu_tx
                    .try_send(ReceivedNpdu {
                        direct_response: None,
                        npdu: msg.payload.clone(),
                        source_mac,
                        link_layer_group: true,
                        data_attributes: Vec::new(),
                        provenance: TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP: NPDU channel full, dropping forwarded frame");
                }

                let orig_ip = msg.originating_ip.unwrap();
                let orig_port = msg.originating_port.unwrap();

                // Forward to FDT entries (BDT peers don't need it — they got it directly)
                let (fdt_targets, dedup_count) = {
                    let mut state = bbmd::lock(bbmd);
                    let before = state.fdt_counters().destinations_deduplicated;
                    let targets = state.fdt_forwarding_targets(orig_ip, orig_port);
                    let after = state.fdt_counters().destinations_deduplicated;
                    (targets, after.saturating_sub(before))
                };
                // Annex J.4.5: one that came by broadcast already reached the
                // local subnet, whatever the sender's BDT mask says. Never
                // rebroadcasting it also ends any loop through a BDT row that
                // is this BBMD under another address.
                if needs_local_broadcast && delivery == Delivery::Broadcast {
                    debug!("Forwarded-NPDU arrived by broadcast; skipping local rebroadcast");
                } else if needs_local_broadcast {
                    let local_dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                    let _ = send_forwarded_npdu(
                        &ctx.socket,
                        local_dest,
                        orig_ip,
                        orig_port,
                        &msg.payload,
                    )
                    .await;
                }
                if let Some(fanout) = &ctx.fanout {
                    let addrs: Vec<SocketAddrV4> = fdt_targets
                        .into_iter()
                        .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
                        .collect();
                    fanout.dispatch_forwarded_npdu(
                        orig_ip,
                        orig_port,
                        &msg.payload,
                        addrs,
                        dedup_count,
                    );
                } else {
                    let _ =
                        forward_npdu(&ctx.socket, &msg.payload, orig_ip, orig_port, &fdt_targets)
                            .await;
                }
            } else {
                // Non-BBMD: use originating address as source_mac (spec J.2.5).
                if ctx
                    .npdu_tx
                    .try_send(ReceivedNpdu {
                        direct_response: None,
                        npdu: msg.payload.clone(),
                        source_mac,
                        link_layer_group: true,
                        data_attributes: Vec::new(),
                        provenance: TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP: NPDU channel full, dropping forwarded frame");
                }
            }
        }

        f if f == BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            // If BBMD, verify sender is a registered foreign device
            if let Some(bbmd) = &ctx.bbmd {
                let is_registered = {
                    let mut state = bbmd::lock(bbmd);
                    state.is_registered_foreign_device(sender.0, sender.1)
                };
                if !is_registered {
                    debug!("Rejecting DISTRIBUTE_BROADCAST_TO_NETWORK from non-registered sender {:?}:{}",
                        Ipv4Addr::from(sender.0), sender.1);
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                    )
                    .await;
                    return;
                }

                if ctx
                    .npdu_tx
                    .try_send(ReceivedNpdu {
                        direct_response: None,
                        npdu: msg.payload.clone(),
                        source_mac,
                        link_layer_group: true,
                        data_attributes: Vec::new(),
                        provenance: TransportProvenance::unverified(),
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP: NPDU channel full, dropping distributed broadcast frame");
                }

                let (targets, dedup_count) = {
                    let mut state = bbmd::lock(bbmd);
                    let before = state.fdt_counters().destinations_deduplicated;
                    let targets = state.forwarding_targets(sender.0, sender.1);
                    let after = state.fdt_counters().destinations_deduplicated;
                    (targets, after.saturating_sub(before))
                };

                let local_dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let local_ok = if should_force_dbtn_forward_failure(ctx) {
                    warn!("Forced DBTN forwarding failure");
                    false
                } else {
                    send_forwarded_npdu(&ctx.socket, local_dest, sender.0, sender.1, &msg.payload)
                        .await
                };

                let remote_ok = if let Some(fanout) = &ctx.fanout {
                    let addrs: Vec<SocketAddrV4> = targets
                        .into_iter()
                        .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
                        .collect();
                    fanout.dispatch_forwarded_npdu(
                        sender.0,
                        sender.1,
                        &msg.payload,
                        addrs,
                        dedup_count,
                    )
                } else {
                    forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await
                };

                let forwarding_ok = local_ok && remote_ok;
                if !forwarding_ok {
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                    )
                    .await;
                }
            } else {
                // Non-BBMD: reject with NAK (spec J.4.5)
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::REGISTER_FOREIGN_DEVICE => {
            if let Some(bbmd) = &ctx.bbmd {
                if msg.payload.len() != 2 {
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                    )
                    .await;
                    return;
                }
                let ttl = u16::from_be_bytes([msg.payload[0], msg.payload[1]]);
                let result = {
                    let mut state = bbmd::lock(bbmd);
                    state.register_foreign_device(sender.0, sender.1, ttl)
                };
                debug!(
                    ip = ?Ipv4Addr::from(sender.0),
                    port = sender.1,
                    ttl = ttl,
                    "Foreign device registered"
                );
                send_bvlc_result(&ctx.socket, sender, result).await;
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
                let mut payload = BytesMut::new();
                bbmd::lock(bbmd).encode_bdt(&mut payload);
                let resp_len = 4 + payload.len();
                let allowed = match ctx.management_limiter.lock() {
                    Ok(mut limiter) => limiter.check_and_record_bdt_response(sender.0, resp_len),
                    Err(poison) => poison
                        .into_inner()
                        .check_and_record_bdt_response(sender.0, resp_len),
                };
                if !allowed {
                    debug!(
                        ip = %Ipv4Addr::from(sender.0),
                        bytes = resp_len,
                        "Throttling Read-BDT response due to amplification budget"
                    );
                    return;
                }
                let mut buf = BytesMut::with_capacity(resp_len);
                match encode_bvll(
                    &mut buf,
                    BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                    &payload,
                ) {
                    Ok(()) => {
                        let dest = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
                        let _ = ctx.socket.send_to(&buf, dest).await;
                    }
                    Err(e) => warn!(error = %e, "Failed to encode Read-BDT-Ack"),
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => {
            // Annex J.4.4.2 requires receivers to answer Write-BDT with the
            // not-supported result. No decoding, state change, or persistence.
            send_bvlc_result(
                &ctx.socket,
                sender,
                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
            )
            .await;
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
                let mut payload = BytesMut::new();
                bbmd::lock(bbmd).encode_fdt(&mut payload);
                let resp_len = 4 + payload.len();
                let allowed = match ctx.management_limiter.lock() {
                    Ok(mut limiter) => limiter.check_and_record_fdt_response(sender.0, resp_len),
                    Err(poison) => poison
                        .into_inner()
                        .check_and_record_fdt_response(sender.0, resp_len),
                };
                if !allowed {
                    debug!(
                        ip = %Ipv4Addr::from(sender.0),
                        bytes = resp_len,
                        "Throttling Read-FDT response due to amplification budget"
                    );
                    return;
                }
                let mut buf = BytesMut::with_capacity(resp_len);
                match encode_bvll(
                    &mut buf,
                    BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                    &payload,
                ) {
                    Ok(()) => {
                        let dest = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
                        let _ = ctx.socket.send_to(&buf, dest).await;
                    }
                    Err(e) => warn!(error = %e, "Failed to encode Read-FDT-Ack"),
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
            if let Some(bbmd) = &ctx.bbmd {
                // Check management ACL before accepting Delete-FDT-Entry
                let allowed = {
                    let state = bbmd::lock(bbmd);
                    state.is_management_allowed(&sender.0)
                };
                if !allowed {
                    debug!(
                        "Rejecting Delete-FDT-Entry from non-ACL sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                    )
                    .await;
                } else if msg.payload.len() == 6 {
                    let ip = [
                        msg.payload[0],
                        msg.payload[1],
                        msg.payload[2],
                        msg.payload[3],
                    ];
                    let port = u16::from_be_bytes([msg.payload[4], msg.payload[5]]);
                    let result = {
                        let mut state = bbmd::lock(bbmd);
                        state.delete_foreign_device(ip, port)
                    };
                    send_bvlc_result(&ctx.socket, sender, result).await;
                } else {
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                    )
                    .await;
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::BVLC_RESULT => {
            if !complete_pending_bvlc_response(msg, sender, ctx) {
                match decode_bvlc_result_code(msg) {
                    Ok(code) => {
                        debug!(code = ?code, "Received unmatched BVLC-Result");
                    }
                    Err(err) => {
                        warn!(error = %err, "Received malformed BVLC-Result");
                    }
                }
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => {
            if !complete_pending_bvlc_response(msg, sender, ctx) {
                debug!("Received Read-BDT-ACK with no pending request");
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
            if !complete_pending_bvlc_response(msg, sender, ctx) {
                debug!("Received Read-FDT-ACK with no pending request");
            }
        }

        _ => {
            debug!(function = msg.function.to_raw(), "Unknown BVLC function");
        }
    }
}

/// Send a BVLC-Result to a destination.
async fn send_bvlc_result(socket: &UdpSocket, dest: ([u8; 4], u16), code: BvlcResultCode) {
    let payload = code.to_raw().to_be_bytes().to_vec();
    let mut buf = BytesMut::with_capacity(6);
    if let Err(e) = encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &payload) {
        warn!(error = %e, "Failed to encode BVLC-Result");
        return;
    }
    let addr = SocketAddrV4::new(Ipv4Addr::from(dest.0), dest.1);
    let _ = socket.send_to(&buf, addr).await;
}

#[cfg(test)]
fn should_force_dbtn_forward_failure(ctx: &RecvContext) -> bool {
    ctx.force_dbtn_forward_failure
}

#[cfg(not(test))]
fn should_force_dbtn_forward_failure(_ctx: &RecvContext) -> bool {
    false
}

async fn send_forwarded_npdu(
    socket: &UdpSocket,
    dest: SocketAddrV4,
    orig_ip: [u8; 4],
    orig_port: u16,
    npdu: &[u8],
) -> bool {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
        warn!(error = %e, "Failed to encode Forwarded-NPDU");
        return false;
    }

    match socket.send_to(&buf, dest).await {
        Ok(_) => true,
        Err(e) => {
            warn!(error = %e, dest = %dest, "Failed to send Forwarded-NPDU");
            false
        }
    }
}

/// Forward an NPDU as Forwarded-NPDU to a list of targets.
///
/// Yields between sends for large target lists to avoid starving the recv loop
/// when there are many FDT entries (up to 512).
async fn forward_npdu(
    socket: &UdpSocket,
    npdu: &[u8],
    orig_ip: [u8; 4],
    orig_port: u16,
    targets: &[([u8; 4], u16)],
) -> bool {
    if targets.is_empty() {
        return true;
    }
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
        warn!(error = %e, "Failed to encode Forwarded-NPDU");
        return false;
    }
    let frame = buf.freeze();
    let mut all_sent = true;

    for (i, &(ip, port)) in targets.iter().enumerate() {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        if let Err(e) = socket.send_to(&frame, dest).await {
            all_sent = false;
            warn!(error = %e, dest = %dest, "Failed to forward NPDU");
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
        }
    }
    all_sent
}
