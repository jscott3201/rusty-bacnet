//! Segmentation on a session (#2617): both roles segment, in both
//! directions, beside each other, and as the node's Device says.
//!
//! The node's identity takes a small APDU (206 octets), so a read of a long
//! name, or of 120 property values, needs segments either way. Its peers are
//! a `bacnet-server` device and standalone clients on the same in-memory
//! link.
use super::*;
use bacnet_types::enums::{AbortReason, Segmentation};
use bacnet_types::error::{Error, RemoteError, SegmentationError};

/// The APDU every device in these tests takes.
const SMALL: u16 = 206;

/// A name long enough to need several segments of [`SMALL`] octets.
fn long_name() -> String {
    "segmented-".repeat(60)
}

/// AI-2, whose name is [`long_name`].
fn ai2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
}

/// Device `instance` taking [`SMALL`] octets, segmenting as `mode`, with a
/// segment timeout of 2000 ms.
pub(super) fn identity(instance: u32, mode: Segmentation) -> DeviceIdentity {
    DeviceIdentity::new(instance, 42)
        .unwrap()
        .with_max_apdu(SMALL)
        .unwrap()
        .with_segmentation(mode)
        .with_apdu_segment_timeout_ms(2_000)
}

/// `identity`'s database: AI-1 at 42.0 and AI-2 named [`long_name`].
fn database_of(identity: &DeviceIdentity) -> bacnet_objects::database::ObjectDatabase {
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(42.0);
    let named = AnalogInputObject::new(2, long_name(), EngineeringUnits::DEGREES_CELSIUS).unwrap();
    build_database_with_extra(identity, vec![Box::new(input), Box::new(named)]).unwrap()
}

/// The node: a session on [`NODE`] with `role` and `identity`, and a full
/// server any peer may write to when it has a server role, started.
pub(super) async fn node(
    wire: &Wire,
    role: SessionRole,
    identity: DeviceIdentity,
) -> (EndpointSession<TestTransport>, SendLog, TestTransportHandle) {
    let port = wire.port(NODE);
    let (sent, link) = (port.sent(), port.handle());
    let mut session = EndpointSession::new(port.build(), role, SessionConfig::default())
        .unwrap()
        .with_database(database_of(&identity))
        .device(identity);
    if role != SessionRole::ClientOnly {
        session = session.with_server(BACnetServer::generic_builder());
    }
    session.start().await.unwrap();
    (session, sent, link)
}

/// A `bacnet-server` device on node `n` with `identity`.
async fn device(wire: &Wire, n: u8, identity: DeviceIdentity) -> BACnetServer<TestTransport> {
    BACnetServer::generic_builder()
        .database(database_of(&identity))
        .device(identity)
        .transport(wire.port(n).build())
        .build()
        .await
        .unwrap()
}

/// A standalone client on node `n` that takes [`SMALL`] octets and gives an
/// answer two seconds, once.
async fn reader(wire: &Wire, n: u8) -> BACnetClient<TestTransport> {
    reader_with(wire, n, |builder| builder).await
}

/// [`reader`], with what `tune` sets on its builder besides.
async fn reader_with(
    wire: &Wire,
    n: u8,
    tune: impl FnOnce(
        bacnet_client::client::ClientBuilder<TestTransport>,
    ) -> bacnet_client::client::ClientBuilder<TestTransport>,
) -> BACnetClient<TestTransport> {
    tune(
        BACnetClient::generic_builder()
            .transport(wire.port(n).build())
            .max_apdu_length(SMALL)
            .apdu_timeout_ms(2_000)
            .apdu_retries(0),
    )
    .build()
    .await
    .unwrap()
}

/// 120 reads of AI-1's Present_Value: a request too long for [`SMALL`]
/// octets, and an answer longer still.
fn many_reads() -> Vec<ReadAccessSpecification> {
    vec![ReadAccessSpecification {
        object_identifier: ai1(),
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            };
            120
        ],
    }]
}

/// `asker` hears the I-Am of Device `instance`, which tells it the device's
/// APDU and segmentation.
async fn learn(asker: &BACnetClient<TestTransport>, instance: u32) {
    asker.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the I-Am", async || {
        asker
            .discovered_devices()
            .await
            .iter()
            .any(|device| device.object_identifier.instance_number() == instance)
    })
    .await;
}

/// The 120 values of [`many_reads`] came back.
fn assert_many(answer: Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error>) {
    let answer = answer.unwrap();
    let results = &answer.list_of_read_access_results[0].list_of_results;
    assert_eq!(results.len(), 120);
    assert!(results.iter().all(|result| result.property_value.is_some()));
}

/// The node's Confirmed-Requests to `to`, segments included.
fn requests_to(log: &SendLog, to: &MacAddr) -> Vec<ConfirmedRequest> {
    sent_to(log, to)
        .into_iter()
        .filter_map(|apdu| match apdu {
            Apdu::ConfirmedRequest(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// `answer` is an Abort for `reason` from the device asked.
fn assert_aborted<T: std::fmt::Debug>(answer: Result<T, Error>, reason: AbortReason) {
    match answer {
        Err(Error::Remote(RemoteError::Abort { reason: got })) => assert_eq!(got, reason),
        other => panic!("expected an Abort for {reason:?}, got {other:?}"),
    }
}

/// While the client reads a segmented answer from one peer, the server
/// sends a segmented answer to another; each completes, and each
/// Segment-ACK comes from the role whose transfer it is.
#[tokio::test(start_paused = true)]
async fn both_roles_receive_and_send_segments_at_once() {
    let wire = Wire::new();
    let mut peer = device(
        &wire,
        PEER,
        identity(PEER_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let mut other = reader(&wire, OTHER).await;
    // The identity's default: segmented both ways.
    let node_identity = DeviceIdentity::new(NODE_DEVICE, 42)
        .unwrap()
        .with_max_apdu(SMALL)
        .unwrap();
    let (mut session, node_sent, _link) = node(&wire, SessionRole::Both, node_identity).await;
    let client = session.client().unwrap();
    let (peer_mac, node_mac, other_mac) = (Wire::mac(PEER), Wire::mac(NODE), Wire::mac(OTHER));

    let (asked, answered) = tokio::join!(
        client.read_property(&peer_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None),
        other.read_property(&node_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None),
    );
    assert!(
        asked
            .unwrap()
            .property_value
            .ends_with(long_name().as_bytes())
    );
    assert!(
        answered
            .unwrap()
            .property_value
            .ends_with(long_name().as_bytes())
    );

    // The client acknowledged each segment of the peer's answer.
    let to_peer = sent_to(&node_sent, &peer_mac);
    let acks: Vec<_> = to_peer
        .iter()
        .filter_map(|apdu| match apdu {
            Apdu::SegmentAck(ack) => Some(ack),
            _ => None,
        })
        .collect();
    assert!(acks.len() >= 3, "{to_peer:?}");
    assert!(
        acks.iter()
            .all(|ack| !ack.sent_by_server && !ack.negative_ack)
    );
    // The server sent its answer to the other reader in segments.
    let to_other = sent_to(&node_sent, &other_mac);
    let segments = to_other
        .iter()
        .filter(|apdu| matches!(apdu, Apdu::ComplexAck(ack) if ack.segmented))
        .count();
    assert!(segments >= 3, "{to_other:?}");
    assert!(
        !to_other
            .iter()
            .any(|apdu| matches!(apdu, Apdu::SegmentAck(_))),
        "nothing the other reader sent was segmented"
    );
    assert!(
        !to_peer
            .iter()
            .chain(&to_other)
            .any(|apdu| matches!(apdu, Apdu::Abort(_)))
    );
    assert_eq!(session.client_seat_counters().full_drops, 0);
    assert_eq!(session.server_seat_counters().full_drops, 0);

    drop(client);
    session.stop().await.unwrap();
    other.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// The client sends a segmented request to one peer while another sends a
/// segmented request to the server: both are reassembled and answered, in
/// segments, both ways.
#[tokio::test(start_paused = true)]
async fn both_roles_send_and_receive_segmented_requests_at_once() {
    let wire = Wire::new();
    let mut peer = device(
        &wire,
        PEER,
        identity(PEER_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let mut other = reader(&wire, OTHER).await;
    let (mut session, node_sent, _link) = node(
        &wire,
        SessionRole::Both,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    // Each asker learns the other side's APDU from its I-Am, so it segments.
    learn(&client, PEER_DEVICE).await;
    learn(&other, NODE_DEVICE).await;
    let (peer_mac, node_mac, other_mac) = (Wire::mac(PEER), Wire::mac(NODE), Wire::mac(OTHER));

    let (asked, answered) = tokio::join!(
        client.read_property_multiple(&peer_mac, many_reads()),
        other.read_property_multiple(&node_mac, many_reads()),
    );
    assert_many(asked);
    assert_many(answered);

    // The client's request went in segments; the server acknowledged the
    // other reader's segments as the answering side.
    let segments = requests_to(&node_sent, &peer_mac)
        .into_iter()
        .filter(|request| request.segmented)
        .count();
    assert!(segments >= 2);
    assert!(
        sent_to(&node_sent, &other_mac)
            .iter()
            .any(|apdu| matches!(apdu, Apdu::SegmentAck(ack) if ack.sent_by_server))
    );

    drop(client);
    session.stop().await.unwrap();
    other.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A client-only session's client and a server-only session's server each
/// segment both ways on their own.
#[tokio::test(start_paused = true)]
async fn one_role_sessions_segment_both_ways() {
    // A client-only node reads from a device.
    let wire = Wire::new();
    let mut peer = device(
        &wire,
        PEER,
        identity(PEER_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let (mut session, _sent, _link) = node(
        &wire,
        SessionRole::ClientOnly,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let client = session.client().unwrap();
    learn(&client, PEER_DEVICE).await;
    let peer_mac = Wire::mac(PEER);
    let name = client
        .read_property(&peer_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();
    assert!(name.property_value.ends_with(long_name().as_bytes()));
    assert_many(client.read_property_multiple(&peer_mac, many_reads()).await);
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();

    // A reader reads from a server-only node.
    let wire = Wire::new();
    let mut other = reader(&wire, OTHER).await;
    let (mut session, _sent, _link) = node(
        &wire,
        SessionRole::ServerOnly,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    learn(&other, NODE_DEVICE).await;
    let node_mac = Wire::mac(NODE);
    let name = other
        .read_property(&node_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();
    assert!(name.property_value.ends_with(long_name().as_bytes()));
    assert_many(other.read_property_multiple(&node_mac, many_reads()).await);
    session.stop().await.unwrap();
    other.stop().await.unwrap();
}

/// The Device's three segmentation properties, read from the node.
async fn served(
    reader: &BACnetClient<TestTransport>,
) -> (Vec<u8>, Result<Vec<u8>, Error>, Result<Vec<u8>, Error>) {
    let node_mac = Wire::mac(NODE);
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let read = async |property| {
        reader
            .read_property(&node_mac, device, property, None)
            .await
            .map(|ack| ack.property_value)
    };
    (
        read(PropertyIdentifier::SEGMENTATION_SUPPORTED)
            .await
            .unwrap(),
        read(PropertyIdentifier::MAX_SEGMENTS_ACCEPTED).await,
        read(PropertyIdentifier::APDU_SEGMENT_TIMEOUT).await,
    )
}

/// The node's Device states its identity's segmentation, and both roles do
/// what it states: the client's requests say whether it takes segmented
/// answers and how many segments, it segments a request only when the Device
/// transmits segments, and the server reassembles and segments only as the
/// Device says. A peer that reads the node's I-Am honours it too.
#[tokio::test(start_paused = true)]
async fn the_device_states_what_the_session_does() {
    for mode in [
        Segmentation::NO_SEGMENTATION,
        Segmentation::SEGMENTED_RECEIVE,
        Segmentation::SEGMENTED_TRANSMIT,
        Segmentation::SEGMENTED_BOTH,
    ] {
        let receives = matches!(
            mode,
            Segmentation::SEGMENTED_RECEIVE | Segmentation::SEGMENTED_BOTH
        );
        let transmits = matches!(
            mode,
            Segmentation::SEGMENTED_TRANSMIT | Segmentation::SEGMENTED_BOTH
        );
        let wire = Wire::new();
        let mut peer = device(
            &wire,
            PEER,
            identity(PEER_DEVICE, Segmentation::SEGMENTED_BOTH),
        )
        .await;
        let mut other = reader(&wire, OTHER).await;
        let (mut session, node_sent, _link) =
            node(&wire, SessionRole::Both, identity(NODE_DEVICE, mode)).await;
        let client = session.client().unwrap();
        let (peer_mac, node_mac) = (Wire::mac(PEER), Wire::mac(NODE));

        // The Device's properties.
        let (supported, max_segments, timeout) = served(&other).await;
        assert_eq!(supported, [0x91, mode.to_raw()], "{mode:?}");
        if mode == Segmentation::NO_SEGMENTATION {
            assert!(max_segments.is_err() && timeout.is_err(), "{mode:?}");
        } else {
            let accepted = if receives { 65 } else { 1 };
            assert_eq!(max_segments.unwrap(), [0x21, accepted], "{mode:?}");
            assert_eq!(timeout.unwrap(), [0x22, 0x07, 0xd0], "{mode:?}: 2000 ms");
        }

        // The client's request header.
        client
            .read_property(&peer_mac, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap();
        let header = requests_to(&node_sent, &peer_mac).pop().unwrap();
        assert_eq!(header.segmented_response_accepted, receives, "{mode:?}");
        // B'111', more than 64, decodes as 255.
        assert_eq!(header.max_segments, receives.then_some(255), "{mode:?}");

        // A request that needs segments, with a short answer: a long
        // Description for the peer's AI-1.
        learn(&client, PEER_DEVICE).await;
        let description = {
            let mut value = BytesMut::new();
            bacnet_encoding::primitives::encode_app_character_string(
                &mut value,
                &"described-".repeat(30),
            )
            .unwrap();
            value.to_vec()
        };
        let sent_before = requests_to(&node_sent, &peer_mac).len();
        let written = client
            .write_property(
                &peer_mac,
                ai1(),
                PropertyIdentifier::DESCRIPTION,
                None,
                description.clone(),
                None,
            )
            .await;
        if transmits {
            written.unwrap();
        } else {
            assert!(
                matches!(
                    written,
                    Err(Error::Segmentation(SegmentationError::LocalCannotTransmit))
                ),
                "{mode:?}: {written:?}"
            );
            assert_eq!(
                requests_to(&node_sent, &peer_mac).len(),
                sent_before,
                "{mode:?}: nothing was sent"
            );
        }

        // The other reader learns the node's segmentation from its I-Am, and
        // sends it segments only when it receives them.
        learn(&other, NODE_DEVICE).await;
        let written = other
            .write_property(
                &node_mac,
                ai1(),
                PropertyIdentifier::DESCRIPTION,
                None,
                description,
                None,
            )
            .await;
        if receives {
            written.unwrap();
        } else {
            assert!(
                matches!(
                    written,
                    Err(Error::Segmentation(
                        SegmentationError::PeerCannotReceive { .. }
                    ))
                ),
                "{mode:?}: {written:?}"
            );
        }
        // The server answers in segments only when it transmits them.
        let read = other
            .read_property(&node_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None)
            .await;
        if transmits {
            assert!(
                read.unwrap()
                    .property_value
                    .ends_with(long_name().as_bytes())
            );
        } else {
            assert_aborted(read, AbortReason::SEGMENTATION_NOT_SUPPORTED);
        }

        drop(client);
        session.stop().await.unwrap();
        other.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}

/// A reader that takes a window of one segment and few segments gets the
/// node's answer within its limits, and one needing more segments than it
/// takes is aborted, as the node's server honours what a request states.
#[tokio::test(start_paused = true)]
async fn a_conservative_peer_gets_answers_within_its_limits() {
    let wire = Wire::new();
    let mut careful = reader_with(&wire, OTHER, |builder| {
        builder.max_segments(Some(4)).proposed_window_size(1)
    })
    .await;
    let mut stingy = reader_with(&wire, OTHER + 1, |builder| {
        builder.max_segments(Some(2)).proposed_window_size(1)
    })
    .await;
    let (mut session, node_sent, _link) = node(
        &wire,
        SessionRole::Both,
        identity(NODE_DEVICE, Segmentation::SEGMENTED_BOTH),
    )
    .await;
    let node_mac = Wire::mac(NODE);

    let name = careful
        .read_property(&node_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();
    assert!(name.property_value.ends_with(long_name().as_bytes()));
    let segments: Vec<_> = sent_to(&node_sent, &Wire::mac(OTHER))
        .into_iter()
        .filter_map(|apdu| match apdu {
            Apdu::ComplexAck(ack) if ack.segmented => Some(ack),
            _ => None,
        })
        .collect();
    assert!((2..=4).contains(&segments.len()), "{segments:?}");
    // One segment a window: each proposes a window of one.
    assert!(
        segments
            .iter()
            .all(|segment| segment.proposed_window_size == Some(1))
    );

    let read = stingy
        .read_property(&node_mac, ai2(), PropertyIdentifier::OBJECT_NAME, None)
        .await;
    assert_aborted(read, AbortReason::BUFFER_OVERFLOW);

    session.stop().await.unwrap();
    careful.stop().await.unwrap();
    stingy.stop().await.unwrap();
}
