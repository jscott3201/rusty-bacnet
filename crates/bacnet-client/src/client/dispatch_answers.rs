//! The answer arms of the client's dispatcher that a plain request's round
//! trip doesn't take (#2558): an Error, a Reject, an Abort and a SegmentACK,
//! each in a function of its own that `dispatch_apdu` calls from its match.
//! Each body is the match arm it was, moved as it stood: the same statements
//! in the same order, taking the same locks. `dispatch_apdu` awaits each one
//! before it returns, so the dispatch loop still handles one frame's effects
//! before it takes the next, except a requester's Abort, which
//! `dispatch_abort` hands to the reassembly task to run out of line
//! (`hand_off_abort`, #2646). Log lines here carry the target
//! `bacnet_client::client::dispatch::answers`, which a filter on
//! `bacnet_client::client::dispatch` still covers.

use super::super::response_admission::current_reassembly_owner;
use super::*;
use crate::tsm::{CompletionOutcome, CoordinatedCompletion};
use bacnet_encoding::apdu::ErrorPdu;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// An Error PDU: it aborts the reassembly under way for its transaction,
    /// or completes the transaction with the error.
    pub(super) async fn dispatch_error(
        context: &DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        answer: AnswerSource<'_>,
        err: ErrorPdu,
    ) {
        let DispatchContext { tsm, network, .. } = *context;
        let AnswerSource {
            source_mac,
            source_network,
            provenance,
            tsm_mac,
            canonical_peer,
        } = answer;
        // Mid-reassembly, an Error PDU is Clause 5.4.4.4
        // UnexpectedPDU_Received — "BACnet-Error-PDU" is in its list —
        // so the local surface is an ABORT.indication with
        // INVALID_APDU_IN_THIS_STATE, not the error content (#367).
        // Checked before the TSM lock below: `abort_reassembly` takes
        // that lock itself, and tokio's Mutex is not reentrant.
        let key = (tsm_mac, err.invoke_id, provenance);
        let tsm_mac = &key.0;
        let coordinator_apdu = Apdu::Error(err.clone());
        admit_terminal_during_reassembly(tsm, seg_state, &key, &canonical_peer, &coordinator_apdu)
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

    /// A Reject PDU: it aborts the reassembly under way for its transaction,
    /// or completes the transaction with the rejection.
    pub(super) async fn dispatch_reject(
        context: &DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        answer: AnswerSource<'_>,
        rej: RejectPdu,
    ) {
        let DispatchContext { tsm, network, .. } = *context;
        let AnswerSource {
            provenance,
            tsm_mac,
            canonical_peer,
            ..
        } = answer;
        // Same Clause 5.4.4.4 UnexpectedPDU_Received diversion as the
        // Error arm above ("BACnet-Reject-PDU" is in the same list),
        // with the same lock-ordering constraint (#367).
        let key = (tsm_mac, rej.invoke_id, provenance);
        let tsm_mac = &key.0;
        let coordinator_apdu = Apdu::Reject(rej.clone());
        admit_terminal_during_reassembly(tsm, seg_state, &key, &canonical_peer, &coordinator_apdu)
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

    /// An Abort PDU: a requester's goes to the reassembly task; a server's
    /// completes its transaction and ends the reassembly under way for it.
    pub(super) async fn dispatch_abort(
        context: &DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        answer: AnswerSource<'_>,
        abt: AbortPdu,
    ) {
        let DispatchContext { tsm, .. } = *context;
        let AnswerSource {
            source_mac,
            source_network,
            provenance,
            tsm_mac,
            canonical_peer,
        } = answer;
        // Clause 5.4.4.3 AbortPDU_Received fires only for an Abort
        // with `server` = TRUE. An Abort this client
        // itself sent — 5.4.4.4 emits them with 'server' = FALSE —
        // must not, echoed back or spoofed, complete the very
        // transaction it was trying to abort.
        if !abt.sent_by_server {
            // A requester's: it may end a request the client is
            // reassembling (#2646), out of line.
            Self::hand_off_abort(tsm, source_mac, source_network, abt.invoke_id, &provenance);
            return;
        }
        // Clause 5.4.4.4 AbortPDU_Received delivers ABORT.indication
        // locally and returns to IDLE —
        // ending the reassembly session is the IDLE half, and no
        // reply PDU is prescribed (#367). This removal must stay
        // below the server=FALSE guard above, or an echoed copy of
        // this client's own Abort would tear down a healthy
        // reassembly. Behaviorally redundant with the per-segment
        // pending gate in `admit_segment` — a later
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

    /// A SegmentACK: a server's goes to the segmented request it
    /// acknowledges; one that finds a reassembly, or nothing outstanding,
    /// draws an Abort.
    pub(super) async fn dispatch_segment_ack(
        context: &DispatchContext<'_, T>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        answer: AnswerSource<'_>,
        sa: SegmentAckPdu,
    ) {
        let DispatchContext {
            tsm,
            network,
            seg_ack_senders,
            ..
        } = *context;
        let AnswerSource {
            source_mac,
            source_network,
            provenance,
            tsm_mac,
            canonical_peer,
        } = answer;
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
