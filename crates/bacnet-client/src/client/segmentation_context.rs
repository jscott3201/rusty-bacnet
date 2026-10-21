//! Parameter bundles for the segmentation entry points (#902).

use super::*;

/// Where an inbound segmented ComplexAck came from.
#[derive(Clone, Copy)]
pub(super) struct InboundSegmentSource<'a> {
    /// Immediate MAC the segment arrived from.
    pub(super) mac: &'a [u8],
    /// The peer's SNET/SADR when the segment arrived through a router.
    pub(super) network: &'a Option<NpduAddress>,
    /// Transport provenance snapshot for this segment.
    pub(super) provenance: TransportProvenance,
    /// The client's throttle on warnings about responses it refuses.
    pub(super) warning: &'a WarnThrottle,
}

/// Limits and routing state for one segmented confirmed request.
#[derive(Clone, Copy)]
pub(super) struct SegmentedRequestLimits<'a> {
    /// Largest APDU the peer accepts.
    pub(super) remote_max_apdu: u16,
    /// Most segments the peer accepts, when it advertised a bound.
    pub(super) remote_max_segments: Option<u32>,
    /// Forwarded NPCI length reserved for a routed request.
    pub(super) routed_forwarded_npci_len: Option<u16>,
    /// Routed-path lease to mark terminal when the request finishes.
    pub(super) routed_path_lease: Option<&'a RoutedPathLease>,
}

/// Where and for whom a reassembly abort is sent and completed.
#[derive(Clone, Copy)]
pub(super) struct ReassemblyAbortTarget<'a> {
    /// TSM key MAC of the transaction being aborted.
    pub(super) tsm_mac: &'a MacAddr,
    /// Owner of the transaction being aborted.
    pub(super) owner: &'a TransactionOwner,
    /// Immediate MAC the Abort is sent to.
    pub(super) reply_mac: &'a MacAddr,
    /// The peer's SNET/SADR when the segments arrived through a router.
    pub(super) reply_network: &'a Option<NpduAddress>,
}

impl SegmentedReceiveState {
    /// Abort target that replies along the route this session was opened on.
    pub(super) fn abort_target<'a>(&'a self, tsm_mac: &'a MacAddr) -> ReassemblyAbortTarget<'a> {
        ReassemblyAbortTarget {
            tsm_mac,
            owner: &self.owner,
            reply_mac: &self.reply_mac,
            reply_network: &self.reply_network,
        }
    }
}

/// The transaction a confirmed-response wait is bound to.
#[derive(Clone, Copy)]
pub(super) struct ConfirmedWait<'a> {
    /// Where the request was sent.
    pub(super) target: ConfirmedTarget<'a>,
    /// TSM key MAC of the transaction.
    pub(super) tsm_mac: &'a MacAddr,
    /// Invoke ID of the transaction.
    pub(super) invoke_id: u8,
    /// Owner of the transaction.
    pub(super) owner: &'a TransactionOwner,
    /// Encoded request to retransmit on AWAIT_CONFIRMATION timeout, if retryable.
    pub(super) retry_apdu: Option<&'a [u8]>,
}
