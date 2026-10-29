//! Admitted direct limits constrain sizing independently of response authority.
use super::*;

fn name_payload(name: &str) -> Bytes {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString(name.into())).unwrap();
    let mut service = BytesMut::new();
    ReadPropertyACK {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut service);
    service.freeze()
}
fn request(accepts_segments: bool) -> Apdu {
    let Apdu::ConfirmedRequest(mut request) = read_name(70) else {
        unreachable!()
    };
    request.max_apdu_length = 480;
    request.max_segments = Some(2);
    request.segmented_response_accepted = accepts_segments;
    Apdu::ConfirmedRequest(request)
}
async fn fixture(npdu_limit: u16, bvlc_limit: u16, name: &str) -> (Fixture, Peer) {
    let ca = TestCa::new();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(CharacterStringValueObject::new(1, name).unwrap()))
        .unwrap();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH),
            ..Default::default()
        },
        db,
    )
    .await;
    let peer = f
        .peer_config(ca.tls("small"), |connection| {
            connection.max_bvlc_length = bvlc_limit;
            connection.max_apdu_length = npdu_limit;
        })
        .await;
    (f, peer)
}
async fn wire_response(
    f: &mut Fixture,
    peer: &Peer,
    npdu_limit: u16,
    bvlc_limit: u16,
    destination: Option<&NpduAddress>,
) -> (Apdu, usize) {
    let bytes = bounded(async {
        tokio::select! {
            reply = peer.ws.recv() => reply.unwrap(),
            fallback = f.responses.recv() => panic!("unexpected generic fallback: {fallback:?}"),
        }
    })
    .await;
    assert!(bytes.len() <= usize::from(bvlc_limit));
    let frame = decode_sc_message(&bytes).unwrap();
    assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
    assert!(frame.originating_vmac.is_none() && frame.destination_vmac.is_none());
    assert!(frame.payload.len() <= usize::from(npdu_limit));
    assert_eq!(
        bytes.len(),
        frame.payload.len() + 4,
        "direct BVLC wire header"
    );
    let npdu = decode_npdu(frame.payload).unwrap();
    assert_eq!(npdu.destination.as_ref(), destination);
    let apdu_len = npdu.payload.len();
    (apdu::decode_apdu(npdu.payload).unwrap(), apdu_len)
}
async fn finish_transfer(
    f: &mut Fixture,
    peer: &mut Peer,
    limits: (u16, u16),
    source: Option<NpduAddress>,
    expected_budget: usize,
    expected: &[u8],
) {
    let mut received = Vec::new();
    for sequence in 0..2 {
        let (response, length) = wire_response(f, peer, limits.0, limits.1, source.as_ref()).await;
        let Apdu::ComplexAck(segment) = response else {
            panic!("expected response segment")
        };
        assert!(segment.segmented);
        assert_eq!(segment.sequence_number, Some(sequence));
        assert_eq!(segment.proposed_window_size, Some(1));
        assert_eq!(
            segment.more_follows,
            sequence == 0,
            "fewest fitting segments"
        );
        assert!(length <= expected_budget);
        if sequence == 0 {
            assert_eq!(
                length, expected_budget,
                "first segment fills actual path budget"
            );
            assert_eq!(segment.service_ack.len(), expected_budget - 5);
            assert!(!f.server.seg_ack_senders.lock().is_empty());
        }
        received.extend_from_slice(&segment.service_ack);
        let ack = Apdu::SegmentAck(SegmentAckPdu {
            sent_by_server: false,
            invoke_id: 70,
            sequence_number: sequence,
            actual_window_size: 1,
            negative_ack: false,
        });
        let ack = f.capture_from(peer, &ack, source.clone()).await;
        f.feed(ack).await;
    }
    assert_eq!(received, expected);
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
}
async fn delivered(limits: (u16, u16), source: Option<NpduAddress>, name: &str, budget: usize) {
    let expected = name_payload(name);
    let mut full = BytesMut::new();
    encode_apdu(
        &mut full,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: 70,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: expected.clone(),
        }),
    )
    .unwrap();
    assert!(full.len() > budget && full.len() <= 480);
    if name.len() == 462 {
        assert_eq!(full.len(), 479);
    }
    let (mut f, mut peer) = fixture(limits.0, limits.1, name).await;
    // Repeat the exact request after the final ACK: actual delivery proves both
    // segmented child cleanup and pending InvokeID ownership release.
    for _ in 0..2 {
        let envelope = f
            .capture_from(&mut peer, &request(true), source.clone())
            .await;
        f.feed(envelope).await;
        finish_transfer(&mut f, &mut peer, limits, source.clone(), budget, &expected).await;
    }
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

#[tokio::test]
async fn direct_response_peer_npdu_budget_segments_below_requester_apdu_limit() {
    delivered(
        (300, 1000),
        Some(NpduAddress {
            network: 123,
            mac_address: MacAddr::from_slice(&[3]),
        }),
        &"n".repeat(340),
        293,
    )
    .await;
}
#[tokio::test]
async fn direct_response_479_byte_ack_obeys_local_480_npdu_budget() {
    delivered((480, 484), None, &"n".repeat(462), 478).await;
}
#[tokio::test]
async fn direct_response_bvlc_budget_independently_limits_segments() {
    delivered((1000, 484), None, &"n".repeat(462), 478).await;
}
#[tokio::test]
async fn direct_response_routed_header_reduces_available_segment_payload() {
    delivered(
        (480, 484),
        Some(NpduAddress {
            network: 123,
            mac_address: MacAddr::from_slice(&[1, 2, 3, 4, 5, 6]),
        }),
        &"n".repeat(462),
        468,
    )
    .await;
}

#[tokio::test]
async fn direct_response_small_budget_no_segmentation_and_segment_capacity_abort() {
    for (npdu, bvlc, accepts, reason) in [
        (300, 1000, false, AbortReason::SEGMENTATION_NOT_SUPPORTED),
        (100, 104, true, AbortReason::BUFFER_OVERFLOW), // would need >2 segments
        (5, 9, true, AbortReason::BUFFER_OVERFLOW),     // Abort fits; segment header does not
    ] {
        let (mut f, mut peer) = fixture(npdu, bvlc, &"n".repeat(340)).await;
        for _ in 0..2 {
            let envelope = f.capture_from(&mut peer, &request(accepts), None).await;
            f.feed(envelope).await;
            let (response, _) = wire_response(&mut f, &peer, npdu, bvlc, None).await;
            assert!(
                matches!(response, Apdu::Abort(a) if a.sent_by_server && a.invoke_id == 70 && a.abort_reason == reason)
            );
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
        }
        f.stop().await;
    }
}

#[tokio::test]
async fn direct_response_budget_too_small_for_abort_finishes_without_fallback() {
    let (mut f, mut peer) = fixture(4, 8, &"n".repeat(340)).await;
    let mut saved_route = None;
    for _ in 0..2 {
        let envelope = f.capture_from(&mut peer, &request(true), None).await;
        saved_route = Some(bacnet_network::response_route::ResponseRoute::new(
            envelope.provenance.clone(),
            envelope.direct_response.clone(),
        ));
        assert_eq!(
            saved_route
                .as_ref()
                .unwrap()
                .max_apdu_length(480, None)
                .unwrap(),
            2
        );
        let db = f.server.db.clone();
        let held = db.write().await;
        f.feed(envelope).await;
        f.active(1).await; // exact retry must be admitted after failed reply
        drop(held);
        bounded(async {
            while !f.server.request_tasks.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(f.server.seg_ack_senders.lock().is_empty());
        assert_eq!(
            f.server.seg_send_permits.available_permits(),
            MAX_SEG_SENDERS
        );
    }
    f.listener.stop().await; // EOF makes absence of buffered application data observable
    assert_eq!(
        saved_route.unwrap().max_apdu_length(480, None).unwrap(),
        2,
        "sizing snapshot remains readable after retirement; it grants no send authority"
    );
    assert!(bounded(peer.ws.recv()).await.is_err());
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

#[tokio::test]
async fn direct_response_exact_path_budget_remains_unsegmented() {
    let name = "n".repeat(461);
    let expected = name_payload(&name);
    let (mut f, mut peer) = fixture(480, 484, &name).await;
    let envelope = f.capture_from(&mut peer, &request(true), None).await;
    f.feed(envelope).await;
    let (response, length) = wire_response(&mut f, &peer, 480, 484, None).await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("expected unsegmented response")
    };
    assert!(!ack.segmented && !ack.more_follows);
    assert_eq!(length, 478);
    assert_eq!(ack.service_ack, expected);
    assert!(f.server.seg_ack_senders.lock().is_empty());
    f.stop().await;
}
