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

#[tokio::test(start_paused = true)]
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
#[tokio::test(start_paused = true)]
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
#[tokio::test(start_paused = true)]
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

type Events = Arc<std::sync::Mutex<Vec<String>>>;

/// A full server and this workspace's client at a window of four, with a
/// segment timeout of 300 ms, and one log of what the server takes in and
/// sends, in order. Setting the flag loses the next segment on its way in.
async fn logged_server_and_client() -> (
    BACnetServer<TestTransport>,
    BACnetClient<LoopbackTransport>,
    Events,
    Arc<AtomicBool>,
) {
    let (server_end, client_end) = LoopbackTransport::pair(SERVER_MAC.to_vec(), vec![0x01]);
    let events: Events = Arc::default();
    let lose = Arc::new(AtomicBool::new(false));
    let (inbound, outbound, lose_next) =
        (Arc::clone(&events), Arc::clone(&events), Arc::clone(&lose));
    let transport = TestTransport::builder()
        .inner(server_end)
        .start(StartMode::Relay(64))
        .on_inbound(move |frame| {
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            let mut kept = Some(frame);
            if let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) {
                let seq = request.sequence_number.unwrap_or(0);
                if lose_next.swap(false, Ordering::SeqCst) {
                    kept = None;
                    inbound.lock().unwrap().push(format!("lost {seq}"));
                } else {
                    inbound.lock().unwrap().push(format!("in {seq}"));
                }
            }
            async move { kept }
        })
        .on_send(move |frame| {
            let npdu = decode_npdu(frame.npdu).unwrap();
            let event = match decode_apdu(npdu.payload).unwrap() {
                Apdu::SegmentAck(ack) if !ack.negative_ack => {
                    format!("ack {}", ack.sequence_number)
                }
                Apdu::SimpleAck(_) => "answer".into(),
                Apdu::Abort(_) => "abort".into(),
                other => format!("{other:?}"),
            };
            outbound.lock().unwrap().push(event);
            async { Ok(()) }
        })
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Ignore)
        .build();
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
        .apdu_timeout_ms(300)
        .max_apdu_length(480)
        .proposed_window_size(4)
        .build()
        .await
        .unwrap();
    (server, client, events, lose)
}

/// What the log holds once the link is quiet, emptied for the next request.
async fn take_events(events: &Events) -> Vec<String> {
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::mem::take(&mut *events.lock().unwrap())
}

/// Wait until the log satisfies `done`, in steps of a millisecond of the
/// paused clock, for at most a second of it.
async fn wait_for(events: &Events, done: impl Fn(&[String]) -> bool) {
    for _ in 0..1_000 {
        if done(&events.lock().unwrap()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("never happened: {:?}", events.lock().unwrap());
}

/// This server lost one copy of segment zero, so the client remembered it
/// and sends its next request's whole first window (#2742). The server
/// acknowledges segment zero at once, which forgets it. That request is held
/// executing (the database's write lock) until the segment the client sends
/// again after segment zero's acknowledgment is acknowledged again (#2751);
/// it ends with the answer and no Abort, and the next request sends
/// segment zero alone. Paused time: the one segment timeout is the lost
/// copy's.
#[tokio::test(start_paused = true)]
async fn a_server_that_missed_segment_zero_once_is_forgotten_at_its_next_answer() {
    let (mut server, client, events, lose_next) = logged_server_and_client().await;
    let client = Arc::new(client);
    let text = "r".repeat(1_100);
    lose_next.store(true, Ordering::SeqCst);
    write_text(&client, &text).await.unwrap();
    assert_eq!(
        take_events(&events).await,
        ["lost 0", "in 0", "ack 0", "in 1", "in 2", "ack 2", "answer"]
    );

    let db = Arc::clone(server.database());
    let guard = db.write().await;
    let writing = {
        let (client, text) = (Arc::clone(&client), text.clone());
        tokio::spawn(async move { write_text(&client, &text).await })
    };
    // It sends segment 1 again (the final one goes once), and that is
    // acknowledged again with the last segment.
    let count = |log: &[String], event: &str| log.iter().filter(|e| *e == event).count();
    let acknowledged_again = |log: &[String]| count(log, "in 1") == 2 && count(log, "ack 2") == 2;
    wait_for(&events, acknowledged_again).await;
    drop(guard);
    writing.await.unwrap().unwrap();
    let whole = take_events(&events).await;
    assert_eq!(whole[..3], ["in 0", "in 1", "in 2"], "{whole:?}");
    assert_eq!(
        whole.last().map(String::as_str),
        Some("answer"),
        "{whole:?}"
    );
    assert!(!whole.iter().any(|event| event == "abort"), "{whole:?}");

    write_text(&client, &text).await.unwrap();
    assert_eq!(
        take_events(&events).await,
        ["in 0", "ack 0", "in 1", "in 2", "ack 2", "answer"]
    );
    assert_eq!(present_value(&server).await, text);
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
    server.stop().await.unwrap();
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
#[tokio::test(start_paused = true)]
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
