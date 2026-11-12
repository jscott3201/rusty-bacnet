//! Fair selection among the session's single-consumer inputs and worker
//! joins, and the handling of terminal answers and ingress policy outcomes.
use super::*;
use std::future::{Future, poll_fn};
use std::task::Poll;

pub(super) enum DispatchEvent {
    Worker(Result<(), tokio::task::JoinError>),
    Inbound(ReceivedApdu),
    Terminal(ReceivedApdu),
    Policy(PolicyOutcome),
    Closed,
}

pub(super) async fn next_event(parts: &mut DispatchParts, next: &mut usize) -> DispatchEvent {
    let notifications = &parts.notifications;
    let joined = async {
        match notifications {
            Some(owner) => owner.join_next().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(joined);
    let mut join_closed = false;
    poll_fn(|cx| {
        let mut receivers_closed = 0;
        // A continuously ready source gets one turn before the other ready
        // sources. This bounds both terminal delay and completed-task retention
        // to four selections, even when producers replenish on every turn.
        for offset in 0..4 {
            let index = (*next + offset) % 4;
            let event = match index {
                0 if !join_closed => match joined.as_mut().poll(cx) {
                    Poll::Ready(Some(result)) => Some(DispatchEvent::Worker(result)),
                    Poll::Ready(None) => {
                        join_closed = true;
                        None
                    }
                    Poll::Pending => None,
                },
                1 => match parts.inbound.poll_recv(cx) {
                    Poll::Ready(Some(received)) => Some(DispatchEvent::Inbound(received)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                2 => match parts.terminal.poll_recv(cx) {
                    Poll::Ready(Some(received)) => Some(DispatchEvent::Terminal(received)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                3 => match parts.policy.poll_recv(cx) {
                    Poll::Ready(Some(outcome)) => Some(DispatchEvent::Policy(outcome)),
                    Poll::Ready(None) => {
                        receivers_closed += 1;
                        None
                    }
                    Poll::Pending => None,
                },
                _ => None,
            };
            if let Some(event) = event {
                *next = (index + 1) % 4;
                return Poll::Ready(event);
            }
        }
        if receivers_closed == 3 {
            Poll::Ready(DispatchEvent::Closed)
        } else {
            Poll::Pending
        }
    })
    .await
}

pub(super) async fn handle_terminal(parts: &mut DispatchParts, received: ReceivedApdu) {
    let Some(apdu) = decode_terminal(&received) else {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    };
    // Inbound server transactions never allocate here: requests are not
    // terminal traffic. A Confirmed/Unconfirmed request in this queue is a
    // classifier violation → policy-owned, no coordinator interaction.
    if matches!(
        apdu,
        Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
    ) {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    }
    // Exactly one shared-coordinator admit per terminal APDU (exact-once
    // terminal claim). Equal numeric IDs are unambiguous: the admitted
    // lease owner selects requester vs notification.
    let outcome = admit_once(
        &parts.coordinator,
        &received,
        parts.local_network.get(),
        &apdu,
    );
    let admission = match outcome {
        Ok(AdmissionOutcome::Admitted(admission)) => {
            if let Some(routers) = &parts.source_routers {
                routers.observe(&received);
            }
            admission
        }
        Ok(_) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
        Err(CoordinatorError::StatePoisoned) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
    };
    if is_requester_lease(&admission) {
        let Some(requester) = parts.requester.as_ref() else {
            // Lease was admitted but has no consumer: release the exact
            // lease so exhaustion → release → reuse still holds.
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        // `complete_pre_admitted` delivers via the exact token without
        // re-admitting; a `false` means the TSM no longer owns it (lost
        // consumer / stale), the coordinator lease was already completed
        // inside the TSM path on success.
        let _ = requester
            .complete_pre_admitted(admission, apdu, received)
            .await;
    } else {
        // An answer to one of the COV reporting's confirmed notifications
        // ends it there (#2490).
        if let Some(cov) = &parts.cov
            && cov.answer(&admission, &received.apdu)
        {
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
        let Some(notifications) = parts.notifications.as_ref() else {
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        let _ = notifications.complete_pre_admitted(admission, &apdu);
        // Notification replies carry no reply sender; drop one if present.
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
    }
}

pub(super) async fn handle_ingress_policy(shared: &Arc<SessionShared>, outcome: PolicyOutcome) {
    // Policy-outcome ownership: the session counts every classifier outcome
    // (malformed/unsupported/route-full/route-closed) instead of dropping it.
    let mut counters = shared.counters.lock().await;
    counters.ingress_policy += 1;
    let _ = outcome.reason;
    // Policy outcomes own the full envelope including a possible one-use
    // reply sender; releasing here preserves single-consumer + one-use
    // ownership without sending bytes.
    if let Some(reply_tx) = outcome.received.reply_tx {
        drop(reply_tx);
    }
    let _ = PolicyReason::MalformedApdu;
}

#[cfg(test)]
#[path = "session_dispatch_tests.rs"]
mod tests;
