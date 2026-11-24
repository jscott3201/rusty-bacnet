//! Bursts of traffic a node can't keep up with (#2635): the I-Ams a global
//! Who-Is brings, a device that floods them, confirmed requests, answers
//! nobody asked for. A full queue drops and counts what it has no room for;
//! the session stays up, and an Unconfirmed-Request burst can't crowd out
//! the answers to the node's own requests or its replies to a peer.
use super::*;
use bacnet_encoding::apdu::SimpleAck;
use bacnet_endpoint::session::PolicyCountersSnapshot;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};

/// Deep enough for the longest burst here to reach the node whole, as a
/// socket buffer with room for it would hand it over.
const DEEP: usize = 4_096;

/// The queue size the overflow cases run with: the default before #2635,
/// small enough that each burst here overflows it.
const SMALL: usize = 16;

/// [`lab`] with queues of `capacity`, a receive queue [`DEEP`] frames deep
/// and no COV reporting.
async fn burst_lab(capacity: usize) -> Lab {
    let wire = Wire::new();
    let peer_port = wire.port(PEER);
    let peer_sent = peer_port.sent();
    let peer = peer_server(peer_port.build()).await;
    let port = wire.port_with_queue(NODE, DEEP);
    let (node_sent, node_link) = (port.sent(), port.handle());
    let identity = DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    input.set_present_value(20.0);
    let database = build_database_with_extra(&identity, vec![Box::new(input)]).unwrap();
    let config = SessionConfig {
        queue_capacity: capacity,
        ..SessionConfig::default()
    };
    let mut session = EndpointSession::new(port.build(), SessionRole::Both, config)
        .unwrap()
        .device(identity)
        .with_database(database);
    session.start().await.unwrap();
    Lab {
        wire,
        session,
        node_sent,
        node_link,
        peer,
        peer_sent,
    }
}

/// Station `i`'s B/IP MAC, on a subnet of its own (up to 65,536 of them).
fn station(i: usize) -> MacAddr {
    MacAddr::from_slice(&encode_bip_mac([10, 0, (i >> 8) as u8, i as u8], 47_808))
}

/// The I-Am of device `instance` in an NPDU with no addresses.
fn i_am(instance: u32) -> Bytes {
    let object = (ObjectType::DEVICE.to_raw() << 22) | instance;
    let mut npdu = vec![0x01, 0x00, 0x10, 0x00, 0xC4];
    npdu.extend_from_slice(&object.to_be_bytes());
    // Max_APDU 480, no segmentation, vendor 42.
    npdu.extend_from_slice(&[0x22, 0x01, 0xE0, 0x91, 0x03, 0x21, 0x2A]);
    Bytes::from(npdu)
}

/// One frame of each kind of burst, from station `i`.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// The answers to a global Who-Is: an I-Am from each of many devices.
    IAmFromMany,
    /// A device that floods its own I-Am.
    IAmFromOne,
    /// A ReadProperty of AI-1's name from each of many devices.
    ReadFromMany,
    /// Simple-ACKs no lease holds.
    StrayAnswers,
}

impl Kind {
    fn frame(self, i: usize) -> ReceivedNpdu {
        let invoke_id = (i % 256) as u8;
        match self {
            Kind::IAmFromMany => wire::frame(&station(i), i_am(10_000 + i as u32), true),
            Kind::IAmFromOne => wire::frame(&station(0), i_am(10_000), true),
            Kind::ReadFromMany => wire::frame(
                &station(i),
                npdu_of(
                    &confirmed(
                        invoke_id,
                        ConfirmedServiceChoice::READ_PROPERTY,
                        &read_name(ai1()),
                    ),
                    true,
                ),
                false,
            ),
            Kind::StrayAnswers => wire::frame(
                &station(i),
                npdu_of(
                    &Apdu::SimpleAck(SimpleAck {
                        invoke_id,
                        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                    }),
                    false,
                ),
                false,
            ),
        }
    }
}

/// Every full-queue drop the session counted.
fn full_drops(counters: &PolicyCountersSnapshot) -> u64 {
    counters.unconfirmed_full_drops
        + counters.confirmed_full_drops
        + counters.answer_full_drops
        + counters.policy_full_drops
}

/// Hold the node's deliveries, put `frames` in front of it, and release
/// them at once: the node reads the whole burst before its roles run.
fn burst(lab: &Lab, frames: impl IntoIterator<Item = ReceivedNpdu>) {
    let node = Wire::mac(NODE);
    lab.wire.hold(&node);
    for frame in frames {
        lab.wire.deliver(&node, frame);
    }
    lab.wire.release(&node);
}

/// Every kind of burst at every size, into queues of the old default and of
/// today's: the session stays up, still answers and reads, and counts each
/// drop under its own class.
#[tokio::test(start_paused = true)]
async fn no_burst_of_any_kind_or_size_ends_the_session() {
    let kinds = [
        Kind::IAmFromMany,
        Kind::IAmFromOne,
        Kind::ReadFromMany,
        Kind::StrayAnswers,
    ];
    let capacities = [SMALL, SessionConfig::default().queue_capacity];
    for (capacity, kind) in capacities
        .into_iter()
        .flat_map(|capacity| kinds.map(|kind| (capacity, kind)))
    {
        for size in [50, 100, 300, 1_000] {
            let lab = burst_lab(capacity).await;
            let client = lab.session.client().unwrap();
            burst(&lab, (0..size).map(|i| kind.frame(i)));
            tokio::time::sleep(Duration::from_secs(2)).await;

            let what = format!("{kind:?} x {size} into {capacity}");
            assert!(lab.session.is_running(), "{what}");
            lab.session.broadcast_i_am().await.expect(&what);
            let counters = lab.session.policy_counters();
            // Requests go to the server's seat, whose own queue counts what
            // it drops; the session's queues keep the rest.
            let server_drops = lab.session.server_seat_counters().full_drops;
            let drops = full_drops(&counters) + server_drops;
            // The server answers each read it takes, an Abort when its
            // handlers have no room.
            let answered = apdus(&lab.node_sent)
                .iter()
                .filter(|(_, apdu)| matches!(apdu, Apdu::ComplexAck(_) | Apdu::Abort(_)))
                .count() as u64;
            // Past its handlers' room and the Aborts it may send, the
            // server drops a request and counts it.
            let admission = lab.session.server().unwrap().request_admission_counters();
            let unanswered = server_drops + admission.confirmed_fallback_dropped_total;
            match kind {
                Kind::IAmFromMany | Kind::IAmFromOne => {
                    assert_eq!(full_drops(&counters), 0, "{what}");
                }
                // Each request is answered or counted, never both.
                Kind::ReadFromMany => {
                    assert_eq!(full_drops(&counters), 0, "{what}");
                    assert_eq!(answered + unanswered, size as u64, "{what}");
                    assert!(answered > 0, "{what}");
                }
                Kind::StrayAnswers => {
                    assert_eq!(drops, counters.answer_full_drops, "{what}");
                }
            }
            if capacity == SMALL && size >= 100 && !matches!(kind, Kind::IAmFromOne) {
                assert!(drops > 0, "{what}: the burst overflowed");
            }

            // The node still answers a peer, and its client still reads.
            let other = other_client(&lab.wire).await;
            let name = other
                .read_property(
                    &Wire::mac(NODE),
                    ai1(),
                    PropertyIdentifier::OBJECT_NAME,
                    None,
                )
                .await
                .expect(&what);
            assert_eq!(name.object_identifier, ai1());
            client
                .read_property(
                    &Wire::mac(PEER),
                    ai1(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                )
                .await
                .expect(&what);

            drop(client);
            let mut other = other;
            other.stop().await.unwrap();
            stop(lab).await;
        }
    }
}

/// The session hands ingress its coordinator: a flood of answers no lease
/// holds, ahead of the answer to the client's read, overflows the session's
/// small answer queue and is dropped there, while the answer the client's
/// lease holds goes to its seat and still gets in.
#[tokio::test(start_paused = true)]
async fn a_client_read_completes_through_a_flood_of_stray_answers() {
    let lab = burst_lab(SMALL).await;
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    lab.wire.hold(&node);
    for i in 0..100 {
        lab.wire.deliver(&node, Kind::StrayAnswers.frame(i));
    }
    let drive = async {
        eventually(WAIT, "the peer's answer held for the node", async || {
            lab.wire.held(&node) == 101
        })
        .await;
        lab.wire.release(&node);
    };
    let client = lab.session.client().unwrap();
    let (read, ()) = tokio::join!(
        client.read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None),
        drive,
    );
    assert_eq!(read.unwrap().object_identifier, ai1());
    let counters = lab.session.policy_counters();
    assert!(counters.answer_full_drops > 0, "{counters:?}");
    stop(lab).await;
}

/// A read by the node's client and a peer's read of the node both land in
/// the middle of a 1,000-frame I-Am burst: the burst's overflow is dropped
/// and counted, both reads are answered, and the node answers after it too.
#[tokio::test(start_paused = true)]
async fn reads_in_flight_complete_during_a_thousand_frame_i_am_burst() {
    let lab = burst_lab(SMALL).await;
    let client = lab.session.client().unwrap();
    let mut other = other_client(&lab.wire).await;
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));

    lab.wire.hold(&node);
    for i in 0..500 {
        lab.wire.deliver(&node, Kind::IAmFromMany.frame(i));
    }
    let drive = async {
        // The peer's answer to the client, and the other node's request.
        eventually(WAIT, "both reads held for the node", async || {
            lab.wire.held(&node) == 502
        })
        .await;
        for i in 500..1_000 {
            lab.wire.deliver(&node, Kind::IAmFromMany.frame(i));
        }
        lab.wire.release(&node);
    };
    let (own, peers, ()) = tokio::join!(
        client.read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None),
        other.read_property(&node, ai1(), PropertyIdentifier::OBJECT_NAME, None),
        drive,
    );
    assert_eq!(own.unwrap().object_identifier, ai1());
    assert_eq!(peers.unwrap().object_identifier, ai1());

    // The server's seat dropped I-Ams; the session's own queues none.
    let counters = lab.session.policy_counters();
    let seat = lab.session.server_seat_counters();
    assert!(seat.full_drops > 0, "{seat:?}");
    assert_eq!(full_drops(&counters), 0, "{counters:?}");
    assert!(lab.session.is_running());
    other
        .read_property(&node, ai1(), PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();

    drop(client);
    other.stop().await.unwrap();
    stop(lab).await;
}

/// MS/TP: a request whose sender waits for the reply comes with a one-use
/// reply sender. In the middle of a burst it still gets its queue slot and
/// is answered through that sender, at once; one the node can't take has
/// its sender released, which has the MAC send Reply Postponed.
#[tokio::test(start_paused = true)]
async fn an_mstp_request_expecting_a_reply_is_answered_in_the_middle_of_a_burst() {
    let lab = burst_lab(SMALL).await;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let request = ReceivedNpdu {
        direct_response: None,
        npdu: npdu_of(
            &confirmed(7, ConfirmedServiceChoice::READ_PROPERTY, &read_name(ai1())),
            true,
        ),
        source_mac: station(5_000),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(reply_tx),
    };
    let frames = (0..500)
        .map(|i| Kind::IAmFromMany.frame(i))
        .chain([request])
        .chain((500..1_000).map(|i| Kind::IAmFromMany.frame(i)));
    burst(&lab, frames);

    let reply = tokio::time::timeout(WAIT, reply_rx).await.unwrap().unwrap();
    let Apdu::ComplexAck(ack) = decode_apdu(decode_npdu(reply).unwrap().payload).unwrap() else {
        panic!("a ComplexACK");
    };
    assert_eq!(ack.invoke_id, 7);
    let dropped = offered(&lab, 1_001).await;
    assert!(dropped.0 > 0, "the server's seat dropped I-Ams");

    // With the queue held full of confirmed requests (the share kept for
    // them used up too), the next one's reply sender is released unanswered.
    let held: Vec<_> = (0..200)
        .map(|i| {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let mut frame = Kind::ReadFromMany.frame(i);
            frame.reply_tx = Some(reply_tx);
            (frame, reply_rx)
        })
        .collect();
    let (frames, replies): (Vec<_>, Vec<_>) = held.into_iter().unzip();
    burst(&lab, frames);
    let mut answered = 0;
    let mut postponed = 0;
    for reply in replies {
        match tokio::time::timeout(WAIT, reply).await.unwrap() {
            Ok(_) => answered += 1,
            Err(_) => postponed += 1,
        }
    }
    assert!(
        postponed > 0 && answered > 0,
        "{answered} answered, {postponed} postponed"
    );
    // Each one released unanswered was dropped by the server's seat, or by
    // its admission past the Aborts it may send.
    let now = offered(&lab, 1_201).await;
    assert_eq!(postponed, (now.0 - dropped.0) + (now.1 - dropped.1));
    stop(lab).await;
}

/// Once `count` requests have reached the server's seat, taken or dropped:
/// how many its seat dropped, and how many its admission dropped.
async fn offered(lab: &Lab, count: u64) -> (u64, u64) {
    eventually(WAIT, "every request offered to the seat", async || {
        let seat = lab.session.server_seat_counters();
        seat.apdus + seat.full_drops == count
    })
    .await;
    let seat = lab.session.server_seat_counters();
    let server = lab.session.server().unwrap();
    let admission = server.request_admission_counters();
    (seat.full_drops, admission.confirmed_fallback_dropped_total)
}
