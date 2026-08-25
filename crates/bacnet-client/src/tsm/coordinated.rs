use std::collections::hash_map::Entry;
use std::fmt;
use std::sync::Arc;

use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint_core::coordinator::{
    AdmissionKind, AdmissionOutcome, CanonicalPeer, LeaseMetadata, LeaseToken,
    OutboundTransactionCoordinator, ReserveError, TerminalPolicy,
};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::MacAddr;

use super::{
    CompletionOutcome, FinalSegmentIssue, PendingTransaction, SegmentAckPhase,
    SegmentedResponseAdmission, TransactionOwner, TransactionPhase, TransactionRegistration, Tsm,
    TsmConfig, TsmResponse,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingLease {
    Legacy,
    Coordinated(LeaseToken),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingRelease {
    Complete,
    Cancel,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoordinatedRegistrationError {
    CoordinatorUnavailable,
    Reserve(ReserveError),
    DuplicateInvokeId,
}

impl fmt::Display for CoordinatedRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoordinatorUnavailable => {
                formatter.write_str("coordinator is unavailable for client transaction")
            }
            Self::Reserve(ReserveError::Exhausted) => {
                formatter.write_str("all invoke IDs exhausted for destination")
            }
            Self::Reserve(error) => write!(formatter, "invoke ID coordinator failed: {error}"),
            Self::DuplicateInvokeId => {
                formatter.write_str("coordinator reserved an invoke ID already pending in the TSM")
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CoordinatedCompletion {
    Completed(CompletionOutcome),
    ServiceChoiceMismatch {
        expected: ConfirmedServiceChoice,
        observed: ConfirmedServiceChoice,
    },
    Rejected,
}

#[derive(Debug)]
pub(crate) enum CoordinatedTerminalPhase {
    Active(TransactionOwner),
    FinalSegmentSendPolling {
        owner: TransactionOwner,
        issue: FinalSegmentIssue,
    },
    PrematureSegmentedRequest(TransactionOwner),
    NoTransaction,
}

impl Tsm {
    pub(crate) fn current_owner(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
    ) -> Option<TransactionOwner> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        self.pending.get(&key).map(|pending| pending.owner.clone())
    }

    pub(crate) fn new_coordinated(
        config: TsmConfig,
        coordinator: Arc<OutboundTransactionCoordinator>,
    ) -> Self {
        Self {
            config,
            allocators: Default::default(),
            pending: Default::default(),
            coordinator: Some(coordinator),
        }
    }

    pub(crate) fn register_coordinated_transaction_with_progress(
        &mut self,
        destination_mac: MacAddr,
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        segmented: bool,
    ) -> Result<(u8, TransactionRegistration), CoordinatedRegistrationError> {
        let coordinator = self
            .coordinator
            .as_ref()
            .ok_or(CoordinatedRegistrationError::CoordinatorUnavailable)?;
        let metadata = if segmented {
            LeaseMetadata::segmented_requester(peer, service_choice, TerminalPolicy::EitherAck)
        } else {
            LeaseMetadata::requester(peer, service_choice, TerminalPolicy::EitherAck)
        };
        let token = coordinator
            .reserve(metadata)
            .map_err(CoordinatedRegistrationError::Reserve)?;
        let invoke_id = token.invoke_id();
        let phase = if segmented {
            TransactionPhase::SegmentedRequest {
                sent_all_segments: false,
            }
        } else {
            TransactionPhase::AwaitingResponse
        };
        let (pending, registration) =
            Self::new_pending_transaction(service_choice, phase, PendingLease::Coordinated(token));

        match self.pending.entry((destination_mac, invoke_id)) {
            Entry::Vacant(entry) => {
                entry.insert(pending);
                Ok((invoke_id, registration))
            }
            Entry::Occupied(_) => {
                let _ = coordinator.cancel(token);
                Err(CoordinatedRegistrationError::DuplicateInvokeId)
            }
        }
    }

    pub(crate) fn complete_coordinated_terminal_response(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: Option<&TransactionOwner>,
        peer: &CanonicalPeer,
        apdu: &Apdu,
        response: TsmResponse,
    ) -> CoordinatedCompletion {
        let Some(current_owner) = self.current_owner(source_mac, invoke_id) else {
            return CoordinatedCompletion::Rejected;
        };
        if owner.is_some_and(|owner| !current_owner.same_as(owner)) {
            return CoordinatedCompletion::Rejected;
        }
        match self.claim_coordinated_terminal(source_mac, invoke_id, &current_owner, peer, apdu) {
            Ok(()) => {}
            Err(AdmissionOutcome::ServiceMismatch { expected, observed }) => {
                return CoordinatedCompletion::ServiceChoiceMismatch { expected, observed };
            }
            Err(_) => return CoordinatedCompletion::Rejected,
        }

        let observed_service_choice = match apdu {
            Apdu::SimpleAck(pdu) => Some(pdu.service_choice),
            Apdu::ComplexAck(pdu) if !pdu.segmented => Some(pdu.service_choice),
            Apdu::Error(_) | Apdu::Reject(_) | Apdu::Abort(_) => None,
            _ => return CoordinatedCompletion::Rejected,
        };
        CoordinatedCompletion::Completed(self.complete_transaction_inner(
            source_mac,
            invoke_id,
            Some(&current_owner),
            observed_service_choice,
            response,
            PendingRelease::Complete,
        ))
    }

    pub(crate) fn coordinated_terminal_phase(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: Option<&TransactionOwner>,
    ) -> CoordinatedTerminalPhase {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return CoordinatedTerminalPhase::NoTransaction;
        };
        if owner.is_some_and(|owner| !pending.owner.same_as(owner)) {
            return CoordinatedTerminalPhase::NoTransaction;
        }
        let current_owner = pending.owner.clone();
        if matches!(
            pending.phase,
            TransactionPhase::SegmentedRequest {
                sent_all_segments: false
            }
        ) {
            if let Some(issue) = pending
                .final_segment_issue
                .as_ref()
                .filter(|issue| issue.is_polling())
            {
                return CoordinatedTerminalPhase::FinalSegmentSendPolling {
                    owner: current_owner,
                    issue: issue.clone(),
                };
            }
            return CoordinatedTerminalPhase::PrematureSegmentedRequest(current_owner);
        }
        CoordinatedTerminalPhase::Active(current_owner)
    }

    pub(crate) fn try_claim_coordinated_terminal(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> bool {
        self.claim_coordinated_terminal(source_mac, invoke_id, owner, peer, apdu)
            .is_ok()
    }

    fn claim_coordinated_terminal(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> Result<(), AdmissionOutcome> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self
            .pending
            .get(&key)
            .filter(|pending| pending.owner.same_as(owner))
        else {
            return Err(AdmissionOutcome::UnknownInvokeId);
        };
        let PendingLease::Coordinated(token) = pending.lease else {
            return Ok(());
        };
        let Some(coordinator) = self.coordinator.as_ref() else {
            return Err(AdmissionOutcome::UnknownInvokeId);
        };
        match coordinator.admit(peer, apdu) {
            Ok(AdmissionOutcome::Admitted(admission))
                if admission.kind() == AdmissionKind::Terminal && admission.token() == token =>
            {
                Ok(())
            }
            Ok(outcome) => Err(outcome),
            Err(_) => Err(AdmissionOutcome::UnknownInvokeId),
        }
    }

    pub(crate) fn coordinated_segment_ack_phase(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> SegmentAckPhase {
        let phase = self.segment_ack_phase(source_mac, invoke_id);
        let SegmentAckPhase::SegmentedRequest(owner) = phase else {
            return phase;
        };
        if self.coordinated_nonterminal_admitted(source_mac, invoke_id, &owner, peer, apdu) {
            SegmentAckPhase::SegmentedRequest(owner)
        } else {
            SegmentAckPhase::CoordinatorRejected
        }
    }

    pub(crate) fn coordinated_admit_segmented_complex_ack(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        sequence_number: u8,
        segmented_response_accepted: bool,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> SegmentedResponseAdmission {
        let admission = self.admit_segmented_complex_ack(
            source_mac,
            invoke_id,
            sequence_number,
            segmented_response_accepted,
        );
        self.coordinate_segmented_response_admission(source_mac, invoke_id, peer, apdu, admission)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn coordinated_admit_segmented_complex_ack_for_owner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        sequence_number: u8,
        segmented_response_accepted: bool,
        owner: &TransactionOwner,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> SegmentedResponseAdmission {
        let admission = self.admit_segmented_complex_ack_for_owner(
            source_mac,
            invoke_id,
            sequence_number,
            segmented_response_accepted,
            owner,
        );
        self.coordinate_segmented_response_admission(source_mac, invoke_id, peer, apdu, admission)
    }

    fn coordinate_segmented_response_admission(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        peer: &CanonicalPeer,
        apdu: &Apdu,
        admission: SegmentedResponseAdmission,
    ) -> SegmentedResponseAdmission {
        let SegmentedResponseAdmission::Active(owner) = admission else {
            return admission;
        };
        if self.coordinated_nonterminal_admitted(source_mac, invoke_id, &owner, peer, apdu) {
            SegmentedResponseAdmission::Active(owner)
        } else {
            SegmentedResponseAdmission::CoordinatorRejected
        }
    }

    fn coordinated_nonterminal_admitted(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> bool {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self
            .pending
            .get(&key)
            .filter(|pending| pending.owner.same_as(owner))
        else {
            return false;
        };
        let PendingLease::Coordinated(token) = pending.lease else {
            return true;
        };
        let Some(coordinator) = self.coordinator.as_ref() else {
            return false;
        };
        matches!(
            coordinator.admit(peer, apdu),
            Ok(AdmissionOutcome::Admitted(admission))
                if admission.kind() == AdmissionKind::NonTerminal && admission.token() == token
        )
    }

    pub(super) fn release_pending(
        &mut self,
        mac: &[u8],
        invoke_id: u8,
        pending: PendingTransaction,
        release: PendingRelease,
    ) {
        self.release_pending_lease(mac, invoke_id, pending.lease, release);
    }

    pub(super) fn release_pending_lease(
        &mut self,
        mac: &[u8],
        invoke_id: u8,
        lease: PendingLease,
        release: PendingRelease,
    ) {
        match lease {
            PendingLease::Legacy => self.release_invoke_id(mac, invoke_id),
            PendingLease::Coordinated(token) => {
                let Some(coordinator) = self.coordinator.as_ref() else {
                    return;
                };
                let _ = match release {
                    PendingRelease::Complete => coordinator.complete(token),
                    PendingRelease::Cancel => coordinator.cancel(token),
                    PendingRelease::Release => coordinator.release(token),
                };
            }
        }
    }

    pub(crate) fn cancel_all_transactions(&mut self) {
        let coordinated: Vec<_> = self
            .pending
            .drain()
            .filter_map(|(_, pending)| match pending.lease {
                PendingLease::Legacy => None,
                PendingLease::Coordinated(token) => Some(token),
            })
            .collect();
        self.allocators.clear();
        if let Some(coordinator) = self.coordinator.as_ref() {
            for token in coordinated {
                let _ = coordinator.cancel(token);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn coordinated_active_count(&self) -> usize {
        self.coordinator
            .as_ref()
            .and_then(|coordinator| coordinator.active_count().ok())
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn coordinated_token(&self, mac: &[u8], invoke_id: u8) -> Option<LeaseToken> {
        let pending = self.pending.get(&(MacAddr::from_slice(mac), invoke_id))?;
        match pending.lease {
            PendingLease::Legacy => None,
            PendingLease::Coordinated(token) => Some(token),
        }
    }
}

impl Drop for Tsm {
    fn drop(&mut self) {
        let Some(coordinator) = self.coordinator.as_ref() else {
            return;
        };
        for (_, pending) in self.pending.drain() {
            if let PendingLease::Coordinated(token) = pending.lease {
                let _ = coordinator.release(token);
            }
        }
    }
}
