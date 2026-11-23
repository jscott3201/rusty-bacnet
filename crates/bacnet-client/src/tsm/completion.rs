use super::{
    CompletionOutcome, PendingRelease, PendingTransaction, TransactionOwner, TransactionPhase, Tsm,
    TsmResponse,
};
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::RemoteError;
use std::collections::hash_map::Entry;

impl Tsm {
    /// Deliver a response to a pending transaction.
    ///
    /// `observed_service_choice` is the service the response claims to answer,
    /// or `None` for PDUs that carry no service choice at all — Reject and
    /// Abort (Clauses 20.1.8 and 20.1.9), which can only be correlated by
    /// invoke ID.
    ///
    /// A mismatch leaves the transaction pending and the invoke ID allocated.
    /// Completing it would hand the caller a payload belonging to a different
    /// service and free the ID for reuse while the real response is still in
    /// flight.
    pub fn complete_transaction(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        observed_service_choice: Option<ConfirmedServiceChoice>,
        response: TsmResponse,
    ) -> CompletionOutcome {
        self.complete_transaction_inner(
            source_mac,
            invoke_id,
            None,
            observed_service_choice,
            response,
            PendingRelease::Complete,
        )
    }

    pub(crate) fn complete_transaction_for_owner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        observed_service_choice: Option<ConfirmedServiceChoice>,
        response: TsmResponse,
    ) -> CompletionOutcome {
        self.complete_transaction_inner(
            source_mac,
            invoke_id,
            Some(owner),
            observed_service_choice,
            response,
            PendingRelease::Release,
        )
    }

    /// End the transaction with a router's refusal: `response` is a
    /// [`TsmResponse::Failed`] with the reject's error.
    pub(crate) fn complete_network_refusal_for_owner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        response: TsmResponse,
    ) -> CompletionOutcome {
        self.complete_transaction_inner(
            source_mac,
            invoke_id,
            Some(owner),
            None,
            response,
            PendingRelease::Complete,
        )
    }

    pub(crate) fn complete_admitted_transaction_for_owner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        observed_service_choice: Option<ConfirmedServiceChoice>,
        response: TsmResponse,
    ) -> CompletionOutcome {
        self.complete_transaction_inner(
            source_mac,
            invoke_id,
            Some(owner),
            observed_service_choice,
            response,
            PendingRelease::Complete,
        )
    }

    pub(super) fn complete_transaction_inner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: Option<&TransactionOwner>,
        observed_service_choice: Option<ConfirmedServiceChoice>,
        mut response: TsmResponse,
        release: PendingRelease,
    ) -> CompletionOutcome {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Entry::Occupied(entry) = self.pending.entry(key) else {
            return CompletionOutcome::NoTransaction;
        };
        if owner.is_some_and(|owner| !entry.get().owner.same_as(owner)) {
            return CompletionOutcome::NoTransaction;
        }
        // Which kind of answer this is, read once for both checks below.
        let slot = super::ack_form::slot(&response);
        let requires_sent_all_segments = super::ack_form::is_ack(slot)
            || matches!(
                &response,
                TsmResponse::Remote {
                    error: RemoteError::Error { .. },
                    ..
                }
            );
        if requires_sent_all_segments
            && matches!(
                entry.get().phase,
                TransactionPhase::SegmentedRequest {
                    sent_all_segments: false
                }
            )
        {
            let pending = entry.remove();
            self.abort_pending_invalid_state(source_mac, invoke_id, pending);
            return CompletionOutcome::Delivered;
        }
        let expected = entry.get().expected_service_choice;
        if let Some(observed) = observed_service_choice
            && observed != expected
        {
            return CompletionOutcome::ServiceChoiceMismatch { expected, observed };
        }
        // The device's acknowledgement of this transaction in the other
        // form than its service takes still ends the transaction, as any
        // final acknowledgement does, but fails the request (#2721).
        if super::ack_form::wrong(expected, slot) {
            let accept = self.config.accept_complex_ack_for_simple_services;
            super::ack_form::refuse(expected, slot, accept, &mut response);
        }
        let pending = entry.remove();
        let responder = pending.responder;
        self.release_pending_lease(source_mac, invoke_id, pending.lease, release);
        let _ = responder.send(response);
        CompletionOutcome::Delivered
    }

    fn abort_pending_invalid_state(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        pending: PendingTransaction,
    ) {
        let responder = pending.responder;
        self.release_pending_lease(
            source_mac,
            invoke_id,
            pending.lease,
            PendingRelease::Release,
        );
        let _ = responder.send(TsmResponse::remote(RemoteError::Abort {
            reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
        }));
    }
}
