//! Hub message dispatch; its registration lease is owned by the outer runner.

use super::*;
use crate::sc::diagnostic_throttle::DiagnosticThrottle;

pub(super) async fn run(
    peer_addr: SocketAddr,
    hub: (Vmac, DeviceUuid),
    mut read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: (Clients, &mut super::retirement::Lease),
    deadline: &super::deadlines::ConnectDeadline,
    on_heartbeat_ack: impl Fn() + Send,
) {
    let (hub_vmac, hub_uuid) = hub;
    let (clients, lease) = clients;
    let close_requested = lease.closed.clone();
    let close_notify = lease.notify.clone();
    let client_activity: Arc<AtomicU64> = Arc::new(AtomicU64::new(now_secs()));
    // Owner-local bound for malformed-frame diagnostics only. Fresh per
    // connection so one peer's flood cannot suppress another connection's
    // first diagnostic. NAK/relay/silence decisions are unchanged.
    let mut malformed_diag = DiagnosticThrottle::new();

    loop {
        // A stream of immediately ready frames must not starve the timer.
        if deadline.expired() {
            break;
        }
        // Heartbeat retirement may precede the next wait, or a previously
        // selected notification may have been consumed. The predicate owns close.
        if close_requested.load(Ordering::Acquire) {
            break;
        }
        let msg_result = read.next().await;
        let Some(msg_result) = msg_result else {
            break;
        };
        #[cfg(test)]
        deadline.received.fetch_add(1, Ordering::Release);
        if close_requested.load(Ordering::Acquire) {
            break;
        }

        let data = match msg_result {
            Ok(Message::Binary(data)) => data,
            Ok(Message::Close(_)) => {
                debug!("Hub: client {peer_addr} sent close");
                break;
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(_) => {
                warn!("Hub: non-binary frame from {peer_addr}, closing with 1003");
                let mut w = write.lock().await;
                let _ = w
                    .send(Message::Close(Some(
                        tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Unsupported,
                            reason: "BACnet/SC requires binary frames".into(),
                        },
                    )))
                    .await;
                break;
            }
            Err(e) => {
                warn!("Hub: recv error from {peer_addr}: {e}");
                break;
            }
        };

        if data.len() > HUB_MAX_BVLC_LENGTH as usize {
            super::malformed_diag::oversize(&mut malformed_diag, peer_addr, data.len());
            continue;
        }

        let sc_msg = match decode_sc_message(&data) {
            Ok(m) => m,
            Err(e) => {
                super::malformed_diag::decode_error(&mut malformed_diag, peer_addr, &e);
                continue;
            }
        };

        if close_requested.load(Ordering::Acquire) {
            debug!("Hub: client {peer_addr} received message after replacement");
            break;
        }

        if let Some(registered_vmac) = lease.vmac {
            if !registered_client_matches_sink(&clients, registered_vmac, &write).await {
                debug!("Hub: client {peer_addr} (vmac={registered_vmac:02x?}) was superseded");
                break;
            }
        }

        // This accepting hub neither initiates Connect nor waits for a
        // Disconnect-ACK. AB.2 forbids replying to these responses. Discard
        // before admission/activity, even with invalid function-specific fields;
        // they cannot establish a connection or satisfy a liveness probe.
        if matches!(
            sc_msg.function,
            ScFunction::ConnectAccept | ScFunction::DisconnectAck
        ) {
            continue;
        }

        if let Err(nak) = crate::sc_frame::validate_connect_request(&sc_msg, &data) {
            if let Some(nak) = nak {
                if let Err(e) = write.lock().await.send(Message::Binary(nak)).await {
                    warn!("Hub: failed to send Connect NAK to {peer_addr}: {e}");
                    break;
                }
            }
            // Preserve the existing pre-registration rejection lifecycle;
            // a malformed repeat must not retire an established connection.
            if lease.vmac.is_none() {
                break;
            }
            continue;
        }

        if let Err(nak) = crate::sc_frame::validate_control(
            &sc_msg,
            &data,
            crate::sc_frame::ControlRecipient::AcceptingHub,
        ) {
            if let Some(nak) = nak {
                if let Err(e) = write.lock().await.send(Message::Binary(nak)).await {
                    warn!("Hub: failed to send control NAK to {peer_addr}: {e}");
                    break;
                }
            }
            continue;
        }

        if sc_msg.function == ScFunction::EncapsulatedNpdu
            && sc_msg.payload.len() > HUB_MAX_NPDU_LENGTH as usize
        {
            super::malformed_diag::npdu_exceeds(&mut malformed_diag);
            continue;
        }

        if lease.vmac.is_some() && crate::sc_frame::missing_npdu_payload(&sc_msg) {
            // Keep pre-registration behavior and routing-envelope silence. The
            // hub does not interpret relayed destination options, including MU.
            if let Ok(HubRelayTarget::Unicast(_)) = hub_relay_target(&sc_msg) {
                let nak = build_bvlc_result_nak(
                    sc_msg.message_id,
                    sc_msg.function,
                    ErrorClass::COMMUNICATION,
                    ErrorCode::PAYLOAD_EXPECTED,
                );
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                // Connection-local reply; existing registered retirement can
                // interrupt this handler, not a new per-NAK hub deadline.
                if let Err(e) = write.lock().await.send(Message::Binary(buf.freeze())).await {
                    warn!("Hub: failed to send missing NPDU payload NAK to {peer_addr}: {e}");
                    break;
                }
            }
            continue;
        }

        if matches!(sc_msg.function, ScFunction::Unknown(_)) {
            if let Some(registered_vmac) = lease.vmac.filter(|_| sc_msg.destination_vmac.is_some())
            {
                let Ok(target) = hub_relay_target(&sc_msg) else {
                    // An explicit origin on transit is not trusted, even if it
                    // names a valid peer. Never reflect a NAK for this envelope.
                    continue;
                };
                if target == HubRelayTarget::Unicast(registered_vmac) {
                    // Unknown-only no-echo rule: self drops do not count as activity.
                    continue;
                }
                // Accepted transit follows the existing NPDU activity policy,
                // including absent/oversized recipient drops. No probe mutation.
                client_activity.store(now_secs(), Ordering::Release);
                if super::opaque_relay::relay(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    target,
                    &clients,
                    &write,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            } else if let Some(nak) = super::unknown_transit::local_nak(&sc_msg) {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                // Same socket only, including supplied response address metadata.
                // Retain the fallback's ignored immediate send error and existing
                // supervised retirement/absolute pre-registration deadline.
                let _ = write.lock().await.send(Message::Binary(buf.freeze())).await;
            }
            continue;
        }

        if matches!(
            sc_msg.function,
            ScFunction::AddressResolution | ScFunction::AddressResolutionAck
        ) {
            if let Some(registered_vmac) = lease.vmac.filter(|_| sc_msg.destination_vmac.is_some())
            {
                // Both functions are unicast-only. Reject broadcast, explicit
                // origin and self before activity; never reflect a transit NAK.
                let Ok(target @ HubRelayTarget::Unicast(dest)) = hub_relay_target(&sc_msg) else {
                    continue;
                };
                if dest == registered_vmac {
                    continue;
                }
                // Opaque options/body (including zero-byte ACK URI lists) are
                // not NPDUs or endpoint fields to validate here. Accepted transit
                // follows NPDU/Unknown activity even for missing/capped targets.
                client_activity.store(now_secs(), Ordering::Release);
                if super::opaque_relay::relay(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    target,
                    &clients,
                    &write,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            } else if let Some(nak) = super::resolution_transit::local_nak(&sc_msg) {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                // Same socket only. Keep ignored immediate write failures and
                // the existing retirement/absolute Connect deadline owners.
                let _ = write.lock().await.send(Message::Binary(buf.freeze())).await;
            }
            continue;
        }

        if matches!(
            sc_msg.function,
            ScFunction::Advertisement | ScFunction::AdvertisementSolicitation
        ) {
            if let Some(registered_vmac) = lease.vmac.filter(|_| sc_msg.destination_vmac.is_some())
            {
                // Both functions are unicast-only. Reject broadcast, explicit
                // origin and self before activity; never reflect a transit NAK.
                // Transit stays opaque: the shape validator owns hub-local
                // and node destinations, while AB.5.3.2 forwarding preserves
                // the wire frame bytes for the destination to judge.
                let Ok(target @ HubRelayTarget::Unicast(dest)) = hub_relay_target(&sc_msg) else {
                    continue;
                };
                if dest == registered_vmac {
                    continue;
                }
                // Accepted transit follows NPDU/Unknown activity even for missing/capped targets.
                client_activity.store(now_secs(), Ordering::Release);
                if super::opaque_relay::relay(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    target,
                    &clients,
                    &write,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            } else if let Some(nak) = super::advertisement_transit::local_nak(&sc_msg, &data) {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                // Same socket only. Keep ignored immediate write failures and
                // the existing retirement/absolute Connect deadline owners.
                let _ = write.lock().await.send(Message::Binary(buf.freeze())).await;
            }
            continue;
        }

        if sc_msg.function == ScFunction::ProprietaryMessage {
            if let Some(registered_vmac) = lease.vmac.filter(|_| sc_msg.destination_vmac.is_some())
            {
                // Proprietary is unicast-or-broadcast per AB.2.16. Unicast
                // strips the destination and stamps the origin with no echo;
                // broadcast fans out except the source with origin-stamp and
                // preserved broadcast destination. Explicit origins and self
                // unicast stay silent before activity; never reflect a NAK.
                // Transit stays opaque with BVLC-only caps and no NPDU caps.
                let Ok(target) = hub_relay_target(&sc_msg) else {
                    continue;
                };
                if target == HubRelayTarget::Unicast(registered_vmac) {
                    // Proprietary no-echo rule: self drops do not count as activity.
                    continue;
                }
                // Accepted transit follows the existing NPDU activity policy,
                // including absent/oversized recipient drops. No probe mutation.
                client_activity.store(now_secs(), Ordering::Release);
                if super::opaque_relay::relay(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    target,
                    &clients,
                    &write,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            } else if let Some(nak) = super::proprietary_transit::local_nak(&sc_msg, &data) {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                // Same socket only. Keep ignored immediate write failures and
                // the existing retirement/absolute Connect deadline owners.
                let _ = write.lock().await.send(Message::Binary(buf.freeze())).await;
            }
            continue;
        }

        // Decoded remaining BVLC messages that pass response/Connect/control admission,
        // registered NPDU payload presence and local capacity checks count as activity.
        // WebSocket control, oversized, and undecodable frames do not.
        client_activity.store(now_secs(), std::sync::atomic::Ordering::Release);

        match sc_msg.function {
            ScFunction::ConnectRequest => {
                if let Some(registered_vmac) = lease.vmac {
                    warn!(
                        "Hub: ConnectRequest from already connected client {peer_addr} (vmac={registered_vmac:02x?}), closing"
                    );
                    let mut w = write.lock().await;
                    let _ = w.send(Message::Close(None)).await;
                    break;
                }

                let mut vmac = [0u8; 6];
                vmac.copy_from_slice(&sc_msg.payload[0..6]);
                // Parse Device UUID (bytes 6..22) and max lengths (bytes 22..26).
                let mut client_uuid = [0u8; 16];
                client_uuid.copy_from_slice(&sc_msg.payload[6..22]);
                let client_max_bvlc = u16::from_be_bytes([sc_msg.payload[22], sc_msg.payload[23]]);
                let client_max_npdu = u16::from_be_bytes([sc_msg.payload[24], sc_msg.payload[25]]);
                debug!("Hub: ConnectRequest from {peer_addr} vmac={vmac:02x?} max_bvlc={client_max_bvlc} max_npdu={client_max_npdu}");

                match connect_request_vmac_disposition(vmac, hub_vmac) {
                    ConnectRequestVmacDisposition::Accept => {}
                    ConnectRequestVmacDisposition::CloseReserved => {
                        warn!("Hub: rejecting reserved VMAC {vmac:02x?} from {peer_addr}");
                        break;
                    }
                    ConnectRequestVmacDisposition::Nak(error_class, error_code) => {
                        warn!("Hub: VMAC collision for {vmac:02x?} from {peer_addr}");
                        let error_result = build_bvlc_result_nak(
                            sc_msg.message_id,
                            ScFunction::ConnectRequest,
                            error_class,
                            error_code,
                        );
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &error_result);
                        let mut w = write.lock().await;
                        let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                        break;
                    }
                }

                // Check for VMAC collision / Device UUID replacement and
                // register atomically under a single lock to prevent TOCTOU races.
                const MAX_SC_CLIENTS: usize = 256;
                {
                    #[cfg(test)]
                    deadline.admission_started.store(true, Ordering::Release);
                    let mut map = clients.lock().await;
                    let decision = hub_client_registration_decision(
                        vmac,
                        client_uuid,
                        map.iter().map(|(vmac, client)| (*vmac, client.device_uuid)),
                        MAX_SC_CLIENTS,
                    );
                    // The clock is checked under the registry lock, immediately
                    // before the first irreversible replacement/insertion. No await
                    // separates deadline retirement, registry commit, and lease.vmac.
                    if matches!(
                        decision,
                        HubClientRegistrationDecision::Accept
                            | HubClientRegistrationDecision::Replace { .. }
                    ) && !deadline.commit()
                    {
                        break;
                    }
                    match decision {
                        HubClientRegistrationDecision::Accept => {}
                        HubClientRegistrationDecision::Replace { old_vmac } => {
                            let old_client = map.remove(&old_vmac);
                            if old_vmac == vmac {
                                debug!(
                                    "Hub: replacing existing connection for VMAC {vmac:02x?} and Device UUID from {peer_addr}"
                                );
                            } else {
                                debug!(
                                    "Hub: replacing existing Device UUID connection from VMAC {old_vmac:02x?} with {vmac:02x?}"
                                );
                            }
                            if let Some(client) = old_client {
                                client.closed.store(true, Ordering::Release);
                                super::retirement::wake(&client.close_notify);
                            }
                        }
                        HubClientRegistrationDecision::NakDuplicateVmac => {
                            warn!("Hub: VMAC collision for {vmac:02x?} from {peer_addr}");
                            drop(map); // release lock before sending
                            let error_result = build_bvlc_result_nak(
                                sc_msg.message_id,
                                ScFunction::ConnectRequest,
                                ErrorClass::COMMUNICATION,
                                ErrorCode::NODE_DUPLICATE_VMAC,
                            );
                            let mut buf = BytesMut::new();
                            encode_sc_message(&mut buf, &error_result);
                            let mut w = write.lock().await;
                            let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                            break;
                        }
                        HubClientRegistrationDecision::NakMaxClients => {
                            warn!("SC Hub: max clients reached, rejecting connection");
                            drop(map);
                            let error_result = build_bvlc_result_nak(
                                sc_msg.message_id,
                                ScFunction::ConnectRequest,
                                ErrorClass::RESOURCES,
                                ErrorCode::OTHER,
                            );
                            let mut buf = BytesMut::new();
                            encode_sc_message(&mut buf, &error_result);
                            let mut w = write.lock().await;
                            let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                            break;
                        }
                    };
                    map.insert(
                        vmac,
                        HubClient::new(
                            write.clone(),
                            close_requested.clone(),
                            close_notify.clone(),
                            client_uuid,
                            client_max_bvlc,
                            client_max_npdu,
                            client_activity.clone(),
                        ),
                    );
                    lease.vmac = Some(vmac);
                }

                let mut accept_payload = Vec::with_capacity(26);
                accept_payload.extend_from_slice(&hub_vmac);
                accept_payload.extend_from_slice(&hub_uuid);
                accept_payload.extend_from_slice(&HUB_MAX_BVLC_LENGTH.to_be_bytes());
                accept_payload.extend_from_slice(&HUB_MAX_NPDU_LENGTH.to_be_bytes());
                let accept = ScMessage {
                    function: ScFunction::ConnectAccept,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::from(accept_payload),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &accept);

                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Binary(buf.to_vec().into())).await {
                    warn!("Hub: failed to send ConnectAccept to {peer_addr}: {e}");
                    break;
                }
            }

            ScFunction::HeartbeatRequest => {
                let ack = ScMessage {
                    function: ScFunction::HeartbeatAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);

                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Binary(buf.to_vec().into())).await {
                    warn!("Hub: failed to send HeartbeatAck to {peer_addr}: {e}");
                    break;
                }
            }

            ScFunction::HeartbeatAck => {
                if let Some(registered_vmac) = lease.vmac {
                    heartbeat::clear_matching_heartbeat_ack(
                        &clients,
                        registered_vmac,
                        &write,
                        sc_msg.message_id,
                    )
                    .await;
                    on_heartbeat_ack();
                }
            }

            ScFunction::DisconnectRequest => {
                debug!("Hub: DisconnectRequest from {peer_addr}");
                let ack = ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: sc_msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);

                let mut w = write.lock().await;
                let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                break;
            }

            ScFunction::Result => {
                let Some(registered_vmac) = lease.vmac else {
                    super::malformed_diag::result_before_connect(&mut malformed_diag, peer_addr);
                    continue;
                };
                if relay_result(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    &clients,
                    &write,
                    &close_requested,
                    &mut malformed_diag,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            }

            ScFunction::EncapsulatedNpdu => {
                let Some(registered_vmac) = lease.vmac else {
                    super::malformed_diag::npdu_before_connect(&mut malformed_diag, peer_addr);
                    let nak = build_bvlc_result_nak(
                        sc_msg.message_id,
                        ScFunction::EncapsulatedNpdu,
                        ErrorClass::COMMUNICATION,
                        ErrorCode::OTHER,
                    );
                    let mut buf = BytesMut::new();
                    encode_sc_message(&mut buf, &nak);
                    let mut w = write.lock().await;
                    let _ = w.send(Message::Binary(buf.to_vec().into())).await;
                    continue;
                };

                let relay_target = match hub_relay_target(&sc_msg) {
                    Ok(target) => target,
                    Err(HubRelayReject::OriginatingVmacPresent) => {
                        super::malformed_diag::originating_vmac(&mut malformed_diag, peer_addr);
                        continue;
                    }
                    Err(HubRelayReject::MissingDestinationVmac) => {
                        super::malformed_diag::missing_destination(&mut malformed_diag, peer_addr);
                        continue;
                    }
                };

                let npdu_len = sc_msg.payload.len();

                let Some(relay_buf) =
                    encode_hub_relay_frame(&data, &sc_msg, registered_vmac, relay_target)
                else {
                    super::malformed_diag::preserve_failure(&mut malformed_diag, peer_addr);
                    continue;
                };
                let relay_bytes: Vec<u8> = relay_buf.to_vec();
                let relay_len = relay_bytes.len();

                if relay_target == HubRelayTarget::Broadcast {
                    // Parallel broadcast relay with per-client timeout
                    let sinks: Vec<HubRelaySink> = {
                        let map = clients.lock().await;
                        if !registered_client_matches_sink_in_map(&map, registered_vmac, &write) {
                            debug!(
                                "Hub: client {peer_addr} (vmac={registered_vmac:02x?}) was superseded before broadcast relay"
                            );
                            break;
                        }
                        let recipients = hub_relay_recipient_vmacs(
                            relay_target,
                            registered_vmac,
                            map.keys().copied(),
                        );
                        recipients
                            .into_iter()
                            .filter_map(|vmac| {
                                let c = map.get(&vmac)?;
                                match relay_limit_decision(
                                    npdu_len, relay_len, c.max_npdu, c.max_bvlc,
                                ) {
                                    RelayLimitDecision::Send => {
                                        Some(HubRelaySink::capture(vmac, c))
                                    }
                                    RelayLimitDecision::DropMaxNpdu => {
                                        super::malformed_diag::broadcast_npdu_drop(
                                            &mut malformed_diag,
                                            npdu_len,
                                            c.max_npdu,
                                            vmac,
                                        );
                                        None
                                    }
                                    RelayLimitDecision::DropMaxBvlc => {
                                        super::malformed_diag::broadcast_bvlc_drop(
                                            &mut malformed_diag,
                                            relay_len,
                                            c.max_bvlc,
                                            vmac,
                                        );
                                        None
                                    }
                                }
                            })
                            .collect()
                    };
                    let relay_shared = Bytes::from(relay_bytes);
                    let futs: Vec<_> = sinks
                        .into_iter()
                        .map(|target| {
                            let data = relay_shared.clone();
                            let clients = &clients;
                            async move {
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(5),
                                    super::relay_send::send(
                                        &target,
                                        clients,
                                        Message::Binary(data.to_vec().into()),
                                        &super::relay_send::SocketIo,
                                    ),
                                )
                                .await;
                                if let Err(_) | Ok(Err(_)) = result {
                                    warn!("Hub: broadcast relay failed to {:02x?}", target.vmac);
                                }
                            }
                        })
                        .collect();
                    futures_util::future::join_all(futs).await;
                } else if let HubRelayTarget::Unicast(dest) = relay_target {
                    let target = {
                        let map = clients.lock().await;
                        if !registered_client_matches_sink_in_map(&map, registered_vmac, &write) {
                            debug!(
                                "Hub: client {peer_addr} (vmac={registered_vmac:02x?}) was superseded before unicast relay"
                            );
                            break;
                        }
                        let recipients = hub_relay_recipient_vmacs(
                            relay_target,
                            registered_vmac,
                            map.keys().copied(),
                        );
                        recipients.into_iter().next().and_then(|vmac| {
                            map.get(&vmac)
                                .map(|c| (HubRelaySink::capture(vmac, c), c.max_npdu, c.max_bvlc))
                        })
                    };
                    if let Some((target, max_npdu, max_bvlc)) = target {
                        match relay_limit_decision(npdu_len, relay_len, max_npdu, max_bvlc) {
                            RelayLimitDecision::Send => {
                                if let Err(e) = super::relay_send::send(
                                    &target,
                                    &clients,
                                    Message::Binary(relay_bytes.into()),
                                    &super::relay_send::SocketIo,
                                )
                                .await
                                {
                                    warn!("Hub: unicast relay error to {dest:02x?}: {e}");
                                }
                            }
                            RelayLimitDecision::DropMaxNpdu => {
                                super::malformed_diag::unicast_npdu_drop(
                                    &mut malformed_diag,
                                    npdu_len,
                                    max_npdu,
                                    dest,
                                );
                            }
                            RelayLimitDecision::DropMaxBvlc => {
                                super::malformed_diag::unicast_bvlc_drop(
                                    &mut malformed_diag,
                                    relay_len,
                                    max_bvlc,
                                    dest,
                                );
                            }
                        }
                    } else {
                        super::malformed_diag::no_unicast_target(&mut malformed_diag, dest);
                    }
                }
            }

            other => {
                super::malformed_diag::unknown_function(&mut malformed_diag, peer_addr, &other);
                let nak = build_bvlc_result_nak(
                    sc_msg.message_id,
                    other,
                    ErrorClass::COMMUNICATION,
                    unexpected_bvlc_function_error_code(other),
                );
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &nak);
                let mut w = write.lock().await;
                let _ = w.send(Message::Binary(buf.to_vec().into())).await;
            }
        }
    }
}
