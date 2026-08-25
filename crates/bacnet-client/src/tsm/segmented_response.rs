use bacnet_types::enums::AbortReason;
use bacnet_types::MacAddr;

use super::{FinalSegmentIssue, TransactionOwner, TransactionPhase, Tsm};

#[derive(Debug)]
pub(crate) enum SegmentedResponseAdmission {
    Active(TransactionOwner),
    FinalSegmentSendPolling {
        owner: TransactionOwner,
        issue: FinalSegmentIssue,
    },
    InitialResponseAborted {
        wire_reason: AbortReason,
    },
    CoordinatorRejected,
    NoTransaction,
}

impl Tsm {
    /// Gate an initial segmented ComplexACK before receive state is allocated.
    pub(crate) fn admit_segmented_complex_ack(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        sequence_number: u8,
        segmented_response_accepted: bool,
    ) -> SegmentedResponseAdmission {
        self.admit_segmented_complex_ack_inner(
            source_mac,
            invoke_id,
            sequence_number,
            segmented_response_accepted,
            None,
        )
    }

    pub(crate) fn admit_segmented_complex_ack_for_owner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        sequence_number: u8,
        segmented_response_accepted: bool,
        owner: &TransactionOwner,
    ) -> SegmentedResponseAdmission {
        self.admit_segmented_complex_ack_inner(
            source_mac,
            invoke_id,
            sequence_number,
            segmented_response_accepted,
            Some(owner),
        )
    }

    fn admit_segmented_complex_ack_inner(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        sequence_number: u8,
        segmented_response_accepted: bool,
        expected_owner: Option<&TransactionOwner>,
    ) -> SegmentedResponseAdmission {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return SegmentedResponseAdmission::NoTransaction;
        };
        if expected_owner.is_some_and(|owner| !pending.owner.same_as(owner)) {
            return SegmentedResponseAdmission::NoTransaction;
        }

        let owner = pending.owner.clone();
        let phase = pending.phase;
        let polling_issue = pending
            .final_segment_issue
            .as_ref()
            .filter(|issue| issue.is_polling())
            .cloned();

        if phase != TransactionPhase::SegmentedResponse {
            // Sequence validity wins when both initial-response conditions fail.
            let wire_reason = if sequence_number != 0 {
                Some(AbortReason::INVALID_APDU_IN_THIS_STATE)
            } else if !segmented_response_accepted {
                Some(AbortReason::SEGMENTATION_NOT_SUPPORTED)
            } else {
                None
            };
            if let Some(wire_reason) = wire_reason {
                self.abort_invalid_apdu_in_current_state(source_mac, invoke_id, &owner);
                return SegmentedResponseAdmission::InitialResponseAborted { wire_reason };
            }
        }

        if let TransactionPhase::SegmentedRequest {
            sent_all_segments: false,
        } = phase
        {
            if let Some(issue) = polling_issue {
                return SegmentedResponseAdmission::FinalSegmentSendPolling { owner, issue };
            }
            self.abort_invalid_apdu_in_current_state(source_mac, invoke_id, &owner);
            return SegmentedResponseAdmission::InitialResponseAborted {
                wire_reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
            };
        }

        SegmentedResponseAdmission::Active(owner)
    }
}
