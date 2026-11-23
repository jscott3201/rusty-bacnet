//! Segmentation on a session, frame by frame (#2617). The peer is the test:
//! it writes each segment, Segment-ACK and Abort itself, so one peer can hold
//! a transfer with each of the node's roles at once, under one invoke ID, and
//! break either one off.
use super::segmentation::{identity, node};
use super::*;
use bacnet_encoding::apdu::{AbortPdu, ComplexAck, SegmentAck};
use bacnet_encoding::primitives::encode_app_character_string;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::enums::{AbortReason, Segmentation};
use bacnet_types::error::{Error, RemoteError};
use std::sync::Arc;

/// The peer's MAC. No device answers on it: what the node sends it is only
/// logged.
pub(super) fn peer() -> MacAddr {
    Wire::mac(PEER)
}

/// Put `apdu` from the peer in front of the node.
pub(super) fn from_peer(wire: &Wire, apdu: &Apdu) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire.deliver(&Wire::mac(NODE), wire::frame(&peer(), npdu.freeze(), false));
}

/// Segment `seq` of a request from the peer for `service`.
pub(super) fn request_segment(
    invoke_id: u8,
    seq: u8,
    more_follows: bool,
    service: ConfirmedServiceChoice,
    data: Bytes,
) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: true,
        more_follows,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(seq),
        proposed_window_size: Some(1),
        service_choice: service,
        service_request: data,
    })
}

/// Segment `seq` of two of the peer's ReadProperty request: the node's
/// AI-1 Present_Value, whose answer fits one APDU.
pub(super) fn read_segment(invoke_id: u8, seq: u8) -> Apdu {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ai1(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut request);
    let request = request.freeze();
    let half = request.len() / 2;
    let data = if seq == 0 {
        request.slice(..half)
    } else {
        request.slice(half..)
    };
    request_segment(
        invoke_id,
        seq,
        seq == 0,
        ConfirmedServiceChoice::READ_PROPERTY,
        data,
    )
}

/// The peer's answer to a read of its AI-1's name, `name`, in `parts`
/// segments.
fn answer_segments(invoke_id: u8, name: &str, parts: usize) -> Vec<Apdu> {
    let mut value = BytesMut::new();
    encode_app_character_string(&mut value, name).unwrap();
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: ai1(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut ack)
    .unwrap();
    let ack = ack.freeze();
    let size = ack.len().div_ceil(parts);
    (0..parts)
        .map(|seq| {
            Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: seq + 1 < parts,
                invoke_id,
                sequence_number: Some(seq as u8),
                proposed_window_size: Some(1),
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: ack.slice(seq * size..ack.len().min((seq + 1) * size)),
            })
        })
        .collect()
}

/// The first APDU the node sent the peer that `want` takes, once it has.
pub(super) async fn sent(log: &SendLog, what: &str, want: impl Fn(&Apdu) -> bool) -> Apdu {
    eventually(WAIT, what, async || sent_to(log, &peer()).iter().any(&want)).await;
    sent_to(log, &peer())
        .into_iter()
        .find(|apdu| want(apdu))
        .unwrap()
}

/// How many APDUs the node sent the peer that `want` takes.
pub(super) fn count(log: &SendLog, want: impl Fn(&Apdu) -> bool) -> usize {
    sent_to(log, &peer())
        .iter()
        .filter(|apdu| want(apdu))
        .count()
}

/// A positive Segment-ACK for `seq` under `invoke_id`, sent by the side
/// that answers (`server`) or by the side that asked.
pub(super) fn ack(server: bool, invoke_id: u8, seq: u8) -> impl Fn(&Apdu) -> bool {
    move |apdu| {
        matches!(apdu, Apdu::SegmentAck(ack)
            if ack.sent_by_server == server && !ack.negative_ack
                && ack.invoke_id == invoke_id && ack.sequence_number == seq)
    }
}

/// An Abort under `invoke_id` sent by the answering side (`server`) or the
/// asking side.
pub(super) fn abort(server: bool, invoke_id: u8) -> impl Fn(&Apdu) -> bool {
    move |apdu| matches!(apdu, Apdu::Abort(abort) if abort.sent_by_server == server && abort.invoke_id == invoke_id)
}

/// The server's answer under `invoke_id`.
pub(super) fn answer(invoke_id: u8) -> impl Fn(&Apdu) -> bool {
    move |apdu| matches!(apdu, Apdu::ComplexAck(ack) if ack.invoke_id == invoke_id)
}

/// A node with both roles, segmented both ways: the session, what it sent,
/// its link, and its client, which [`Reading::read`] reads the peer with.
struct Reading {
    session: EndpointSession<TestTransport>,
    log: SendLog,
    link: TestTransportHandle,
    client: Arc<BACnetClient<TestTransport>>,
}

impl Reading {
    async fn new(wire: &Wire) -> Self {
        let (session, log, link) = node(
            wire,
            SessionRole::Both,
            identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
        )
        .await;
        let client = Arc::new(session.client().unwrap());
        Self {
            session,
            log,
            link,
            client,
        }
    }

    /// The client reads the peer's AI-1 name: the read, and the invoke ID
    /// its request went under.
    async fn read(&self) -> (tokio::task::JoinHandle<Result<ReadPropertyACK, Error>>, u8) {
        let asked = count(&self.log, |apdu| matches!(apdu, Apdu::ConfirmedRequest(_)));
        let client = Arc::clone(&self.client);
        let read = tokio::spawn(async move {
            client
                .read_property(&peer(), ai1(), PropertyIdentifier::OBJECT_NAME, None)
                .await
        });
        eventually(WAIT, "the client's request", async || {
            count(&self.log, |apdu| matches!(apdu, Apdu::ConfirmedRequest(_))) > asked
        })
        .await;
        let Some(Apdu::ConfirmedRequest(request)) = sent_to(&self.log, &peer())
            .into_iter()
            .filter(|apdu| matches!(apdu, Apdu::ConfirmedRequest(_)))
            .last()
        else {
            unreachable!()
        };
        (read, request.invoke_id)
    }
}

/// One peer reads from the node's server while answering the node's
/// client, both in segments and both under the same invoke ID: each segment
/// and each Segment-ACK reaches the role whose transfer it is, and both
/// complete.
#[tokio::test(start_paused = true)]
async fn one_peer_holds_a_transfer_with_each_role_under_one_invoke_id() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    let (read, id) = lab.read().await;
    let name = "peer-name-".repeat(30);
    let answers = answer_segments(id, &name, 2);

    from_peer(&wire, &read_segment(id, 0));
    sent(&lab.log, "the server's Segment-ACK", ack(true, id, 0)).await;
    from_peer(&wire, &answers[0]);
    sent(&lab.log, "the client's Segment-ACK", ack(false, id, 0)).await;
    from_peer(&wire, &read_segment(id, 1));
    let Apdu::ComplexAck(answered) = sent(&lab.log, "the server's answer", answer(id)).await else {
        unreachable!()
    };
    assert!(!answered.segmented);
    let read_by_peer = ReadPropertyACK::decode(&answered.service_ack).unwrap();
    assert_eq!(read_by_peer.object_identifier, ai1());
    // REAL 42.0.
    assert_eq!(read_by_peer.property_value, [0x44, 0x42, 0x28, 0x00, 0x00]);
    from_peer(&wire, &answers[1]);
    let read_by_node = read.await.unwrap().unwrap();
    assert!(read_by_node.property_value.ends_with(name.as_bytes()));
    assert_eq!(count(&lab.log, |apdu| matches!(apdu, Apdu::Abort(_))), 0);

    drop(lab.client);
    lab.session.stop().await.unwrap();
}

/// The peer aborts its own request mid-transfer: the server drops it, and
/// the client's transfer under the same invoke ID goes on to its end. Then
/// the peer aborts an answer to the client: the client's read fails, and the
/// server's transfer under that invoke ID goes on to its end.
#[tokio::test(start_paused = true)]
async fn an_abort_in_one_role_leaves_the_others_transfer_alone() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    let name = "peer-name-".repeat(30);

    // The peer gives up its request.
    let (read, id) = lab.read().await;
    let answers = answer_segments(id, &name, 2);
    from_peer(&wire, &read_segment(id, 0));
    sent(&lab.log, "the server's Segment-ACK", ack(true, id, 0)).await;
    from_peer(&wire, &answers[0]);
    sent(&lab.log, "the client's Segment-ACK", ack(false, id, 0)).await;
    from_peer(
        &wire,
        &Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id: id,
            abort_reason: AbortReason::OTHER,
        }),
    );
    from_peer(&wire, &answers[1]);
    let read_by_node = read.await.unwrap().unwrap();
    assert!(read_by_node.property_value.ends_with(name.as_bytes()));
    assert_eq!(count(&lab.log, answer(id)), 0, "the server dropped it");

    // The peer gives up its answer to the client.
    let (read, id) = lab.read().await;
    let answers = answer_segments(id, &name, 2);
    from_peer(&wire, &read_segment(id, 0));
    sent(&lab.log, "the server's Segment-ACK", ack(true, id, 0)).await;
    from_peer(&wire, &answers[0]);
    sent(&lab.log, "the client's Segment-ACK", ack(false, id, 0)).await;
    from_peer(
        &wire,
        &Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id: id,
            abort_reason: AbortReason::OTHER,
        }),
    );
    assert!(matches!(
        read.await.unwrap(),
        Err(Error::Remote(RemoteError::Abort { reason })) if reason == AbortReason::OTHER
    ));
    from_peer(&wire, &read_segment(id, 1));
    sent(&lab.log, "the server's answer", answer(id)).await;
    // Neither role aborted anything itself.
    assert_eq!(count(&lab.log, |apdu| matches!(apdu, Apdu::Abort(_))), 0);

    drop(lab.client);
    lab.session.stop().await.unwrap();
}

/// A segment of the peer's answer goes missing: the client asks for it
/// again, the server's transfer under the same invoke ID completes
/// meanwhile, and the client's completes once the peer sends the rest.
#[tokio::test(start_paused = true)]
async fn a_missing_segment_in_one_role_leaves_the_other_alone() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    let name = "peer-name-".repeat(30);
    let (read, id) = lab.read().await;
    let answers = answer_segments(id, &name, 3);

    from_peer(&wire, &read_segment(id, 0));
    sent(&lab.log, "the server's Segment-ACK", ack(true, id, 0)).await;
    from_peer(&wire, &answers[0]);
    sent(&lab.log, "the client's Segment-ACK", ack(false, id, 0)).await;
    // Segment 1 is lost; segment 2 draws the client's negative Segment-ACK.
    from_peer(&wire, &answers[2]);
    let Apdu::SegmentAck(negative) = sent(
        &lab.log,
        "the client's negative Segment-ACK",
        |apdu| matches!(apdu, Apdu::SegmentAck(ack) if ack.negative_ack),
    )
    .await
    else {
        unreachable!()
    };
    assert!(!negative.sent_by_server);
    assert_eq!((negative.invoke_id, negative.sequence_number), (id, 0));
    // The server's transfer isn't held up by it.
    from_peer(&wire, &read_segment(id, 1));
    sent(&lab.log, "the server's answer", answer(id)).await;
    // The peer sends the rest again.
    from_peer(&wire, &answers[1]);
    from_peer(&wire, &answers[2]);
    let read_by_node = read.await.unwrap().unwrap();
    assert!(read_by_node.property_value.ends_with(name.as_bytes()));
    assert_eq!(count(&lab.log, |apdu| matches!(apdu, Apdu::Abort(_))), 0);

    drop(lab.client);
    lab.session.stop().await.unwrap();
}

/// A segmented answer, or an answering side's Segment-ACK, that no request
/// of the node's holds draws one Abort, from the client. A requester's
/// Segment-ACK for nothing the server sends is the server's, and the client
/// takes no part in it.
#[tokio::test(start_paused = true)]
async fn an_answer_no_request_holds_is_aborted_once_by_the_client() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    let stray = |server: bool, invoke_id: u8| {
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: server,
            invoke_id,
            sequence_number: 0,
            actual_window_size: 1,
        })
    };
    from_peer(&wire, &answer_segments(0x77, "stray", 2)[0]);
    sent(&lab.log, "the client's Abort", abort(false, 0x77)).await;
    from_peer(&wire, &stray(true, 0x78));
    sent(&lab.log, "the client's Abort", abort(false, 0x78)).await;
    from_peer(&wire, &stray(false, 0x79));
    // Long enough for anything else to have gone out.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(count(&lab.log, abort(false, 0x77)), 1);
    assert_eq!(count(&lab.log, abort(false, 0x78)), 1);
    assert_eq!(count(&lab.log, abort(false, 0x79)), 0, "not the client's");
    assert_eq!(count(&lab.log, abort(true, 0x77)), 0, "not the server's");
    assert_eq!(count(&lab.log, abort(true, 0x78)), 0, "not the server's");

    drop(lab.client);
    lab.session.stop().await.unwrap();
}

/// Each role midway through a transfer: the client reassembling an answer
/// and the server a request.
async fn midway(
    wire: &Wire,
    lab: &Reading,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let (read, id) = lab.read().await;
    from_peer(wire, &answer_segments(id, &"peer-name-".repeat(30), 2)[0]);
    sent(&lab.log, "the client's Segment-ACK", ack(false, id, 0)).await;
    from_peer(wire, &read_segment(id, 0));
    sent(&lab.log, "the server's Segment-ACK", ack(true, id, 0)).await;
    assert_eq!(lab.session.active_leases(), 1);
    read
}

/// A stop midway through a transfer in each role ends both: the client's
/// read fails, its invoke ID goes back to the pool, and nothing more is
/// sent, not even once every segment timer has run out.
#[tokio::test(start_paused = true)]
async fn a_stop_midway_ends_both_transfers_and_holds_nothing() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    let read = midway(&wire, &lab).await;
    let before = lab.log.len();

    lab.session.stop().await.unwrap();
    assert!(read.await.unwrap().is_err());
    assert_eq!(lab.session.active_leases(), 0);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(lab.log.len(), before);
    assert_eq!(lab.link.stops(), 1);
    drop(lab.client);
}

/// A session dropped midway through a transfer in each role ends both as a
/// stop does, and aborts its link (#2729).
#[tokio::test(start_paused = true)]
async fn a_drop_midway_ends_both_transfers_and_holds_nothing() {
    let wire = Wire::new();
    let lab = Reading::new(&wire).await;
    let read = midway(&wire, &lab).await;
    let before = lab.log.len();
    let coordinator = lab.session.coordinator();
    let Reading {
        session,
        log,
        link,
        client,
    } = lab;

    drop(session);
    assert!(read.await.unwrap().is_err());
    eventually(WAIT, "the client's invoke ID back", async || {
        coordinator.active_count().unwrap() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(log.len(), before);
    assert_eq!(link.aborts(), 1);
    drop(client);
}

/// The client paces a segmented request by its Device's
/// APDU_Segment_Timeout (2000 ms here), not by its APDU timeout (6000 ms):
/// an unacknowledged first segment goes again 2000 ms after it went.
#[tokio::test(start_paused = true)]
async fn the_clients_segment_timeout_is_its_devices() {
    let wire = Wire::new();
    let mut lab = Reading::new(&wire).await;
    // The peer's I-Am tells the client it takes 206 octets and segments.
    let i_am = bacnet_endpoint::identity::encode_iam_apdu(&identity(
        PEER_DEVICE,
        Segmentation::SEGMENTED_BOTH,
    ))
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: Bytes::from(i_am),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire.deliver(&Wire::mac(NODE), wire::frame(&peer(), npdu.freeze(), true));
    eventually(WAIT, "the peer's I-Am", async || {
        lab.client
            .discovered_devices()
            .await
            .iter()
            .any(|device| device.object_identifier.instance_number() == PEER_DEVICE)
    })
    .await;

    let mut value = BytesMut::new();
    encode_app_character_string(&mut value, &"described-".repeat(30)).unwrap();
    let client = Arc::clone(&lab.client);
    let write = tokio::spawn(async move {
        client
            .write_property(
                &peer(),
                ai1(),
                PropertyIdentifier::DESCRIPTION,
                None,
                value.to_vec(),
                None,
            )
            .await
    });
    let first = |apdu: &Apdu| {
        matches!(apdu, Apdu::ConfirmedRequest(request)
            if request.segmented && request.sequence_number == Some(0))
    };
    sent(&lab.log, "the first segment", first).await;
    let went = tokio::time::Instant::now();
    eventually(WAIT, "the first segment again", async || {
        count(&lab.log, first) == 2
    })
    .await;
    let again = went.elapsed();
    assert!(
        again >= Duration::from_millis(1_990) && again <= Duration::from_millis(2_010),
        "sent again after {again:?}"
    );

    lab.session.stop().await.unwrap();
    assert!(write.await.unwrap().is_err());
    drop(lab.client);
}
