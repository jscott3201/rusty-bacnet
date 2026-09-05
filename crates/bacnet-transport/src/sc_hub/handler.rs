//! Hub message dispatch and sink-identity cleanup.

use super::*;

pub(super) async fn run(
    peer_addr: SocketAddr,
    hub: (Vmac, DeviceUuid),
    mut read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: Clients,
    deadline: &super::deadlines::ConnectDeadline,
    on_heartbeat_ack: impl Fn() + Send,
) {
    let (hub_vmac, hub_uuid) = hub;
    let mut client_vmac: Option<Vmac> = None;
    let close_requested = Arc::new(AtomicBool::new(false));
    let close_notify = Arc::new(Notify::new());
    let client_activity: Arc<AtomicU64> = Arc::new(AtomicU64::new(now_secs()));

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
        let msg_result = tokio::select! {
            _ = close_notify.notified() => {
                debug!("Hub: client {peer_addr} was superseded");
                break;
            }
            msg = read.next() => msg,
        };
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
            warn!(
                "Hub: frame from {peer_addr} is {} bytes, exceeds hub Max-BVLC-Length {}, dropping",
                data.len(),
                HUB_MAX_BVLC_LENGTH
            );
            continue;
        }

        let sc_msg = match decode_sc_message(&data) {
            Ok(m) => m,
            Err(e) => {
                warn!("Hub: decode error from {peer_addr}: {e}");
                continue;
            }
        };

        if close_requested.load(Ordering::Acquire) {
            debug!("Hub: client {peer_addr} received message after replacement");
            break;
        }

        if let Some(registered_vmac) = client_vmac {
            if !registered_client_matches_sink(&clients, registered_vmac, &write).await {
                debug!("Hub: client {peer_addr} (vmac={registered_vmac:02x?}) was superseded");
                break;
            }
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
            if client_vmac.is_none() {
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
            warn!("Hub: NPDU exceeds local Max-NPDU-Length, dropping");
            continue;
        }

        // Decoded BVLC messages that pass Connect/control admission and local
        // NPDU capacity checks count as activity.
        // WebSocket control, oversized, and undecodable frames do not.
        client_activity.store(now_secs(), std::sync::atomic::Ordering::Release);

        match sc_msg.function {
            ScFunction::ConnectRequest => {
                if let Some(registered_vmac) = client_vmac {
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
                let superseded = {
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
                    // separates deadline retirement, registry commit, and client_vmac.
                    if matches!(
                        decision,
                        HubClientRegistrationDecision::Accept
                            | HubClientRegistrationDecision::Replace { .. }
                    ) && !deadline.commit()
                    {
                        break;
                    }
                    let superseded = match decision {
                        HubClientRegistrationDecision::Accept => None,
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
                            old_client.and_then(|client| {
                                client.closed.store(true, Ordering::Release);
                                if Arc::ptr_eq(&client.sink, &write) {
                                    None
                                } else {
                                    Some((client.sink, client.close_notify))
                                }
                            })
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
                    superseded
                };
                client_vmac = Some(vmac);

                if let Some((sink, notify)) = superseded {
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                            let mut old = sink.lock().await;
                            old.send(Message::Close(None)).await?;
                            old.flush().await
                        })
                        .await;
                        notify.notify_waiters();
                    });
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
                if let Some(registered_vmac) = client_vmac {
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
                let Some(registered_vmac) = client_vmac else {
                    debug!("Hub: Result before ConnectRequest from {peer_addr}, dropping");
                    continue;
                };
                if relay_result(
                    &data,
                    &sc_msg,
                    registered_vmac,
                    &clients,
                    &write,
                    &close_requested,
                )
                .await
                    == ResultRelayDisposition::CloseSource
                {
                    break;
                }
            }

            ScFunction::EncapsulatedNpdu => {
                let Some(registered_vmac) = client_vmac else {
                    warn!("Hub: EncapsulatedNpdu before ConnectRequest from {peer_addr} — sending NAK");
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
                        warn!(
                            "Hub: EncapsulatedNpdu from {peer_addr} had Originating VMAC, dropping"
                        );
                        continue;
                    }
                    Err(HubRelayReject::MissingDestinationVmac) => {
                        warn!(
                            "Hub: EncapsulatedNpdu from {peer_addr} missing Destination VMAC, dropping"
                        );
                        continue;
                    }
                };

                let npdu_len = sc_msg.payload.len();

                let Some(relay_buf) =
                    encode_hub_relay_frame(&data, &sc_msg, registered_vmac, relay_target)
                else {
                    warn!("Hub: failed to preserve EncapsulatedNpdu frame from {peer_addr}");
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
                                    npdu_len,
                                    relay_len,
                                    c.max_npdu,
                                    c.max_bvlc,
                                ) {
                                    RelayLimitDecision::Send => Some(HubRelaySink {
                                        vmac,
                                        sink: Arc::clone(&c.sink),
                                        closed: Arc::clone(&c.closed),
                                    }),
                                    RelayLimitDecision::DropMaxNpdu => {
                                        warn!(
                                            "Hub: broadcast NPDU ({npdu_len} bytes) exceeds target max_npdu ({}) for {vmac:02x?}, dropping for target",
                                            c.max_npdu
                                        );
                                        None
                                    }
                                    RelayLimitDecision::DropMaxBvlc => {
                                        warn!(
                                            "Hub: broadcast BVLC ({relay_len} bytes) exceeds target max_bvlc ({}) for {vmac:02x?}, dropping for target",
                                            c.max_bvlc
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
                            let close_requested = close_requested.clone();
                            async move {
                                if close_requested.load(Ordering::Acquire)
                                    || target.closed.load(Ordering::Acquire)
                                {
                                    return;
                                }
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(5),
                                    async {
                                        let mut w = target.sink.lock().await;
                                        if close_requested.load(Ordering::Acquire)
                                            || target.closed.load(Ordering::Acquire)
                                        {
                                            return Ok::<(), tokio_tungstenite::tungstenite::Error>(
                                                (),
                                            );
                                        }
                                        w.send(Message::Binary(data.to_vec().into())).await
                                    },
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
                            map.get(&vmac).map(|c| {
                                (
                                    Arc::clone(&c.sink),
                                    Arc::clone(&c.closed),
                                    c.max_npdu,
                                    c.max_bvlc,
                                )
                            })
                        })
                    };
                    if let Some((sink, target_closed, max_npdu, max_bvlc)) = target {
                        match relay_limit_decision(npdu_len, relay_len, max_npdu, max_bvlc) {
                            RelayLimitDecision::Send => {
                                if close_requested.load(Ordering::Acquire)
                                    || target_closed.load(Ordering::Acquire)
                                {
                                    break;
                                }
                                let mut w = sink.lock().await;
                                if close_requested.load(Ordering::Acquire)
                                    || target_closed.load(Ordering::Acquire)
                                {
                                    break;
                                }
                                if let Err(e) = w.send(Message::Binary(relay_bytes.into())).await {
                                    warn!("Hub: unicast relay error to {dest:02x?}: {e}");
                                }
                            }
                            RelayLimitDecision::DropMaxNpdu => warn!(
                                "Hub: NPDU ({npdu_len} bytes) exceeds target max_npdu ({max_npdu}) for {dest:02x?}, dropping"
                            ),
                            RelayLimitDecision::DropMaxBvlc => warn!(
                                "Hub: BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {dest:02x?}, dropping"
                            ),
                        }
                    } else {
                        debug!("Hub: no client with vmac {dest:02x?} for unicast relay");
                    }
                }
            }

            other => {
                debug!("Hub: unknown function {other:?} from {peer_addr}, sending NAK");
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

    if let Some(vmac) = client_vmac {
        let mut map = clients.lock().await;
        let removed = map
            .get(&vmac)
            .is_some_and(|client| Arc::ptr_eq(&client.sink, &write));
        if removed {
            map.remove(&vmac);
            debug!("Hub: client {peer_addr} (vmac={vmac:02x?}) disconnected");
        } else {
            debug!("Hub: client {peer_addr} (vmac={vmac:02x?}) disconnected after replacement");
        }
    }
}
