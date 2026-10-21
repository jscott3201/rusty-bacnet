use super::*;

fn segment(invoke: u8, sequence: u8, data: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: sequence == 0,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: invoke,
        sequence_number: Some(sequence),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: data,
    })
}
fn abort(invoke: u8) -> Apdu {
    Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id: invoke,
        abort_reason: AbortReason::OTHER,
    })
}
async fn ack(f: &mut Fixture, sequence: u8) {
    assert!(
        matches!(f.response().await, Apdu::SegmentAck(a) if a.sequence_number == sequence && !a.negative_ack)
    );
}

async fn isolated_segments(same_leaf: bool, cancel_a: bool) {
    let ca = TestCa::new();
    let a_tls = ca.tls("a");
    let b_tls = if same_leaf {
        a_tls.clone()
    } else {
        ca.tls("b")
    };
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let observer = seen.clone();
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
        policy: Some(Arc::new(crate::FnPolicy::new().mutation(move |context| {
            observer
                .lock()
                .unwrap()
                .push(context.direct_sc_identity().unwrap());
            true
        }))),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;
    let mut a = f.peer(a_tls).await;
    let a_payload = write_payload("A segmented");
    let b_payload = write_payload("B segmented");
    let cut = a_payload.len() / 2;
    let a0 = f
        .capture(&mut a, &segment(31, 0, a_payload.slice(..cut)))
        .await;
    let identity_a = a0.provenance.direct_sc_identity().unwrap();
    // These frames are admitted BEFORE retirement. Later delivery models work
    // already queued by ingress; no fresh NPDU is accepted from retired A.
    let a1 = f
        .capture(&mut a, &segment(31, 1, a_payload.slice(cut..)))
        .await;
    let a_abort = f.capture(&mut a, &abort(31)).await;
    f.feed(a0).await;
    ack(&mut f, 0).await;

    let mut b = f.peer(b_tls).await;
    let b1 = f
        .capture(&mut b, &segment(31, 1, b_payload.slice(cut..)))
        .await;
    let identity_b = b1.provenance.direct_sc_identity().unwrap();
    assert_ne!(identity_a, identity_b);
    f.feed(b1).await;
    assert!(
        matches!(f.response().await, Apdu::Abort(a) if a.abort_reason == AbortReason::INVALID_APDU_IN_THIS_STATE)
    );
    // An unverified address-level sweep must not erase a direct-owned context.
    let mut unverified_abort = f.capture(&mut b, &abort(31)).await;
    unverified_abort.provenance = bacnet_transport::port::TransportProvenance::unverified();
    f.feed(unverified_abort).await;
    let b0 = f
        .capture(&mut b, &segment(31, 0, b_payload.slice(..cut)))
        .await;
    f.feed(b0).await;
    ack(&mut f, 0).await;

    let (winner, expected) = if cancel_a {
        f.feed(a_abort).await;
        f.feed(a1).await;
        // The queued old-A continuation now has no receive context. Its
        // generated Abort is confined to retired A and cannot reach B.
        f.dispatch_barrier(&mut b, 239).await;
        assert!(f.responses.try_recv().is_err());
        let b1 = f
            .capture(&mut b, &segment(31, 1, b_payload.slice(cut..)))
            .await;
        f.feed(b1).await;
        (identity_b, "B segmented")
    } else {
        let b_abort = f.capture(&mut b, &abort(31)).await;
        f.feed(b_abort).await;
        let b1 = f
            .capture(&mut b, &segment(31, 1, b_payload.slice(cut..)))
            .await;
        f.feed(b1).await;
        assert!(
            matches!(f.response().await, Apdu::Abort(a) if a.abort_reason == AbortReason::INVALID_APDU_IN_THIS_STATE)
        );
        f.feed(a1).await;
        (identity_a, "A segmented")
    };
    if cancel_a {
        ack(&mut f, 1).await;
        assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    } else {
        // A's already-admitted final segment executes under A's snapshot;
        // neither its SegmentACK nor terminal ACK may use B's live socket.
        f.dispatch_barrier(&mut b, 240).await;
    }
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());
    assert_eq!(*seen.lock().unwrap(), vec![winner]);
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&csv_oid())
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString(expected.into())
    );
    f.stop().await;
}

#[tokio::test]
async fn direct_principal_new_leaf_segments_and_abort_preserve_queued_old_context() {
    isolated_segments(false, false).await;
}
#[tokio::test]
async fn direct_principal_same_leaf_segments_and_abort_preserve_queued_old_context() {
    isolated_segments(true, false).await;
}
#[tokio::test]
async fn direct_principal_queued_old_abort_preserves_new_context() {
    isolated_segments(false, true).await;
}

#[tokio::test]
async fn direct_principal_two_admitted_contexts_finish_without_merging() {
    let ca = TestCa::new();
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let observer = seen.clone();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            policy: Some(Arc::new(crate::FnPolicy::new().mutation(move |c| {
                observer
                    .lock()
                    .unwrap()
                    .push(c.direct_sc_identity().unwrap());
                true
            }))),
            ..Default::default()
        },
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let payload = write_payload("first admitted");
    let cut = payload.len() / 2;
    let a0 = f
        .capture(&mut a, &segment(33, 0, payload.slice(..cut)))
        .await;
    let id_a = a0.provenance.direct_sc_identity().unwrap();
    let a1 = f
        .capture(&mut a, &segment(33, 1, payload.slice(cut..)))
        .await;
    f.feed(a0).await;
    ack(&mut f, 0).await;
    let mut b = f.peer(ca.tls("b")).await;
    let payload = write_payload("second admitted");
    let b0 = f
        .capture(&mut b, &segment(33, 0, payload.slice(..cut)))
        .await;
    let id_b = b0.provenance.direct_sc_identity().unwrap();
    f.feed(b0).await;
    ack(&mut f, 0).await;
    f.feed(a1).await;
    f.dispatch_barrier(&mut b, 240).await;
    f.active(0).await;
    assert_eq!(*seen.lock().unwrap(), vec![id_a]);
    assert!(f.responses.try_recv().is_err());
    let b1 = f
        .capture(&mut b, &segment(33, 1, payload.slice(cut..)))
        .await;
    f.feed(b1).await;
    ack(&mut f, 1).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    f.active(0).await;
    assert_eq!(*seen.lock().unwrap(), vec![id_a, id_b]);
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&csv_oid())
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("second admitted".into())
    );
    f.stop().await;
}

#[tokio::test]
async fn direct_principal_reconnect_does_not_expand_reassembly_peer_capacity() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    for invoke in 0..16 {
        let a0 = f
            .capture(&mut a, &segment(invoke, 0, Bytes::from_static(&[12])))
            .await;
        f.feed(a0).await;
        ack(&mut f, 0).await;
    }
    let a_abort = f.capture(&mut a, &abort(0)).await;
    let mut b = f.peer(ca.tls("b")).await;
    let b0 = f
        .capture(&mut b, &segment(0, 0, Bytes::from_static(&[12])))
        .await;
    f.feed(b0).await;
    assert!(
        matches!(f.response().await, Apdu::Abort(a) if a.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    // Exact cancellation releases A's slot, even after its socket retires.
    f.feed(a_abort).await;
    let b0 = f
        .capture(&mut b, &segment(0, 0, Bytes::from_static(&[12])))
        .await;
    f.feed(b0).await;
    ack(&mut f, 0).await;
    // Global stop also owns the remaining partial contexts.
    f.stop().await;
}

#[tokio::test]
async fn direct_response_reassembly_uses_segment_zero_capability_not_final_metadata() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let payload = write_payload("saved route");
    let cut = payload.len() / 2;
    let first = f
        .capture(&mut a, &segment(61, 0, payload.slice(..cut)))
        .await;
    f.feed(first).await;
    ack(&mut f, 0).await;
    let mut final_segment = f
        .capture(&mut a, &segment(61, 1, payload.slice(cut..)))
        .await;
    // Model a downstream metadata loss on the final envelope. Its immediate
    // SegmentACK fails closed; the completed request must use segment zero's
    // saved capability instead of copying this absent field.
    final_segment.direct_response = None;
    f.feed(final_segment).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(ack) if ack.invoke_id == 61));
    f.active(0).await;
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&csv_oid())
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("saved route".into())
    );
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

#[tokio::test]
async fn direct_response_receive_segment_gap_nak_and_final_ack_use_original_socket() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let payload = write_payload("after gap");
    let cut = payload.len() / 2;
    let first = f
        .capture(&mut a, &segment(62, 0, payload.slice(..cut)))
        .await;
    f.feed(first).await;
    ack(&mut f, 0).await;
    let gap = f
        .capture(&mut a, &segment(62, 2, payload.slice(cut..)))
        .await;
    f.feed(gap).await;
    assert!(
        matches!(f.response().await, Apdu::SegmentAck(a) if a.sent_by_server && a.invoke_id == 62 && a.sequence_number == 0 && a.negative_ack)
    );
    let last = f
        .capture(&mut a, &segment(62, 1, payload.slice(cut..)))
        .await;
    f.feed(last).await;
    ack(&mut f, 1).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(a) if a.invoke_id == 62));
    f.active(0).await;
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&csv_oid())
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("after gap".into())
    );
    f.stop().await;
}
