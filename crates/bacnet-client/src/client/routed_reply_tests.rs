//! Replies to a routed peer must retrace the request's path (issue #366).
//!
//! A segmented ComplexACK from a peer behind a router arrives with the peer's
//! SNET/SADR in the NPDU. Every PDU the client sends back — SegmentACKs and
//! Aborts — must carry that pair as DNET/DADR, unicast to the router, or the
//! router treats the reply as locally addressed and the peer never sees it.
//!
//! Split from `tests.rs`, which is at the 700-LOC cap.

use bacnet_encoding::apdu::{self, Apdu, ComplexAck, SegmentAck as SegmentAckPdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bytes::Bytes;
use tokio::time::{timeout, Duration};

use super::tests::send_routed_response;
use super::BACnetClient;

/// Every SegmentACK for a router-mediated segmented ComplexACK carries the
/// peer's network and address as DNET/DADR, and the exchange completes.
#[tokio::test]
async fn routed_segmented_complex_ack_replies_carry_dnet_dadr() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let router_mac_for_request = router_mac.clone();
    let remote_mac_for_request = remote_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request_routed(
                &router_mac_for_request,
                remote_network,
                &remote_mac_for_request,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x01],
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for the request")
        .expect("router channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let Apdu::ConfirmedRequest(req) = apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected ConfirmedRequest");
    };
    let invoke_id = req.invoke_id;

    let segments: Vec<Bytes> = vec![
        Bytes::from_static(&[0xAA, 0x01]),
        Bytes::from_static(&[0xBB, 0x02]),
        Bytes::from_static(&[0xCC]),
    ];

    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        let ack = Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: !is_last,
            invoke_id,
            sequence_number: Some(i as u8),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: seg.clone(),
        });
        send_routed_response(
            &router_transport,
            &client_mac,
            remote_network,
            &remote_mac,
            ack,
        )
        .await;

        let reply = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .expect("router timed out waiting for a SegmentAck")
            .expect("router channel closed");
        let reply_npdu = decode_npdu(reply.npdu).unwrap();
        let destination = reply_npdu
            .destination
            .expect("SegmentAck for a routed peer must carry DNET/DADR");
        assert_eq!(destination.network, remote_network);
        assert_eq!(&destination.mac_address[..], &remote_mac[..]);

        let Apdu::SegmentAck(sa) = apdu::decode_apdu(reply_npdu.payload).unwrap() else {
            panic!("expected SegmentAck");
        };
        assert_eq!(sa.invoke_id, invoke_id);
        assert_eq!(sa.sequence_number, i as u8);
        assert!(!sa.negative_ack);
        assert!(!sa.sent_by_server);
    }

    let result = request_task.await.unwrap().unwrap();
    assert_eq!(result, vec![0xAA, 0x01, 0xBB, 0x02, 0xCC]);
    router_transport.stop().await.unwrap();
}

/// Killing a live routed reassembly with a spurious server SegmentACK sends
/// an Abort that carries the peer's SNET/SADR. This exercises the
/// stored-state read at the dispatch site, but cannot distinguish it from the
/// inbound pair: a session-key match forces the two to be equal (the key is
/// derived from the SNET/SADR), so only `reply_mac` can genuinely diverge.
#[tokio::test]
async fn routed_reassembly_abort_carries_dnet_dadr() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let router_mac_for_request = router_mac.clone();
    let remote_mac_for_request = remote_mac.clone();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request_routed(
                &router_mac_for_request,
                remote_network,
                &remote_mac_for_request,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x01],
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for the request")
        .expect("router channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let Apdu::ConfirmedRequest(req) = apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected ConfirmedRequest");
    };
    let invoke_id = req.invoke_id;

    // Open a reassembly session with segment 0, and consume its SegmentAck.
    let segment_zero = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xAA]),
    });
    send_routed_response(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        segment_zero,
    )
    .await;
    let seg_ack = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for the SegmentAck")
        .expect("router channel closed");
    let seg_ack_npdu = decode_npdu(seg_ack.npdu).unwrap();
    assert!(matches!(
        apdu::decode_apdu(seg_ack_npdu.payload).unwrap(),
        Apdu::SegmentAck(_)
    ));

    // A server SegmentACK does not belong in SEGMENTED_CONF (Clause 5.4.4.4).
    let spurious = Apdu::SegmentAck(SegmentAckPdu {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    });
    send_routed_response(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        spurious,
    )
    .await;

    let reply = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for the Abort")
        .expect("router channel closed");
    let reply_npdu = decode_npdu(reply.npdu).unwrap();
    let destination = reply_npdu
        .destination
        .expect("Abort from stored session state must carry DNET/DADR");
    assert_eq!(destination.network, remote_network);
    assert_eq!(&destination.mac_address[..], &remote_mac[..]);

    let Apdu::Abort(abort) = apdu::decode_apdu(reply_npdu.payload).unwrap() else {
        panic!("expected Abort");
    };
    assert_eq!(abort.invoke_id, invoke_id);
    assert!(!abort.sent_by_server);
    assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);

    let result = request_task.await.unwrap();
    assert!(
        matches!(result, Err(super::Error::Abort { .. })),
        "the caller must observe the abort, got {result:?}"
    );
    router_transport.stop().await.unwrap();
}

/// An unsolicited routed segmented ComplexACK earns an Abort that retraces
/// the path — Clause 5.4.4.1's answer, addressed so the peer can hear it.
#[tokio::test]
async fn routed_unsolicited_segmented_ack_abort_carries_dnet_dadr() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac);
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let unsolicited = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id: 42,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xEE]),
    });
    send_routed_response(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        unsolicited,
    )
    .await;

    let reply = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for the Abort")
        .expect("router channel closed");
    let reply_npdu = decode_npdu(reply.npdu).unwrap();
    let destination = reply_npdu
        .destination
        .expect("Abort for a routed peer must carry DNET/DADR");
    assert_eq!(destination.network, remote_network);
    assert_eq!(&destination.mac_address[..], &remote_mac[..]);

    let Apdu::Abort(abort) = apdu::decode_apdu(reply_npdu.payload).unwrap() else {
        panic!("expected Abort");
    };
    assert_eq!(abort.invoke_id, 42);
    assert!(!abort.sent_by_server);
    assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}
