use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::MacAddr;

use crate::bbmd::BbmdState;
use crate::bvll::{self, encode_bip_mac, encode_bvll, encode_bvll_forwarded, BvllMessage};
use crate::port::ReceivedNpdu;

/// Send a Register-Foreign-Device message to a BBMD.
pub(super) async fn send_register_foreign_device(
    socket: &UdpSocket,
    bbmd_addr: SocketAddrV4,
    ttl: u16,
) {
    let payload = ttl.to_be_bytes().to_vec();
    let mut buf = BytesMut::with_capacity(6);
    if let Err(e) = encode_bvll(&mut buf, BvlcFunction::REGISTER_FOREIGN_DEVICE, &payload) {
        warn!(error = %e, "Failed to encode Register-Foreign-Device");
        return;
    }
    if let Err(e) = socket.send_to(&buf, bbmd_addr).await {
        warn!(error = %e, "Failed to send Register-Foreign-Device");
    } else {
        debug!(bbmd = %bbmd_addr, ttl = ttl, "Sent Register-Foreign-Device");
    }
}

/// Context for the BIP receive loop — holds all shared state needed to
/// process incoming BVLL messages.
pub(super) struct RecvContext {
    pub(super) local_mac: [u8; 6],
    pub(super) socket: Arc<UdpSocket>,
    pub(super) npdu_tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) bbmd: Option<Arc<Mutex<BbmdState>>>,
    pub(super) broadcast_addr: Ipv4Addr,
    pub(super) broadcast_port: u16,
    pub(super) bvlc_response: Arc<Mutex<Option<oneshot::Sender<BvllMessage>>>>,
    pub(super) bdt_persist_path: Option<std::path::PathBuf>,
}

/// Handle a decoded BVLL message in the recv loop.
pub(super) async fn handle_bvll_message(
    msg: &bvll::BvllMessage,
    sender: ([u8; 4], u16),
    ctx: &RecvContext,
) {
    match msg.function {
        f if f == BvlcFunction::ORIGINAL_UNICAST_NPDU => {
            let source_mac = MacAddr::from(encode_bip_mac(sender.0, sender.1));
            if *source_mac == ctx.local_mac[..] {
                return;
            }
            if ctx
                .npdu_tx
                .try_send(ReceivedNpdu {
                    npdu: msg.payload.clone(),
                    source_mac,
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
                    npdu: msg.payload.clone(),
                    source_mac,
                    reply_tx: None,
                })
                .is_err()
            {
                warn!("BIP: NPDU channel full, dropping incoming broadcast frame");
            }

            // If BBMD, forward as Forwarded-NPDU to BDT peers + FDT entries
            if let Some(bbmd) = &ctx.bbmd {
                let targets = {
                    let mut state = bbmd.lock().await;
                    state.forwarding_targets(sender.0, sender.1)
                };
                forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU.
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                match encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload) {
                    Ok(()) => {
                        let _ = ctx.socket.send_to(&buf, dest).await;
                    }
                    Err(e) => warn!(error = %e, "Failed to encode Forwarded-NPDU rebroadcast"),
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
            if *source_mac == ctx.local_mac[..] {
                return;
            }

            // BBMD mode: only accept FORWARDED_NPDU from BDT peers
            if let Some(bbmd) = &ctx.bbmd {
                let is_bdt_peer = {
                    let state = bbmd.lock().await;
                    state.is_bdt_peer(sender.0, sender.1)
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
                        npdu: msg.payload.clone(),
                        source_mac,
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP: NPDU channel full, dropping forwarded frame");
                }

                let orig_ip = msg.originating_ip.unwrap();
                let orig_port = msg.originating_port.unwrap();

                // Forward to FDT entries (BDT peers don't need it — they got it directly)
                let fdt_targets = {
                    let mut state = bbmd.lock().await;
                    state.purge_expired();
                    state
                        .fdt()
                        .iter()
                        .filter(|e| !(e.ip == orig_ip && e.port == orig_port))
                        .map(|e| (e.ip, e.port))
                        .collect::<Vec<_>>()
                };
                forward_npdu(&ctx.socket, &msg.payload, orig_ip, orig_port, &fdt_targets).await;

                // Re-broadcast on local subnet as Forwarded-NPDU
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                match encode_bvll_forwarded(&mut buf, orig_ip, orig_port, &msg.payload) {
                    Ok(()) => {
                        let _ = ctx.socket.send_to(&buf, dest).await;
                    }
                    Err(e) => warn!(error = %e, "Failed to encode Forwarded-NPDU rebroadcast"),
                }
            } else {
                // Non-BBMD: use originating address as source_mac (spec J.2.5).
                if ctx
                    .npdu_tx
                    .try_send(ReceivedNpdu {
                        npdu: msg.payload.clone(),
                        source_mac,
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
                    let mut state = bbmd.lock().await;
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
                        npdu: msg.payload.clone(),
                        source_mac,
                        reply_tx: None,
                    })
                    .is_err()
                {
                    warn!("BIP: NPDU channel full, dropping distributed broadcast frame");
                }

                let targets = {
                    let mut state = bbmd.lock().await;
                    state.forwarding_targets(sender.0, sender.1)
                };
                forward_npdu(&ctx.socket, &msg.payload, sender.0, sender.1, &targets).await;

                // Broadcast locally as Forwarded-NPDU
                let dest = SocketAddrV4::new(ctx.broadcast_addr, ctx.broadcast_port);
                let mut buf = BytesMut::with_capacity(10 + msg.payload.len());
                match encode_bvll_forwarded(&mut buf, sender.0, sender.1, &msg.payload) {
                    Ok(()) => {
                        let _ = ctx.socket.send_to(&buf, dest).await;
                    }
                    Err(e) => warn!(error = %e, "Failed to encode Forwarded-NPDU broadcast"),
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
                if msg.payload.len() < 2 {
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
                    let mut state = bbmd.lock().await;
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
                let state = bbmd.lock().await;
                let mut payload = BytesMut::new();
                state.encode_bdt(&mut payload);
                let mut buf = BytesMut::with_capacity(4 + payload.len());
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
            if let Some(bbmd) = &ctx.bbmd {
                // Check management ACL before accepting Write-BDT
                let allowed = {
                    let state = bbmd.lock().await;
                    state.is_management_allowed(&sender.0)
                };
                if !allowed {
                    debug!(
                        "Rejecting Write-BDT from non-ACL sender {:?}:{}",
                        Ipv4Addr::from(sender.0),
                        sender.1
                    );
                    send_bvlc_result(
                        &ctx.socket,
                        sender,
                        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                    )
                    .await;
                } else {
                    match BbmdState::decode_bdt(&msg.payload) {
                        Ok(entries) => {
                            let mut state = bbmd.lock().await;
                            match state.set_bdt(entries) {
                                Ok(()) => {
                                    // Persist BDT to disk if configured
                                    if let Some(ref path) = ctx.bdt_persist_path {
                                        let mut buf = BytesMut::new();
                                        state.encode_bdt(&mut buf);
                                        if let Err(e) = std::fs::write(path, &buf) {
                                            warn!(
                                                error = %e,
                                                path = %path.display(),
                                                "Failed to persist BDT"
                                            );
                                        }
                                    }
                                    send_bvlc_result(
                                        &ctx.socket,
                                        sender,
                                        BvlcResultCode::SUCCESSFUL_COMPLETION,
                                    )
                                    .await;
                                }
                                Err(_) => {
                                    send_bvlc_result(
                                        &ctx.socket,
                                        sender,
                                        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(_) => {
                            send_bvlc_result(
                                &ctx.socket,
                                sender,
                                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                            )
                            .await;
                        }
                    }
                }
            } else {
                send_bvlc_result(
                    &ctx.socket,
                    sender,
                    BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
                )
                .await;
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE => {
            if let Some(bbmd) = &ctx.bbmd {
                let mut state = bbmd.lock().await;
                let mut payload = BytesMut::new();
                state.encode_fdt(&mut payload);
                drop(state);
                let mut buf = BytesMut::with_capacity(4 + payload.len());
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
                    let state = bbmd.lock().await;
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
                } else if msg.payload.len() >= 6 {
                    let ip = [
                        msg.payload[0],
                        msg.payload[1],
                        msg.payload[2],
                        msg.payload[3],
                    ];
                    let port = u16::from_be_bytes([msg.payload[4], msg.payload[5]]);
                    let result = {
                        let mut state = bbmd.lock().await;
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
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else if msg.payload.len() >= 2 {
                let code =
                    BvlcResultCode::from_raw(u16::from_be_bytes([msg.payload[0], msg.payload[1]]));
                match code {
                    BvlcResultCode::SUCCESSFUL_COMPLETION => {
                        debug!("Received BVLC-Result: successful");
                    }
                    BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK => {
                        tracing::error!(
                            "BVLC-Result NAK: foreign device registration rejected by BBMD"
                        );
                    }
                    BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK => {
                        tracing::error!(
                            "BVLC-Result NAK: broadcast distribution rejected — \
                             foreign device registration may have failed or expired"
                        );
                    }
                    _ => {
                        warn!(code = ?code, "Received BVLC-Result NAK");
                    }
                }
            }
        }

        f if f == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => {
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else {
                debug!("Received Read-BDT-ACK with no pending request");
            }
        }

        f if f == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
            let sender_opt = {
                let mut slot = ctx.bvlc_response.lock().await;
                slot.take()
            };
            if let Some(response_tx) = sender_opt {
                let _ = response_tx.send(msg.clone());
            } else {
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
) {
    if targets.is_empty() {
        return;
    }
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
        warn!(error = %e, "Failed to encode Forwarded-NPDU");
        return;
    }
    let frame = buf.freeze();

    for (i, &(ip, port)) in targets.iter().enumerate() {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        if let Err(e) = socket.send_to(&frame, dest).await {
            warn!(error = %e, dest = %dest, "Failed to forward NPDU");
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
        }
    }
}

/// Resolve the local IPv4 address by connecting a UDP socket to a remote
/// address and reading back the local address. This doesn't actually send
/// any packets.
pub(super) fn resolve_local_ip() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to a public IP — doesn't actually send anything
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
        _ => None,
    }
}
