//! Opening route survives request reassembly and every segmented reply path.
use super::*;
use crate::server::segmented_receive::{
    check_opening_response_route, initial_state, RequestPayload,
};
use crate::server::segmented_send::{segmented_receive_key, segmented_transaction_key};
use bacnet_types::enums::EngineeringUnits;

fn open(
    received: &ReceivedApdu,
    request: &ConfirmedRequestPdu,
) -> (SegRecvKey, SegmentedRequestState, Option<SegmentAckPdu>) {
    let mut payload = RequestPayload::new(request);
    payload
        .save_new(0, request.service_request.clone(), Some(0))
        .unwrap();
    let key = segmented_receive_key(
        &received.source_mac,
        received.source_network.as_ref(),
        request.invoke_id,
        received.provenance,
    );
    let (state, ack) = initial_state(
        payload,
        received.response_route(),
        received.source_mac.clone(),
        received.source_network.clone(),
        request,
    );
    (key, state, ack)
}
async fn send_ack(
    context: &DispatchContext<TestTransport>,
    received: &ReceivedApdu,
    route: &bacnet_network::response_route::ResponseRoute,
    ack: SegmentAckPdu,
) {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &Apdu::SegmentAck(ack)).unwrap();
    BACnetServer::<TestTransport>::send_confirmed_response_apdu(
        &context.services.network,
        &bytes,
        &received.source_mac,
        received.source_network.as_ref(),
        route,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn r7_segment_opening_context_drives_ack_reassembly_and_retarget_abort() {
    let mut routed = Routed::new().await;
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(7, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    let context = context(db, ServerConfig::default());
    let Apdu::ConfirmedRequest(mut first) = read(4) else {
        unreachable!()
    };
    let full = first.service_request.clone();
    let middle = full.len() / 2;
    first.segmented = true;
    first.more_follows = true;
    first.sequence_number = Some(0);
    first.proposed_window_size = Some(1);
    first.service_request = full.slice(..middle);
    let received = routed
        .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
        .await;
    let (key, state, ack) = open(&received, &first);
    let mut states = HashMap::from([(key.clone(), state)]);
    send_ack(
        &context,
        &received,
        &received.response_route(),
        ack.unwrap(),
    )
    .await;
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::SegmentAck(_)
    ));
    let mut last = first.clone();
    last.more_follows = false;
    last.sequence_number = Some(1);
    last.service_request = full.slice(middle..);
    let mut final_envelope = routed
        .receive(2000, &Apdu::ConfirmedRequest(last.clone()), None)
        .await;
    check_opening_response_route(&mut states, &key, &final_envelope.response_route())
        .unwrap_or_else(|_| panic!("matching opening route"));
    let mut state = states.remove(&key).unwrap();
    state
        .payload
        .save_new(1, last.service_request, Some(middle))
        .unwrap();
    final_envelope.set_response_route(state.route.clone());
    let complete = state.payload.complete(2).unwrap();
    assert_eq!(complete.service_request, full);
    BACnetServer::dispatch(
        &context,
        &final_envelope.source_mac.clone(),
        Apdu::ConfirmedRequest(complete),
        final_envelope,
    )
    .await;
    assert!(matches!(
        routed.response(2000, 4).await.0,
        Apdu::ComplexAck(_)
    ));
    settled(&context).await;

    let (key, state, _) = open(&received, &first);
    states.insert(key.clone(), state);
    let retarget = routed
        .receive(3000, &Apdu::ConfirmedRequest(first), None)
        .await;
    let opening = match check_opening_response_route(&mut states, &key, &retarget.response_route())
    {
        Err(state) => state,
        Ok(_) => panic!("changed endpoint must fail closed"),
    };
    assert!(states.is_empty());
    BACnetServer::<TestTransport>::send_server_abort(
        &context.services.network,
        &opening.source_mac,
        opening.source_network.as_ref(),
        &opening.route,
        4,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    )
    .await;
    assert!(
        matches!(routed.response(2000,4).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::INVALID_APDU_IN_THIS_STATE)
    );
    assert!(routed.wire.try_recv().is_err());
    routed.router.stop().await;
}

#[tokio::test]
async fn r7_segment_capacity_abort_and_quiet_progress_expiry_keep_opening_route() {
    let mut routed = Routed::new().await;
    let context = context(ObjectDatabase::new(), ServerConfig::default());
    let Apdu::ConfirmedRequest(mut request) = read(5) else {
        unreachable!()
    };
    request.segmented = true;
    request.more_follows = true;
    request.sequence_number = Some(0);
    request.proposed_window_size = Some(1);
    let received = routed
        .receive(2000, &Apdu::ConfirmedRequest(request.clone()), None)
        .await;
    let (key, mut state, _) = open(&received, &request);
    assert!(state
        .payload
        .save_new(1, Bytes::from_static(&[1]), Some(usize::MAX))
        .is_err());
    BACnetServer::<TestTransport>::send_server_abort(
        &context.services.network,
        &state.source_mac,
        state.source_network.as_ref(),
        &state.route,
        5,
        AbortReason::BUFFER_OVERFLOW,
    )
    .await;
    assert!(
        matches!(routed.response(2000,5).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::BUFFER_OVERFLOW)
    );
    let now = crate::runtime_clock::now();
    state.last_activity = now;
    state.last_progress = now - Duration::from_secs(16);
    let mut states = HashMap::from([(key, state)]);
    BACnetServer::<TestTransport>::reap_expired_requests(
        &context.services.network,
        &mut states,
        now,
        Duration::from_secs(20),
    )
    .await;
    assert!(states.is_empty());
    assert!(
        matches!(routed.response(2000,5).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::OTHER)
    );
    let (key, mut state, _) = open(&received, &request);
    state.last_activity = now - Duration::from_secs(5);
    state.last_progress = now - Duration::from_secs(16);
    let mut states = HashMap::from([(key, state)]);
    BACnetServer::<TestTransport>::reap_expired_requests(
        &context.services.network,
        &mut states,
        now,
        Duration::from_secs(4),
    )
    .await;
    assert!(
        states.is_empty() && routed.wire.try_recv().is_err(),
        "protocol inactivity expiry stays silent"
    );
    routed.router.stop().await;
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
    routed.router.stop().await;
}

#[tokio::test]
async fn r7_segment_alternate_return_paths_keep_opening_reply_plan() {
    for arrival_port in [0, 2] {
        let mut routed = Routed::new().await;
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            AnalogInputObject::new(7, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
        ))
        .unwrap();
        let context = context(db, ServerConfig::default());
        let Apdu::ConfirmedRequest(mut first) = read(17) else {
            unreachable!()
        };
        let full = first.service_request.clone();
        let cut = full.len() / 2;
        first.segmented = true;
        first.more_follows = true;
        first.sequence_number = Some(0);
        first.proposed_window_size = Some(1);
        first.service_request = full.slice(..cut);
        let opening = routed
            .receive(2000, &Apdu::ConfirmedRequest(first.clone()), None)
            .await;
        let (key, state, _) = open(&opening, &first);
        let mut states = HashMap::from([(key.clone(), state)]);
        let mut last = first.clone();
        last.more_follows = false;
        last.sequence_number = Some(1);
        last.service_request = full.slice(cut..);
        let mut continuation = routed
            .receive_via(
                arrival_port,
                &[10],
                Some(address(9000, &[7])),
                Some(address(2000, &[2])),
                &Apdu::ConfirmedRequest(last.clone()),
                None,
            )
            .await;
        assert_eq!(
            key,
            segmented_receive_key(
                &continuation.source_mac,
                continuation.source_network.as_ref(),
                last.invoke_id,
                continuation.provenance,
            ),
            "canonical routed requester did not change"
        );
        assert!(
            BACnetServer::<TestTransport>::accept_segment_route(
                &context.services.network,
                &mut states,
                &key,
                &continuation.response_route(),
            )
            .await,
            "same routed requester and addressed endpoint may change return path"
        );
        let mut state = states.remove(&key).unwrap();
        state
            .payload
            .save_new(1, last.service_request, Some(cut))
            .unwrap();
        continuation.set_response_route(state.route.clone());
        let complete = state.payload.complete(2).unwrap();
        BACnetServer::dispatch(
            &context,
            &continuation.source_mac.clone(),
            Apdu::ConfirmedRequest(complete),
            continuation,
        )
        .await;
        assert!(
            matches!(routed.response(2000, 17).await.0, Apdu::ComplexAck(_)),
            "completion uses original port 1000 and peer 9, never the alternate path"
        );
        settled(&context).await;

        let (key, mut state, _) = open(&opening, &first);
        let now = crate::runtime_clock::now();
        state.last_activity = now;
        state.last_progress = now - Duration::from_secs(16);
        let mut states = HashMap::from([(key.clone(), state)]);
        let continuation = routed
            .receive_via(
                arrival_port,
                &[10],
                Some(address(9000, &[7])),
                Some(address(2000, &[2])),
                &Apdu::ConfirmedRequest(first),
                None,
            )
            .await;
        assert!(
            BACnetServer::<TestTransport>::accept_segment_route(
                &context.services.network,
                &mut states,
                &key,
                &continuation.response_route(),
            )
            .await
        );
        BACnetServer::<TestTransport>::reap_expired_requests(
            &context.services.network,
            &mut states,
            now,
            Duration::from_secs(20),
        )
        .await;
        assert!(
            matches!(routed.response(2000,17).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::OTHER)
        );
        assert!(states.is_empty() && routed.wire.try_recv().is_err());
        routed.router.stop().await;
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
    assert!(!local_a
        .response_route()
        .same_origin(&local_peer.response_route()));
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
    assert!(!remote
        .response_route()
        .same_origin(&different_remote.response_route()));
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
    routed.router.stop().await;
    replacement.router.stop().await;
}
