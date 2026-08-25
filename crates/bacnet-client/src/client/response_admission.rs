use super::*;
use crate::tsm::{CompletionOutcome, CoordinatedCompletion, CoordinatedTerminalPhase};

fn log_mismatch(outcome: &CompletionOutcome, invoke_id: u8, pdu: &str) {
    if let CompletionOutcome::ServiceChoiceMismatch { expected, observed } = outcome {
        warn!(
            invoke_id,
            pdu,
            expected = expected.to_raw(),
            observed = observed.to_raw(),
            "Ignoring acknowledgment labelled for a different confirmed service"
        );
    }
}

pub(super) fn log_coordinated_mismatch(outcome: &CoordinatedCompletion, invoke_id: u8, pdu: &str) {
    match outcome {
        CoordinatedCompletion::Completed(outcome) => {
            log_mismatch(outcome, invoke_id, pdu);
        }
        CoordinatedCompletion::ServiceChoiceMismatch { expected, observed } => {
            warn!(
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

pub(super) async fn take_current_reassembly(
    tsm: &Arc<Mutex<Tsm>>,
    seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
) -> Option<SegmentedReceiveState> {
    let state = seg_state.remove(key)?;
    if tsm
        .lock()
        .await
        .owner_is_current(&key.0, key.1, &state.owner)
    {
        Some(state)
    } else {
        debug!(invoke_id = key.1, "Reclaimed stale segmented receive state");
        None
    }
}

pub(super) async fn current_reassembly_owner(
    tsm: &Arc<Mutex<Tsm>>,
    seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
    key: &SegKey,
) -> Option<TransactionOwner> {
    let owner = seg_state.get(key)?.owner.clone();
    if tsm.lock().await.owner_is_current(&key.0, key.1, &owner) {
        Some(owner)
    } else {
        seg_state.remove(key);
        debug!(invoke_id = key.1, "Reclaimed stale segmented receive state");
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

#[allow(clippy::too_many_arguments)]
pub(super) async fn complete_terminal_response(
    tsm: &Arc<Mutex<Tsm>>,
    tsm_mac: &MacAddr,
    invoke_id: u8,
    peer: &CanonicalPeer,
    apdu: &Apdu,
    response: TsmResponse,
    phase_gate: bool,
    owner: Option<TransactionOwner>,
) -> TerminalDispatchOutcome {
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
                    TsmResponse::Abort {
                        reason: bacnet_types::enums::AbortReason::INVALID_APDU_IN_THIS_STATE
                            .to_raw(),
                    },
                );
                return TerminalDispatchOutcome::PrematureSegmentedRequestAborted;
            }
            CoordinatedTerminalPhase::NoTransaction => {
                return TerminalDispatchOutcome::Completion(CoordinatedCompletion::Rejected);
            }
        }
    }
}
