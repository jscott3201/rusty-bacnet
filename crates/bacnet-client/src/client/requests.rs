use super::*;
use bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE;

/// Which term of Clause 5.2.1.2's minimum bound the transmittable length down.
///
/// Named separately so the error blames the right one. The checked value is a
/// minimum over several terms, and the peer is only sometimes the binding one:
/// BACnet/SC recomputes its own limit from the hub's Connect-Accept, so a
/// transport can fall below the floor while the peer is perfectly conformant.
enum LengthBoundedBy {
    /// A length advertised by a discovered peer, from I-Am.
    DiscoveredPeer(u16),
    /// This client's own configured maximum.
    LocalConfig(u16),
    /// The data link, after any routed NPDU header.
    Transport(u16),
}

impl core::fmt::Display for LengthBoundedBy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DiscoveredPeer(v) => {
                write!(f, "the peer's advertised Max APDU Length Accepted of {v}")
            }
            Self::LocalConfig(v) => write!(f, "this client's configured maximum of {v}"),
            Self::Transport(v) => write!(f, "the transport's limit of {v}"),
        }
    }
}

/// Reject a maximum transmittable length no conformant device could accept.
///
/// Clause 5.2.1.2 derives this length as the smallest of the local capability,
/// the internetwork limit, and "(c) the maximum APDU size accepted by the
/// remote peer device, which must be at least 50 octets". Below that floor no
/// conformant APDU can be formed at all. Clause 20.1.2.5 gives the same number
/// a name, spelling the lowest max-APDU-length-accepted code `B'0000'` as "Up
/// to MinimumMessageSize (50 octets)".
///
/// The check is a floor, not membership of the six values Clause 20.1.2.5
/// encodes. A discovered peer's length comes from I-Am's `Max APDU Length
/// Accepted`, an Unsigned octet count rather than the four-bit code, and
/// Clause 20.1.2.5 notes the true value "may be larger than indicated in this
/// parameter" — so 600 and 1500 are legitimate and must not be rejected.
///
/// Failing rather than clamping up to 50 keeps the client from inventing a
/// capability the peer never claimed: a device advertising less than 50 is
/// already non-conformant, and a typed error names that where a silent clamp
/// would send it frames it said it cannot hold.
fn check_transmittable_length(peer_or_local: LengthBoundedBy, transport: u16) -> Result<(), Error> {
    let advertised = match peer_or_local {
        LengthBoundedBy::DiscoveredPeer(v) | LengthBoundedBy::LocalConfig(v) => v,
        LengthBoundedBy::Transport(v) => v,
    };
    let combined = advertised.min(transport);
    if combined >= MINIMUM_MESSAGE_SIZE {
        return Ok(());
    }
    let binding = if transport < advertised {
        LengthBoundedBy::Transport(transport)
    } else {
        peer_or_local
    };
    Err(Error::Encoding(format!(
        "maximum transmittable length {combined} is below the {MINIMUM_MESSAGE_SIZE}-octet \
         MinimumMessageSize every BACnet device accepts (Clause 5.2.1.2); the binding limit is \
         {binding}"
    )))
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a confirmed request and wait for the response.
    ///
    /// Returns the service response data (empty for SimpleAck). Automatically
    /// uses segmented transfer when the payload exceeds the remote device's
    /// max APDU length.
    pub async fn confirmed_request(
        &self,
        destination_mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        self.confirmed_request_inner(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            service_choice,
            service_data,
        )
        .await
    }

    /// Send a confirmed request routed through a BACnet router.
    ///
    /// The NPDU is sent as a unicast to `router_mac` with DNET/DADR set so
    /// the router forwards it to `dest_network`/`dest_mac`.
    pub async fn confirmed_request_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        self.confirmed_request_inner(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            service_choice,
            service_data,
        )
        .await
    }

    pub(super) async fn confirmed_request_inner(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        // The lease serializes one active request per (immediate router, DNET)
        // and remains live through every return path, including cancellation.
        let path_lease = match target {
            ConfirmedTarget::Local { .. } => None,
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                ..
            } => Some(
                self.routed_path_limits
                    .acquire(router_mac, dest_network)
                    .await?,
            ),
        };
        let (routed_path_max_apdu, routed_forwarded_npci_len) = match (path_lease.as_ref(), target)
        {
            (
                Some(lease),
                ConfirmedTarget::Routed {
                    dest_mac,
                    dest_network: _,
                    router_mac: _,
                },
            ) => (
                Some(lease.max_apdu(dest_mac.len(), self.local_mac.len())?),
                Some(lease.forwarded_npci_len(dest_mac.len(), self.local_mac.len())?),
            ),
            _ => (None, None),
        };
        let transaction_peer = target.transaction_peer();
        let tsm_mac = transaction_peer.tsm_mac;
        let unsegmented_apdu_size = 4 + service_data.len();
        let target_transport_max_apdu = self.target_transport_max_apdu_length(target);

        let (peer_max_apdu, remote_max_segments, advertised, peer_segmentation) = {
            let dt = self.device_table.lock().await;
            // A routed peer is recorded under the router's MAC, so only the
            // SNET/SADR of the NPDU that carried its I-Am identifies it
            // (Clause 5.2.1.2 term (c) binds the peer's Max APDU Length
            // Accepted regardless of how the peer is reached).
            let (device, peer_segmentation) = match target {
                ConfirmedTarget::Local { mac } => {
                    (dt.get_by_mac(mac), dt.local_peer_segmentation(mac))
                }
                ConfirmedTarget::Routed {
                    dest_network,
                    dest_mac,
                    ..
                } => (
                    dt.get_by_network_address(dest_network, dest_mac),
                    dt.routed_peer_segmentation(dest_network, dest_mac),
                ),
            };
            let max_apdu = device
                .map(|d| u16::try_from(d.max_apdu_length).unwrap_or(u16::MAX))
                .unwrap_or(self.config.max_apdu_length);
            let max_seg = device.and_then(|d| d.max_segments_accepted);
            let advertised = if device.is_some() {
                LengthBoundedBy::DiscoveredPeer(max_apdu)
            } else {
                LengthBoundedBy::LocalConfig(max_apdu)
            };
            (max_apdu, max_seg, advertised, peer_segmentation)
        };
        check_transmittable_length(advertised, target_transport_max_apdu)?;
        if routed_path_max_apdu.is_some_and(|path_max_apdu| {
            path_max_apdu < MINIMUM_MESSAGE_SIZE
                && path_max_apdu <= peer_max_apdu.min(target_transport_max_apdu)
        }) {
            let ConfirmedTarget::Routed { dest_network, .. } = target else {
                unreachable!("a routed path limit exists only for a routed target")
            };
            return Err(Error::RoutedPathTooLong { dnet: dest_network });
        }
        let remote_max_apdu = peer_max_apdu
            .min(target_transport_max_apdu)
            .min(routed_path_max_apdu.unwrap_or(u16::MAX));
        if unsegmented_apdu_size > remote_max_apdu as usize {
            // Clause 12.11: a NO_SEGMENTATION or SEGMENTED_TRANSMIT peer
            // accepts exactly one segment — only unsegmented requests — and
            // Clause 18's SEGMENTATION_NOT_SUPPORTED abort is the certain
            // outcome of sending anyway. Refuse locally when the capability
            // is authoritative (I-Am or explicit configuration); a legacy
            // placeholder row stays unknown and keeps today's behavior.
            if let Some(crate::discovery::PeerSegmentation::Authoritative(capability)) =
                peer_segmentation
            {
                if matches!(
                    capability,
                    bacnet_types::enums::Segmentation::NONE
                        | bacnet_types::enums::Segmentation::TRANSMIT
                ) {
                    return Err(Error::Segmentation(format!(
                        "request requires segmentation but the peer advertised \
                         {capability:?} and cannot receive segmented requests"
                    )));
                }
            }
            return self
                .segmented_confirmed_request(
                    target,
                    service_choice,
                    service_data,
                    remote_max_apdu,
                    remote_max_segments,
                    routed_forwarded_npci_len,
                    path_lease.as_ref(),
                )
                .await;
        }

        let advertised_max_apdu = self.advertised_max_apdu_length_for_target(target)?;
        let (invoke_id, registration) = {
            let mut tsm = self.tsm.lock().await;
            tsm.register_coordinated_transaction_with_progress(
                tsm_mac.clone(),
                transaction_peer.canonical,
                service_choice,
                false,
            )
            .map_err(|error| Error::Encoding(error.to_string()))?
        };

        let owner = registration.owner.clone();
        let mut guard = TransactionGuard::new(
            Arc::clone(&self.tsm),
            self.cleanup_tx.clone(),
            tsm_mac.clone(),
            invoke_id,
            owner.clone(),
            None,
        );

        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: self.config.segmented_response_accepted,
            max_segments: self.config.max_segments,
            max_apdu_length: advertised_max_apdu,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(6 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        if let Some(forwarded_npci_len) = routed_forwarded_npci_len {
            self.routed_path_limits.install_active(
                target,
                tsm_mac.clone(),
                invoke_id,
                owner.clone(),
                forwarded_npci_len,
                self.network.network_control_ingress_sequence(),
            );
        }
        if !self.routed_path_limits.authorize_attempt(
            target,
            &tsm_mac,
            invoke_id,
            &owner,
            buf.len(),
        ) {
            guard.mark_completed();
            self.tsm
                .lock()
                .await
                .cancel_transaction_for_owner(&tsm_mac, invoke_id, &owner);
            self.enqueue_transaction_cleanup(&tsm_mac, invoke_id, &owner, false, None);
            return Err(Error::Encoding(
                "routed send lost its active path authorization".into(),
            ));
        }

        if let Err(e) = self.send_confirmed_target_apdu(target, &buf).await {
            guard.mark_completed();
            let mut tsm = self.tsm.lock().await;
            tsm.cancel_transaction_for_owner(&tsm_mac, invoke_id, &owner);
            drop(tsm);
            self.enqueue_transaction_cleanup(&tsm_mac, invoke_id, &owner, false, None);
            return Err(e);
        }

        let response = self
            .wait_for_confirmed_response(
                target,
                &tsm_mac,
                invoke_id,
                &owner,
                registration.response,
                registration.progress,
                Some(&buf),
            )
            .await;
        if response.is_ok() {
            if let Some(lease) = path_lease.as_ref() {
                lease.mark_terminal_observed();
            }
        }
        guard.mark_completed();
        response.and_then(Self::confirmed_response_result)
    }

    /// Wait through AWAIT_CONFIRMATION and SEGMENTED_CONF without allowing
    /// their timers to cancel each other's phase.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn wait_for_confirmed_response(
        &self,
        target: ConfirmedTarget<'_>,
        tsm_mac: &MacAddr,
        invoke_id: u8,
        owner: &TransactionOwner,
        mut response_rx: oneshot::Receiver<TsmResponse>,
        mut progress_rx: tokio::sync::watch::Receiver<TransactionProgress>,
        retry_apdu: Option<&[u8]>,
    ) -> Result<TsmResponse, Error> {
        let (request_timeout, segment_timeout, max_retries) = {
            let tsm = self.tsm.lock().await;
            let config = tsm.config();
            (
                Duration::from_millis(config.apdu_timeout_ms),
                Duration::from_millis(config.apdu_segment_timeout_ms.saturating_mul(4)),
                config.apdu_retries,
            )
        };
        let mut retries_sent = 0u16;
        let mut progress = *progress_rx.borrow_and_update();

        loop {
            let wait = match progress {
                TransactionProgress::AwaitingResponse => request_timeout,
                TransactionProgress::SegmentedResponse { .. } => segment_timeout,
            };
            let timer = tokio::time::sleep(wait);
            tokio::pin!(timer);

            tokio::select! {
                response = &mut response_rx => {
                    return response.map_err(|_| Error::Encoding("TSM response channel closed".into()));
                }
                changed = progress_rx.changed() => {
                    if changed.is_err() {
                        return (&mut response_rx)
                            .await
                            .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                    }
                    progress = *progress_rx.borrow_and_update();
                }
                _ = &mut timer => {
                    match progress {
                        TransactionProgress::AwaitingResponse => {
                            let retry = retry_apdu.is_some()
                                && retries_sent < u16::from(max_retries);
                            let mut tsm = self.tsm.lock().await;
                            match tsm.expire_request_timer(tsm_mac, invoke_id, owner, !retry) {
                                RequestTimerExpiration::Retry => {
                                    // Authorization and segment admission use the TSM lock,
                                    // but transport I/O must not. Once authorized, this retry
                                    // may finish even if segment zero is admitted meanwhile.
                                    let Some(retry_apdu) = retry_apdu else {
                                        tsm.cancel_transaction_for_owner(tsm_mac, invoke_id, owner);
                                        drop(tsm);
                                        self.enqueue_transaction_cleanup(
                                            tsm_mac,
                                            invoke_id,
                                            owner,
                                            false,
                                            None,
                                        );
                                        return Err(Error::Timeout(request_timeout));
                                    };
                                    retries_sent += 1;
                                    drop(tsm);
                                    if !self.routed_path_limits.authorize_attempt(
                                        target,
                                        tsm_mac,
                                        invoke_id,
                                        owner,
                                        retry_apdu.len(),
                                    ) {
                                        return (&mut response_rx).await.map_err(|_| {
                                            Error::Encoding("TSM response channel closed".into())
                                        });
                                    }
                                    let send_result = self
                                        .send_confirmed_target_apdu(target, retry_apdu)
                                        .await;
                                    if let Err(error) = send_result {
                                        let mut tsm = self.tsm.lock().await;
                                        match tsm.expire_request_timer(tsm_mac, invoke_id, owner, true) {
                                            RequestTimerExpiration::SegmentedResponse { generation } => {
                                                progress = TransactionProgress::SegmentedResponse { generation };
                                                continue;
                                            }
                                            RequestTimerExpiration::NoTransaction => {
                                                drop(tsm);
                                                return (&mut response_rx)
                                                    .await
                                                    .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                                            }
                                            RequestTimerExpiration::TimedOut => {
                                                drop(tsm);
                                                self.enqueue_transaction_cleanup(
                                                    tsm_mac,
                                                    invoke_id,
                                                    owner,
                                                    false,
                                                    None,
                                                );
                                                return Err(error);
                                            }
                                            RequestTimerExpiration::Retry => unreachable!(
                                                "final retry-send failure disposition cannot authorize another retry"
                                            ),
                                        }
                                    }
                                    debug!(
                                        invoke_id,
                                        attempt = retries_sent,
                                        max_retries,
                                        "APDU timeout, retrying confirmed request"
                                    );
                                }
                                RequestTimerExpiration::SegmentedResponse { generation } => {
                                    progress = TransactionProgress::SegmentedResponse { generation };
                                }
                                RequestTimerExpiration::TimedOut => {
                                    drop(tsm);
                                    self.enqueue_transaction_cleanup(
                                        tsm_mac,
                                        invoke_id,
                                        owner,
                                        false,
                                        None,
                                    );
                                    return Err(Error::Abort {
                                        reason: bacnet_types::enums::AbortReason::TSM_TIMEOUT.to_raw(),
                                    });
                                }
                                RequestTimerExpiration::NoTransaction => {
                                    drop(tsm);
                                    return (&mut response_rx)
                                        .await
                                        .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                                }
                            }
                        }
                        TransactionProgress::SegmentedResponse { generation } => {
                            let mut tsm = self.tsm.lock().await;
                            match tsm.expire_segment_timer(tsm_mac, invoke_id, owner, generation) {
                                SegmentTimerExpiration::Activity { generation } => {
                                    progress = TransactionProgress::SegmentedResponse { generation };
                                }
                                SegmentTimerExpiration::AwaitingResponse => {
                                    progress = TransactionProgress::AwaitingResponse;
                                }
                                SegmentTimerExpiration::TimedOut => {
                                    drop(tsm);
                                    #[cfg(test)]
                                    self.segmented_cleanup.pause_if_enabled().await;
                                    self.enqueue_transaction_cleanup(
                                        tsm_mac,
                                        invoke_id,
                                        owner,
                                        false,
                                        None,
                                    );
                                    return Err(Error::Abort {
                                        reason: bacnet_types::enums::AbortReason::TSM_TIMEOUT.to_raw(),
                                    });
                                }
                                SegmentTimerExpiration::NoTransaction => {
                                    drop(tsm);
                                    return (&mut response_rx)
                                        .await
                                        .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn enqueue_transaction_cleanup(
        &self,
        mac: &MacAddr,
        invoke_id: u8,
        owner: &TransactionOwner,
        cancel_tsm: bool,
        seg_ack_sender: Option<mpsc::Sender<SegmentAckPdu>>,
    ) {
        let _ = self.cleanup_tx.send(TransactionCleanup {
            mac: mac.clone(),
            invoke_id,
            owner: owner.clone(),
            cancel_tsm,
            seg_ack_sender,
        });
    }

    pub(super) fn confirmed_response_result(response: TsmResponse) -> Result<Bytes, Error> {
        super::confirmed_response_result(response)
    }

    pub(super) async fn send_confirmed_target_apdu(
        &self,
        target: ConfirmedTarget<'_>,
        apdu: &[u8],
    ) -> Result<(), Error> {
        match target {
            ConfirmedTarget::Local { mac } => {
                self.network
                    .send_apdu(apdu, mac, true, NetworkPriority::NORMAL)
                    .await
            }
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            } => {
                self.network
                    .send_apdu_routed(
                        apdu,
                        dest_network,
                        dest_mac,
                        router_mac,
                        true,
                        NetworkPriority::NORMAL,
                    )
                    .await
            }
        }
    }

    pub(super) fn target_transport_max_apdu_length(&self, target: ConfirmedTarget<'_>) -> u16 {
        self.network
            .transport()
            .max_apdu_length()
            .saturating_sub(target.additional_npdu_header_len())
    }

    pub(super) fn advertised_max_apdu_length_for_target(
        &self,
        target: ConfirmedTarget<'_>,
    ) -> Result<u16, Error> {
        cap_max_apdu_to_transport(
            self.config.max_apdu_length,
            self.target_transport_max_apdu_length(target),
        )
    }

    /// Send an unconfirmed request (fire-and-forget) to a specific destination.
    pub async fn unconfirmed_request(
        &self,
        destination_mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.network
            .send_apdu(&buf, destination_mac, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request on the local network.
    pub async fn broadcast_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.network
            .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request globally (DNET=0xFFFF).
    pub async fn broadcast_global_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.network
            .broadcast_global_apdu(&buf, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request to a specific remote network.
    pub async fn broadcast_network_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
        dest_network: u16,
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.network
            .broadcast_to_network(&buf, dest_network, false, NetworkPriority::NORMAL)
            .await
    }
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod transmittable_length_tests;
