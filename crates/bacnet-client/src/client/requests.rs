use super::segmentation_context::{ConfirmedWait, SegmentedRequestLimits};
use super::source_audit::Refused;
use super::*;
use crate::tsm::{CoordinatedRegistrationError, TransactionRegistration};
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

/// Whether `response` is the device's own answer to a confirmed request: an
/// acknowledgment, or its Error, Reject or Abort. Any of them shows that the
/// request reached the device (Clause 19.4). A router's refusal, a reason 4
/// ending the request as [`TsmResponse::NetworkPathTooLong`], is no answer
/// from the device.
pub(super) fn answered_by_peer(response: &Result<TsmResponse, Error>) -> bool {
    matches!(
        response,
        Ok(TsmResponse::SimpleAck | TsmResponse::ComplexAck { .. } | TsmResponse::Remote { .. })
    )
}

/// Reject a maximum transmittable length no conformant device could accept.
///
/// Clause 5.2.1.2 derives this length as the smallest of the local capability,
/// the internetwork limit, and the peer's receive capacity, whose minimum
/// is 50 octets. Below that floor no
/// conformant APDU can be formed at all. Clause 20.1.2.5 gives the same number
/// a name: MinimumMessageSize, the 50-octet capacity represented by the
/// lowest max-APDU-length-accepted code `B'0000'`.
///
/// The check is a floor, not membership of the six values Clause 20.1.2.5
/// encodes. A discovered peer's length comes from I-Am's `Max APDU Length
/// Accepted`, an Unsigned octet count rather than the four-bit code, and
/// Clause 20.1.2.5 allows actual capacity to exceed the encoded value,
/// so 600 and 1500 are legitimate and must not be rejected.
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
    /// Send a confirmed request to the station `to` and wait for its answer:
    /// the service data of its acknowledgement (empty for a SimpleAck). The
    /// typed calls are this with their request encoded and their answer
    /// checked. A request larger than the device takes is sent segmented
    /// when it accepts segments.
    ///
    /// `to` is a MAC on this link, a [`BACnetAddress`] or a [`Station`]
    /// ([`ToStation`] lists the forms). A station on another network goes
    /// through the router the caller names or, for an address, the one the
    /// discovery table's row at it names (Clause 6.2.2), with its network
    /// and MAC as the NPDU's DNET and DADR. Once the client has learned its
    /// own network's number, a station on it is sent to at its MAC on this
    /// link, with no DNET and not through any router (#1358); an answer from
    /// it with no SNET completes the request, and so does one a router
    /// relays back with that network and MAC as its SNET and SADR (#1465),
    /// since both name the same station. The row at the station's address,
    /// when the table has one, bounds the request by the device's Max APDU
    /// Length Accepted and segmentation, and an answer from it starts the
    /// row's age again; with no row the client's configured limits apply.
    ///
    /// # Errors
    ///
    /// Before any request state is taken: [`Error::Encoding`] for a station
    /// a confirmed request can't go to. That is a MAC that reaches a group
    /// of nodes on this link ([`TransportPort::is_group_destination`]: the
    /// link's broadcast MAC, or on B/IP the limited broadcast, the
    /// configured broadcast IP or a multicast address at any port), which
    /// would make it a local broadcast (Clause 6.3, #1479); an empty MAC in
    /// an address; and on another network a network of 0 or 65535 or a MAC
    /// longer than [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets, which
    /// no router carries to one station. [`Error::UnknownDevice`] for a
    /// remote address with no named router and no row in the discovery
    /// table, since the row is the route. Then the errors of the request
    /// itself.
    ///
    /// [`BACnetAddress`]: bacnet_types::constructed::BACnetAddress
    pub async fn confirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        to.confirmed(self, service_choice, service_data).await
    }

    /// Register a confirmed request with the TSM, its lock held for the
    /// call alone. The segmented request matches on the result where it is
    /// made, without naming it, so its future doesn't keep the result
    /// through its later awaits (#2617).
    #[inline(always)]
    pub(super) fn register_transaction(
        &self,
        tsm_mac: MacAddr,
        canonical: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        segmented: bool,
    ) -> Result<(u8, TransactionRegistration), CoordinatedRegistrationError> {
        self.tsm
            .lock()
            .register_coordinated_transaction_with_progress(
                tsm_mac,
                canonical,
                service_choice,
                segmented,
            )
    }

    pub(super) async fn confirmed_request_inner(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        // A destination that is not one device (#1278), or a DADR or local
        // source MAC that no router could carry (#1267), fails here, before
        // the path gate is reserved or awaited and before TSM registration.
        // A routed target on this client's own network is then sent as the
        // local target it is (#1358); the peer's limits still come from the
        // device-table row of the target as named.
        let named = target;
        if let ConfirmedTarget::Routed {
            dest_network,
            dest_mac,
            ..
        } = named
        {
            check_routed_unicast(dest_network, dest_mac.len())?;
        }
        let target = named.localized(self.network.local_network_number().get());
        if let ConfirmedTarget::Local { mac } = target {
            // An empty MAC names no one station: refused as an address on
            // network 0 with no MAC is, whichever form named it (#2588).
            Self::station_of(0, mac)?;
            // With no DNET, a group destination is a local broadcast, which
            // carries no confirmed request (Clause 6.3, #1479).
            if self.network.transport().is_group_destination(mac) {
                return Err(Error::Encoding(
                    "a confirmed request goes to one device, not to a broadcast or group \
                     address (a local broadcast carries only an UNCONFIRMED_REQUEST APDU, \
                     Clause 6.3)"
                        .into(),
                ));
            }
        }
        let routed_forwarded_npci_len = match target {
            ConfirmedTarget::Local { .. } => None,
            ConfirmedTarget::Routed { dest_mac, .. } => {
                Some(forwarded_npci_len(dest_mac.len(), self.local_mac.len())?)
            }
        };
        // Pace before the path lease, so the wait holds up no other request
        // through the same router. The guard lives until this request
        // returns, by any path or by being dropped, and then starts the
        // interval for the next request to this destination.
        let mut _pace = self.pacer.wait(pacing::PaceKey::of(target)).await;
        // The lease serializes one active request per (immediate router, DNET)
        // and remains live through every return path, including cancellation.
        let mut path_lease = match target {
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
        let routed_path_max_apdu = path_lease
            .as_ref()
            .zip(routed_forwarded_npci_len)
            .map(|(lease, npci_len)| lease.max_apdu(npci_len));
        let transaction_peer = target.transaction_peer();
        let tsm_mac = transaction_peer.tsm_mac;
        let unsegmented_apdu_size = 4 + service_data.len();
        let target_transport_max_apdu = self.target_transport_max_apdu_length(target);

        let (peer_max_apdu, remote_max_segments, advertised, peer_segmentation, sized_by) = {
            let dt = self.device_table.lock();
            // A routed peer's row is keyed by the SNET/SADR of the NPDU that
            // carried its I-Am, never by the router's MAC (Clause 5.2.1.2
            // term (c) binds the peer's Max APDU Length Accepted regardless
            // of how the peer is reached).
            // A routed target on this client's own network is the local
            // device it is sent to, whose row sizes it when it has no
            // routed row (#1358).
            let mut sized_by = named.station();
            let mut found = dt.peer(sized_by.0, sized_by.1);
            if found.is_none()
                && let (ConfirmedTarget::Routed { .. }, ConfirmedTarget::Local { mac }) =
                    (named, target)
            {
                sized_by = (0, mac);
                found = dt.peer(0, mac);
            }
            let (device, peer_segmentation) = found.unzip();
            let max_apdu = device
                .map(|d| u16::try_from(d.max_apdu_length).unwrap_or(u16::MAX))
                .unwrap_or(self.config.max_apdu_length);
            let max_seg = device.and_then(|d| d.max_segments_accepted);
            let advertised = if device.is_some() {
                LengthBoundedBy::DiscoveredPeer(max_apdu)
            } else {
                LengthBoundedBy::LocalConfig(max_apdu)
            };
            let sized_by = device.is_some().then_some(sized_by);
            (max_apdu, max_seg, advertised, peer_segmentation, sized_by)
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
                && matches!(
                    capability,
                    bacnet_types::enums::Segmentation::NO_SEGMENTATION
                        | bacnet_types::enums::Segmentation::SEGMENTED_TRANSMIT
                )
            {
                return Err(bacnet_types::error::SegmentationError::PeerCannotReceive {
                    capability,
                }
                .into());
            }
            let mut limits = SegmentedRequestLimits {
                named: &named,
                remote_max_apdu,
                remote_max_segments,
                routed_forwarded_npci_len,
                routed_path_lease: &mut path_lease,
                pace: &mut _pace,
                sized_by,
            };
            return self
                .segmented_confirmed_request(target, service_choice, service_data, &mut limits)
                .await;
        }

        let advertised_max_apdu = self.advertised_max_apdu_length_for_target(target)?;
        // The TSM keys below copy `tsm_mac` with `from_slice`, which copies
        // it at once; `clone` copies a `MacAddr` a byte at a time (#1978).
        let registered = {
            let mut tsm = self.tsm.lock();
            tsm.register_coordinated_transaction_with_progress(
                MacAddr::from_slice(&tsm_mac),
                transaction_peer.canonical,
                service_choice,
                false,
            )
        };
        // A refusal is a closed seat, the node's DeviceCommunicationControl,
        // or a request the node audits (#2617).
        let (invoke_id, registration) = match registered {
            Ok(registered) => registered,
            Err(refused) => match self.on_refused(refused, target, service_choice, false)? {
                Refused::Registered(registered) => registered,
                Refused::Audited(auditor) => {
                    let request = self.audited_request(
                        auditor,
                        named,
                        service_choice,
                        service_data,
                        _pace,
                        path_lease,
                    );
                    return request.await;
                }
            },
        };

        let owner = registration.owner.clone();
        let mut guard = TransactionGuard::new(
            Arc::clone(&self.tsm),
            self.cleanup_tx.clone(),
            MacAddr::from_slice(&tsm_mac),
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
                MacAddr::from_slice(&tsm_mac),
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
                .cancel_transaction_for_owner(&tsm_mac, invoke_id, &owner);
            self.enqueue_transaction_cleanup(&tsm_mac, invoke_id, &owner, false, None);
            return Err(Error::Encoding(
                "routed send lost its active path authorization".into(),
            ));
        }

        if let Err(e) = self.send_confirmed_target_apdu(target, &buf).await {
            guard.mark_completed();
            let mut tsm = self.tsm.lock();
            tsm.cancel_transaction_for_owner(&tsm_mac, invoke_id, &owner);
            drop(tsm);
            self.enqueue_transaction_cleanup(&tsm_mac, invoke_id, &owner, false, None);
            return Err(e);
        }

        let response = self
            .wait_for_confirmed_response(
                ConfirmedWait {
                    target,
                    tsm_mac: &tsm_mac,
                    invoke_id,
                    owner: &owner,
                    retry_apdu: Some(&buf),
                },
                registration.response,
                registration.progress,
            )
            .await;
        // Checked here too, so a request no row sized makes no call.
        if sized_by.is_some() {
            self.note_answer(sized_by, &response);
        }
        if let Some(lease) = path_lease.as_ref()
            && answered_by_peer(&response)
        {
            lease.mark_answered();
        }
        guard.mark_completed();
        response.and_then(Self::confirmed_response_result)
    }

    /// An answer from the device ([`answered_by_peer`]) starts the age of
    /// the discovery-table row that sized the request, `sized_by`, again, so
    /// a device that keeps answering stays in the table without repeating
    /// its I-Am (#2560). With no such row the table isn't locked again.
    pub(super) fn note_answer(
        &self,
        sized_by: Option<(u16, &[u8])>,
        response: &Result<TsmResponse, Error>,
    ) {
        if let Some((network, mac)) = sized_by
            && answered_by_peer(response)
        {
            self.device_table.lock().answered(network, mac);
        }
    }

    /// Wait through AWAIT_CONFIRMATION and SEGMENTED_CONF without allowing
    /// their timers to cancel each other's phase.
    pub(super) async fn wait_for_confirmed_response(
        &self,
        wait_for: ConfirmedWait<'_>,
        mut response_rx: oneshot::Receiver<TsmResponse>,
        mut progress_rx: ProgressReceiver,
    ) -> Result<TsmResponse, Error> {
        let ConfirmedWait {
            target,
            tsm_mac,
            invoke_id,
            owner,
            retry_apdu,
        } = wait_for;
        let (request_timeout, segment_timeout, max_retries) = {
            let tsm = self.tsm.lock();
            let config = tsm.config();
            (
                Duration::from_millis(config.apdu_timeout_ms),
                Duration::from_millis(config.apdu_segment_timeout_ms.saturating_mul(4)),
                config.apdu_retries,
            )
        };
        let mut retries_sent = 0u16;
        let mut progress = progress_rx.borrow_and_update();

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
                // Progress is never published after the transaction leaves
                // the TSM; the response arm above resolves then instead.
                () = progress_rx.changed() => {
                    progress = progress_rx.borrow_and_update();
                }
                _ = &mut timer => {
                    match progress {
                        TransactionProgress::AwaitingResponse => {
                            let retry = retry_apdu.is_some()
                                && retries_sent < u16::from(max_retries);
                            // The expiry and, for a retry with nothing to resend,
                            // the cancel are one step under the TSM lock.
                            let expiration = {
                                let mut tsm = self.tsm.lock();
                                let expiration =
                                    tsm.expire_request_timer(tsm_mac, invoke_id, owner, !retry);
                                if matches!(expiration, RequestTimerExpiration::Retry)
                                    && retry_apdu.is_none()
                                {
                                    tsm.cancel_transaction_for_owner(tsm_mac, invoke_id, owner);
                                }
                                expiration
                            };
                            match expiration {
                                RequestTimerExpiration::Retry => {
                                    // Authorization and segment admission use the TSM lock,
                                    // but transport I/O must not. Once authorized, this retry
                                    // may finish even if segment zero is admitted meanwhile.
                                    let Some(retry_apdu) = retry_apdu else {
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
                                        let expiration = self
                                            .tsm
                                            .lock()
                                            .expire_request_timer(tsm_mac, invoke_id, owner, true);
                                        match expiration {
                                            RequestTimerExpiration::SegmentedResponse { generation } => {
                                                progress = TransactionProgress::SegmentedResponse { generation };
                                                continue;
                                            }
                                            RequestTimerExpiration::NoTransaction => {
                                                return (&mut response_rx)
                                                    .await
                                                    .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                                            }
                                            RequestTimerExpiration::TimedOut => {
                                                self.enqueue_transaction_cleanup(
                                                    tsm_mac,
                                                    invoke_id,
                                                    owner,
                                                    false,
                                                    None,
                                                );
                                                return Err(error);
                                            }
                                            RequestTimerExpiration::Retry
                                            | RequestTimerExpiration::Withdrawn => unreachable!(
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
                                // DeviceCommunicationControl restricts the
                                // node: no retry goes out (#2617).
                                RequestTimerExpiration::Withdrawn => {
                                    self.enqueue_transaction_cleanup(
                                        tsm_mac, invoke_id, owner, false, None,
                                    );
                                    return Err(Error::InitiationDisabled { sent: true });
                                }
                                RequestTimerExpiration::TimedOut => {
                                    self.enqueue_transaction_cleanup(
                                        tsm_mac,
                                        invoke_id,
                                        owner,
                                        false,
                                        None,
                                    );
                                    return Err(Error::abort(bacnet_types::enums::AbortReason::TSM_TIMEOUT));
                                }
                                RequestTimerExpiration::NoTransaction => {
                                    return (&mut response_rx)
                                        .await
                                        .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                                }
                            }
                        }
                        TransactionProgress::SegmentedResponse { generation } => {
                            let expiration = self
                                .tsm
                                .lock()
                                .expire_segment_timer(tsm_mac, invoke_id, owner, generation);
                            match expiration {
                                SegmentTimerExpiration::Activity { generation } => {
                                    progress = TransactionProgress::SegmentedResponse { generation };
                                }
                                SegmentTimerExpiration::AwaitingResponse => {
                                    progress = TransactionProgress::AwaitingResponse;
                                }
                                SegmentTimerExpiration::TimedOut => {
                                    #[cfg(test)]
                                    self.segmented_cleanup.pause_if_enabled().await;
                                    self.enqueue_transaction_cleanup(
                                        tsm_mac,
                                        invoke_id,
                                        owner,
                                        false,
                                        None,
                                    );
                                    return Err(Error::abort(bacnet_types::enums::AbortReason::TSM_TIMEOUT));
                                }
                                SegmentTimerExpiration::NoTransaction => {
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
                    .send_apdu(apdu, mac, true, NetworkPriority::Normal)
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
                        NetworkPriority::Normal,
                    )
                    .await
            }
        }
    }

    pub(super) fn target_transport_max_apdu_length(&self, target: ConfirmedTarget<'_>) -> u16 {
        self.network
            .transport()
            .egress_apdu_limit()
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

    /// Before the first Who-Is, of any scope, ask this network's number
    /// (#2572): the I-Ams about to fill the discovery table may include a
    /// device a router relays back from this network. Only one frame goes
    /// ahead of the Who-Is, and a question that fails to send holds nothing
    /// back. The seat is checked again after it: the node may have stopped,
    /// or restricted initiation, while the question went out (#2617).
    pub(super) async fn ask_before(
        &self,
        service_choice: UnconfirmedServiceChoice,
    ) -> Result<(), Error> {
        if service_choice == UnconfirmedServiceChoice::WHO_IS {
            self.device_table.asking.first_need(&self.network).await;
            self.seat_open()?;
        }
        Ok(())
    }

    /// Send an unconfirmed request to the station `to`, once, with no answer
    /// waited for: success means it was sent.
    ///
    /// `to` takes the forms [`Self::confirmed_request`] takes and is routed
    /// the same way. A MAC on this link is sent to as given; an address on
    /// this client's own network, once it has learned the number, is the
    /// station at that MAC on this link, which must then be one station, not
    /// a broadcast or group address.
    ///
    /// # Errors
    ///
    /// As [`Self::confirmed_request`] before sending, and the transport's
    /// error.
    pub async fn unconfirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        to.unconfirmed(self, service_choice, service_data).await
    }

    /// An unconfirmed request to `destination_mac` on this link: one
    /// station, so an empty MAC, which names none, is refused as an address
    /// on network 0 with no MAC is (#2588).
    pub(super) async fn unconfirmed_local(
        &self,
        destination_mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        Self::station_of(0, destination_mac)?;
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .send_apdu(&buf, destination_mac, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request on the local network.
    pub async fn broadcast_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .broadcast_apdu(&buf, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request globally (DNET=0xFFFF).
    pub async fn broadcast_global_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .broadcast_global_apdu(&buf, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request to a specific remote network.
    ///
    /// Once the client has learned its own network's number, a
    /// `dest_network` naming it is this network, and the request goes as a
    /// local broadcast with no DNET (#1358).
    pub async fn broadcast_network_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
        dest_network: u16,
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        // Network 0 and 65535 are refused below, before anything goes: a
        // refused Who-Is asks nothing.
        if matches!(dest_network, 1..=0xFFFE) {
            self.ask_before(service_choice).await?;
        }
        if self.network.local_network_number().get() == Some(dest_network) {
            return self
                .network
                .broadcast_apdu(&buf, false, NetworkPriority::Normal)
                .await;
        }
        self.network
            .broadcast_to_network(&buf, dest_network, false, NetworkPriority::Normal)
            .await
    }
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod transmittable_length_tests;
