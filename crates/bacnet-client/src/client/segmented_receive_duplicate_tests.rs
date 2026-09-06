//! Duplicate classification while receiving segmented ComplexACK responses (#383).

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ComplexAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: &[u8] = &[0x01];
const SERVER_MAC: &[u8] = &[0x02];

async fn send_to_client(transport: &LoopbackTransport, apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, CLIENT_MAC).await.unwrap();
}

async fn send_segment(
    transport: &LoopbackTransport,
    invoke_id: u8,
    sequence_number: u8,
    window_size: u8,
    more_follows: bool,
    payload: &[u8],
) {
    send_to_client(
        transport,
        Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows,
            invoke_id,
            sequence_number: Some(sequence_number),
            proposed_window_size: Some(window_size),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::copy_from_slice(payload),
        }),
    )
    .await;
}

async fn recv_apdu(rx: &mut mpsc::Receiver<ReceivedNpdu>, context: &str) -> Apdu {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for a PDU"))
        .unwrap_or_else(|| panic!("{context}: channel closed"));
    let npdu = decode_npdu(received.npdu).unwrap();
    apdu::decode_apdu(npdu.payload).unwrap()
}

async fn expect_segment_ack(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    invoke_id: u8,
    sequence_number: u8,
    window_size: u8,
    negative: bool,
    context: &str,
) {
    match recv_apdu(rx, context).await {
        Apdu::SegmentAck(ack) => {
            assert_eq!(ack.negative_ack, negative, "{context}");
            assert!(!ack.sent_by_server, "{context}: client ACK role");
            assert_eq!(ack.invoke_id, invoke_id, "{context}");
            assert_eq!(ack.sequence_number, sequence_number, "{context}");
            assert_eq!(ack.actual_window_size, window_size, "{context}");
        }
        other => panic!("{context}: expected SegmentAck, got {other:?}"),
    }
}

async fn expect_silence(rx: &mut mpsc::Receiver<ReceivedNpdu>, context: &str) {
    match timeout(Duration::from_millis(100), rx.recv()).await {
        Err(_) => {}
        Ok(Some(frame)) => {
            let npdu = decode_npdu(frame.npdu).unwrap();
            let apdu = apdu::decode_apdu(npdu.payload).unwrap();
            panic!("{context}: expected silence, got {apdu:?}");
        }
        Ok(None) => panic!("{context}: channel closed"),
    }
}

async fn start_reassembly() -> (
    JoinHandle<(BACnetClient<LoopbackTransport>, Result<Bytes, Error>)>,
    LoopbackTransport,
    mpsc::Receiver<ReceivedNpdu>,
    u8,
) {
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let mut server_rx = server_transport.start().await.unwrap();
    let client = BACnetClient::start(ClientConfig::default(), client_transport)
        .await
        .unwrap();
    let request_task = tokio::spawn(async move {
        let result = client
            .confirmed_request(
                SERVER_MAC,
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C; 8],
            )
            .await;
        (client, result)
    });
    let invoke_id = match recv_apdu(&mut server_rx, "ReadProperty request").await {
        Apdu::ConfirmedRequest(request) => request.invoke_id,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    (request_task, server_transport, server_rx, invoke_id)
}

async fn finish(
    task: JoinHandle<(BACnetClient<LoopbackTransport>, Result<Bytes, Error>)>,
    expected_payload: &[u8],
) {
    let (mut client, result) = timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap().as_ref(), expected_payload);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn window_one_duplicate_gets_immediate_negative_ack_without_payload_mutation() {
    let (task, server, mut rx, invoke_id) = start_reassembly().await;

    send_segment(&server, invoke_id, 0, 1, true, &[0x10]).await;
    expect_segment_ack(&mut rx, invoke_id, 0, 1, false, "initial ACK").await;

    send_segment(&server, invoke_id, 0, 1, true, &[0xEE]).await;
    expect_segment_ack(&mut rx, invoke_id, 0, 1, true, "duplicate NAK").await;

    send_segment(&server, invoke_id, 1, 1, false, &[0x20]).await;
    expect_segment_ack(&mut rx, invoke_id, 1, 1, false, "final ACK").await;
    finish(task, &[0x10, 0x20]).await;
}

#[tokio::test]
async fn incomplete_window_silences_ndup_duplicates_then_naks_the_next() {
    let (task, server, mut rx, invoke_id) = start_reassembly().await;

    send_segment(&server, invoke_id, 0, 3, true, &[0x10]).await;
    send_segment(&server, invoke_id, 1, 3, true, &[0x20]).await;
    for _ in 0..3 {
        send_segment(&server, invoke_id, 0, 3, true, &[0xEE]).await;
    }
    expect_silence(&mut rx, "first three duplicates").await;

    send_segment(&server, invoke_id, 0, 3, true, &[0xEE]).await;
    expect_segment_ack(&mut rx, invoke_id, 1, 3, true, "fourth duplicate NAK").await;

    // The client resets DuplicateCount, but Addendum 135-2020ch does not move
    // InitialSequenceNumber on TooManyDuplicateSegmentsReceived.
    send_segment(&server, invoke_id, 0, 3, true, &[0xEE]).await;
    expect_silence(&mut rx, "duplicate after counter reset").await;

    send_segment(&server, invoke_id, 2, 3, true, &[0x30]).await;
    expect_segment_ack(&mut rx, invoke_id, 2, 3, false, "window-boundary ACK").await;

    send_segment(&server, invoke_id, 3, 3, true, &[0x40]).await;
    for _ in 0..3 {
        send_segment(&server, invoke_id, 3, 3, true, &[0xEE]).await;
    }
    expect_silence(&mut rx, "duplicates after window-boundary reset").await;
    send_segment(&server, invoke_id, 3, 3, true, &[0xEE]).await;
    expect_segment_ack(
        &mut rx,
        invoke_id,
        3,
        3,
        true,
        "post-boundary threshold NAK",
    )
    .await;

    send_segment(&server, invoke_id, 4, 3, false, &[0x50]).await;
    expect_segment_ack(&mut rx, invoke_id, 4, 3, false, "final ACK").await;
    finish(task, &[0x10, 0x20, 0x30, 0x40, 0x50]).await;
}

#[tokio::test]
async fn out_of_order_and_window_boundary_reset_the_duplicate_baseline() {
    let (task, server, mut rx, invoke_id) = start_reassembly().await;

    send_segment(&server, invoke_id, 0, 4, true, &[0x10]).await;
    send_segment(&server, invoke_id, 1, 4, true, &[0x20]).await;
    send_segment(&server, invoke_id, 0, 4, true, &[0xEE]).await;
    expect_silence(&mut rx, "duplicate before gap").await;

    send_segment(&server, invoke_id, 3, 4, true, &[0xDD]).await;
    expect_segment_ack(&mut rx, invoke_id, 1, 4, true, "gap NAK").await;

    send_segment(&server, invoke_id, 2, 4, true, &[0x30]).await;
    for _ in 0..4 {
        send_segment(&server, invoke_id, 2, 4, true, &[0xEE]).await;
    }
    expect_silence(&mut rx, "duplicates after gap reset").await;
    send_segment(&server, invoke_id, 2, 4, true, &[0xEE]).await;
    expect_segment_ack(&mut rx, invoke_id, 2, 4, true, "post-gap threshold NAK").await;

    send_segment(&server, invoke_id, 3, 4, true, &[0x40]).await;
    expect_segment_ack(&mut rx, invoke_id, 3, 4, false, "window-boundary ACK").await;
    send_segment(&server, invoke_id, 3, 4, true, &[0xEE]).await;
    expect_segment_ack(&mut rx, invoke_id, 3, 4, true, "post-boundary baseline NAK").await;

    send_segment(&server, invoke_id, 4, 4, false, &[0x50]).await;
    expect_segment_ack(&mut rx, invoke_id, 4, 4, false, "final ACK").await;
    finish(task, &[0x10, 0x20, 0x30, 0x40, 0x50]).await;
}
