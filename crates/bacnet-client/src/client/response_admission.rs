use super::*;
use crate::tsm::{CompletionOutcome, CoordinatedCompletion, CoordinatedTerminalPhase};

fn log_mismatch(outcome: &CompletionOutcome, invoke_id: u8, pdu: &str, warning: &WarnThrottle) {
    if let CompletionOutcome::ServiceChoiceMismatch { expected, observed } = outcome {
        warn_throttled!(
            warning,
            invoke_id,
            pdu,
            expected = expected.to_raw(),
            observed = observed.to_raw(),
            "Ignoring acknowledgment labelled for a different confirmed service"
        );
    }
}

/// Log an acknowledgment labelled for another service, through `warning`,
/// the client's response throttle (#1950).
pub(super) fn log_coordinated_mismatch(
    outcome: &CoordinatedCompletion,
    invoke_id: u8,
    pdu: &str,
    warning: &WarnThrottle,
) {
    match outcome {
        CoordinatedCompletion::Completed(outcome) => {
            log_mismatch(outcome, invoke_id, pdu, warning);
        }
        CoordinatedCompletion::ServiceChoiceMismatch { expected, observed } => {
            warn_throttled!(
                warning,
                invoke_id,
                pdu,
                expected = expected.to_raw(),
                observed = observed.to_raw(),
                "Ignoring acknowledgment labelled for a different confirmed service"
            );
        }
        CoordinatedCompletion::Rejected => {}
    }
}

/// Find any reassembly key for this (mac, invoke), preferring the exact
/// provenance match. Terminal UnexpectedPDU handling stays provenance-agnostic
/// (fail-closed): any in-progress session for the transaction aborts,
/// regardless of which trust context it opened under.
fn find_reassembly_key(
    seg_state: &HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
) -> Option<SegKey> {
    if seg_state.contains_key(key) {
        return Some(key.clone());
    }
    seg_state
        .keys()
        .find(|existing| existing.0 == key.0 && existing.1 == key.1)
        .cloned()
}

/// Fail-closed conflict for segmented ComplexAck reassembly (RB-07).
/// Returns the conflicting key when the same (mac, invoke) exists under a
/// different provenance snapshot; the caller aborts that session rather than
/// merging. Snapshots compare by value; the assertion expires with its
/// session.
pub(super) fn find_provenance_conflict(
    seg_state: &HashMap<SegKey, SegmentedReceiveState>,
    tsm_mac: &MacAddr,
    invoke_id: u8,
    provenance: &TransportProvenance,
) -> Option<SegKey> {
    seg_state
        .keys()
        .find(|existing| {
            existing.0 == *tsm_mac && existing.1 == invoke_id && existing.2 != *provenance
        })
        .cloned()
}

pub(super) async fn take_current_reassembly(
    tsm: &Arc<Mutex<Tsm>>,
    seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
) -> Option<SegmentedReceiveState> {
    let found = find_reassembly_key(seg_state, key)?;
    let state = seg_state.remove(&found)?;
    if tsm
        .lock()
        .await
        .owner_is_current(&found.0, found.1, &state.owner)
    {
        Some(state)
    } else {
        debug!(
            invoke_id = found.1,
            "Reclaimed stale segmented receive state"
        );
        None
    }
}

pub(super) async fn current_reassembly_owner(
    tsm: &Arc<Mutex<Tsm>>,
    seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
) -> Option<TransactionOwner> {
    let found = find_reassembly_key(seg_state, key)?;
    let owner = seg_state.get(&found)?.owner.clone();
    if tsm.lock().await.owner_is_current(&found.0, found.1, &owner) {
        Some(owner)
    } else {
        seg_state.remove(&found);
        debug!(
            invoke_id = found.1,
            "Reclaimed stale segmented receive state"
        );
        None
    }
}

pub(super) async fn admit_terminal_during_reassembly(
    tsm: &Arc<Mutex<Tsm>>,
    seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
    peer: &CanonicalPeer,
    apdu: &Apdu,
) {
    let Some(owner) = current_reassembly_owner(tsm, seg_state, key).await else {
        return;
    };
    let _ = tsm
        .lock()
        .await
        .try_claim_coordinated_terminal(&key.0, key.1, &owner, peer, apdu);
}

pub(super) enum TerminalDispatchOutcome {
    Completion(CoordinatedCompletion),
    PrematureSegmentedRequestAborted,
}

/// A terminal PDU received for a transaction, and how to complete it.
pub(super) struct TerminalResponse<'a> {
    /// TSM key MAC of the transaction.
    pub(super) tsm_mac: &'a MacAddr,
    /// Invoke ID of the transaction.
    pub(super) invoke_id: u8,
    /// Canonical identity of the responding peer.
    pub(super) peer: &'a CanonicalPeer,
    /// The received APDU, as presented to the outbound coordinator.
    pub(super) apdu: &'a Apdu,
    /// Response delivered to the waiting caller.
    pub(super) response: TsmResponse,
    /// Whether the completion is gated on the transaction's current phase.
    pub(super) phase_gate: bool,
    /// Owner the completion must match, when the caller already knows it.
    pub(super) owner: Option<TransactionOwner>,
}

pub(super) async fn complete_terminal_response(
    tsm: &Arc<Mutex<Tsm>>,
    terminal: TerminalResponse<'_>,
) -> TerminalDispatchOutcome {
    let TerminalResponse {
        tsm_mac,
        invoke_id,
        peer,
        apdu,
        response,
        phase_gate,
        owner,
    } = terminal;
    let mut expected_owner = owner;
    let mut response = Some(response);
    loop {
        let mut tsm = tsm.lock().await;
        let admission = if phase_gate {
            tsm.coordinated_terminal_phase(tsm_mac, invoke_id, expected_owner.as_ref())
        } else {
            let owner = match expected_owner.clone() {
                Some(owner) if tsm.owner_is_current(tsm_mac, invoke_id, &owner) => Some(owner),
                Some(_) => None,
                None => tsm.current_owner(tsm_mac, invoke_id),
            };
            match owner {
                Some(owner) => CoordinatedTerminalPhase::Active(owner),
                None => CoordinatedTerminalPhase::NoTransaction,
            }
        };
        match admission {
            CoordinatedTerminalPhase::FinalSegmentSendPolling { owner, issue } => {
                drop(tsm);
                issue.wait_until_polled().await;
                expected_owner = Some(owner);
            }
            CoordinatedTerminalPhase::Active(owner) => {
                let Some(response) = response.take() else {
                    return TerminalDispatchOutcome::Completion(CoordinatedCompletion::Rejected);
                };
                let completion = tsm.complete_coordinated_terminal_response(
                    tsm_mac,
                    invoke_id,
                    Some(&owner),
                    peer,
                    apdu,
                    response,
                );
                return TerminalDispatchOutcome::Completion(completion);
            }
            CoordinatedTerminalPhase::PrematureSegmentedRequest(owner) => {
                let _ = tsm.try_claim_coordinated_terminal(tsm_mac, invoke_id, &owner, peer, apdu);
                tsm.complete_transaction_for_owner(
                    tsm_mac,
                    invoke_id,
                    &owner,
                    None,
                    TsmResponse::remote(RemoteError::Abort {
                        reason: bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE,
                    }),
                );
                return TerminalDispatchOutcome::PrematureSegmentedRequestAborted;
            }
            CoordinatedTerminalPhase::NoTransaction => {
                return TerminalDispatchOutcome::Completion(CoordinatedCompletion::Rejected);
            }
        }
    }
}
