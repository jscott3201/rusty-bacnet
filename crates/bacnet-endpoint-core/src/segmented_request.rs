//! The answering side's reassembly of segmented Confirmed-Requests (Clause
//! 5.4.5), without I/O (#2646).
//!
//! [`SegmentedRequests`] keeps every transfer in progress on one role, decides
//! what each received segment draws (a Segment-ACK, nothing, an Abort) and
//! hands back the whole request once its last segment is in. The caller sends
//! what it is told to, along the segment's own reply path, and executes the
//! request it gets back as if it had come in one APDU. A full client uses it
//! for the notifications it executes; its rules are the full server's (the
//! same limits, reasons and timers), so the server can move onto it too.
//!
//! # What a segment draws
//!
//! - Segment zero opens a transfer and is acknowledged at once with the
//!   window the requester proposed (1 to 127; any other proposal draws
//!   WINDOW_SIZE_OUT_OF_RANGE). A conforming requester sends segment zero
//!   alone and waits for that acknowledgment (Clause 5.4.4.1).
//! - Each next segment in order is saved. The one that fills the window
//!   (segment zero's number, or the last acknowledged one's, plus the window)
//!   and the last segment are acknowledged; the others draw nothing.
//! - A segment already acknowledged in this window is a duplicate: up to a
//!   window's worth are discarded, and the next draws a negative Segment-ACK
//!   for the last segment saved. Any other segment out of order draws that
//!   negative acknowledgment at once (Clause 5.4.5.2).
//! - A segment other than zero with no transfer open draws
//!   INVALID_APDU_IN_THIS_STATE, as does a segment whose transfer opened
//!   from another trust context or through another local route; the transfer
//!   then ends (RB-07, fail closed).
//! - Every Segment-ACK and Abort here has the server bit set: the side that
//!   reassembles a request is the one answering it.
//!
//! # Limits
//!
//! Fixed, and the full server's: [`MAX_TRANSFERS`] transfers at once,
//! [`MAX_TRANSFERS_PER_PEER`] of them from one peer, [`MAX_SAVED_BYTES`] of
//! saved segments across them, [`MAX_SEGMENTS`] segments in one request
//! (the sequence number's range: one more would overwrite segment zero).
//! A new transfer past the first two draws BUFFER_OVERFLOW or
//! OUT_OF_RESOURCES; a segment past the last two ends its transfer with
//! BUFFER_OVERFLOW.
//!
//! # Timers
//!
//! A transfer that hears nothing for four segment timeouts ends without a
//! word, as Clause 5.4.5.2 has it: the requester has given up by then. One
//! that hears only duplicates and gaps for [`PROGRESS_TIMEOUT`] ends with an
//! Abort (OTHER), so a peer can't hold a slot by repeating itself.
//! [`SegmentedRequests::expire`] applies both; the caller runs it at
//! [`SegmentedRequests::next_deadline`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bacnet_encoding::apdu::{ConfirmedRequest, SegmentAck};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_encoding::segmentation::{SegmentReceiver, duplicate_in_window};
use bacnet_network::response_route::ResponseRoute;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bacnet_types::enums::AbortReason;
use bytes::Bytes;

/// Transfers one role reassembles at once.
pub const MAX_TRANSFERS: usize = 128;

/// Transfers from one peer (its address, or its network and address behind
/// a router) at once. A resource share, not a guarantee to an authenticated
/// peer.
pub const MAX_TRANSFERS_PER_PEER: usize = 16;

/// Saved segment bytes across one role's transfers: the service data only,
/// not the bookkeeping around it.
pub const MAX_SAVED_BYTES: usize = 4 * 1024 * 1024;

/// Segments in one request. The sequence number counts modulo 256, so the
/// 257th would land on segment zero's slot.
pub const MAX_SEGMENTS: usize = 256;

/// How long a transfer may go without saving a new segment, whatever else
/// it hears, before it ends with an Abort. A local defence, not a timer of
/// the standard's.
pub const PROGRESS_TIMEOUT: Duration = Duration::from_secs(16);

/// A transfer's identity: the peer (its address, or for a routed peer its
/// network and address with the router left out), its invoke ID, and the
/// trust context its first segment came in (RB-07).
pub type TransferKey = (MacAddr, Option<NpduAddress>, u8, TransportProvenance);

/// The key a segment from `source_mac` (and `source_network`, behind a
/// router) with `invoke_id` belongs to.
pub fn transfer_key(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    invoke_id: u8,
    provenance: TransportProvenance,
) -> TransferKey {
    match source_network {
        // A routed peer keeps one transfer whichever router brings it.
        Some(address)
            if (1..=0xFFFE).contains(&address.network) && !address.mac_address.is_empty() =>
        {
            (MacAddr::new(), Some(address.clone()), invoke_id, provenance)
        }
        _ => (
            MacAddr::from_slice(source_mac),
            source_network.cloned(),
            invoke_id,
            provenance,
        ),
    }
}

/// One received segment of a Confirmed-Request, with where it came from.
pub struct Segment<'a> {
    /// The link address it came from.
    pub source_mac: &'a [u8],
    /// The peer's network and address when it came through a router.
    pub source_network: Option<&'a NpduAddress>,
    /// The route an answer to it takes.
    pub route: &'a ResponseRoute,
    /// The segment itself; `segmented` is set.
    pub request: &'a ConfirmedRequest,
}

/// What one segment draws. Every PDU named here goes to the segment's
/// sender along its own reply path, except a [`Receipt::Retired`] one.
#[derive(Debug)]
pub enum Receipt {
    /// Saved; nothing goes back until the window fills or the request ends.
    Saved,
    /// A duplicate within the window, discarded; nothing goes back.
    Discarded,
    /// Send this Segment-ACK, positive or negative.
    Ack(SegmentAck),
    /// Send an Abort with this reason. No transfer is left for the segment.
    Abort(AbortReason),
    /// The segment came through another local route than its transfer's
    /// first: the transfer is over, and its Abort goes back the way the
    /// first segment came. The segment itself is dropped unanswered.
    Retired(Box<Opening>),
    /// The last segment: send `ack`, then execute `request` and answer it
    /// along the opening's route.
    Complete(Box<Completed>),
}

/// Where a transfer's first segment came from: the path its answers and its
/// Abort take.
#[derive(Debug)]
pub struct Opening {
    /// The link address the first segment came from.
    pub source_mac: MacAddr,
    /// The peer's network and address when it came through a router.
    pub source_network: Option<NpduAddress>,
    /// The route an answer takes.
    pub route: ResponseRoute,
    /// The request's invoke ID.
    pub invoke_id: u8,
}

/// A request whose last segment is in.
#[derive(Debug)]
pub struct Completed {
    /// The Segment-ACK for the last segment.
    pub ack: SegmentAck,
    /// The whole request, unsegmented, as if it had come in one APDU.
    pub request: ConfirmedRequest,
    /// Where it came from, and how its answer goes back.
    pub opening: Opening,
}

/// One transfer in progress.
struct Transfer {
    opening: Opening,
    /// The first segment's header, with no service data: the whole
    /// request's.
    header: ConfirmedRequest,
    segments: SegmentReceiver,
    saved_bytes: usize,
    /// When any segment last came: the protocol's segment timer.
    last_activity: Instant,
    /// When a new segment was last saved.
    last_progress: Instant,
    expected_sequence: u8,
    /// The last segment the previous acknowledgment covered.
    initial_sequence: u8,
    /// The last segment saved.
    last_sequence: u8,
    /// Duplicates discarded since the last acknowledgment.
    duplicates: u8,
    window: u8,
    /// Segments saved. The sequence number wraps; this doesn't.
    accepted: usize,
}

impl Transfer {
    fn ack(&self, negative: bool, sequence_number: u8) -> SegmentAck {
        SegmentAck {
            negative_ack: negative,
            sent_by_server: true,
            invoke_id: self.opening.invoke_id,
            sequence_number,
            actual_window_size: self.window,
        }
    }
}

/// The limits one table keeps; see the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub(crate) transfers: usize,
    pub(crate) per_peer: usize,
    pub(crate) saved_bytes: usize,
    pub(crate) segments: usize,
    pub(crate) progress: Duration,
}

impl Limits {
    const FIXED: Self = Self {
        transfers: MAX_TRANSFERS,
        per_peer: MAX_TRANSFERS_PER_PEER,
        saved_bytes: MAX_SAVED_BYTES,
        segments: MAX_SEGMENTS,
        progress: PROGRESS_TIMEOUT,
    };
}

/// A transfer [`SegmentedRequests::expire`] ended with an Abort (OTHER), to
/// send along its opening's route.
pub type Expired = Opening;

/// Every segmented Confirmed-Request one role is reassembling; see the
/// [module docs](self).
pub struct SegmentedRequests {
    transfers: HashMap<TransferKey, Transfer>,
    saved_bytes: usize,
    /// Four segment timeouts: silence past this ends a transfer.
    receive_timeout: Duration,
    limits: Limits,
}

impl SegmentedRequests {
    /// An empty table whose transfers end after `segment_timeout` four times
    /// over with nothing heard (Clause 5.4.5.2).
    pub fn new(segment_timeout: Duration) -> Self {
        Self::with_limits(segment_timeout, Limits::FIXED)
    }

    pub(crate) fn with_limits(segment_timeout: Duration, limits: Limits) -> Self {
        Self {
            transfers: HashMap::new(),
            saved_bytes: 0,
            receive_timeout: segment_timeout.saturating_mul(4),
            limits,
        }
    }

    /// Transfers in progress.
    pub fn len(&self) -> usize {
        self.transfers.len()
    }

    /// Whether no transfer is in progress.
    pub fn is_empty(&self) -> bool {
        self.transfers.is_empty()
    }

    /// Saved segment bytes across every transfer.
    pub fn saved_bytes(&self) -> usize {
        self.saved_bytes
    }

    /// End every transfer, without a word.
    pub fn clear(&mut self) {
        self.transfers.clear();
        self.saved_bytes = 0;
    }

    /// Take in one segment at `now`; see [`Receipt`] for what it draws.
    pub fn receive(&mut self, segment: Segment<'_>, now: Instant) -> Receipt {
        let request = segment.request;
        let provenance = segment.route.provenance();
        let key = transfer_key(
            segment.source_mac,
            segment.source_network,
            request.invoke_id,
            provenance,
        );
        if let Some(conflict) = self.provenance_conflict(&key) {
            self.remove(&conflict);
            return Receipt::Abort(AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        let sequence = request.sequence_number.unwrap_or(0);
        let Some(transfer) = self.transfers.get(&key) else {
            return self.open(key, segment, now);
        };
        if !transfer.opening.route.same_origin(segment.route) {
            let transfer = self.remove(&key).expect("transfer looked up above");
            return Receipt::Retired(Box::new(transfer.opening));
        }
        self.next_segment(key, sequence, request, now)
    }

    /// End the transfers a requester's Abort (server bit clear) names. One
    /// from a direct peer ends only its own; any other ends the transfer
    /// under that peer and invoke ID in every context but a direct one.
    pub fn abort(
        &mut self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        provenance: TransportProvenance,
    ) {
        let key = transfer_key(source_mac, source_network, invoke_id, provenance);
        if key.3.is_direct_peer() {
            self.remove(&key);
            return;
        }
        let doomed: Vec<TransferKey> = self
            .transfers
            .keys()
            .filter(|other| same_transfer_elsewhere(other, &key))
            .cloned()
            .collect();
        for doomed in doomed {
            self.remove(&doomed);
        }
    }

    /// End every transfer due at `now`: one silent for four segment
    /// timeouts without a word, one without progress for
    /// [`PROGRESS_TIMEOUT`] with an Abort (OTHER) to send. The first wins
    /// when both are due.
    pub fn expire(&mut self, now: Instant) -> Vec<Expired> {
        let receive_timeout = self.receive_timeout;
        let progress = self.limits.progress;
        let mut expired = Vec::new();
        let mut freed = 0usize;
        self.transfers.retain(|_, transfer| {
            if now.saturating_duration_since(transfer.last_activity) > receive_timeout {
                freed += transfer.saved_bytes;
                return false;
            }
            if now.saturating_duration_since(transfer.last_progress) >= progress {
                freed += transfer.saved_bytes;
                expired.push(Opening {
                    source_mac: transfer.opening.source_mac.clone(),
                    source_network: transfer.opening.source_network.clone(),
                    route: transfer.opening.route.clone(),
                    invoke_id: transfer.opening.invoke_id,
                });
                return false;
            }
            true
        });
        self.saved_bytes -= freed;
        expired
    }

    /// When [`expire`](Self::expire) next has work, if any transfer is in
    /// progress. Silence ends a transfer only once it is strictly longer
    /// than four segment timeouts, so that deadline is one nanosecond past.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.transfers
            .values()
            .flat_map(|transfer| {
                let silent = transfer
                    .last_activity
                    .checked_add(self.receive_timeout)
                    .and_then(|at| at.checked_add(Duration::from_nanos(1)));
                let stalled = transfer.last_progress.checked_add(self.limits.progress);
                [silent, stalled].into_iter().flatten()
            })
            .min()
    }

    /// A transfer under the same peer and invoke ID in another trust context
    /// that isn't a direct one. A direct peer's transfers are its own.
    fn provenance_conflict(&self, key: &TransferKey) -> Option<TransferKey> {
        if key.3.is_direct_peer() {
            return None;
        }
        self.transfers
            .keys()
            .find(|other| same_transfer_elsewhere(other, key) && other.3 != key.3)
            .cloned()
    }

    fn open(&mut self, key: TransferKey, segment: Segment<'_>, now: Instant) -> Receipt {
        let request = segment.request;
        if request.sequence_number.unwrap_or(0) != 0 {
            return Receipt::Abort(AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        let window = request.proposed_window_size.unwrap_or(0);
        if !(1..=127).contains(&window) {
            return Receipt::Abort(AbortReason::WINDOW_SIZE_OUT_OF_RANGE);
        }
        if self.transfers.len() >= self.limits.transfers {
            return Receipt::Abort(AbortReason::BUFFER_OVERFLOW);
        }
        let from_peer = self
            .transfers
            .keys()
            .filter(|other| other.0 == key.0 && other.1 == key.1)
            .count();
        if from_peer >= self.limits.per_peer {
            return Receipt::Abort(AbortReason::OUT_OF_RESOURCES);
        }
        let mut transfer = Transfer {
            opening: Opening {
                source_mac: MacAddr::from_slice(segment.source_mac),
                source_network: segment.source_network.cloned(),
                route: segment.route.clone(),
                invoke_id: request.invoke_id,
            },
            header: ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: request.segmented_response_accepted,
                max_segments: request.max_segments,
                max_apdu_length: request.max_apdu_length,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_request: Bytes::new(),
            },
            segments: SegmentReceiver::new(),
            saved_bytes: 0,
            last_activity: now,
            last_progress: now,
            expected_sequence: 1,
            initial_sequence: 0,
            last_sequence: 0,
            duplicates: 0,
            window,
            accepted: 0,
        };
        if save(
            &mut self.saved_bytes,
            &self.limits,
            &mut transfer,
            0,
            request,
        )
        .is_err()
        {
            return Receipt::Abort(AbortReason::BUFFER_OVERFLOW);
        }
        let ack = transfer.ack(false, 0);
        if !request.more_follows {
            return self.complete(transfer, ack);
        }
        self.transfers.insert(key, transfer);
        Receipt::Ack(ack)
    }

    fn next_segment(
        &mut self,
        key: TransferKey,
        sequence: u8,
        request: &ConfirmedRequest,
        now: Instant,
    ) -> Receipt {
        let Self {
            transfers,
            saved_bytes,
            limits,
            ..
        } = self;
        let transfer = transfers.get_mut(&key).expect("caller found it");
        // Any segment restarts the segment timer, a duplicate or a gap too.
        transfer.last_activity = now;
        if sequence != transfer.expected_sequence {
            let duplicate =
                duplicate_in_window(sequence, transfer.initial_sequence, transfer.last_sequence);
            if duplicate && transfer.duplicates < transfer.window {
                transfer.duplicates += 1;
                return Receipt::Discarded;
            }
            // Too many duplicates, or a gap: ask again from the segment
            // after the last one saved.
            transfer.initial_sequence = transfer.last_sequence;
            transfer.duplicates = 0;
            return Receipt::Ack(transfer.ack(true, transfer.last_sequence));
        }
        // Only a new segment in order counts against the cap: a
        // retransmission is discarded above, never punished.
        if transfer.accepted >= limits.segments
            || save(saved_bytes, limits, transfer, sequence, request).is_err()
        {
            self.remove(&key);
            return Receipt::Abort(AbortReason::BUFFER_OVERFLOW);
        }
        transfer.last_progress = now;
        transfer.expected_sequence = sequence.wrapping_add(1);
        transfer.last_sequence = sequence;
        let ack = transfer.ack(false, sequence);
        if !request.more_follows {
            let transfer = transfers.remove(&key).expect("caller found it");
            return self.complete(transfer, ack);
        }
        if sequence == transfer.initial_sequence.wrapping_add(transfer.window) {
            transfer.initial_sequence = sequence;
            transfer.duplicates = 0;
            return Receipt::Ack(ack);
        }
        Receipt::Saved
    }

    /// Hand back the whole request of a transfer already out of the table,
    /// and give its bytes back to the budget.
    fn complete(&mut self, transfer: Transfer, ack: SegmentAck) -> Receipt {
        self.saved_bytes -= transfer.saved_bytes;
        // Saved strictly in order and at most MAX_SEGMENTS of them, so every
        // segment up to the count is there.
        let Ok(data) = transfer.segments.reassemble(transfer.accepted) else {
            return Receipt::Abort(AbortReason::OTHER);
        };
        Receipt::Complete(Box::new(Completed {
            ack,
            request: ConfirmedRequest {
                service_request: Bytes::from(data),
                ..transfer.header
            },
            opening: transfer.opening,
        }))
    }

    fn remove(&mut self, key: &TransferKey) -> Option<Transfer> {
        let transfer = self.transfers.remove(key)?;
        self.saved_bytes -= transfer.saved_bytes;
        Some(transfer)
    }
}

/// Save one new segment of `request` in `transfer`, charged against the
/// table's byte budget `saved`. A copy is kept, so a small segment doesn't
/// hold the buffer it came in.
fn save(
    saved: &mut usize,
    limits: &Limits,
    transfer: &mut Transfer,
    sequence: u8,
    request: &ConfirmedRequest,
) -> Result<(), ()> {
    let data = &request.service_request;
    let total = saved.checked_add(data.len()).ok_or(())?;
    if total > limits.saved_bytes {
        return Err(());
    }
    transfer
        .segments
        .receive(sequence, Bytes::copy_from_slice(data))
        .map_err(|_| ())?;
    *saved = total;
    transfer.saved_bytes += data.len();
    transfer.accepted += 1;
    Ok(())
}

/// Whether `other` is a transfer under `key`'s peer and invoke ID in a
/// context that isn't a direct one.
fn same_transfer_elsewhere(other: &TransferKey, key: &TransferKey) -> bool {
    !other.3.is_direct_peer() && other.0 == key.0 && other.1 == key.1 && other.2 == key.2
}

#[cfg(test)]
#[path = "segmented_request_tests.rs"]
mod tests;
