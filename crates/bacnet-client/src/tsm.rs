//! Transaction State Machine (TSM) per ASHRAE 135-2020 Clause 5.4.
//!
//! Tracks in-flight confirmed requests. Each request gets a unique invoke_id
//! (0-255) scoped per destination MAC. Responses are delivered via oneshot channels.

use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::MacAddr;
use bytes::Bytes;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use tokio::sync::{oneshot, watch};

/// TSM configuration.
#[derive(Debug, Clone)]
pub struct TsmConfig {
    /// APDU timeout in milliseconds (default 6000).
    pub apdu_timeout_ms: u64,
    /// APDU segment timeout in milliseconds (default = apdu_timeout_ms).
    pub apdu_segment_timeout_ms: u64,
    /// Number of APDU retries (default 3).
    pub apdu_retries: u8,
}

impl Default for TsmConfig {
    fn default() -> Self {
        Self {
            apdu_timeout_ms: 6000,
            apdu_segment_timeout_ms: 6000,
            apdu_retries: 3,
        }
    }
}

/// Response types that complete a transaction.
///
/// Non-exhaustive: the TSM gains completion reasons as more of Clause 5.4's
/// state machine is implemented, and those should be additive for callers.
#[derive(Debug)]
#[non_exhaustive]
pub enum TsmResponse {
    /// SimpleACK — confirmed service completed with no return data.
    SimpleAck,
    /// ComplexACK — confirmed service returned data.
    ComplexAck { service_data: Bytes },
    /// Error PDU.
    Error { class: u32, code: u32 },
    /// Reject PDU.
    Reject { reason: u8 },
    /// Abort PDU.
    Abort { reason: u8 },
}

/// Invoke ID allocator scoped to a single destination MAC.
struct InvokeIdAllocator {
    next_id: u8,
    in_use: [bool; 256],
}

impl InvokeIdAllocator {
    fn new() -> Self {
        Self {
            next_id: 0,
            in_use: [false; 256],
        }
    }

    fn allocate(&mut self) -> Option<u8> {
        let start = self.next_id;
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.in_use[id as usize] {
                self.in_use[id as usize] = true;
                return Some(id);
            }
            if self.next_id == start {
                return None;
            }
        }
    }

    fn release(&mut self, id: u8) {
        self.in_use[id as usize] = false;
    }

    fn all_free(&self) -> bool {
        !self.in_use.iter().any(|&used| used)
    }
}

/// Maximum number of distinct destination MACs tracked by the TSM.
/// Prevents unbounded memory growth from spoofed source addresses.
const MAX_TSM_DESTINATIONS: usize = 1024;

/// What `complete_transaction` did with a response.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionOutcome {
    /// The response matched a pending transaction and was delivered.
    Delivered,
    /// No transaction was pending for this source and invoke ID.
    NoTransaction,
    /// A transaction was pending, but the response was labelled for a
    /// different confirmed service. The transaction is left pending and the
    /// invoke ID stays allocated, so the legitimate response can still arrive.
    ServiceChoiceMismatch {
        /// The service the pending request asked for.
        expected: ConfirmedServiceChoice,
        /// The service the response claimed to answer.
        observed: ConfirmedServiceChoice,
    },
}

/// Request-side timer state delivered to the task waiting for a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionProgress {
    AwaitingResponse,
    SegmentedResponse { generation: u64 },
}

pub(crate) struct TransactionRegistration {
    pub(crate) response: oneshot::Receiver<TsmResponse>,
    pub(crate) progress: watch::Receiver<TransactionProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestTimerExpiration {
    Retry,
    SegmentedResponse { generation: u64 },
    TimedOut,
    NoTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentTimerExpiration {
    Activity { generation: u64 },
    AwaitingResponse,
    TimedOut,
    NoTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionPhase {
    AwaitingResponse,
    SegmentedResponse,
}

/// A confirmed request awaiting its response.
struct PendingTransaction {
    responder: oneshot::Sender<TsmResponse>,
    progress: watch::Sender<TransactionProgress>,
    phase: TransactionPhase,
    /// Monotonic token used to reject a SegmentTimer expiry observed before
    /// newer segment activity acquired the TSM lock.
    segment_generation: u64,
    /// The service this request asked for. Clause 20.1.4.2 and 20.1.5.6 both
    /// require an acknowledgment's service-ack-choice to "contain the value of
    /// the BACnetConfirmedServiceChoice corresponding to the service contained
    /// in the previous BACnet-Confirmed-Service-Request that has resulted in
    /// this acknowledgment", so anything else is not this transaction's
    /// response.
    expected_service_choice: ConfirmedServiceChoice,
}

/// Transaction State Machine.
///
/// Tracks pending confirmed requests and correlates responses by
/// `(destination_mac, invoke_id)` and by the confirmed service each request
/// asked for.
pub struct Tsm {
    config: TsmConfig,
    allocators: HashMap<MacAddr, InvokeIdAllocator>,
    pending: HashMap<(MacAddr, u8), PendingTransaction>,
}

impl Tsm {
    pub fn new(config: TsmConfig) -> Self {
        Self {
            config,
            allocators: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    pub fn config(&self) -> &TsmConfig {
        &self.config
    }

    /// Allocate an invoke ID for the given destination MAC.
    /// Returns `None` if all 256 IDs are in use for this destination,
    /// or if the maximum number of tracked destinations has been reached.
    pub fn allocate_invoke_id(&mut self, destination_mac: &[u8]) -> Option<u8> {
        let key = MacAddr::from_slice(destination_mac);
        if !self.allocators.contains_key(&key) && self.allocators.len() >= MAX_TSM_DESTINATIONS {
            return None;
        }
        let allocator = self
            .allocators
            .entry(key)
            .or_insert_with(InvokeIdAllocator::new);
        allocator.allocate()
    }

    /// Release an invoke ID back to the pool for the given destination.
    /// Removes the allocator entry if all IDs are now free (prevents unbounded growth).
    pub fn release_invoke_id(&mut self, destination_mac: &[u8], invoke_id: u8) {
        let key = MacAddr::from_slice(destination_mac);
        if let Some(allocator) = self.allocators.get_mut(&key) {
            allocator.release(invoke_id);
            if allocator.all_free() {
                self.allocators.remove(&key);
            }
        }
    }

    /// Register a pending transaction. Returns a receiver that will deliver
    /// the response when it arrives.
    ///
    /// `service_choice` is the confirmed service being requested; a response
    /// labelled for any other service will not complete this transaction.
    pub fn register_transaction(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
    ) -> oneshot::Receiver<TsmResponse> {
        self.register_transaction_with_progress(destination_mac, invoke_id, service_choice)
            .response
    }

    pub(crate) fn register_transaction_with_progress(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
    ) -> TransactionRegistration {
        let (tx, rx) = oneshot::channel();
        let (progress_tx, progress_rx) = watch::channel(TransactionProgress::AwaitingResponse);
        debug_assert!(
            !self
                .pending
                .contains_key(&(destination_mac.clone(), invoke_id)),
            "duplicate TSM registration for invoke_id {}",
            invoke_id
        );
        self.pending.insert(
            (destination_mac, invoke_id),
            PendingTransaction {
                responder: tx,
                progress: progress_tx,
                phase: TransactionPhase::AwaitingResponse,
                segment_generation: 0,
                expected_service_choice: service_choice,
            },
        );
        TransactionRegistration {
            response: rx,
            progress: progress_rx,
        }
    }

    /// Enter SEGMENTED_CONF after the first segment has been saved.
    ///
    /// The transition and RequestTimer retry authorization use the same TSM
    /// lock. Whichever changes or observes the phase first wins that decision;
    /// an authorized retry performs transport I/O only after releasing it.
    pub(crate) fn begin_segmented_response(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
    ) -> Option<u64> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let pending = self.pending.get_mut(&key)?;
        pending.segment_generation = pending.segment_generation.wrapping_add(1);
        pending.phase = TransactionPhase::SegmentedResponse;
        let generation = pending.segment_generation;
        pending
            .progress
            .send_replace(TransactionProgress::SegmentedResponse { generation });
        Some(generation)
    }

    /// Restart SegmentTimer for a segment handled in SEGMENTED_CONF.
    pub(crate) fn record_segmented_response_activity(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
    ) -> Option<u64> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let pending = self.pending.get_mut(&key)?;
        if pending.phase != TransactionPhase::SegmentedResponse {
            return None;
        }
        pending.segment_generation = pending.segment_generation.wrapping_add(1);
        let generation = pending.segment_generation;
        pending
            .progress
            .send_replace(TransactionProgress::SegmentedResponse { generation });
        Some(generation)
    }

    /// Resume AWAIT_CONFIRMATION when a completed reassembly is not this
    /// transaction's response or cannot be delivered.
    pub(crate) fn reset_segmented_response(&mut self, source_mac: &[u8], invoke_id: u8) -> bool {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self.pending.get_mut(&key) else {
            return false;
        };
        if pending.phase != TransactionPhase::SegmentedResponse {
            return false;
        }
        pending.phase = TransactionPhase::AwaitingResponse;
        pending
            .progress
            .send_replace(TransactionProgress::AwaitingResponse);
        true
    }

    /// Arbitrate RequestTimer against the receive-side phase transition.
    ///
    /// Returning [`RequestTimerExpiration::Retry`] authorizes one retry. The
    /// caller then releases the enclosing TSM lock before transport I/O; a
    /// segmented-response transition that follows does not revoke that send.
    pub(crate) fn expire_request_timer(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        final_timeout: bool,
    ) -> RequestTimerExpiration {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return RequestTimerExpiration::NoTransaction;
        };
        if pending.phase == TransactionPhase::SegmentedResponse {
            return RequestTimerExpiration::SegmentedResponse {
                generation: pending.segment_generation,
            };
        }
        if !final_timeout {
            return RequestTimerExpiration::Retry;
        }
        self.pending.remove(&key);
        self.release_invoke_id(destination_mac, invoke_id);
        RequestTimerExpiration::TimedOut
    }

    /// Cancel only if no segment activity has advanced past `generation`.
    pub(crate) fn expire_segment_timer(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        generation: u64,
    ) -> SegmentTimerExpiration {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return SegmentTimerExpiration::NoTransaction;
        };
        if pending.phase == TransactionPhase::AwaitingResponse {
            return SegmentTimerExpiration::AwaitingResponse;
        }
        if pending.segment_generation != generation {
            return SegmentTimerExpiration::Activity {
                generation: pending.segment_generation,
            };
        }
        self.pending.remove(&key);
        self.release_invoke_id(destination_mac, invoke_id);
        SegmentTimerExpiration::TimedOut
    }

    /// The confirmed service a pending transaction is waiting on, if any.
    ///
    /// Doubles as the "is a transaction pending" predicate: an inbound
    /// segmented response for an invoke ID nobody is waiting on should not be
    /// allocated a reassembly session.
    pub fn expected_service_choice(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
    ) -> Option<ConfirmedServiceChoice> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        self.pending.get(&key).map(|p| p.expected_service_choice)
    }

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
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Entry::Occupied(entry) = self.pending.entry(key) else {
            return CompletionOutcome::NoTransaction;
        };
        let expected = entry.get().expected_service_choice;
        if let Some(observed) = observed_service_choice {
            if observed != expected {
                return CompletionOutcome::ServiceChoiceMismatch { expected, observed };
            }
        }
        // Taking the entry ends the borrow of `pending`, so the invoke ID can
        // be released without a second lookup that would have to be `expect`ed.
        let pending = entry.remove();
        self.release_invoke_id(source_mac, invoke_id);
        let _ = pending.responder.send(response);
        CompletionOutcome::Delivered
    }

    /// Cancel a pending transaction. Returns `true` if found.
    pub fn cancel_transaction(&mut self, destination_mac: &[u8], invoke_id: u8) -> bool {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        if self.pending.remove(&key).is_some() {
            self.release_invoke_id(destination_mac, invoke_id);
            true
        } else {
            false
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// Drop guard that cleans up invoke IDs if a confirmed request task is cancelled.
///
/// Uses `try_lock` in Drop — best-effort cleanup. If the mutex is contended
/// at drop time, the invoke ID leaks (acceptable: blocking in Drop is worse).
pub(crate) struct TsmGuard {
    tsm: std::sync::Arc<tokio::sync::Mutex<Tsm>>,
    mac: MacAddr,
    invoke_id: u8,
    completed: bool,
}

impl TsmGuard {
    pub(crate) fn new(
        tsm: std::sync::Arc<tokio::sync::Mutex<Tsm>>,
        mac: MacAddr,
        invoke_id: u8,
    ) -> Self {
        Self {
            tsm,
            mac,
            invoke_id,
            completed: false,
        }
    }

    /// Mark the transaction as completed (prevents cleanup on drop).
    pub(crate) fn mark_completed(&mut self) {
        self.completed = true;
    }
}

impl Drop for TsmGuard {
    fn drop(&mut self) {
        if !self.completed {
            if let Ok(mut tsm) = self.tsm.try_lock() {
                tsm.cancel_transaction(&self.mac, self.invoke_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_invoke_id_sequential() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        let id1 = tsm.allocate_invoke_id(&mac);
        let id2 = tsm.allocate_invoke_id(&mac);
        assert_eq!(id1, Some(0));
        assert_eq!(id2, Some(1));
    }

    #[test]
    fn allocate_invoke_id_per_destination() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac_a = [10, 0, 0, 1, 0xBA, 0xC0];
        let mac_b = [10, 0, 0, 2, 0xBA, 0xC0];
        let id_a = tsm.allocate_invoke_id(&mac_a);
        let id_b = tsm.allocate_invoke_id(&mac_b);
        assert_eq!(id_a, Some(0));
        assert_eq!(id_b, Some(0));
    }

    #[test]
    fn allocate_invoke_id_wraps() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        for i in 0..256 {
            assert_eq!(tsm.allocate_invoke_id(&mac), Some(i as u8));
        }
        assert_eq!(tsm.allocate_invoke_id(&mac), None);
    }

    #[test]
    fn release_makes_id_available() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        let id0 = tsm.allocate_invoke_id(&mac).unwrap();
        let id1 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        tsm.release_invoke_id(&mac, id0);
        let id2 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id2, 2);
        tsm.release_invoke_id(&mac, id1);
        tsm.release_invoke_id(&mac, id2);
        let id3 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id3, 0);
    }

    #[tokio::test]
    async fn register_and_complete_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();

        let rx = tsm.register_transaction(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        let response = TsmResponse::ComplexAck {
            service_data: Bytes::from_static(&[0xDE, 0xAD]),
        };
        let outcome = tsm.complete_transaction(
            &mac,
            invoke_id,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            response,
        );
        assert_eq!(outcome, CompletionOutcome::Delivered);

        let result = rx.await.unwrap();
        match result {
            TsmResponse::ComplexAck { service_data } => {
                assert_eq!(service_data, vec![0xDE, 0xAD]);
            }
            _ => panic!("Expected ComplexAck"),
        }
    }

    #[tokio::test]
    async fn mismatched_service_choice_leaves_the_transaction_pending() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let rx = tsm.register_transaction(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        let outcome = tsm.complete_transaction(
            &mac,
            invoke_id,
            Some(ConfirmedServiceChoice::WRITE_PROPERTY),
            TsmResponse::ComplexAck {
                service_data: Bytes::from_static(&[0xBA, 0xD0]),
            },
        );
        assert_eq!(
            outcome,
            CompletionOutcome::ServiceChoiceMismatch {
                expected: ConfirmedServiceChoice::READ_PROPERTY,
                observed: ConfirmedServiceChoice::WRITE_PROPERTY,
            }
        );
        assert_eq!(tsm.pending_count(), 1, "transaction must stay pending");
        assert_eq!(
            tsm.allocate_invoke_id(&mac),
            Some(invoke_id.wrapping_add(1)),
            "the invoke ID must not have been handed back for reuse"
        );

        // The genuine response still completes it.
        let outcome = tsm.complete_transaction(
            &mac,
            invoke_id,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::ComplexAck {
                service_data: Bytes::from_static(&[0xDE, 0xAD]),
            },
        );
        assert_eq!(outcome, CompletionOutcome::Delivered);
        match rx.await.unwrap() {
            TsmResponse::ComplexAck { service_data } => {
                assert_eq!(service_data.as_ref(), &[0xDE, 0xAD]);
            }
            other => panic!("expected ComplexAck, got {other:?}"),
        }
    }

    /// Reject and Abort carry no service choice (Clauses 20.1.8, 20.1.9), and
    /// Clause 20.1.7.2 lets an Error PDU answer any service with the
    /// `BACnet-Error` `other [127]` choice. All three correlate by invoke ID
    /// alone.
    #[tokio::test]
    async fn a_response_without_a_service_choice_completes_any_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let rx = tsm.register_transaction(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        let outcome =
            tsm.complete_transaction(&mac, invoke_id, None, TsmResponse::Reject { reason: 9 });
        assert_eq!(outcome, CompletionOutcome::Delivered);
        assert!(matches!(
            rx.await.unwrap(),
            TsmResponse::Reject { reason: 9 }
        ));
    }

    #[tokio::test]
    async fn complete_unknown_transaction_reports_no_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let outcome = tsm.complete_transaction(
            &mac,
            42,
            Some(ConfirmedServiceChoice::READ_PROPERTY),
            TsmResponse::SimpleAck,
        );
        assert_eq!(outcome, CompletionOutcome::NoTransaction);
    }

    #[test]
    fn cancel_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let _rx = tsm.register_transaction(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        assert_eq!(tsm.pending_count(), 1);

        let cancelled = tsm.cancel_transaction(&mac, invoke_id);
        assert!(cancelled);
        assert_eq!(tsm.pending_count(), 0);
    }

    #[test]
    fn segmented_admission_and_request_timeout_are_serialized() {
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);

        let mut admitted_first = Tsm::new(TsmConfig::default());
        let invoke_id = admitted_first.allocate_invoke_id(&mac).unwrap();
        let registration = admitted_first.register_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        let generation = admitted_first
            .begin_segmented_response(&mac, invoke_id)
            .unwrap();
        assert_eq!(
            *registration.progress.borrow(),
            TransactionProgress::SegmentedResponse { generation }
        );
        assert_eq!(
            admitted_first.expire_request_timer(&mac, invoke_id, true),
            RequestTimerExpiration::SegmentedResponse { generation }
        );
        assert_eq!(admitted_first.pending_count(), 1);

        let mut timeout_first = Tsm::new(TsmConfig::default());
        let invoke_id = timeout_first.allocate_invoke_id(&mac).unwrap();
        let _registration = timeout_first.register_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        assert_eq!(
            timeout_first.expire_request_timer(&mac, invoke_id, true),
            RequestTimerExpiration::TimedOut
        );
        assert_eq!(
            timeout_first.begin_segmented_response(&mac, invoke_id),
            None
        );
        assert_eq!(timeout_first.pending_count(), 0);
    }

    #[test]
    fn retry_send_failure_recheck_preserves_later_segmented_admission() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let _registration = tsm.register_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        assert_eq!(
            tsm.expire_request_timer(&mac, invoke_id, false),
            RequestTimerExpiration::Retry
        );
        let generation = tsm.begin_segmented_response(&mac, invoke_id).unwrap();
        assert_eq!(
            tsm.expire_request_timer(&mac, invoke_id, true),
            RequestTimerExpiration::SegmentedResponse { generation }
        );
        assert_eq!(tsm.pending_count(), 1);
    }

    #[test]
    fn stale_segment_timer_generation_cannot_cancel_new_activity() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let registration = tsm.register_transaction_with_progress(
            mac.clone(),
            invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY,
        );

        let stale_generation = tsm.begin_segmented_response(&mac, invoke_id).unwrap();
        let current_generation = tsm
            .record_segmented_response_activity(&mac, invoke_id)
            .unwrap();
        assert_ne!(stale_generation, current_generation);
        assert_eq!(
            tsm.expire_segment_timer(&mac, invoke_id, stale_generation),
            SegmentTimerExpiration::Activity {
                generation: current_generation
            }
        );
        assert_eq!(tsm.pending_count(), 1);
        assert_eq!(
            *registration.progress.borrow(),
            TransactionProgress::SegmentedResponse {
                generation: current_generation
            }
        );

        assert_eq!(
            tsm.expire_segment_timer(&mac, invoke_id, current_generation),
            SegmentTimerExpiration::TimedOut
        );
        assert_eq!(tsm.pending_count(), 0);
        assert_eq!(tsm.allocate_invoke_id(&mac), Some(0));
    }
}
