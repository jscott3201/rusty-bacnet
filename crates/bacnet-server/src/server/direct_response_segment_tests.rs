//! Real TLS response segmentation and exact-incarnation control admission.
use super::*;

async fn fixture() -> (TestCa, Fixture) {
    let ca = TestCa::new();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(1, "x".repeat(200)).unwrap(),
    ))
    .unwrap();
    let f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        db,
    )
    .await;
    (ca, f)
}
fn segmented_read(invoke: u8) -> Apdu {
    let Apdu::ConfirmedRequest(mut request) = read_name(invoke) else {
        unreachable!()
    };
    request.segmented_response_accepted = true;
    request.max_apdu_length = 50;
    Apdu::ConfirmedRequest(request)
}
fn segment_ack(invoke: u8, sequence: u8, negative_ack: bool) -> Apdu {
    Apdu::SegmentAck(SegmentAckPdu {
        sent_by_server: false,
        invoke_id: invoke,
        sequence_number: sequence,
        actual_window_size: 1,
        negative_ack,
    })
}
fn segment(response: Apdu, invoke: u8, sequence: u8) -> ComplexAck {
    let Apdu::ComplexAck(ack) = response else {
        panic!("expected response segment")
    };
    assert!(ack.segmented);
    assert_eq!(ack.invoke_id, invoke);
    assert_eq!(ack.sequence_number, Some(sequence));
    ack
}

#[tokio::test]
async fn direct_response_segments_retry_and_final_ack_stay_on_original_socket() {
    let (ca, mut f) = fixture().await;
    let mut a = f.peer(ca.tls("a")).await;
    let request = f.capture(&mut a, &segmented_read(11)).await;
    f.feed(request).await;
    let first = segment(socket_response(&mut f, &a).await, 11, 0);
    let mut payload = first.service_ack.to_vec();
    let ack = f.capture(&mut a, &segment_ack(11, 0, false)).await;
    f.feed(ack).await;
    let second = segment(socket_response(&mut f, &a).await, 11, 1);
    let retry = f.capture(&mut a, &segment_ack(11, 0, true)).await;
    f.feed(retry).await;
    assert_eq!(segment(socket_response(&mut f, &a).await, 11, 1), second);
    let mut current = second;
    loop {
        payload.extend_from_slice(&current.service_ack);
        let sequence = current.sequence_number.unwrap();
        let ack = f.capture(&mut a, &segment_ack(11, sequence, false)).await;
        f.feed(ack).await;
        if !current.more_follows {
            break;
        }
        current = segment(socket_response(&mut f, &a).await, 11, sequence + 1);
    }
    let value = ReadPropertyACK::decode(&payload).unwrap();
    let mut expected = BytesMut::new();
    encode_property_value(
        &mut expected,
        &PropertyValue::CharacterString("x".repeat(200)),
    )
    .unwrap();
    assert_eq!(value.property_value, expected.to_vec());
    bounded(async {
        while !f.server.seg_ack_senders.lock().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

async fn replacement_controls(same_leaf: bool) {
    let (ca, mut f) = fixture().await;
    let tls = ca.tls("a");
    let mut a = f.peer(tls.clone()).await;
    let request = f.capture(&mut a, &segmented_read(12)).await;
    let provenance_a = request.provenance.clone();
    f.feed(request).await;
    segment(socket_response(&mut f, &a).await, 12, 0);
    let own_ack = f.capture(&mut a, &segment_ack(12, 0, false)).await;
    let key = segmented_transaction_key(
        PEER_MAC.as_slice(),
        Some(&NpduAddress {
            network: 123,
            mac_address: MacAddr::from_slice(&[3]),
        }),
        12,
        provenance_a.clone(),
    );
    let handle = f.server.seg_ack_senders.lock().get(&key).unwrap().clone();
    let mut b = f.peer(if same_leaf { tls } else { ca.tls("b") }).await;
    let wrong_ack = f.capture(&mut b, &segment_ack(12, 0, false)).await;
    assert_ne!(wrong_ack.provenance, provenance_a);
    f.feed(wrong_ack).await;
    let wrong_abort = f
        .capture(
            &mut b,
            &Apdu::Abort(AbortPdu {
                sent_by_server: false,
                invoke_id: 12,
                abort_reason: AbortReason::OTHER,
            }),
        )
        .await;
    f.feed(wrong_abort).await;
    f.dispatch_barrier(&mut b, 240).await;
    assert!(!handle.closed.load(Ordering::Acquire));
    assert_eq!(handle.current_sequence.load(Ordering::Acquire), 0);
    assert!(f.server.seg_ack_senders.lock().contains_key(&key));

    // A's ACK was admitted before retirement. It still belongs to A, but the
    // next segment write cannot migrate to B. The failed send retires its child.
    f.feed(own_ack).await;
    bounded(async {
        while f.server.seg_ack_senders.lock().contains_key(&key) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    f.dispatch_barrier(&mut b, 241).await;
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

#[tokio::test]
async fn direct_response_replacement_ack_abort_cannot_control_old_segment_sender() {
    replacement_controls(false).await;
}
#[tokio::test]
async fn direct_response_same_leaf_reconnect_cannot_control_old_segment_sender() {
    replacement_controls(true).await;
}

#[tokio::test]
async fn direct_response_segmented_terminal_abort_and_peer_abort_release_owners() {
    let (ca, mut f) = fixture().await;
    let mut a = f.peer(ca.tls("a")).await;
    let original = segmented_read(51);
    let request = f.capture(&mut a, &original).await;
    f.feed(request).await;
    segment(socket_response(&mut f, &a).await, 51, 0);
    let ack = f.capture(&mut a, &segment_ack(51, 0, false)).await;
    f.feed(ack).await;
    let second = segment(socket_response(&mut f, &a).await, 51, 1);
    let duplicate = f.capture(&mut a, &original).await;
    f.feed(duplicate).await;
    f.dispatch_barrier(&mut a, 240).await; // no extra segment from pending retry
    for _ in 0..DEFAULT_APDU_SEGMENT_RETRIES {
        let nak = f.capture(&mut a, &segment_ack(51, 0, true)).await;
        f.feed(nak).await;
        assert_eq!(segment(socket_response(&mut f, &a).await, 51, 1), second);
    }
    let exhausted = f.capture(&mut a, &segment_ack(51, 0, true)).await;
    f.feed(exhausted).await;
    assert!(
        matches!(socket_response(&mut f, &a).await, Apdu::Abort(abort) if abort.invoke_id == 51 && abort.sent_by_server && abort.abort_reason == AbortReason::TSM_TIMEOUT)
    );
    bounded(async {
        while !f.server.seg_ack_senders.lock().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let retry = f.capture(&mut a, &original).await;
    f.feed(retry).await;
    segment(socket_response(&mut f, &a).await, 51, 0);
    let abort = f
        .capture(
            &mut a,
            &Apdu::Abort(AbortPdu {
                sent_by_server: false,
                invoke_id: 51,
                abort_reason: AbortReason::OTHER,
            }),
        )
        .await;
    f.feed(abort).await;
    bounded(async {
        while !f.server.seg_ack_senders.lock().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        f.server.seg_send_permits.available_permits(),
        MAX_SEG_SENDERS
    );
    let retry = f.capture(&mut a, &original).await;
    f.feed(retry).await;
    segment(socket_response(&mut f, &a).await, 51, 0);
    f.stop().await;
}
