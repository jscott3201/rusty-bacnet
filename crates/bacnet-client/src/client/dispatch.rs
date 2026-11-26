use super::dispatch_context::{AnswerSource, DispatchContext, InboundApdu};
use super::response_admission::{
    TerminalDispatchOutcome, TerminalResponse, admit_terminal_during_reassembly,
    complete_terminal_response, log_coordinated_mismatch, take_current_reassembly,
};
use super::segmentation_context::InboundSegmentSource;
use super::*;
use bacnet_network::layer::ReceivedApdu;

#[path = "dispatch_answers.rs"]
mod answers;
#[cfg(test)]
#[path = "dispatch_order_tests.rs"]
mod order_tests;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Dispatch a received APDU to the appropriate handler.
    ///
    /// The two answers a plain request's round trip takes, a SimpleACK and a
    /// ComplexACK, are handled here; every other PDU type has a function of
    /// its own (#2558): the other answers' in `dispatch_answers.rs`, the
    /// requests' below. The two stay inline because a call into a future of
    /// their own cost each ReadProperty, WriteProperty and
    /// ReadPropertyMultiple round trip 28 to 44 more instructions in the
    /// counting harness (`scripts/perf/perf_counts.py` against 38bb02c02),
    /// 14 of them the child future's drop glue.
    ///
    /// The dispatch loop awaits this before it takes the next frame, so each
    /// frame's effects come before the next frame's, except what a frame
    /// hands to the reassembly task (`hand_off_segment`, `hand_off_abort`,
    /// #2646), which runs out of line. Two orders the stack
    /// relies on (#2603) live in what the request functions call: an I-Am's
    /// device events go out under the discovery table's lock
    /// (`device_events::heard_i_am`), and a confirmed COV notification's
    /// event is published before its acknowledgment is sent
    /// (`receive_confirmed_cov_notification`).
    pub(super) async fn dispatch_apdu(
        context: &DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        inbound: InboundApdu<'_>,
        apdu: Apdu,
        limits: ResponseLimits,
    ) {
        let DispatchContext {
            tsm,
            network,
            response_warning,
            ..
        } = *context;
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
        // A routed device's answer counts only from the router its request
        // went to: its SNET and SADR are its sender's word (#2831). A client
        // on a router's application port names no router (the router's own
        // table routes its requests), so its answers are not checked here.
        if let CanonicalPeer::Routed { network: snet, .. } = &canonical_peer
            && let Some(invoke_id) = answers_own_request(&apdu)
            && network.transport().attached_networks().is_empty()
            && !context
                .routed_path_limits
                .answer_through(source_mac, *snet, &tsm_mac, invoke_id)
        {
            debug!(
                invoke_id,
                "Dropping a routed device's answer from another station than its router"
            );
            return;
        }
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
                let answer = AnswerSource {
                    source_mac,
                    source_network,
                    provenance,
                    tsm_mac,
                    canonical_peer,
                };
                Self::dispatch_error(context, seg_state, answer, err).await;
            }
            Apdu::Reject(rej) => {
                let answer = AnswerSource {
                    source_mac,
                    source_network,
                    provenance,
                    tsm_mac,
                    canonical_peer,
                };
                Self::dispatch_reject(context, seg_state, answer, rej).await;
            }
            Apdu::Abort(abt) => {
                let answer = AnswerSource {
                    source_mac,
                    source_network,
                    provenance,
                    tsm_mac,
                    canonical_peer,
                };
                Self::dispatch_abort(context, seg_state, answer, abt).await;
            }
            Apdu::ConfirmedRequest(req) => {
                let reply = InboundReply::new(response_route, reply_tx);
                Self::dispatch_confirmed_request(
                    context,
                    source_mac,
                    source_network,
                    reply,
                    is_group,
                    req,
                    limits,
                )
                .await;
            }
            Apdu::UnconfirmedRequest(req) => {
                Self::dispatch_unconfirmed_request(context, source_mac, source_network, req);
            }
            Apdu::SegmentAck(sa) => {
                let answer = AnswerSource {
                    source_mac,
                    source_network,
                    provenance,
                    tsm_mac,
                    canonical_peer,
                };
                Self::dispatch_segment_ack(context, seg_state, answer, sa).await;
            }
        }
    }

    /// A confirmed request, which the client executes only when it is a
    /// notification: a segment goes to the reassembly task (or draws an
    /// Abort), and any other service is rejected.
    async fn dispatch_confirmed_request(
        context: &DispatchContext<'_, T>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        is_group: bool,
        req: ConfirmedRequestPdu,
        limits: ResponseLimits,
    ) {
        // Any frame a routed row's own router carries from the device keeps
        // the row young (#2823).
        if source_network.is_some() {
            device_events::heard_routed(&context.device_table.table, source_mac, source_network);
        }
        let DispatchContext {
            tsm,
            network,
            cov_tx,
            event_tx,
            confirmed_cov_ack_policy,
            decode_warning,
            ..
        } = *context;
        if is_group {
            debug!(
                invoke_id = req.invoke_id,
                service = req.service_choice.to_raw(),
                "Ignoring group-addressed ConfirmedRequest"
            );
            return;
        }
        if req.segmented {
            // A client that receives segments hands the segment to
            // its reassembly task (#2646), out of line.
            if limits.segmented_response_accepted {
                Self::hand_off_segment(
                    request_reassembly::HandOff {
                        tsm,
                        network,
                        cov_tx,
                        event_tx,
                        policy: confirmed_cov_ack_policy,
                        limits,
                    },
                    source_mac,
                    source_network,
                    reply,
                    req,
                );
                return;
            }
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
        } else if req.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION {
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

    /// An unconfirmed request: an I-Am goes into the discovery table, a
    /// notification or an I-Have to its subscribers, and the rest is
    /// ignored. Nothing here waits.
    fn dispatch_unconfirmed_request(
        context: &DispatchContext<'_, T>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        req: bacnet_encoding::apdu::UnconfirmedRequest,
    ) {
        let DispatchContext {
            device_table,
            network,
            cov_tx,
            event_tx,
            device_tx,
            device_collision_tx,
            i_have_tx,
            decode_warning,
            ..
        } = *context;
        // Any frame a routed row's own router carries from the device keeps
        // the row young (#2823); an I-Am is taken into the table below.
        if source_network.is_some() && req.service_choice != UnconfirmedServiceChoice::I_AM {
            device_events::heard_routed(&device_table.table, source_mac, source_network);
        }
        if req.service_choice == UnconfirmedServiceChoice::I_AM {
            match bacnet_services::who_is::IAmRequest::decode(&req.service_request) {
                Ok(i_am) => {
                    debug!(
                        device = i_am.object_identifier.instance_number(),
                        vendor = i_am.vendor_id,
                        "Received IAm"
                    );
                    let wants = device_events::heard_i_am(
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
                    if wants.number {
                        device_table.asking.want();
                    }
                    // A routed row named through another station:
                    // the purge task checks the path (#2823).
                    if wants.route_check {
                        device_table.route_cue.notify_one();
                    }
                }
                Err(e) => {
                    warn_throttled!(decode_warning, error = %e, "Failed to decode IAm");
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
            || req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
        {
            cov_notifications::receive_unconfirmed_cov_notification(
                cov_tx,
                source_mac,
                source_network,
                &req.service_request,
                req.service_choice,
                decode_warning,
            );
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION {
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

/// The invoke ID of an answer to one of this client's own requests: an
/// acknowledgment, an Error, a Reject, or a server's Abort or Segment-ACK.
/// A client's Abort or Segment-ACK belongs to a request this client is
/// answering, and is `None`, as is every request.
fn answers_own_request(apdu: &Apdu) -> Option<u8> {
    match apdu {
        Apdu::SimpleAck(ack) => Some(ack.invoke_id),
        Apdu::ComplexAck(ack) => Some(ack.invoke_id),
        Apdu::Error(error) => Some(error.invoke_id),
        Apdu::Reject(reject) => Some(reject.invoke_id),
        Apdu::Abort(abort) if abort.sent_by_server => Some(abort.invoke_id),
        Apdu::SegmentAck(ack) if ack.sent_by_server => Some(ack.invoke_id),
        _ => None,
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
/// the client's subscribers: the copy ingress makes of one that comes whole,
/// or the whole one the server hands on once it has reassembled its segments
/// (#2743). A segment alone is never published. The client sends nothing
/// either way.
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
