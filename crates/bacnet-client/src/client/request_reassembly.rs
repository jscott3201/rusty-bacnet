//! The segmented confirmed requests the client executes (#2646).
//!
//! A client executes three confirmed services: ConfirmedCOVNotification,
//! ConfirmedCOVNotificationMultiple and ConfirmedEventNotification (beside a
//! server role, the COV notifications only). A notifier may send any of them
//! as segments, a COV-multiple with many values most of all. When the client
//! receives segments ([`ClientConfig::segmentation`]), it
//! reassembles such a request on the answering side's rules
//! ([`SegmentedRequests`], the full server's) and then executes it as one
//! that came whole: the same decoding, publishing and answer. Otherwise, and
//! for a segmented request of any other service, it answers the first
//! segment with an Abort (SEGMENTATION_NOT_SUPPORTED), as it always has.
//!
//! The transfers live in a table the dispatch task opens on the first one
//! and shares with a timer task, both under a blocking lock no `.await`
//! holds: the dispatch task feeds it segments and a requester's Aborts, and
//! the timer task ends what has gone silent or stalled. Nothing of it is on
//! the path of an unsegmented APDU: the dispatch task reaches it only from
//! the branches a segmented request and a requester's Abort take, and the
//! timer is a task of its own, so the dispatch loop polls no timer for it
//! (#1978 item 8). The table and its timer end with the dispatch task: on
//! [`BACnetClient::stop`], on a drop, and when the node a seated client runs
//! on stops.

use std::sync::Weak;

use bacnet_endpoint_core::segmented_request::{Opening, Receipt, Segment, SegmentedRequests};
use bacnet_types::enums::AbortReason;
use tokio::sync::Notify;

use super::*;

/// The dispatch task's reassembly state: the segmented answers to the
/// client's own requests, then the segmented requests it executes.
///
/// `repr(C)` keeps `responses` first, so the path of an unsegmented answer
/// reaches it through the very pointer it did when it was the whole state.
#[repr(C)]
pub(super) struct ReceiveState {
    /// Segmented Complex-ACKs under reassembly, by peer, invoke ID and
    /// provenance.
    pub(super) responses: HashMap<SegKey, SegmentedReceiveState>,
    /// Segmented requests the client executes.
    pub(super) requests: RequestTransfers,
}

impl ReceiveState {
    /// Empty state for a client whose segment timeout is `segment_timeout`.
    pub(super) fn new(
        segment_timeout: Duration,
        #[cfg(test)] hook: Arc<RequestTransfersHook>,
    ) -> Self {
        Self {
            responses: HashMap::new(),
            requests: RequestTransfers {
                segment_timeout,
                active: None,
                #[cfg(test)]
                hook,
            },
        }
    }
}

/// The segmented requests the client executes, opened on the first one.
pub(super) struct RequestTransfers {
    /// The client's segment timeout: four of them without a segment end a
    /// transfer.
    segment_timeout: Duration,
    active: Option<Active>,
    #[cfg(test)]
    hook: Arc<RequestTransfersHook>,
}

/// The table and the timer that ends its transfers.
struct Active {
    table: Arc<SyncMutex<SegmentedRequests>>,
    /// Wakes the timer when a transfer opens: it sleeps with no deadline
    /// while the table is empty.
    wake: Arc<Notify>,
    _timer: AbortOnDrop,
}

/// Aborts the timer task when the dispatch task's state drops, however the
/// dispatch task ends.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl RequestTransfers {
    /// End the transfer a requester's Abort names (Clause 5.4.5.2), if one
    /// is open.
    pub(super) fn abort(
        &self,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        provenance: TransportProvenance,
    ) {
        if let Some(active) = &self.active {
            active
                .table
                .lock()
                .abort(source_mac, source_network, invoke_id, provenance);
        }
    }

    /// The table, opened with its timer on first use.
    fn open<T: TransportPort + 'static>(&mut self, network: &Arc<NetworkLayer<T>>) -> &Active {
        let segment_timeout = self.segment_timeout;
        #[cfg(test)]
        let hook = &self.hook;
        self.active.get_or_insert_with(|| {
            let table = Arc::new(SyncMutex::new(SegmentedRequests::new(segment_timeout)));
            let wake = Arc::new(Notify::new());
            #[cfg(test)]
            {
                *hook.table.lock().unwrap() = Some(Arc::downgrade(&table));
            }
            let timer = tokio::spawn(BACnetClient::<T>::end_stale_transfers(
                Arc::downgrade(network),
                Arc::downgrade(&table),
                Arc::clone(&wake),
            ));
            Active {
                table,
                wake,
                _timer: AbortOnDrop(timer),
            }
        })
    }
}

/// What executing a reassembled request needs, borrowed from the dispatch
/// task.
pub(super) struct RequestSegmentContext<'a, T: TransportPort + 'static> {
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) cov_tx: &'a broadcast::Sender<COVEvent>,
    pub(super) event_tx: &'a broadcast::Sender<ReceivedEventNotification>,
    pub(super) policy: &'a ConfirmedCOVNotificationAckPolicy,
    pub(super) decode_warning: &'a WarnThrottle,
    pub(super) response_warning: &'a WarnThrottle,
    /// Whether the client receives segments.
    pub(super) receives: bool,
}

/// One segment, where it came from and its reply path.
pub(super) struct RequestSegment<'a> {
    pub(super) source_mac: &'a [u8],
    pub(super) source_network: &'a Option<NpduAddress>,
    pub(super) reply: InboundReply,
    pub(super) request: ConfirmedRequestPdu,
}

/// Now, on tokio's clock, so a paused test steps the timers.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Take one segment of a confirmed request (see the [module
    /// docs](self)).
    pub(super) async fn receive_request_segment(
        context: RequestSegmentContext<'_, T>,
        transfers: &mut RequestTransfers,
        segment: RequestSegment<'_>,
    ) {
        let RequestSegment {
            source_mac,
            source_network,
            reply,
            request,
        } = segment;
        let network = context.network;
        let invoke_id = request.invoke_id;
        if !context.receives || !dispatch::executes_segmented(request.service_choice) {
            Self::send_server_abort(
                network,
                source_mac,
                source_network,
                reply,
                invoke_id,
                AbortReason::SEGMENTATION_NOT_SUPPORTED,
            )
            .await;
            return;
        }
        let active = transfers.open(network);
        let receipt = {
            let mut table = active.table.lock();
            let was_empty = table.is_empty();
            let receipt = table.receive(
                Segment {
                    source_mac,
                    source_network: source_network.as_ref(),
                    route: reply.route(),
                    request: &request,
                },
                now(),
            );
            if was_empty && !table.is_empty() {
                active.wake.notify_one();
            }
            receipt
        };
        match receipt {
            // On MS/TP the reply sender drops unused, so the link answers
            // with Reply Postponed.
            Receipt::Saved | Receipt::Discarded => {}
            Receipt::Ack(ack) => {
                if ack.negative_ack {
                    warn_throttled!(
                        context.response_warning,
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
                    context.response_warning,
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
                    context.response_warning,
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
                if dispatch::is_confirmed_event_notification(request.service_choice) {
                    Self::receive_confirmed_event_notification(
                        network,
                        context.event_tx,
                        &source_mac,
                        &source_network,
                        answer,
                        request,
                        context.decode_warning,
                    )
                    .await;
                } else {
                    Self::receive_confirmed_cov_notification(
                        network,
                        context.cov_tx,
                        context.policy,
                        &source_mac,
                        &source_network,
                        answer,
                        request,
                        context.decode_warning,
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

    /// The timer task: end each transfer when it is due, and sleep until
    /// the next is. It holds the table and the layer weakly, so neither
    /// outlives the dispatch task and the client for it, and ends once the
    /// table is gone; the dispatch task's state aborts it before that.
    async fn end_stale_transfers(
        network: Weak<NetworkLayer<T>>,
        table: Weak<SyncMutex<SegmentedRequests>>,
        wake: Arc<Notify>,
    ) {
        loop {
            let (stalled, next) = {
                let Some(table) = table.upgrade() else {
                    return;
                };
                let mut table = table.lock();
                let stalled = table.expire(now());
                (stalled, table.next_deadline())
            };
            if let Some(network) = network.upgrade() {
                for opening in stalled {
                    Self::abort_opening(&network, opening, AbortReason::OTHER).await;
                }
            }
            match next {
                Some(at) => {
                    tokio::select! {
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(at)) => {}
                        () = wake.notified() => {}
                    }
                }
                None => wake.notified().await,
            }
        }
    }
}

/// Lets a test see the client's request transfers.
#[cfg(test)]
#[derive(Default)]
pub(super) struct RequestTransfersHook {
    table: std::sync::Mutex<Option<Weak<SyncMutex<SegmentedRequests>>>>,
}

#[cfg(test)]
impl RequestTransfersHook {
    /// The transfers in progress and their saved bytes; `None` before the
    /// first transfer opened and once the table is gone.
    pub(super) fn transfers(&self) -> Option<(usize, usize)> {
        let table = self.table.lock().unwrap().as_ref()?.upgrade()?;
        let table = table.lock();
        Some((table.len(), table.saved_bytes()))
    }

    /// Whether a table was opened and is now gone.
    pub(super) fn ended(&self) -> bool {
        self.table
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|table| table.strong_count() == 0)
    }
}

#[cfg(test)]
#[path = "request_reassembly_tests.rs"]
mod tests;
