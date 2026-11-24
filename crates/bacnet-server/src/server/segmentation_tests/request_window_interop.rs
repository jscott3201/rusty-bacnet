//! Segment zero is acknowledged at once (Clause 5.4.5.1, #2742), whatever
//! window the requester proposes: a conforming requester sends it alone and
//! waits (Clause 5.4.4.1), as this workspace's client does. A requester
//! that sends its whole first window without waiting completes too.

use super::*;
use crate::server::test_transport::StartMode;
use bacnet_client::client::BACnetClient;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::Segmentation;
use request_reassembly::{
    expect_positive_ack, present_value, recv_apdu, send_segment_with_window, split_into,
    start_reassembly_server, write_property_payload,
};

async fn expect_simple_ack(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    invoke_id: u8,
) {
    match recv_apdu(rx, "the answer").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, invoke_id),
        other => panic!("expected the SimpleAck, got {other:?}"),
    }
}

async fn expect_quiet(rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>) {
    if let Ok(Some(frame)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
        let apdu = decode_apdu(decode_npdu(frame.npdu).unwrap().payload).unwrap();
        panic!("expected nothing more, got {apdu:?}");
    }
}

#[tokio::test]
async fn segment_zero_is_acknowledged_at_once_at_every_window() {
    for window in [1u8, 2, 8] {
        let (mut server, client, mut rx) =
            start_reassembly_server(Segmentation::SEGMENTED_BOTH).await;
        let text = format!("segment zero answered at once, window {window}");
        let total = 2 * usize::from(window) + 2;
        let chunks = split_into(&write_property_payload(&text), total);
        let invoke = 40 + window;
        // Segment zero alone, and nothing more until it is answered.
        send_segment_with_window(&client, invoke, 0, window, true, &chunks[0]).await;
        match recv_apdu(&mut rx, "segment zero's acknowledgment").await {
            Apdu::SegmentAck(ack) => {
                assert!(!ack.negative_ack && ack.sent_by_server);
                assert_eq!(
                    (ack.invoke_id, ack.sequence_number, ack.actual_window_size),
                    (invoke, 0, window)
                );
            }
            other => panic!("window {window}: expected a SegmentAck, got {other:?}"),
        }
        // Then a group of the window granted at a time, each answered once.
        let mut next = 1;
        while next < total {
            let end = (next + usize::from(window)).min(total);
            for (seq, chunk) in chunks.iter().enumerate().take(end).skip(next) {
                let more = seq + 1 < total;
                send_segment_with_window(&client, invoke, seq as u8, window, more, chunk).await;
            }
            expect_positive_ack(&mut rx, invoke, (end - 1) as u8).await;
            next = end;
        }
        expect_simple_ack(&mut rx, invoke).await;
        assert_eq!(present_value(&server).await, text);
        expect_quiet(&mut rx).await;
        server.stop().await.unwrap();
    }
}

/// Segments 0 to 3 of ten at once, at a window of four: segment zero's
/// acknowledgment comes first, and nothing more until the window counted
/// from it fills. The requester goes on from segment 1, as that answer asks;
/// the three it sends again are duplicates, discarded, and the windows line
/// up from segment 4 on.
#[tokio::test]
async fn a_requester_that_sends_its_first_window_unasked_still_completes() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::SEGMENTED_BOTH).await;
    let text = "a whole first window, sent without waiting";
    let chunks = split_into(&write_property_payload(text), 10);
    let send =
        |seq: usize| send_segment_with_window(&client, 9, seq as u8, 4, seq < 9, &chunks[seq]);
    for seq in 0..4 {
        send(seq).await;
    }
    expect_positive_ack(&mut rx, 9, 0).await;
    expect_quiet(&mut rx).await;
    for seq in 1..5 {
        send(seq).await;
    }
    expect_positive_ack(&mut rx, 9, 4).await;
    for seq in 5..9 {
        send(seq).await;
    }
    expect_positive_ack(&mut rx, 9, 8).await;
    send(9).await;
    expect_positive_ack(&mut rx, 9, 9).await;
    expect_simple_ack(&mut rx, 9).await;
    assert_eq!(present_value(&server).await, text);
    expect_quiet(&mut rx).await;
    server.stop().await.unwrap();
}

const SERVER_MAC: &[u8] = &[0x02];

/// A full server whose sends this test reads, and this workspace's client
/// at the other end of the link, proposing `window` and sizing its segments
/// for 480-octet APDUs. The last item is the sequence number of every
/// segment the server took in, in order.
async fn server_and_client(
    window: u8,
) -> (
    BACnetServer<TestTransport>,
    BACnetClient<LoopbackTransport>,
    SendLog,
    Arc<std::sync::Mutex<Vec<u8>>>,
) {
    let (server_end, client_end) = LoopbackTransport::pair(SERVER_MAC.to_vec(), vec![0x01]);
    let taken = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&taken);
    let transport = TestTransport::builder()
        .inner(server_end)
        .start(StartMode::Relay(64))
        .on_inbound(move |frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            if let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) {
                seen.lock().unwrap().extend(request.sequence_number);
            }
            async move { Some(frame) }
        })
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Ignore)
        .build();
    let sent = transport.sent();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(1, "CSV-1").unwrap(),
    ))
    .unwrap();
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_segmentation(Segmentation::SEGMENTED_BOTH),
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_end)
        .max_apdu_length(480)
        .proposed_window_size(window)
        .build()
        .await
        .unwrap();
    (server, client, sent, taken)
}

/// What the server sent the client, in order, once the link is quiet.
async fn server_replies(sent: &SendLog) -> Vec<Apdu> {
    tokio::time::sleep(Duration::from_millis(100)).await;
    (0..sent.len())
        .map(|index| sent.frame(index).apdu())
        .collect()
}

async fn write_text(client: &BACnetClient<LoopbackTransport>, text: &str) -> Result<(), Error> {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString(text.into())).unwrap();
    client
        .write_property(
            &MacAddr::from_slice(SERVER_MAC),
            ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value.to_vec(),
            None,
        )
        .await
}

/// This workspace's client against this server, at windows of 1, 2 and 8,
/// with requests inside and past the first window: segment zero goes alone,
/// each later group as the answer grants, no segment goes twice, and the
/// server sends Segment-ACKs and the answer, never an Abort (#2742).
#[tokio::test]
async fn this_workspaces_client_sends_each_segment_once() {
    for window in [1u8, 2, 8] {
        for length in [700, 4_500] {
            let (mut server, mut client, sent, taken) = server_and_client(window).await;
            let text = "w".repeat(length);
            write_text(&client, &text).await.unwrap();
            assert_eq!(present_value(&server).await, text);
            let replies = server_replies(&sent).await;
            let taken = taken.lock().unwrap().clone();
            let total = taken.len();
            assert!(total >= 2, "window {window}, {length}: segmented");
            assert_eq!(
                taken,
                (0..total as u8).collect::<Vec<_>>(),
                "window {window}, {length}: each segment once"
            );
            // Segment zero, every group the window grants, and the last.
            let mut expected: Vec<String> = (0..total)
                .filter(|seq| *seq % usize::from(window) == 0 || *seq + 1 == total)
                .map(|seq| format!("ack {seq}"))
                .collect();
            expected.push("answer".into());
            let kinds: Vec<String> = replies
                .iter()
                .map(|apdu| match apdu {
                    Apdu::SegmentAck(ack) if !ack.negative_ack => {
                        format!("ack {}", ack.sequence_number)
                    }
                    Apdu::SimpleAck(_) => "answer".into(),
                    other => panic!("window {window}, {length}: unexpected {other:?}"),
                })
                .collect();
            assert_eq!(kinds, expected, "window {window}, {length}");
            client.stop().await.unwrap();
            server.stop().await.unwrap();
        }
    }
}

/// A requester that sends its whole first window unasked (as this
/// workspace's client did before #2742), played by hand: all three segments
/// of a request at once at a window of four, then, on segment zero's
/// acknowledgment, segments 1 and 2 again. The request is whole on the first
/// pass and answered; the two sent again find no transfer and each draws an
/// Abort (INVALID_APDU_IN_THIS_STATE) after the answer, which a requester
/// that is done ignores. While the request still executes, each is
/// acknowledged again instead (Clause 5.4.5.3, #2751; see
/// `request_repeat_while_executing`).
#[tokio::test]
async fn segments_sent_again_after_the_answer_each_draw_an_abort() {
    let (mut server, client, mut rx) = start_reassembly_server(Segmentation::SEGMENTED_BOTH).await;
    let text = "one window, sent twice";
    let chunks = split_into(&write_property_payload(text), 3);
    let send =
        |seq: usize| send_segment_with_window(&client, 7, seq as u8, 4, seq < 2, &chunks[seq]);
    for seq in 0..3 {
        send(seq).await;
    }
    expect_positive_ack(&mut rx, 7, 0).await;
    expect_positive_ack(&mut rx, 7, 2).await;
    expect_simple_ack(&mut rx, 7).await;
    assert_eq!(present_value(&server).await, text);
    for seq in 1..3 {
        send(seq).await;
        match recv_apdu(&mut rx, "a segment sent again").await {
            Apdu::Abort(abort) => {
                assert!(abort.sent_by_server);
                assert_eq!(
                    (abort.invoke_id, abort.abort_reason),
                    (7, AbortReason::INVALID_APDU_IN_THIS_STATE)
                );
            }
            other => panic!("expected an Abort, got {other:?}"),
        }
    }
    expect_quiet(&mut rx).await;
    server.stop().await.unwrap();
}
