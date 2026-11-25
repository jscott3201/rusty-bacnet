//! The server's side of a segmented confirmed request (Clause 5.4.5): the
//! dispatch loop's branches for a segment, a requester's Abort and the
//! reassembly timers, over the shared receiver
//! [`SegmentedRequests`](bacnet_endpoint_core::segmented_request) (#2742).
//!
//! The dispatch loop owns the table and drives it inline: no task, no lock.
//! Its limits are the receiver's fixed ones (128 transfers, 16 from one peer,
//! 4 MiB of saved segments, 256 segments, the 16 s no-progress Abort and the
//! silent end past four segment timeouts). The server takes up to the
//! sequence space whatever its Device states as Max_Segments_Accepted: more
//! than it states is permissive, while a 257th segment would land on segment
//! zero's slot.

use super::*;
use bacnet_endpoint_core::segmented_request::{Opening, Receipt, Segment, SegmentedRequests};
use bacnet_network::layer::ReceivedApdu;
use bacnet_network::response_route::ResponseRoute;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// End every transfer due at `now`, and send the Aborts that ending
    /// draws. The table gives up every due transfer, and its saved segments,
    /// before the first Abort awaits. The dispatch loop owns this future, so
    /// a stop cancels it.
    pub(super) async fn reap_expired_requests(
        network: &Arc<NetworkLayer<T>>,
        receivers: &mut SegmentedRequests,
        now: Instant,
    ) {
        Self::abort_expired(network, receivers.expire(now)).await;
    }

    /// Send an Abort (OTHER) along each expired transfer's opening route; a
    /// failed send leaves the rest to go.
    pub(super) async fn abort_expired(network: &Arc<NetworkLayer<T>>, expired: Vec<Opening>) {
        for opening in expired {
            Self::abort_opening(network, &opening, AbortReason::OTHER).await;
        }
    }

    /// The body of the dispatch loop's branch for a segment of a confirmed
    /// request: take it into `receivers` and send what it draws, along the
    /// segment's own path. A whole request goes on to `dispatch` with the
    /// last segment's envelope and its transfer's opening route, so its
    /// answer goes back the way the first segment came.
    ///
    /// `#[inline]`, so each crate that compiles the dispatch loop compiles
    /// its own copy, and `#[cold]`, so the loop's plain path is laid out
    /// without it: an `#[inline(never)]` generic hand-off cost the client's
    /// plain requests about 28 instructions each in #2646, by bringing in
    /// another crate's copy of the B/IP send path.
    #[cold]
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn receive_request_segment(
        network: &Arc<NetworkLayer<T>>,
        dispatch_context: &DispatchContext<T>,
        config: &ServerConfig,
        receivers: &mut SegmentedRequests,
        warning: &WarnThrottle,
        source_mac: &MacAddr,
        request: &ConfirmedRequestPdu,
        mut received: ReceivedApdu,
    ) {
        let invoke_id = request.invoke_id;
        let route = received.response_route();
        // A device that takes no segments answers the first one it meets
        // with this Abort and keeps nothing (Clause 5.4.5.1), so its table
        // stays empty.
        let segmentation = config.device.segmentation();
        if segmentation != Segmentation::SEGMENTED_BOTH
            && segmentation != Segmentation::SEGMENTED_RECEIVE
        {
            Self::send_server_abort(
                network,
                source_mac,
                received.source_network.as_ref(),
                &route,
                invoke_id,
                AbortReason::SEGMENTATION_NOT_SUPPORTED,
            )
            .await;
            return;
        }
        // A segment of a request already whole and still executing is
        // acknowledged again (Clause 5.4.5.3, #2751); while its answer goes
        // out in segments, a segment zero opens a transfer (#2770).
        if !receivers.is_open(
            source_mac,
            received.source_network.as_ref(),
            invoke_id,
            received.provenance.clone(),
        ) && let Some((sequence_number, actual_window_size)) =
            dispatch_context.confirmed_request_tracker.repeated_segment(
                source_mac,
                received.source_network.as_ref(),
                &received.provenance,
                &route,
                invoke_id,
                request.sequence_number.unwrap_or(0) == 0,
            )
        {
            debug!(
                invoke_id,
                seq = request.sequence_number,
                "Acknowledging again a segment of a request still executing"
            );
            let ack = SegmentAckPdu {
                negative_ack: false,
                sent_by_server: true,
                invoke_id,
                sequence_number,
                actual_window_size,
            };
            let source_network = received.source_network.as_ref();
            Self::send_request_segment_ack(network, source_mac, source_network, &route, ack).await;
            return;
        }
        let receipt = receivers.receive(
            Segment {
                source_mac,
                source_network: received.source_network.as_ref(),
                route: &route,
                request,
            },
            runtime_clock::now(),
        );
        match receipt {
            Receipt::Saved => {}
            Receipt::Discarded => {
                debug!(
                    invoke_id,
                    seq = request.sequence_number,
                    "Discarding a duplicate segment"
                );
            }
            Receipt::Ack(ack) => {
                if ack.negative_ack {
                    warn_throttled!(
                        warning,
                        invoke_id,
                        seq = ack.sequence_number,
                        "Asking again for segments of a confirmed request"
                    );
                }
                Self::send_request_segment_ack(
                    network,
                    source_mac,
                    received.source_network.as_ref(),
                    &route,
                    ack,
                )
                .await;
            }
            Receipt::Abort(reason) => {
                warn_throttled!(
                    warning,
                    invoke_id,
                    reason = reason.to_raw(),
                    "Aborting a segmented confirmed request"
                );
                Self::send_server_abort(
                    network,
                    source_mac,
                    received.source_network.as_ref(),
                    &route,
                    invoke_id,
                    reason,
                )
                .await;
            }
            Receipt::Retired(opening) => {
                warn_throttled!(
                    warning,
                    invoke_id,
                    "Aborting a segmented confirmed request whose segment changed route"
                );
                Self::abort_opening(network, &opening, AbortReason::INVALID_APDU_IN_THIS_STATE)
                    .await;
            }
            Receipt::Complete(done) => {
                // On a node, the client hears the whole request where it
                // hears a copy of one that comes whole, as with a
                // ConfirmedEventNotification (#2743). Handed on before
                // anything is sent, so it is in the client's queue before
                // the answer can draw the requester's next notification;
                // it never waits.
                if let Some(copies) = &dispatch_context.services.client_copies {
                    copies.reassembled(&received, &done.request);
                }
                let (last_sequence, window) =
                    (done.ack.sequence_number, done.ack.actual_window_size);
                Self::send_request_segment_ack(
                    network,
                    source_mac,
                    received.source_network.as_ref(),
                    &route,
                    done.ack,
                )
                .await;
                debug!(
                    invoke_id,
                    payload_len = done.request.service_request.len(),
                    "Reassembled segmented ConfirmedRequest"
                );
                let source_network = received.source_network.clone();
                let provenance = received.provenance.clone();
                let opening = done.opening.route.clone();
                received.set_response_route(done.opening.route);
                let tracker = &dispatch_context.confirmed_request_tracker;
                let admission = tracker.next_admission();
                // On the heap: the loop holds this future in its own state,
                // which this branch's notes would otherwise grow.
                Box::pin(Self::dispatch(
                    dispatch_context,
                    source_mac,
                    Apdu::ConfirmedRequest(done.request),
                    received,
                ))
                .await;
                // Pending now as `admission`, unless already answered or
                // never admitted (then nothing is noted and a repeat draws
                // an Abort, as in IDLE).
                tracker.note_segmented(
                    admission,
                    source_mac,
                    source_network.as_ref(),
                    provenance,
                    opening,
                    invoke_id,
                    last_sequence,
                    window,
                );
            }
        }
    }

    /// Send a Segment-ACK for a request's segment, along `route`.
    async fn send_request_segment_ack(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        route: &ResponseRoute,
        ack: SegmentAckPdu,
    ) {
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &Apdu::SegmentAck(ack)).expect("valid APDU encoding");
        if let Err(e) =
            Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network, route)
                .await
        {
            warn!(
                error = %e,
                "Failed to send SegmentAck for segmented request"
            );
        }
    }

    /// Abort a transfer along the route its first segment came by.
    async fn abort_opening(network: &Arc<NetworkLayer<T>>, opening: &Opening, reason: AbortReason) {
        Self::send_server_abort(
            network,
            &opening.source_mac,
            opening.source_network.as_ref(),
            &opening.route,
            opening.invoke_id,
            reason,
        )
        .await;
    }

    /// Send a `'server' = TRUE` Abort back along the request's path.
    ///
    /// Every Abort this dispatch loop originates answers a client's
    /// request, so the flag is always TRUE (Clause 20.1.9.1).
    pub(super) async fn send_server_abort(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        route: &ResponseRoute,
        invoke_id: u8,
        abort_reason: AbortReason,
    ) {
        let abort_pdu = Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason,
        });
        let mut abort_buf = BytesMut::new();
        encode_apdu(&mut abort_buf, &abort_pdu).expect("valid APDU encoding");
        if let Err(e) = Self::send_confirmed_response_apdu(
            network,
            &abort_buf,
            source_mac,
            source_network,
            route,
        )
        .await
        {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send Abort");
        }
    }
}

/// Check the Device's segment timeout (Tseg) before any network I/O: four of
/// them, the silence that ends a transfer, must be a representable deadline.
pub(super) fn validate_segment_timeout(config: &ServerConfig) -> Result<Duration, Error> {
    config.device.validate()?;
    let segment_timeout = Duration::from_millis(config.device.apdu_segment_timeout_ms());
    let milliseconds = config
        .device
        .apdu_segment_timeout_ms()
        .checked_mul(4)
        .ok_or(Error::invalid_config(
            "receive segment timeout overflows milliseconds",
        ))?;
    let receive_timeout = Duration::from_millis(milliseconds);
    runtime_clock::now()
        .checked_add(receive_timeout)
        .and_then(|deadline| deadline.checked_add(Duration::from_nanos(1)))
        .ok_or(Error::invalid_config(
            "receive segment deadline is not representable",
        ))?;
    Ok(segment_timeout)
}

/// Sleep until the table's next deadline; forever with none.
pub(super) async fn wait_receive_deadline(receivers: &SegmentedRequests) {
    match receivers.next_deadline() {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
}

/// A requester's Abort (server bit clear) ends the transfer it names
/// (Clause 5.4.5.2). A side effect only: the Abort still goes on to
/// `dispatch`, which ends the segmented answers it names.
pub(super) fn remove_peer_aborted_request(
    receivers: &mut SegmentedRequests,
    received: &ReceivedApdu,
    decoded: &Apdu,
) {
    if let Apdu::Abort(abort) = decoded
        && !abort.sent_by_server
    {
        receivers.abort(
            &received.source_mac,
            received.source_network.as_ref(),
            abort.invoke_id,
            received.provenance.clone(),
        );
    }
}
