//! The segmented confirmed requests the client executes (#2646).
//!
//! A client executes three confirmed services: ConfirmedCOVNotification,
//! ConfirmedCOVNotificationMultiple and ConfirmedEventNotification (beside a
//! server role, the COV notifications only). A notifier may send any of them
//! as segments, a COV-multiple with many values most of all. When the client
//! receives segments ([`ClientConfig::segmentation`]), it reassembles a
//! segmented request on the answering side's rules ([`SegmentedRequests`],
//! the full server's) and then answers it as one that came whole: it
//! executes a notification (the same decoding, publishing and answer), and
//! rejects any other service as unrecognized, as it does an unsegmented one
//! (Clause 18.9). A peer that segments to a device declaring segmented
//! receive is never told that it can't. A client that takes no segments
//! answers the first segment with an Abort (SEGMENTATION_NOT_SUPPORTED).
//!
//! # Where it runs
//!
//! In a task of its own, the reassembly task, which the first segment
//! starts and which owns the table and its timers. The dispatch task meets a
//! segment, and a requester's Abort, only in the two branches of
//! `dispatch_apdu` that always took them (the segment's Abort, the Abort it
//! ignored): their bodies hand the PDU over, out of line and without
//! waiting, and nothing else in the dispatch task changed. A request's plain
//! path is the dev path, instruction for instruction (#2646 review: a shape
//! that grew `dispatch_apdu` or its loop cost every answer 11 to 32
//! instructions).
//!
//! The hand-over is one bounded queue, so a transfer's segments and a
//! requester's Abort reach the table in the order the client received
//! them. A full queue drops what doesn't fit and counts it: a segment
//! draws an Abort (BUFFER_OVERFLOW), sent by a task spawned for it; an Abort
//! is dropped, and its transfer ends by its timer.
//!
//! The client's transaction table keeps the queue's sender and the task's
//! handle. The task holds the network layer weakly, so it never keeps the
//! link up, and it ends when the client stops (which waits for it), when
//! the client is dropped, when the node a seated client runs on stops, and
//! when the layer is gone.

use std::sync::Weak;

use bacnet_endpoint_core::segmented_request::{Opening, Receipt, Segment, SegmentedRequests};
use bacnet_types::enums::AbortReason;
use tokio::sync::mpsc::error::TrySendError;

use super::*;

/// Segments and Aborts waiting for the reassembly task. A burst bigger than
/// this, faster than the task takes it, is dropped and counted.
const QUEUE: usize = 64;

/// What the dispatch task hands the reassembly task.
pub(super) enum Handed {
    /// One segment of a confirmed request, with its reply path.
    Segment {
        source_mac: MacAddr,
        source_network: Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    },
    /// A requester's Abort (server bit clear).
    Abort {
        source_mac: MacAddr,
        source_network: Option<NpduAddress>,
        invoke_id: u8,
        provenance: TransportProvenance,
    },
}

/// The reassembly task, as the client's transaction table keeps it: its
/// queue, its handle (dropping this aborts it) and what the queue dropped.
pub(crate) struct RequestReassembly {
    queue: mpsc::Sender<Handed>,
    task: Option<JoinHandle<()>>,
    /// Segments and Aborts a full queue dropped.
    pub(crate) full_drops: u64,
    /// The task's table, for tests to read.
    #[cfg(test)]
    pub(crate) table: Weak<SyncMutex<SegmentedRequests>>,
}

impl RequestReassembly {
    /// A reassembly whose task never takes from `queue`, for a test to
    /// fill it.
    #[cfg(test)]
    pub(super) fn stalled(queue: mpsc::Sender<Handed>) -> Self {
        Self {
            queue,
            task: None,
            full_drops: 0,
            table: Weak::new(),
        }
    }

    /// Abort the task and wait until it is gone, its table with it.
    pub(super) async fn end(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for RequestReassembly {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// What starting the reassembly task and handing it a segment take,
/// borrowed from `dispatch_apdu`.
pub(super) struct HandOff<'a, T: TransportPort + 'static> {
    pub(super) tsm: &'a Arc<SyncMutex<Tsm>>,
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) cov_tx: &'a broadcast::Sender<COVEvent>,
    pub(super) event_tx: &'a broadcast::Sender<ReceivedEventNotification>,
    pub(super) policy: &'a ConfirmedCOVNotificationAckPolicy,
    pub(super) limits: ResponseLimits,
}

/// What the reassembly task publishes to and warns through.
struct Outlets {
    cov_tx: broadcast::Sender<COVEvent>,
    event_tx: broadcast::Sender<ReceivedEventNotification>,
    policy: ConfirmedCOVNotificationAckPolicy,
    decode_warning: WarnThrottle,
    response_warning: WarnThrottle,
}

/// Now, on tokio's clock, so a paused test steps the timers.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The body of `dispatch_apdu`'s branch for a segment of a confirmed
    /// request, when the client receives segments: hand it to the
    /// reassembly task, starting the task on the first one.
    #[cold]
    #[inline(never)]
    pub(super) fn hand_off_segment(
        at: HandOff<'_, T>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    ) {
        let handed = Handed::Segment {
            source_mac: MacAddr::from_slice(source_mac),
            source_network: source_network.clone(),
            reply,
            request,
        };
        let mut tsm = at.tsm.lock();
        let segment_timeout = tsm.segment_timeout();
        let reassembly = tsm
            .request_reassembly
            .get_or_insert_with(|| Box::new(Self::start_reassembly(&at, segment_timeout)));
        let refused = match reassembly.queue.try_send(handed) {
            Ok(()) => return,
            Err(TrySendError::Full(handed)) => {
                reassembly.full_drops = reassembly.full_drops.saturating_add(1);
                handed
            }
            // The task is gone with the layer; a stopped client's link
            // sends nothing either.
            Err(TrySendError::Closed(_)) => {
                tsm.request_reassembly = None;
                return;
            }
        };
        drop(tsm);
        let Handed::Segment {
            source_mac,
            source_network,
            reply,
            request,
        } = refused
        else {
            return;
        };
        let network = Arc::clone(at.network);
        tokio::spawn(async move {
            Self::send_server_abort(
                &network,
                &source_mac,
                &source_network,
                reply,
                request.invoke_id,
                AbortReason::BUFFER_OVERFLOW,
            )
            .await;
        });
    }

    /// The body of `dispatch_apdu`'s branch for an Abort with the server
    /// bit clear: hand it to the reassembly task, if one runs, to end the
    /// transfer it names (Clause 5.4.5.2). It draws nothing back.
    #[cold]
    #[inline(never)]
    pub(super) fn hand_off_abort(
        tsm: &Arc<SyncMutex<Tsm>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        invoke_id: u8,
        provenance: &TransportProvenance,
    ) {
        let mut tsm = tsm.lock();
        let Some(reassembly) = tsm.request_reassembly.as_mut() else {
            return;
        };
        let handed = Handed::Abort {
            source_mac: MacAddr::from_slice(source_mac),
            source_network: source_network.clone(),
            invoke_id,
            provenance: provenance.clone(),
        };
        if reassembly.queue.try_send(handed).is_err() {
            reassembly.full_drops = reassembly.full_drops.saturating_add(1);
        }
    }

    /// Start the reassembly task.
    fn start_reassembly(at: &HandOff<'_, T>, segment_timeout: Duration) -> RequestReassembly {
        let (queue, received) = mpsc::channel(QUEUE);
        let table = SegmentedRequests::new(segment_timeout)
            .with_max_segments(at.limits.max_reassembly_segments);
        let table = Arc::new(SyncMutex::new(table));
        #[cfg(test)]
        let weak = Arc::downgrade(&table);
        let outlets = Outlets {
            cov_tx: at.cov_tx.clone(),
            event_tx: at.event_tx.clone(),
            policy: Arc::clone(at.policy),
            decode_warning: WarnThrottle::default(),
            response_warning: WarnThrottle::default(),
        };
        let task = tokio::spawn(Self::reassemble(
            Arc::downgrade(at.network),
            received,
            table,
            outlets,
        ));
        RequestReassembly {
            queue,
            task: Some(task),
            full_drops: 0,
            #[cfg(test)]
            table: weak,
        }
    }

    /// The reassembly task: take each segment and Abort in turn, and end
    /// each transfer when its timer is due.
    async fn reassemble(
        network: Weak<NetworkLayer<T>>,
        mut queue: mpsc::Receiver<Handed>,
        table: Arc<SyncMutex<SegmentedRequests>>,
        outlets: Outlets,
    ) {
        loop {
            let deadline = table.lock().next_deadline();
            let handed = match deadline {
                Some(at) => {
                    tokio::select! {
                        biased;
                        handed = queue.recv() => handed,
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(at)) => {
                            let stalled = table.lock().expire(now());
                            if let Some(network) = network.upgrade() {
                                for opening in stalled {
                                    Self::abort_opening(&network, opening, AbortReason::OTHER)
                                        .await;
                                }
                            }
                            continue;
                        }
                    }
                }
                None => queue.recv().await,
            };
            let Some(handed) = handed else {
                return;
            };
            let Some(network) = network.upgrade() else {
                return;
            };
            match handed {
                Handed::Segment {
                    source_mac,
                    source_network,
                    reply,
                    request,
                } => {
                    Self::receive_request_segment(
                        &network,
                        &table,
                        &outlets,
                        &source_mac,
                        &source_network,
                        reply,
                        request,
                    )
                    .await;
                }
                Handed::Abort {
                    source_mac,
                    source_network,
                    invoke_id,
                    provenance,
                } => {
                    debug!(
                        invoke_id,
                        "A requester's Abort ends its request's reassembly"
                    );
                    table
                        .lock()
                        .abort(&source_mac, source_network.as_ref(), invoke_id, provenance);
                }
            }
        }
    }

    /// Take one segment of a confirmed request.
    #[allow(clippy::too_many_arguments)]
    async fn receive_request_segment(
        network: &Arc<NetworkLayer<T>>,
        table: &SyncMutex<SegmentedRequests>,
        outlets: &Outlets,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        request: ConfirmedRequestPdu,
    ) {
        let invoke_id = request.invoke_id;
        let receipt = table.lock().receive(
            Segment {
                source_mac,
                source_network: source_network.as_ref(),
                route: reply.route(),
                request: &request,
            },
            now(),
        );
        match receipt {
            // On MS/TP the reply sender drops unused, so the link answers
            // with Reply Postponed.
            Receipt::Saved | Receipt::Discarded => {}
            Receipt::Ack(ack) => {
                if ack.negative_ack {
                    warn_throttled!(
                        outlets.response_warning,
                        invoke_id,
                        seq = ack.sequence_number,
                        "Asking again for segments of a confirmed request"
                    );
                }
                Self::send_request_segment_ack(network, source_mac, source_network, reply, ack)
                    .await;
            }
            Receipt::Abort(reason) => {
                warn_throttled!(
                    outlets.response_warning,
                    invoke_id,
                    reason = reason.to_raw(),
                    "Aborting a segmented confirmed request"
                );
                Self::send_server_abort(
                    network,
                    source_mac,
                    source_network,
                    reply,
                    invoke_id,
                    reason,
                )
                .await;
            }
            Receipt::Retired(opening) => {
                warn_throttled!(
                    outlets.response_warning,
                    invoke_id,
                    "Aborting a segmented confirmed request whose segment changed route"
                );
                Self::abort_opening(network, *opening, AbortReason::INVALID_APDU_IN_THIS_STATE)
                    .await;
            }
            Receipt::Complete(done) => {
                Self::send_request_segment_ack(
                    network,
                    source_mac,
                    source_network,
                    reply,
                    done.ack,
                )
                .await;
                let Opening {
                    source_mac,
                    source_network,
                    route,
                    ..
                } = done.opening;
                let request = done.request;
                debug!(
                    invoke_id,
                    service = request.service_choice.to_raw(),
                    bytes = request.service_request.len(),
                    "Reassembled a segmented confirmed request"
                );
                // Its answer goes back the way its first segment came.
                let answer = InboundReply::new(route, None);
                if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
                    || request.service_choice
                        == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
                {
                    Self::receive_confirmed_cov_notification(
                        network,
                        &outlets.cov_tx,
                        &outlets.policy,
                        &source_mac,
                        &source_network,
                        answer,
                        request,
                        &outlets.decode_warning,
                    )
                    .await;
                } else if request.service_choice
                    == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
                {
                    Self::receive_confirmed_event_notification(
                        network,
                        &outlets.event_tx,
                        &source_mac,
                        &source_network,
                        answer,
                        request,
                        &outlets.decode_warning,
                    )
                    .await;
                } else {
                    Self::send_confirmed_request_reject(
                        network,
                        &source_mac,
                        &source_network,
                        answer,
                        invoke_id,
                        RejectReason::UNRECOGNIZED_SERVICE,
                    )
                    .await;
                }
            }
        }
    }

    /// Send a Segment-ACK for a request segment, server bit set: here the
    /// client is the side that answers.
    async fn send_request_segment_ack(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        ack: SegmentAckPdu,
    ) {
        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &Apdu::SegmentAck(ack)) {
            warn!(error = %e, "Failed to encode a request's SegmentAck");
            return;
        }
        if let Err(e) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(error = %e, "Failed to send a request's SegmentAck");
        }
    }

    /// Abort a transfer along the path its first segment came by.
    async fn abort_opening(network: &Arc<NetworkLayer<T>>, opening: Opening, reason: AbortReason) {
        let Opening {
            source_mac,
            source_network,
            route,
            invoke_id,
        } = opening;
        Self::send_server_abort(
            network,
            &source_mac,
            &source_network,
            InboundReply::new(route, None),
            invoke_id,
            reason,
        )
        .await;
    }
}

#[cfg(test)]
#[path = "request_reassembly_tests.rs"]
mod tests;
