//! Parameter bundles for the segmentation entry points (#902).

use super::*;
use bacnet_encoding::apdu::advertised_max_segments;

/// Size of the sequence-number space, and so the hard reassembly ceiling.
///
/// A local storage bound, not a protocol one. Clause 20.1.5.4 makes the
/// sequence number modulo 256, so a longer response is entirely representable —
/// this client simply keys its segment store by that `u8` and cannot tell
/// segment 257 from segment 1. Clause 5.4.4.4 names this exact situation
/// (`NewSegmentReceived_NoSpace`, a local inability to retain a segment)
/// and prescribes the Abort the reassembly sends.
///
/// Exactly 256 segments reassemble correctly and must keep working; 257 is the
/// first that would corrupt the payload.
const SEQUENCE_NUMBER_SPACE: usize = 256;

impl ResponseLimits {
    /// The receive-side limits `config` puts on the wire.
    pub(super) fn from_config(config: &ClientConfig) -> Self {
        // Clause 20.1.2.4 uses max-segments-accepted to advertise receive
        // capacity in segments. Clause 5.2.1.3 caps a transfer at the smaller
        // of the sender's capacity and the receiver's advertised capacity.
        // For a ComplexACK, the receiver's value comes from Max Segments
        // Accepted in the corresponding BACnet-Confirmed-Request-PDU.
        // Exceeding that limit is non-conformant, not merely unusual.
        //
        // Only the rungs B'001'..B'110' name a number; B'000' and B'111'
        // promise nothing, and `advertised_max_segments` reports those as
        // `None`.
        let advertised =
            advertised_max_segments(config.max_segments).map_or(usize::MAX, usize::from);
        Self {
            segmented_response_accepted: config.segmented_response_accepted,
            max_reassembly_segments: advertised.min(SEQUENCE_NUMBER_SPACE),
        }
    }
}

/// Where an inbound segmented ComplexAck came from.
#[derive(Clone)]
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

/// Limits and routing state for one segmented confirmed request, which
/// the request borrows from its caller.
///
/// Borrowed, so the segmented request's future holds a pointer where it
/// would hold this twice (an async fn keeps its arguments twice). Every
/// typed call's future holds that one, and a debug build keeps several
/// copies of each on the stack of whatever polls them: in #2617, 320 bytes
/// more here cost the TUI's agent tests 48 KiB of stack, past the 1 MiB
/// that CI's stack step gives Windows threads (see `future_size_tests`).
pub(super) struct SegmentedRequestLimits<'a> {
    /// The target as the caller named it, before a station on this client's
    /// own network was localized: what a request its node audits starts
    /// over from (#2617).
    pub(super) named: &'a ConfirmedTarget<'a>,
    /// Largest APDU the peer accepts.
    pub(super) remote_max_apdu: u16,
    /// Most segments the peer accepts, when it advertised a bound.
    pub(super) remote_max_segments: Option<u32>,
    /// Forwarded NPCI length reserved for a routed request.
    pub(super) routed_forwarded_npci_len: Option<u16>,
    /// Routed-path lease to mark terminal when the request finishes. A
    /// request its node audits lets it go (#2617).
    pub(super) routed_path_lease: &'a mut Option<RoutedPathLease>,
    /// The request's turn at the destination, which a request its node
    /// audits takes along (#2617).
    pub(super) pace: &'a mut pacing::PaceGuard,
    /// The network and MAC of the discovery-table row that sized the
    /// request, whose age the device's answer restarts; `None` when no row
    /// did (see `BACnetClient::note_answer`).
    pub(super) sized_by: Option<(u16, &'a [u8])>,
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
