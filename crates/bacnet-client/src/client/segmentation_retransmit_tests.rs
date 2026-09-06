use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ConfirmedRequest, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

async fn send_local_response<T: TransportPort>(transport: &T, client_mac: &[u8], apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn recv_confirmed_segment(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    context: &str,
) -> ConfirmedRequest {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for segment"))
        .unwrap_or_else(|| panic!("{context}: server channel closed"));
    let npdu = decode_npdu(received.npdu).unwrap();
    let decoded = apdu::decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(req) = decoded else {
        panic!("{context}: expected ConfirmedRequest, got {:?}", decoded);
    };
    assert!(req.segmented, "{context}: request must be segmented");
    req
}

/// Clause 5.4.4.2's ack transitions never branch on the 'negative-ack'
/// flag: NAK(0) names segment 0 as the last one accepted, so the client
/// continues from segment 1. (The old reading — "resend from 0" — made a
/// lost first ack unrecoverable: the retransmitted segment 0 draws NAK(0)
/// from the peer's live session, and resending 0 loops forever.)
#[tokio::test]
async fn negative_ack_zero_advances_past_the_acknowledged_segment() {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let mut config = ClientConfig {
        apdu_timeout_ms: 2000,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    config.apdu_retries = 2;
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0u8..100).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut invoke_id = None;
    for expected_seq in [0, 1] {
        let received = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .expect("server timed out waiting for initial window segment")
            .expect("server channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert!(req.segmented);
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = Some(req.invoke_id);
    }

    let invoke_id = invoke_id.unwrap();
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;

    for expected_seq in [1, 2] {
        let received = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .expect("server timed out waiting for the resumed window segment")
            .expect("server channel closed");
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        };
        assert_eq!(req.invoke_id, invoke_id);
        assert_eq!(req.sequence_number, Some(expected_seq));
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 2,
            actual_window_size: 1,
        }),
    )
    .await;

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    server_transport.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_request_ignores_stale_negative_ack_zero_after_progress() {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let mut config = ClientConfig {
        apdu_timeout_ms: 2000,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    config.apdu_retries = 2;
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0..260).map(|i| (i % 251) as u8).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let mut invoke_id = None;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut server_rx, "initial window").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = Some(req.invoke_id);
    }

    let invoke_id = invoke_id.unwrap();
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;

    for expected_seq in [2, 3] {
        let req = recv_confirmed_segment(&mut server_rx, "second window").await;
        assert_eq!(req.invoke_id, invoke_id);
        assert_eq!(req.sequence_number, Some(expected_seq));
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 3,
            actual_window_size: 1,
        }),
    )
    .await;

    loop {
        let req = recv_confirmed_segment(&mut server_rx, "post-stale-ack window").await;
        assert_eq!(req.invoke_id, invoke_id);
        let seq = req.sequence_number.unwrap();
        assert!(
            seq >= 4,
            "stale negative SegmentACK 0 rewound transfer to segment {seq}"
        );
        let more_follows = req.more_follows;

        send_local_response(
            &server_transport,
            &client_mac,
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id,
                sequence_number: seq,
                actual_window_size: 1,
            }),
        )
        .await;

        if !more_follows {
            break;
        }
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;

    let result = request_task.await.unwrap();
    assert!(result.unwrap().is_empty());
    server_transport.stop().await.unwrap();
}

/// The peer stays quiet for `ms` — no retransmission, no new segment. Used
/// after a stale ack that must be discarded without advancing anything.
async fn expect_quiet(rx: &mut mpsc::Receiver<ReceivedNpdu>, ms: u64, context: &str) {
    if let Ok(Some(received)) = timeout(Duration::from_millis(ms), rx.recv()).await {
        let npdu = decode_npdu(received.npdu).unwrap();
        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        panic!("{context}: expected no traffic, got {decoded:?}");
    }
}

/// Common harness for the stale-SegmentAck tests (#368): a 100-byte payload
/// at max-APDU 50 makes exactly three segments (44 + 44 + 12), window 2.
async fn start_three_segment_request(
    payload_len: usize,
) -> (
    tokio::task::JoinHandle<Result<bytes::Bytes, bacnet_types::error::Error>>,
    LoopbackTransport,
    mpsc::Receiver<ReceivedNpdu>,
    Vec<u8>,
    Vec<u8>,
) {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let server_rx = server_transport.start().await.unwrap();

    let config = ClientConfig {
        apdu_timeout_ms: 2000,
        apdu_retries: 0,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0..payload_len).map(|i| i as u8).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });
    (
        request_task,
        server_transport,
        server_rx,
        client_mac,
        server_mac,
    )
}

/// #368: a SegmentAck whose sequence number is at or past this request's
/// segment count — a duplicated ack from an earlier transfer aliased onto a
/// reused invoke ID — is Clause 5.4.4.2 DuplicateACK_Received: discard and
/// keep waiting. Before the fix it failed the whole request with
/// "sequence out of range".
#[tokio::test]
async fn stale_out_of_range_positive_ack_is_discarded() {
    let (task, server_transport, mut rx, client_mac, _server_mac) =
        start_three_segment_request(100).await;

    let mut invoke_id = 0;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut rx, "first window").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = req.invoke_id;
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 19,
            actual_window_size: 2,
        }),
    )
    .await;
    expect_quiet(&mut rx, 250, "after the out-of-range positive ack").await;

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;
    let req = recv_confirmed_segment(&mut rx, "final segment").await;
    assert_eq!(req.sequence_number, Some(2));
    assert!(!req.more_follows);

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 2,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    task.await
        .unwrap()
        .expect("a stale out-of-range ack must not fail the request");
}

/// #368's companion: the stale ack in its negative flavor — before the fix it
/// hit the same fatal range check before the window filter could discard it.
#[tokio::test]
async fn stale_out_of_range_negative_ack_is_discarded() {
    let (task, server_transport, mut rx, client_mac, _server_mac) =
        start_three_segment_request(100).await;

    let mut invoke_id = 0;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut rx, "first window").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = req.invoke_id;
    }

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 19,
            actual_window_size: 2,
        }),
    )
    .await;
    expect_quiet(&mut rx, 250, "after the out-of-range negative ack").await;

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;
    let req = recv_confirmed_segment(&mut rx, "final segment").await;
    assert_eq!(req.sequence_number, Some(2));

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 2,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    task.await
        .unwrap()
        .expect("a stale out-of-range negative ack must not fail the request");
}

/// After #368's fix the window filter is the ONLY gate on an inbound
/// SegmentAck, so its in-range-but-out-of-window discard is pinned here: a
/// stale ack for sequence 0 while the send window is [2, 4) must not rewind
/// `next_seq` (which would replay segment 1 and corrupt the strict sequence
/// the peer asserts). Passed before the fix too — this is the
/// vacuity guard the removal leans on.
#[tokio::test]
async fn stale_in_range_out_of_window_ack_is_discarded() {
    // 260 bytes at max-APDU 50 → six segments (5 × 44 + 40), window 2.
    let (task, server_transport, mut rx, client_mac, _server_mac) =
        start_three_segment_request(260).await;

    let mut invoke_id = 0;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut rx, "window [0,2)").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = req.invoke_id;
    }
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;

    for expected_seq in [2, 3] {
        let req = recv_confirmed_segment(&mut rx, "window [2,4)").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
    }
    // In range (0 < 6), out of window: must be discarded, not rewind to 1.
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;
    expect_quiet(&mut rx, 250, "after the out-of-window ack").await;

    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 3,
            actual_window_size: 2,
        }),
    )
    .await;
    for expected_seq in [4, 5] {
        let req = recv_confirmed_segment(&mut rx, "window [4,6)").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        if expected_seq == 5 {
            assert!(!req.more_follows);
        }
    }
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 5,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    task.await
        .unwrap()
        .expect("an out-of-window ack must not corrupt the transfer");
}

/// The lost-first-ack recovery loop, end to end: the client's ack for
/// segment 0 goes missing, the client retransmits segment 0 on timeout, and
/// the peer — whose session is live and has already accepted segment 0 —
/// answers with NAK(0), exactly what this repo's server sends for a
/// duplicate. Per Clause 5.4.4.2 the client must advance to segment 1; the
/// old "resend from 0" reading looped NAK(0) → segment 0 forever until the
/// NAK bound failed the transfer.
#[tokio::test]
async fn lost_first_ack_recovers_via_negative_ack_zero() {
    let client_mac = vec![0x01];
    let server_mac = vec![0x02];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let config = ClientConfig {
        apdu_timeout_ms: 300,
        apdu_retries: 2,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, client_transport).await.unwrap();

    let request_server_mac = server_mac.clone();
    let service_data: Vec<u8> = (0u8..100).collect();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                &request_server_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    // Segment 0 arrives; the peer "loses" its ack by sending nothing.
    let req = recv_confirmed_segment(&mut server_rx, "initial segment 0").await;
    assert_eq!(req.sequence_number, Some(0));
    let invoke_id = req.invoke_id;

    // The client's SegmentTimer fires and retransmits segment 0; the peer's
    // live session treats it as a duplicate and NAKs naming sequence 0.
    let req = recv_confirmed_segment(&mut server_rx, "retransmitted segment 0").await;
    assert_eq!(req.sequence_number, Some(0));
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 1,
        }),
    )
    .await;

    // Recovery: the next segment must be 1, not another 0.
    let mut seq = 1u8;
    loop {
        let req = recv_confirmed_segment(&mut server_rx, "post-recovery segment").await;
        assert_eq!(req.sequence_number, Some(seq), "transfer must advance");
        let more = req.more_follows;
        send_local_response(
            &server_transport,
            &client_mac,
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id,
                sequence_number: seq,
                actual_window_size: 1,
            }),
        )
        .await;
        if !more {
            break;
        }
        seq += 1;
    }
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    request_task
        .await
        .unwrap()
        .expect("a lost first ack must be recoverable");
}

/// A NAK naming the last segment of the current window means "I have the
/// whole window, continue" — Clause 5.4.2.1's InWindow accepts it, and the
/// client must advance to the next window rather than discard it and stall
/// into a timeout.
#[tokio::test]
async fn negative_ack_naming_last_of_window_advances_the_window() {
    let (task, server_transport, mut rx, client_mac, _server_mac) =
        start_three_segment_request(100).await;

    let mut invoke_id = 0;
    for expected_seq in [0, 1] {
        let req = recv_confirmed_segment(&mut rx, "window [0,2)").await;
        assert_eq!(req.sequence_number, Some(expected_seq));
        invoke_id = req.invoke_id;
    }
    // NAK(1): sequence 1 is the last segment of the [0,2) window.
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: true,
            sent_by_server: true,
            invoke_id,
            sequence_number: 1,
            actual_window_size: 2,
        }),
    )
    .await;

    let req = recv_confirmed_segment(&mut rx, "the window must advance to 2").await;
    assert_eq!(req.sequence_number, Some(2));
    assert!(!req.more_follows);
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id,
            sequence_number: 2,
            actual_window_size: 2,
        }),
    )
    .await;
    send_local_response(
        &server_transport,
        &client_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    task.await
        .unwrap()
        .expect("a NAK naming the window's last segment must advance, not stall");
}
