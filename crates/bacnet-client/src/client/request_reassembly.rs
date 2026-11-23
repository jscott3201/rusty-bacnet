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
//! The transfers live in a table the dispatch task opens on the first one
//! and shares with a timer task, both under a blocking lock no `.await`
//! holds: the dispatch task feeds it segments and a requester's Aborts, and
//! the timer task ends what has gone silent or stalled. Nothing of it is on
//! the path of an unsegmented APDU: the dispatch loop gives a segmented
//! request and a requester's Abort turns of their own, out of line, before
//! `dispatch_apdu`, whose code is left as it was without them (an earlier
//! shape that grew it cost every answer about 15 instructions); the loop
//! holds one pointer, empty until the first transfer; and the timer is a
//! task of its own, so the loop polls no timer for it (#1978 item 8). The
//! table and its timer end with the dispatch task: on
//! [`BACnetClient::stop`], on a drop, and when the node a seated client runs
//! on stops.

use std::future::Future;
use std::pin::Pin;
use std::sync::Weak;

use bacnet_endpoint_core::segmented_request::{Opening, Receipt, Segment, SegmentedRequests};
use bacnet_types::enums::AbortReason;
use tokio::sync::Notify;

use super::*;

/// The segmented requests the client answers: nothing until the first one,
/// then one pointer to the table and its timer. The dispatch loop holds it
/// beside the segmented answers' map, and reaches it only from the turns a
/// segmented request and a requester's Abort take, ahead of
/// `dispatch_apdu`, which keeps the shape it has without them.
pub(super) struct RequestTransfers {
    active: Option<Box<Active>>,
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
    /// None yet.
    pub(super) fn new(#[cfg(test)] hook: Arc<RequestTransfersHook>) -> Self {
        Self {
            active: None,
            #[cfg(test)]
            hook,
        }
    }

    /// The table, opened with its timer on first use.
    fn open<T: TransportPort + 'static>(
        &mut self,
        network: &Arc<NetworkLayer<T>>,
        segment_timeout: Duration,
        max_segments: usize,
    ) -> &Active {
        #[cfg(test)]
        let hook = &self.hook;
        self.active.get_or_insert_with(|| {
            let table = SegmentedRequests::new(segment_timeout).with_max_segments(max_segments);
            let table = Arc::new(SyncMutex::new(table));
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
            Box::new(Active {
                table,
                wake,
                _timer: AbortOnDrop(timer),
            })
        })
    }
}

/// What answering a segmented request needs, borrowed from the dispatch
/// task.
pub(super) struct RequestSegmentContext<'a, T: TransportPort + 'static> {
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    /// The client's transactions, whose segment timeout a new table takes.
    pub(super) tsm: &'a Arc<SyncMutex<Tsm>>,
    pub(super) cov_tx: &'a broadcast::Sender<COVEvent>,
    pub(super) event_tx: &'a broadcast::Sender<ReceivedEventNotification>,
    pub(super) policy: &'a ConfirmedCOVNotificationAckPolicy,
    pub(super) decode_warning: &'a WarnThrottle,
    pub(super) response_warning: &'a WarnThrottle,
    /// Whether the client receives segments, and how many of one request
    /// it keeps.
    pub(super) limits: ResponseLimits,
}

/// One segment, where it came from and its reply path.
struct RequestSegment<'a> {
    source_mac: &'a [u8],
    source_network: &'a Option<NpduAddress>,
    reply: InboundReply,
    request: ConfirmedRequestPdu,
}

/// Now, on tokio's clock, so a paused test steps the timers.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// A requester's Abort (server bit clear) ends the reassembly of its
    /// request, if one is open (Clause 5.4.5.2), and draws nothing back.
    /// The dispatch loop's own turn for it, ahead of `dispatch_apdu`, which
    /// only logs such an Abort.
    #[cold]
    #[inline(never)]
    pub(super) fn requester_abort(
        transfers: &RequestTransfers,
        received: &bacnet_network::layer::ReceivedApdu,
        abort: &AbortPdu,
    ) {
        debug!(
            invoke_id = abort.invoke_id,
            "Abort PDU with server=FALSE ends its request's reassembly"
        );
        if let Some(active) = &transfers.active {
            active.table.lock().abort(
                &received.source_mac,
                received.source_network.as_ref(),
                abort.invoke_id,
                received.provenance.clone(),
            );
        }
    }

    /// The dispatch loop's turn for one segment of a confirmed request
    /// (see the [module docs](self)), ahead of `dispatch_apdu`. One
    /// addressed to a group is ignored, as an unsegmented one is.
    #[cold]
    #[inline(never)]
    pub(super) fn segmented_request<'a>(
        context: RequestSegmentContext<'a, T>,
        transfers: &'a mut RequestTransfers,
        received: bacnet_network::layer::ReceivedApdu,
        request: ConfirmedRequestPdu,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if received.is_group {
                debug!(
                    invoke_id = request.invoke_id,
                    service = request.service_choice.to_raw(),
                    "Ignoring group-addressed ConfirmedRequest"
                );
                return;
            }
            let reply = InboundReply::new(received.response_route(), received.reply_tx);
            Self::receive_request_segment(
                context,
                transfers,
                RequestSegment {
                    source_mac: &received.source_mac,
                    source_network: &received.source_network,
                    reply,
                    request,
                },
            )
            .await;
        })
    }

    /// Take one segment of a confirmed request.
    async fn receive_request_segment(
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
        if !context.limits.segmented_response_accepted {
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
        let segment_timeout = context.tsm.lock().segment_timeout();
        let active = transfers.open(
            network,
            segment_timeout,
            context.limits.max_reassembly_segments,
        );
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
                if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
                    || request.service_choice
                        == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
                {
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
                } else if request.service_choice
                    == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
                {
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
