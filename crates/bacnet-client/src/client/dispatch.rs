use super::dispatch_context::{DispatchContext, InboundApdu};
use super::response_admission::{
    TerminalDispatchOutcome, TerminalResponse, admit_terminal_during_reassembly,
    complete_terminal_response, current_reassembly_owner, log_coordinated_mismatch,
    take_current_reassembly,
};
use super::segmentation_context::InboundSegmentSource;
use super::*;
use crate::tsm::{CompletionOutcome, CoordinatedCompletion};
use bacnet_network::layer::ReceivedApdu;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Dispatch a received APDU to the appropriate handler.
    pub(super) async fn dispatch_apdu(
        context: DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        inbound: InboundApdu<'_>,
        apdu: Apdu,
        limits: ResponseLimits,
    ) {
        let DispatchContext {
            tsm,
            device_table,
            network,
            cov_tx,
            event_tx,
            confirmed_cov_ack_policy,
            device_tx,
            device_collision_tx,
            i_have_tx,
            seg_ack_senders,
            decode_warning,
            response_warning,
        } = context;
        let InboundApdu {
            source_mac,
            source_network,
            provenance,
            response_route,
            is_group,
            reply_tx,
        } = inbound;
        // Each arm moves `tsm_mac` into its reassembly key and borrows it
        // back: cloning a `MacAddr` copies it a byte at a time (#1978).
        let TransactionPeer {
            tsm_mac,
            canonical: canonical_peer,
        } = TransactionPeer::of_answer_in(
            tsm,
            source_mac,
            source_network.as_ref(),
            network.local_network_number().get(),
            &apdu,
        )
        .await;
        match apdu {
            Apdu::SimpleAck(ack) => {
                // Mid-reassembly, a SimpleACK is in Clause 5.4.4.4
                // UnexpectedPDU_Received's list, exactly like the Error and
                // Reject arms below — the segmented response under
                // reassembly IS this transaction's acknowledgment, so a
                // second one is not answering it. Checked before the TSM
                // lock: `abort_reassembly` takes that lock itself (#367).
                let key = (tsm_mac, ack.invoke_id, provenance);
                let tsm_mac = &key.0;
                let coordinator_apdu = Apdu::SimpleAck(ack.clone());
                admit_terminal_during_reassembly(
                    tsm,
                    seg_state,
                    &key,
                    &canonical_peer,
                    &coordinator_apdu,
                )
                .await;
                if let Some(state) = take_current_reassembly(tsm, seg_state, &key).await {
                    debug!(
                        invoke_id = ack.invoke_id,
                        "Aborting reassembly on a SimpleAck"
                    );
                    Self::abort_reassembly(
                        tsm,
                        network,
                        state.abort_target(tsm_mac),
                        ack.invoke_id,
                        bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    )
                    .await;
                    return;
                }
                debug!(invoke_id = ack.invoke_id, "Received SimpleAck");
                match complete_terminal_response(
                    tsm,
                    TerminalResponse {
                        tsm_mac,
                        invoke_id: ack.invoke_id,
                        peer: &canonical_peer,
                        apdu: &coordinator_apdu,
                        response: TsmResponse::SimpleAck,
                        phase_gate: true,
                        owner: None,
                    },
                )
                .await
                {
                    TerminalDispatchOutcome::Completion(outcome) => {
                        log_coordinated_mismatch(
                            &outcome,
                            ack.invoke_id,
                            "SimpleAck",
                            response_warning,
                        );
                    }
                    TerminalDispatchOutcome::PrematureSegmentedRequestAborted => {
                        Self::send_client_abort(
                            network,
                            source_mac,
                            source_network,
                            ack.invoke_id,
                            bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                        )
                        .await;
                    }
                }
            }
            Apdu::ComplexAck(ack) => {
                if ack.segmented {
                    Self::handle_segmented_complex_ack(
                        tsm,
                        network,
                        seg_state,
                        InboundSegmentSource {
                            mac: source_mac,
                            network: source_network,
                            provenance,
                            warning: response_warning,
                        },
                        ack,
                        limits,
                    )
                    .await;
                } else {
                    // An unsegmented BACnet-ComplexACK-PDU is in the same
                    // Clause 5.4.4.4
                    // UnexpectedPDU_Received list: the transaction's answer
                    // is the segmented ComplexACK already under reassembly.
                    let key = (tsm_mac, ack.invoke_id, provenance);
                    let tsm_mac = &key.0;
                    let coordinator_apdu = Apdu::ComplexAck(ack.clone());
                    admit_terminal_during_reassembly(
                        tsm,
                        seg_state,
                        &key,
                        &canonical_peer,
                        &coordinator_apdu,
                    )
                    .await;
                    if let Some(state) = take_current_reassembly(tsm, seg_state, &key).await {
                        debug!(
                            invoke_id = ack.invoke_id,
                            "Aborting reassembly on an unsegmented ComplexAck"
                        );
                        Self::abort_reassembly(
                            tsm,
                            network,
                            state.abort_target(tsm_mac),
                            ack.invoke_id,
                            bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                        )
                        .await;
                        return;
                    }
                    debug!(invoke_id = ack.invoke_id, "Received ComplexAck");
                    match complete_terminal_response(
                        tsm,
                        TerminalResponse {
                            tsm_mac,
                            invoke_id: ack.invoke_id,
                            peer: &canonical_peer,
                            apdu: &coordinator_apdu,
                            response: TsmResponse::ComplexAck {
                                service_data: ack.service_ack,
                            },
                            phase_gate: true,
                            owner: None,
                        },
                    )
                    .await
                    {
                        TerminalDispatchOutcome::Completion(outcome) => {
                            log_coordinated_mismatch(
                                &outcome,
                                ack.invoke_id,
                                "ComplexAck",
                                response_warning,
                            );
                        }
                        TerminalDispatchOutcome::PrematureSegmentedRequestAborted => {
                            Self::send_client_abort(
                                network,
                                source_mac,
                                source_network,
                                ack.invoke_id,
                                bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                            )
                            .await;
                        }
                    }
                }
            }
            Apdu::Error(err) => {
                // Mid-reassembly, an Error PDU is Clause 5.4.4.4
                // UnexpectedPDU_Received — "BACnet-Error-PDU" is in its list —
                // so the local surface is an ABORT.indication with
                // INVALID_APDU_IN_THIS_STATE, not the error content (#367).
                // Checked before the TSM lock below: `abort_reassembly` takes
                // that lock itself, and tokio's Mutex is not reentrant.
                let key = (tsm_mac, err.invoke_id, provenance);
                let tsm_mac = &key.0;
                let coordinator_apdu = Apdu::Error(err.clone());
                admit_terminal_during_reassembly(
                    tsm,
                    seg_state,
                    &key,
                    &canonical_peer,
                    &coordinator_apdu,
                )
                .await;
                if let Some(state) = take_current_reassembly(tsm, seg_state, &key).await {
                    debug!(
                        invoke_id = err.invoke_id,
                        "Aborting reassembly on an Error PDU"
                    );
                    Self::abort_reassembly(
                        tsm,
                        network,
                        state.abort_target(tsm_mac),
                        err.invoke_id,
                        bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    )
                    .await;
                    return;
                }
                debug!(invoke_id = err.invoke_id, "Received Error PDU");
                // Correlated by invoke ID alone. Unlike 20.1.4.2 and 20.1.5.6,
                // Clause 20.1.7.2 imposes no correspondence to the requested
                // service: error-choice identifies the selected BACnet-Error
                // alternative by tag, and Clause 21 opens that production with
                // `other [127] Error`. Nothing in the Standard forbids a peer
                // from answering through that tag, so an exact-match gate here
                // would reject conformant responses.
                match complete_terminal_response(
                    tsm,
                    TerminalResponse {
                        tsm_mac,
                        invoke_id: err.invoke_id,
                        peer: &canonical_peer,
                        apdu: &coordinator_apdu,
                        response: TsmResponse::from_error_pdu(&err),
                        phase_gate: true,
                        owner: None,
                    },
                )
                .await
                {
                    TerminalDispatchOutcome::Completion(_) => {}
                    TerminalDispatchOutcome::PrematureSegmentedRequestAborted => {
                        Self::send_client_abort(
                            network,
                            source_mac,
                            source_network,
                            err.invoke_id,
                            bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                        )
                        .await;
                    }
                }
            }
            Apdu::Reject(rej) => {
                // Same Clause 5.4.4.4 UnexpectedPDU_Received diversion as the
                // Error arm above ("BACnet-Reject-PDU" is in the same list),
                // with the same lock-ordering constraint (#367).
                let key = (tsm_mac, rej.invoke_id, provenance);
                let tsm_mac = &key.0;
                let coordinator_apdu = Apdu::Reject(rej.clone());
                admit_terminal_during_reassembly(
                    tsm,
                    seg_state,
                    &key,
                    &canonical_peer,
                    &coordinator_apdu,
                )
                .await;
                if let Some(state) = take_current_reassembly(tsm, seg_state, &key).await {
                    debug!(
                        invoke_id = rej.invoke_id,
                        "Aborting reassembly on a Reject PDU"
                    );
                    Self::abort_reassembly(
                        tsm,
                        network,
                        state.abort_target(tsm_mac),
                        rej.invoke_id,
                        bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    )
                    .await;
                    return;
                }
                debug!(invoke_id = rej.invoke_id, "Received Reject PDU");
                // Carries no service choice (Clause 20.1.8); invoke ID is the
                // only correlation the Standard provides.
                let _ = complete_terminal_response(
                    tsm,
                    TerminalResponse {
                        tsm_mac,
                        invoke_id: rej.invoke_id,
                        peer: &canonical_peer,
                        apdu: &coordinator_apdu,
                        response: TsmResponse::remote(rej.remote_error()),
                        phase_gate: false,
                        owner: None,
                    },
                )
                .await;
            }
            Apdu::Abort(abt) => {
                // Clause 5.4.4.3 AbortPDU_Received fires only for an Abort
                // with `server` = TRUE. An Abort this client
                // itself sent — 5.4.4.4 emits them with 'server' = FALSE —
                // must not, echoed back or spoofed, complete the very
                // transaction it was trying to abort.
                if !abt.sent_by_server {
                    debug!(
                        invoke_id = abt.invoke_id,
                        "Ignoring Abort PDU with server=FALSE"
                    );
                    return;
                }
                // Clause 5.4.4.4 AbortPDU_Received delivers ABORT.indication
                // locally and returns to IDLE —
                // ending the reassembly session is the IDLE half, and no
                // reply PDU is prescribed (#367). This removal must stay
                // below the server=FALSE guard above, or an echoed copy of
                // this client's own Abort would tear down a healthy
                // reassembly. Behaviorally redundant with the per-segment
                // pending gate in `handle_segmented_complex_ack` — a later
                // segment would find no transaction and clear the session
                // anyway — so no test can pin this line alone; it is kept so
                // the slot frees at the Abort, not at the peer's next move.
                let key = (tsm_mac, abt.invoke_id, provenance);
                let tsm_mac = &key.0;
                let reassembly_owner = current_reassembly_owner(tsm, seg_state, &key).await;
                debug!(invoke_id = abt.invoke_id, "Received Abort PDU");
                // Carries no service choice (Clause 20.1.9).
                let response = TsmResponse::remote(abt.remote_error());
                let coordinator_apdu = Apdu::Abort(abt.clone());
                let outcome = complete_terminal_response(
                    tsm,
                    TerminalResponse {
                        tsm_mac,
                        invoke_id: abt.invoke_id,
                        peer: &canonical_peer,
                        apdu: &coordinator_apdu,
                        response,
                        phase_gate: false,
                        owner: reassembly_owner.clone(),
                    },
                )
                .await;
                if matches!(
                    outcome,
                    TerminalDispatchOutcome::Completion(CoordinatedCompletion::Completed(
                        CompletionOutcome::Delivered
                    ))
                ) && let Some(owner) = reassembly_owner
                    && seg_state
                        .get(&key)
                        .is_some_and(|state| state.owner.same_as(&owner))
                {
                    seg_state.remove(&key);
                }
            }
            Apdu::ConfirmedRequest(req) => {
                let reply = InboundReply::new(response_route, reply_tx);
                if is_group {
                    debug!(
                        invoke_id = req.invoke_id,
                        service = req.service_choice.to_raw(),
                        "Ignoring group-addressed ConfirmedRequest"
                    );
                    return;
                }
                if req.segmented {
                    Self::send_server_abort(
                        network,
                        source_mac,
                        source_network,
                        reply,
                        req.invoke_id,
                        bacnet_types::enums::AbortReason::SEGMENTATION_NOT_SUPPORTED,
                    )
                    .await;
                    return;
                }
                if is_confirmed_cov_notification(req.service_choice) {
                    Self::receive_confirmed_cov_notification(
                        network,
                        cov_tx,
                        confirmed_cov_ack_policy,
                        source_mac,
                        source_network,
                        reply,
                        req,
                        decode_warning,
                    )
                    .await;
                } else if req.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
                {
                    Self::receive_confirmed_event_notification(
                        network,
                        event_tx,
                        source_mac,
                        source_network,
                        reply,
                        req,
                        decode_warning,
                    )
                    .await;
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Rejecting unsupported ConfirmedRequest (client mode)"
                    );
                    Self::send_confirmed_request_reject(
                        network,
                        source_mac,
                        source_network,
                        reply,
                        req.invoke_id,
                        RejectReason::UNRECOGNIZED_SERVICE,
                    )
                    .await;
                }
            }
            Apdu::UnconfirmedRequest(req) => {
                if req.service_choice == UnconfirmedServiceChoice::I_AM {
                    match bacnet_services::who_is::IAmRequest::decode(&req.service_request) {
                        Ok(i_am) => {
                            debug!(
                                device = i_am.object_identifier.instance_number(),
                                vendor = i_am.vendor_id,
                                "Received IAm"
                            );
                            let routed_while_unknown = device_events::heard_i_am(
                                &device_table.table,
                                device_events::IAmSource {
                                    mac: source_mac,
                                    network: source_network,
                                    this_network: network.local_network_number(),
                                },
                                &i_am,
                                (device_tx, device_collision_tx),
                            );
                            // A routed row stored while the number is
                            // unknown may be a local device's twin: ask
                            // (#2572), through the Number owner.
                            if routed_while_unknown {
                                device_table.asking.want();
                            }
                        }
                        Err(e) => {
                            warn_throttled!(decode_warning, error = %e, "Failed to decode IAm");
                        }
                    }
                } else if req.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                    || req.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
                {
                    cov_notifications::receive_unconfirmed_cov_notification(
                        cov_tx,
                        source_mac,
                        source_network,
                        &req.service_request,
                        req.service_choice,
                        decode_warning,
                    );
                } else if req.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
                {
                    event_notifications::receive_unconfirmed_event_notification(
                        event_tx,
                        source_mac,
                        source_network,
                        &req.service_request,
                        decode_warning,
                    );
                } else if req.service_choice == UnconfirmedServiceChoice::I_HAVE {
                    i_have::deliver(
                        i_have_tx,
                        source_mac,
                        source_network,
                        &req.service_request,
                        decode_warning,
                    );
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Ignoring unconfirmed service in client dispatch"
                    );
                }
            }
            Apdu::SegmentAck(sa) => {
                // Clause 5.4.4.2 gates DuplicateACK_Received, NewACK_Received
                // and FinalACK_Received on the 'server' parameter being TRUE.
                // A SegmentACK this client sent while receiving a segmented
                // response carries server=FALSE and must not be fed back into
                // its own segmented send.
                //
                // This check must stay ahead of the Abort below. A client
                // emits SegmentACKs with 'server' = FALSE, so if the order
                // were reversed, two of these clients exchanging a segmented
                // ConfirmedCOVNotification would answer each other's
                // SegmentACKs with Aborts indefinitely. Dropping them here is
                // what keeps the Abort reachable only by a *server's* PDU.
                if !sa.sent_by_server {
                    debug!(
                        invoke_id = sa.invoke_id,
                        "Ignoring SegmentAck with server=FALSE"
                    );
                    return;
                }
                let invoke_id = sa.invoke_id;
                let key = (tsm_mac, invoke_id, provenance);
                let tsm_mac = &key.0;

                // SEGMENTED_CONF (5.4.4.4) `UnexpectedPDU_Received` lists
                // a server-side BACnet-SegmentACK-PDU among the inappropriate
                // PDUs. The response has three parts: send a client-side
                // BACnet-Abort-PDU (`server` = FALSE), raise ABORT.indication to
                // the local user carrying `abort-reason` INVALID_APDU_IN_THIS_STATE
                // (again `server` = FALSE), and go back to IDLE.
                //
                // Receive state is checked before the outgoing phase because
                // SEGMENTED_CONF gives this PDU the opposite disposition.
                if let Some(state) = take_current_reassembly(tsm, seg_state, &key).await {
                    debug!(invoke_id, "Aborting reassembly on an unexpected SegmentAck");
                    // The stored identity, not this PDU's source. A key match
                    // already forces the inbound SNET/SADR to equal the
                    // stored pair (the key is derived from it), but the
                    // immediate MAC can diverge — a second router to the same
                    // peer — and reading both from the state keeps the
                    // (reply_mac, reply_network) pair consistent.
                    Self::abort_reassembly(
                        tsm,
                        network,
                        state.abort_target(tsm_mac),
                        invoke_id,
                        bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    )
                    .await;
                    return;
                }

                // Hold the TSM lock through the route lookup and try_send.
                // Terminal completion uses this same lock, so a raw sender
                // entry cannot outlive the authoritative phase for delivery.
                // Both locks are blocking ones, released before any await.
                let coordinator_apdu = Apdu::SegmentAck(sa.clone());
                let outstanding = {
                    let tsm_guard = tsm.lock();
                    match tsm_guard.coordinated_segment_ack_phase(
                        tsm_mac,
                        invoke_id,
                        &canonical_peer,
                        &coordinator_apdu,
                    ) {
                        SegmentAckPhase::SegmentedRequest(owner) => {
                            // Lock order is TSM then SegmentACK routes. Setup and
                            // cleanup never hold both locks. Compat mode: SegmentACK
                            // delivery stays provenance-agnostic (RB-09 later).
                            let send_key = (MacAddr::from_slice(tsm_mac), invoke_id);
                            let senders = seg_ack_senders.lock();
                            if let Some(route) = senders
                                .get(&send_key)
                                .filter(|route| route.owner.same_as(&owner))
                            {
                                let _ = route.sender.try_send(sa);
                            }
                            return;
                        }
                        SegmentAckPhase::Outstanding => true,
                        SegmentAckPhase::CoordinatorRejected => return,
                        SegmentAckPhase::Idle => false,
                    }
                };
                if outstanding {
                    debug!(
                        invoke_id,
                        "Discarding duplicate SegmentAck for an outstanding transaction"
                    );
                    return;
                }

                // IDLE (5.4.4.1) `UnexpectedSegmentInfoReceived`: nothing is
                // outstanding, so a server SegmentACK earns a client Abort.
                debug!(invoke_id, "Aborting SegmentAck received while idle");
                Self::send_client_abort(
                    network,
                    source_mac,
                    source_network,
                    invoke_id,
                    bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                )
                .await;
            }
        }
    }

    /// Send an ordinarily addressed reply or outgoing-client transaction control.
    ///
    /// A PDU from a routed peer carries the peer's SNET/SADR; the reply must
    /// carry that pair as DNET/DADR, unicast to the router that delivered the
    /// original, or the router treats it as locally addressed and never
    /// forwards it. Inbound direct confirmed-request replies instead use
    /// the checked original-socket path in `inbound_replies`.
    pub(super) async fn send_reply_apdu(
        network: &Arc<NetworkLayer<T>>,
        buf: &[u8],
        reply_mac: &[u8],
        reply_network: &Option<NpduAddress>,
    ) -> Result<(), Error> {
        match reply_network {
            Some(address) if !address.mac_address.is_empty() => {
                network
                    .send_apdu_routed(
                        buf,
                        address.network,
                        &address.mac_address,
                        reply_mac,
                        false,
                        NetworkPriority::Normal,
                    )
                    .await
            }
            _ => {
                network
                    .send_apdu(buf, reply_mac, false, NetworkPriority::Normal)
                    .await
            }
        }
    }
}

/// Whether `service` is a confirmed COV notification, of one object or of
/// several (Clauses 13.7 and 13.17).
fn is_confirmed_cov_notification(service: ConfirmedServiceChoice) -> bool {
    service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        || service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
}

/// Whether a confirmed COV notification is the several-object form
/// (Clause 13.17). Named here, with the other services the client receives.
pub(super) fn is_confirmed_cov_multiple(service: ConfirmedServiceChoice) -> bool {
    service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
}

/// Whether an unconfirmed COV notification is the several-object form
/// (Clause 13.18).
pub(super) fn is_unconfirmed_cov_multiple(service: UnconfirmedServiceChoice) -> bool {
    service == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
}

/// Whether a seated client beside a server role answers `request` itself:
/// only a confirmed COV notification, of either form, which is about its own
/// subscription (#2617, #2654). Every other Confirmed-Request is the server
/// role's.
pub(super) fn seated_client_answers(request: &ConfirmedRequestPdu) -> bool {
    is_confirmed_cov_notification(request.service_choice)
}

/// A confirmed request that reached a seated client beside a server role and
/// is the server's to answer. A ConfirmedEventNotification is published to
/// the client's subscribers; the client sends nothing either way.
pub(super) fn heard_for_the_server(
    event_tx: &broadcast::Sender<ReceivedEventNotification>,
    received: &ReceivedApdu,
    request: &ConfirmedRequestPdu,
    decode_warning: &WarnThrottle,
) {
    if request.service_choice != ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
        || request.segmented
        || received.is_group
    {
        debug!(
            service = request.service_choice.to_raw(),
            "Leaving a ConfirmedRequest to the node's server role"
        );
        return;
    }
    match ReceivedEventNotification::decode(
        &request.service_request,
        &received.source_mac,
        &received.source_network,
        EventNotificationDelivery::Confirmed,
    ) {
        Ok(notification) => {
            let _ = event_tx.send(notification);
        }
        Err(error) => {
            warn_throttled!(
                decode_warning,
                %error,
                "Failed to decode ConfirmedEventNotification"
            );
        }
    }
}
