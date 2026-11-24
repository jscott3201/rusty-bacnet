//! Opening route survives request reassembly and every segmented reply path.
use super::*;
use crate::server::segmented_send::segmented_transaction_key;
use bacnet_endpoint_core::segmented_request::{SegmentedRequests, transfer_key};
use bacnet_types::enums::EngineeringUnits;

/// The Device's default segment timeout.
const TSEG: Duration = Duration::from_secs(5);

/// Take one received segment through the dispatch loop's segment branch.
async fn take(
    context: &DispatchContext<TestTransport>,
    table: &mut SegmentedRequests,
    received: ReceivedApdu,
) {
    let source = received.source_mac.clone();
    let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu.clone()).unwrap() else {
        panic!("a segment of a confirmed request");
    };
    BACnetServer::receive_request_segment(
        &context.services.network,
        context,
        &context.services.config,
        table,
        &context.services.warnings.segment,
        &source,
        &request,
        received,
    )
    .await;
}

/// `read(invoke)` as segment zero of a window of one, carrying the first
/// half of its service data, and the whole of that data.
fn first_half(invoke: u8) -> (ConfirmedRequestPdu, Bytes) {
    let Apdu::ConfirmedRequest(mut first) = read(invoke) else {
        unreachable!()
    };
    let full = first.service_request.clone();
    first.segmented = true;
    first.more_follows = true;
    first.sequence_number = Some(0);
    first.proposed_window_size = Some(1);
    first.service_request = full.slice(..full.len() / 2);
    (first, full)
}

/// The last segment of what `first_half` began.
fn second_half(first: &ConfirmedRequestPdu, full: &Bytes) -> ConfirmedRequestPdu {
    let mut last = first.clone();
    last.more_follows = false;
    last.sequence_number = Some(1);
    last.service_request = full.slice(full.len() / 2..);
    last
}

/// A segment from the same routed requester to the same endpoint, arriving
/// on router port `arrival_port` from next hop 10.
async fn alternate(
    routed: &mut Routed,
    arrival_port: usize,
    request: ConfirmedRequestPdu,
) -> ReceivedApdu {
    routed
        .receive_via(
            arrival_port,
            &[10],
            Some(address(9000, &[7])),
            Some(address(2000, &[2])),
            &Apdu::ConfirmedRequest(request),
            None,
        )
        .await
}

fn analog_input() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(7, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    db
}

#[tokio::test]
async fn r7_segment_opening_context_drives_ack_reassembly_and_retarget_abort() {
    let mut routed = Routed::new().await;
    let context = context(analog_input(), ServerConfig::default());
    let (first, full) = first_half(4);
    let mut table = SegmentedRequests::new(TSEG);
    let opening = routed
        .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
        .await;
    take(&context, &mut table, opening).await;
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::SegmentAck(ack) if ack.sequence_number == 0
    ));
    let last = second_half(&first, &full);
    let final_envelope = routed
        .receive(2000, &Apdu::ConfirmedRequest(last), None)
        .await;
    take(&context, &mut table, final_envelope).await;
    assert!(table.is_empty());
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::SegmentAck(ack) if ack.sequence_number == 1
    ));
    // Reassembled byte for byte, or the read would draw no ComplexAck.
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::ComplexAck(_)
    ));
    settled(&context).await;

    let opening = routed
        .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
        .await;
    take(&context, &mut table, opening).await;
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::SegmentAck(_)
    ));
    // The same transfer through another local endpoint fails closed, and
    // its Abort goes back the way the first segment came.
    let retarget = routed
        .receive(3000, &Apdu::ConfirmedRequest(first), None)
        .await;
    take(&context, &mut table, retarget).await;
    assert!(table.is_empty());
    assert!(
        matches!(routed.response(2000,4).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::INVALID_APDU_IN_THIS_STATE)
    );
    assert!(routed.wire.try_recv().is_err());
    routed.router.stop().await.unwrap();
}

/// A segment repeated while its request executes is acknowledged again only
/// by a route of the same origin as the request's (#2751): through another
/// local endpoint it finds no transfer and draws an Abort.
#[tokio::test]
async fn r7_segment_repeat_while_executing_needs_the_opening_origin() {
    let mut routed = Routed::new().await;
    let context = context(analog_input(), ServerConfig::default());
    let (first, full) = first_half(8);
    let last = second_half(&first, &full);
    let mut table = SegmentedRequests::new(TSEG);
    // The ReadProperty waits for this.
    let guard = context.services.db.write().await;
    let opening = routed
        .receive(2000, &Apdu::ConfirmedRequest(first), None)
        .await;
    take(&context, &mut table, opening).await;
    assert!(matches!(
        routed.response(2000, 8).await.0,
        Apdu::SegmentAck(ack) if ack.sequence_number == 0
    ));
    let whole = routed
        .receive(2000, &Apdu::ConfirmedRequest(last.clone()), None)
        .await;
    take(&context, &mut table, whole).await;
    assert!(matches!(
        routed.response(2000, 8).await.0,
        Apdu::SegmentAck(ack) if ack.sequence_number == 1
    ));
    let again = routed
        .receive(2000, &Apdu::ConfirmedRequest(last.clone()), None)
        .await;
    take(&context, &mut table, again).await;
    assert!(matches!(
        routed.response(2000, 8).await.0,
        Apdu::SegmentAck(ack) if ack.sequence_number == 1 && !ack.negative_ack
    ));
    let elsewhere = routed
        .receive(3000, &Apdu::ConfirmedRequest(last), None)
        .await;
    take(&context, &mut table, elsewhere).await;
    assert!(
        matches!(routed.response(3000, 8).await.0, Apdu::Abort(x) if x.abort_reason == AbortReason::INVALID_APDU_IN_THIS_STATE)
    );
    drop(guard);
    assert!(matches!(
        routed.response(2000, 8).await.0,
        Apdu::ComplexAck(_)
    ));
    settled(&context).await;
    assert!(routed.wire.try_recv().is_err());
    routed.router.stop().await.unwrap();
}

#[tokio::test]
async fn r7_segment_capacity_abort_and_quiet_progress_expiry_keep_opening_route() {
    let mut routed = Routed::new().await;
    let context = context(ObjectDatabase::new(), ServerConfig::default());
    let (first, _) = first_half(5);
    let opening = || Apdu::ConfirmedRequest(first.clone());
    // A table that takes one segment: the next ends the transfer.
    let mut table = SegmentedRequests::new(TSEG).with_max_segments(1);
    take(
        &context,
        &mut table,
        routed.receive(2000, &opening(), None).await,
    )
    .await;
    assert!(matches!(
        routed.response(2000, 5).await.0,
        Apdu::SegmentAck(_)
    ));
    let mut next = first.clone();
    next.sequence_number = Some(1);
    next.service_request = Bytes::from_static(&[1]);
    let next = routed
        .receive(2000, &Apdu::ConfirmedRequest(next), None)
        .await;
    take(&context, &mut table, next).await;
    assert!(table.is_empty());
    assert!(
        matches!(routed.response(2000,5).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::BUFFER_OVERFLOW)
    );

    // No progress for 16 s: the Abort goes the opening's way.
    let mut table = SegmentedRequests::new(TSEG);
    take(
        &context,
        &mut table,
        routed.receive(2000, &opening(), None).await,
    )
    .await;
    assert!(matches!(
        routed.response(2000, 5).await.0,
        Apdu::SegmentAck(_)
    ));
    let now = crate::runtime_clock::now() + Duration::from_secs(16);
    BACnetServer::<TestTransport>::reap_expired_requests(
        &context.services.network,
        &mut table,
        now,
    )
    .await;
    assert!(table.is_empty());
    assert!(
        matches!(routed.response(2000,5).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::OTHER)
    );

    // Four segment timeouts of 1 s run out first, and end it silently.
    let mut table = SegmentedRequests::new(Duration::from_secs(1));
    take(
        &context,
        &mut table,
        routed.receive(2000, &opening(), None).await,
    )
    .await;
    assert!(matches!(
        routed.response(2000, 5).await.0,
        Apdu::SegmentAck(_)
    ));
    let now = crate::runtime_clock::now() + Duration::from_secs(16);
    BACnetServer::<TestTransport>::reap_expired_requests(
        &context.services.network,
        &mut table,
        now,
    )
    .await;
    assert!(
        table.is_empty() && routed.wire.try_recv().is_err(),
        "protocol inactivity expiry stays silent"
    );
    routed.router.stop().await.unwrap();
}

#[tokio::test]
async fn r7_segmented_ack_retry_and_terminal_abort_use_one_saved_route() {
    let mut routed = Routed::new().await;
    let context = context(ObjectDatabase::new(), ServerConfig::default());
    let received = routed.receive(2000, &read(6), None).await;
    let route = received.response_route();
    let network = context.services.network.clone();
    let registry = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let permits = Arc::new(Semaphore::new(1));
    let registry_task = registry.clone();
    let permits_task = permits.clone();
    let task = tokio::spawn(async move {
        BACnetServer::<TestTransport>::send_segmented_complex_ack_with_options(
            SegmentedSendResources {
                network: &network,
                seg_ack_senders: &registry_task,
                seg_send_permits: &permits_task,
            },
            ResponseTarget {
                source_mac: &received.source_mac,
                source_network: received.source_network.as_ref(),
                route: &route,
            },
            ComplexAckParams {
                invoke_id: 6,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                client_max_apdu: 50,
                client_max_segments: None,
            },
            &[0xaa; 70],
            SegmentedSendOptions {
                segment_timeout: Duration::from_millis(100),
                max_retries: 1,
            },
            None,
        )
        .await;
    });
    let (first, _) = routed.response(2000, 6).await;
    assert!(matches!(first,Apdu::ComplexAck(x) if x.segmented&&x.more_follows));
    let handle = registry.lock().values().next().unwrap().clone();
    let ack = SegmentAckPdu {
        sent_by_server: false,
        invoke_id: 6,
        sequence_number: 0,
        actual_window_size: 1,
        negative_ack: false,
    };
    handle.segment_ack_tx.send(ack.clone()).await.unwrap();
    let (second, bytes) = routed.response(2000, 6).await;
    assert!(matches!(second,Apdu::ComplexAck(x) if x.sequence_number==Some(1)));
    let nak = SegmentAckPdu {
        negative_ack: true,
        ..ack
    };
    handle.segment_ack_tx.send(nak.clone()).await.unwrap();
    assert_eq!(
        routed.response(2000, 6).await.1,
        bytes,
        "retry preserves the original endpoint and APDU"
    );
    handle.segment_ack_tx.send(nak).await.unwrap();
    assert!(matches!(routed.response(2000, 6).await.0, Apdu::Abort(_)));
    bounded(task).await.unwrap();
    assert!(registry.lock().is_empty());
    assert_eq!(permits.available_permits(), 1);
    let key = segmented_transaction_key(
        &[9],
        Some(&address(9000, &[7])),
        6,
        bacnet_transport::port::TransportProvenance::unverified(),
    );
    assert!(!registry.lock().contains_key(&key));
    routed.router.stop().await.unwrap();
}

#[tokio::test]
async fn r7_segment_alternate_return_paths_keep_opening_reply_plan() {
    for arrival_port in [0, 2] {
        let mut routed = Routed::new().await;
        let context = context(analog_input(), ServerConfig::default());
        let (first, full) = first_half(17);
        let mut table = SegmentedRequests::new(TSEG);
        let opening = routed
            .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
            .await;
        let key = transfer_key(
            &opening.source_mac,
            opening.source_network.as_ref(),
            17,
            opening.provenance.clone(),
        );
        take(&context, &mut table, opening).await;
        assert!(matches!(
            routed.response(2000, 17).await.0,
            Apdu::SegmentAck(_)
        ));
        let continuation = alternate(&mut routed, arrival_port, second_half(&first, &full)).await;
        assert_eq!(
            key,
            transfer_key(
                &continuation.source_mac,
                continuation.source_network.as_ref(),
                17,
                continuation.provenance.clone(),
            ),
            "canonical routed requester did not change"
        );
        take(&context, &mut table, continuation).await;
        assert!(
            table.is_empty(),
            "same routed requester and addressed endpoint may change return path"
        );
        // The segment's own Segment-ACK goes back the way it came...
        let alternate_port = if arrival_port == 0 { 1000 } else { 3000 };
        let (port, bytes, peer) = bounded(routed.wire.recv()).await.unwrap();
        assert_eq!((port, peer.as_slice()), (alternate_port, &[10][..]));
        assert!(matches!(
            decode_response(bytes, 2000, 17).0,
            Apdu::SegmentAck(ack) if ack.sequence_number == 1
        ));
        assert!(
            matches!(routed.response(2000, 17).await.0, Apdu::ComplexAck(_)),
            "completion uses original port 1000 and peer 9, never the alternate path"
        );
        settled(&context).await;

        let opening = routed
            .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
            .await;
        take(&context, &mut table, opening).await;
        assert!(matches!(
            routed.response(2000, 17).await.0,
            Apdu::SegmentAck(_)
        ));
        // Segment zero again, the alternate way: asked for again, that way.
        let continuation = alternate(&mut routed, arrival_port, first.clone()).await;
        take(&context, &mut table, continuation).await;
        let (port, bytes, peer) = bounded(routed.wire.recv()).await.unwrap();
        assert_eq!((port, peer.as_slice()), (alternate_port, &[10][..]));
        assert!(matches!(
            decode_response(bytes, 2000, 17).0,
            Apdu::SegmentAck(ack) if ack.negative_ack
        ));
        assert_eq!(table.len(), 1);
        let now = crate::runtime_clock::now() + Duration::from_secs(16);
        BACnetServer::<TestTransport>::reap_expired_requests(
            &context.services.network,
            &mut table,
            now,
        )
        .await;
        assert!(
            matches!(routed.response(2000,17).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::OTHER)
        );
        assert!(table.is_empty() && routed.wire.try_recv().is_err());
        routed.router.stop().await.unwrap();
    }
}

#[tokio::test]
async fn r7_segment_origin_comparison_keeps_implicit_and_local_port_isolation() {
    let mut routed = Routed::new().await;
    let request = read(18);
    let endpoint = Some(address(2000, &[2]));
    let local_a = routed
        .receive_via(0, &[9], None, endpoint.clone(), &request, None)
        .await;
    let local_c = routed
        .receive_via(2, &[9], None, endpoint.clone(), &request, None)
        .await;
    assert!(
        !local_a
            .response_route()
            .same_origin(&local_c.response_route()),
        "equal link addresses on different ports are different local requesters"
    );
    let local_peer = routed
        .receive_via(0, &[10], None, endpoint.clone(), &request, None)
        .await;
    assert!(
        !local_a
            .response_route()
            .same_origin(&local_peer.response_route())
    );
    let implicit = routed
        .receive_via(0, &[9], Some(address(9000, &[7])), None, &request, None)
        .await;
    let implicit_peer = routed
        .receive_via(0, &[10], Some(address(9000, &[7])), None, &request, None)
        .await;
    assert!(
        implicit
            .response_route()
            .same_origin(&implicit_peer.response_route()),
        "a routed requester can change next hop on the same implicit endpoint"
    );
    let implicit_port = routed
        .receive_via(2, &[10], Some(address(9000, &[7])), None, &request, None)
        .await;
    assert!(
        !implicit
            .response_route()
            .same_origin(&implicit_port.response_route()),
        "without an explicit endpoint, a different ingress port names another local endpoint"
    );
    let remote = routed
        .receive_via(
            0,
            &[9],
            Some(address(9000, &[7])),
            endpoint.clone(),
            &request,
            None,
        )
        .await;
    let different_remote = routed
        .receive_via(
            0,
            &[9],
            Some(address(9000, &[8])),
            endpoint.clone(),
            &request,
            None,
        )
        .await;
    assert!(
        !remote
            .response_route()
            .same_origin(&different_remote.response_route())
    );
    let mut replacement = Routed::new().await;
    let other_router = replacement
        .receive_via(0, &[9], Some(address(9000, &[7])), endpoint, &request, None)
        .await;
    assert!(
        !remote
            .response_route()
            .same_origin(&other_router.response_route()),
        "a new router instance cannot inherit an opening context"
    );
    routed.router.stop().await.unwrap();
    replacement.router.stop().await.unwrap();
}
