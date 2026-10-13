//! Where the router's Reject-Message-To-Network goes (#1158), on the wire.
//!
//! Per Clause 6.4.4, a reject is meant for whoever first sent the refused NPDU.
//! A relayed NPDU names that node in its SNET/SADR, so the reject carries it
//! as DNET/DADR and goes back to the router that relayed the NPDU; an NPDU
//! from the arrival link draws a plain local unicast, and so does one whose
//! SNET is the arrival network (#1174). One whose SNET is the router's other
//! network gets that local unicast out the other port, and one whose
//! SNET/SADR is the router itself gets nothing (#1219). A received reject is
//! relayed by its DNET/DADR (Clause 6.6.3.5), which is what carries a reject
//! back across a chain of routers to the device, unless it is addressed to
//! the router itself, when it reaches the router's control receiver instead
//! (#1175). Every reject is compared byte for byte, and the MAC each unicast
//! reject goes to is checked too (#1243): a local reject to the SADR has the
//! same bytes as one back to the link sender. A reject also leaves with the
//! data attributes the refused frame arrived with (#1289).

use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::enums::NetworkMessageType;
use bacnet_types::MacAddr;
use bytes::Bytes;

use crate::loopback_fixture::{
    next_frame_from_router, next_received_from_router, recv, wire, FromRouter, RouterFixture,
    ORIGIN, REMOTE, TOO_LONG,
};
use crate::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use crate::router_table::{Mac, PortId, Reachability};

/// A network the fixture router has no route to.
const UNKNOWN: u16 = 5000;
const WHO_IS: u8 = NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw();
/// A reserved network message type, which the router answers with reason 3.
const RESERVED_TYPE: u8 = 0x14;

/// One way the router refuses an NPDU from peer A: the frame, built with the
/// SNET/SADR it is given, and the reject payload it draws (reason, network).
struct Refusal {
    what: &'static str,
    /// Peer B declares network 3000 busy first.
    busy: bool,
    frame: fn(Option<(u16, u8)>) -> Vec<u8>,
    reject: [u8; 3],
}

/// Every reject this router originates, one per call site.
fn refusals() -> [Refusal; 6] {
    [
        Refusal {
            what: "reason 1, APDU to an unknown DNET",
            busy: false,
            frame: |source| wire(Some((UNKNOWN, 0)), source, None),
            reject: [0x01, 0x13, 0x88],
        },
        Refusal {
            what: "reason 1, directed control to an unknown DNET",
            busy: false,
            frame: |source| wire(Some((UNKNOWN, 0)), source, Some(WHO_IS)),
            reject: [0x01, 0x13, 0x88],
        },
        Refusal {
            what: "reason 2, APDU to a busy DNET",
            busy: true,
            frame: |source| wire(Some((REMOTE, 1)), source, None),
            reject: [0x02, 0x0B, 0xB8],
        },
        Refusal {
            what: "reason 2, directed control to a busy DNET",
            busy: true,
            frame: |source| wire(Some((REMOTE, 1)), source, Some(WHO_IS)),
            reject: [0x02, 0x0B, 0xB8],
        },
        Refusal {
            what: "reason 3, reserved message type",
            busy: false,
            frame: |source| wire(None, source, Some(RESERVED_TYPE)),
            reject: [0x03, 0x00, 0x00],
        },
        Refusal {
            what: "reason 6, over-long DADR",
            busy: false,
            frame: |source| wire(Some((REMOTE, TOO_LONG)), source, None),
            reject: [0x06, 0x0B, 0xB8],
        },
    ]
}

/// A fresh fixture router. With `busy`, peer B first declares network 3000
/// busy.
async fn fixture(busy: bool) -> RouterFixture {
    let mut fixture = RouterFixture::start().await;
    if busy {
        let busy = Some(NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw());
        fixture
            .peer_b
            .send_broadcast(&wire(None, None, busy))
            .await
            .unwrap();
        // The router marks 3000 busy before passing the message on to port A.
        let (relayed, _) = next_frame_from_router(&mut fixture.from_router_a).await;
        assert_eq!(relayed[..], [0x01, 0x80, 0x04, 0x0B, 0xB8]);
    }
    fixture
}

/// Send `frame` from peer A to a fresh fixture router and return the first
/// frame the router sends back to peer A, with the MAC it was unicast to.
async fn first_answer(busy: bool, frame: &[u8]) -> (Vec<u8>, MacAddr) {
    let mut fixture = fixture(busy).await;
    fixture.send_from_a(frame).await;
    let (answer, to) = next_frame_from_router(&mut fixture.from_router_a).await;
    fixture.stop().await;
    (answer.to_vec(), to.expect("a reject is a unicast"))
}

#[tokio::test]
async fn router_rejects_a_relayed_npdu_toward_its_snet_sadr() {
    for refusal in refusals() {
        // SNET 4000, SADR 50: the reject's DNET 4000, DLEN 1, DADR 50 and a
        // full hop count, sent back to peer A, the router that relayed it.
        let mut expected = vec![0x01, 0xA0, 0x0F, 0xA0, 0x01, 0x50, 0xFF, 0x03];
        expected.extend_from_slice(&refusal.reject);
        let (answer, to) = first_answer(refusal.busy, &(refusal.frame)(Some((ORIGIN, 1)))).await;
        assert_eq!(answer, expected, "{}", refusal.what);
        assert_eq!(to[..], [0x0A], "{}", refusal.what);
    }
}

#[tokio::test]
async fn router_rejects_a_local_npdu_with_a_local_unicast() {
    for refusal in refusals() {
        let mut expected = vec![0x01, 0x80, 0x03];
        expected.extend_from_slice(&refusal.reject);
        let (answer, to) = first_answer(refusal.busy, &(refusal.frame)(None)).await;
        assert_eq!(answer, expected, "{}", refusal.what);
        assert_eq!(to[..], [0x0A], "{}", refusal.what);
    }
}

#[tokio::test]
async fn router_rejects_an_npdu_from_its_own_snet_with_a_local_unicast() {
    // SNET 1000 is the network peer A's port is on (#1174), so the
    // originator, SADR 50, is on that link: no DNET, the same local unicast
    // an NPDU without SNET/SADR draws, but sent to SADR 50, not to peer A.
    for refusal in refusals() {
        let mut expected = vec![0x01, 0x80, 0x03];
        expected.extend_from_slice(&refusal.reject);
        let (answer, to) = first_answer(refusal.busy, &(refusal.frame)(Some((1000, 1)))).await;
        assert_eq!(answer, expected, "{}", refusal.what);
        assert_eq!(to[..], [0x50], "{}", refusal.what);
    }
}

/// `refusal`'s frame from SNET 1000 with the one-octet SADR `sadr`.
fn from_sadr_on_1000(refusal: &Refusal, sadr: u8) -> Vec<u8> {
    let mut frame = (refusal.frame)(Some((1000, 1)));
    // The SADR follows the version, the control octet, any DNET/DLEN/DADR,
    // and the SNET/SLEN.
    let dnet_fields = if frame[1] & 0x20 != 0 {
        3 + usize::from(frame[4])
    } else {
        0
    };
    frame[2 + dnet_fields + 3] = sadr;
    frame
}

#[tokio::test]
async fn router_rejects_a_node_with_its_other_ports_mac_at_that_mac() {
    // 02 is the router's MAC on 2000, but on 1000, peer A's network, it is
    // some other node (#1219): the reject is a local unicast to 02 on port A,
    // not to peer A, and the router does not take it for itself.
    for refusal in refusals() {
        let mut expected = vec![0x01, 0x80, 0x03];
        expected.extend_from_slice(&refusal.reject);
        let (answer, to) = first_answer(refusal.busy, &from_sadr_on_1000(&refusal, 0x02)).await;
        assert_eq!(answer, expected, "{}", refusal.what);
        assert_eq!(to[..], [0x02], "{}", refusal.what);
    }
}

/// A local APDU from peer A for network 6000, and the reason 1 reject it
/// draws, unlike any reject above. Sent after a frame that should draw
/// nothing on port A, its reject is the next frame there.
const FENCE: [u8; 8] = [0x01, 0x20, 0x17, 0x70, 0x00, 0xFF, 0x10, 0x08];
const FENCE_REJECT: [u8; 6] = [0x01, 0x80, 0x03, 0x01, 0x17, 0x70];

/// The next unicast the router sends to a peer, with the MAC it went to,
/// skipping any broadcast Who-Is-Router-To-Network solicitation for an
/// unknown DNET.
async fn next_unicast(rx: &mut FromRouter) -> (ReceivedNpdu, MacAddr) {
    loop {
        let (frame, to) = next_received_from_router(rx).await;
        if let Some(to) = to {
            return (frame, to);
        }
        assert_eq!(
            frame.npdu[..],
            [0x01, 0x80, WHO_IS, 0x13, 0x88],
            "a solicitation"
        );
    }
}

#[tokio::test]
async fn router_rejects_an_npdu_from_its_other_snet_out_that_port() {
    // SNET 2000 is the network on peer B's port, so this NPDU from peer A
    // looped round to the router (#1219). The reject is a local unicast to
    // SADR 50 out port B, with no DNET, and port A gets nothing before the
    // fence's reject.
    for refusal in refusals() {
        let mut expected = vec![0x01, 0x80, 0x03];
        expected.extend_from_slice(&refusal.reject);
        let mut fixture = fixture(refusal.busy).await;
        fixture.send_from_a(&(refusal.frame)(Some((2000, 1)))).await;
        let (answer, to) = next_unicast(&mut fixture.from_router_b).await;
        assert_eq!(answer.npdu[..], expected, "{}", refusal.what);
        assert_eq!(to[..], [0x50], "{}", refusal.what);
        fixture.send_from_a(&FENCE).await;
        let (fenced, _) = next_frame_from_router(&mut fixture.from_router_a).await;
        assert_eq!(fenced[..], FENCE_REJECT, "{}", refusal.what);
        fixture.stop().await;
    }
}

/// The data attributes peer A sends a refused frame with: one with no data
/// that must be understood, and one with data that need not be.
fn ingress_attributes() -> Vec<DataAttribute> {
    vec![
        DataAttribute {
            option_type: 1,
            must_understand: true,
            data: Vec::new(),
        },
        DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34, 0x56],
        },
    ]
}

#[tokio::test]
async fn router_rejects_with_the_ingress_data_attributes() {
    // Whichever way the reject goes, it carries the attributes the refused
    // frame came in with: back to peer A for a local sender or a relayed
    // originator, or out port B to a node on 2000.
    let attributes = ingress_attributes();
    for refusal in refusals() {
        for (source, out_port_b) in [
            (None, false),
            (Some((ORIGIN, 1)), false),
            (Some((2000, 1)), true),
        ] {
            let what = format!("{}, SNET/SADR {source:?}", refusal.what);
            let mut fixture = fixture(refusal.busy).await;
            fixture
                .send_from_a_with(&(refusal.frame)(source), &attributes)
                .await;
            let from_router = if out_port_b {
                &mut fixture.from_router_b
            } else {
                &mut fixture.from_router_a
            };
            let (reject, _) = next_unicast(from_router).await;
            let npdu = decode_npdu(reject.npdu).unwrap();
            assert_eq!(
                npdu.message_type,
                Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
                "{what}"
            );
            assert_eq!(npdu.payload[..], refusal.reject, "{what}");
            assert_eq!(reject.data_attributes, attributes, "{what}");
            fixture.stop().await;
        }
    }
}

#[tokio::test]
async fn router_sends_no_reject_when_the_snet_sadr_is_its_own_port() {
    // SNET 1000, SADR 01 is the router itself on peer A's network: the NPDU
    // came back to the router that sent it, so nothing answers it (#1219),
    // and the fence's reject is the next frame on port A.
    for refusal in refusals() {
        let mut fixture = fixture(refusal.busy).await;
        fixture
            .send_from_a(&from_sadr_on_1000(&refusal, 0x01))
            .await;
        fixture.send_from_a(&FENCE).await;
        let (answer, _) = next_frame_from_router(&mut fixture.from_router_a).await;
        assert_eq!(answer[..], FENCE_REJECT, "{}", refusal.what);
        fixture.stop().await;
    }
}

#[tokio::test]
async fn router_falls_back_to_a_local_unicast_when_the_sadr_is_too_long() {
    // The over-long SADR names no node, so the link sender gets the reason 6
    // reject, still naming the refused DNET.
    for frame in [
        wire(Some((REMOTE, 1)), Some((ORIGIN, TOO_LONG)), None),
        wire(Some((REMOTE, TOO_LONG)), Some((ORIGIN, TOO_LONG)), None),
    ] {
        let (answer, to) = first_answer(false, &frame).await;
        assert_eq!(answer, [0x01, 0x80, 0x03, 0x06, 0x0B, 0xB8], "{frame:02X?}");
        assert_eq!(to[..], [0x0A], "{frame:02X?}");
    }
}

#[tokio::test]
async fn router_relays_a_received_reject_to_the_node_its_dnet_names() {
    let mut fixture = RouterFixture::start().await;
    // A reject with no DNET is addressed to this router and goes no further,
    // even with SNET/SADR on it.
    fixture
        .send_from_b(&[0x01, 0x88, 0x03, 0xE8, 0x01, 0x0C, 0x03, 0x01, 0x13, 0x88])
        .await;
    // DNET 1000, DADR 0C: directly connected on port A.
    fixture
        .send_from_b(&[
            0x01, 0xA0, 0x03, 0xE8, 0x01, 0x0C, 0xFF, 0x03, 0x01, 0x13, 0x88,
        ])
        .await;
    let (relayed, to) = next_frame_from_router(&mut fixture.from_router_a).await;
    assert_eq!(to.as_deref(), Some(&[0x0C][..]), "straight to the DADR");
    // DNET/DADR come off; SNET 2000 / SADR 0B (peer B) go on.
    assert_eq!(
        relayed[..],
        [0x01, 0x88, 0x07, 0xD0, 0x01, 0x0B, 0x03, 0x01, 0x13, 0x88]
    );
    fixture.stop().await;
}

#[tokio::test]
async fn router_hands_a_reject_addressed_to_itself_to_its_control_receiver() {
    // The router is 01 on 1000 and 02 on 2000. Peer B sends each reject,
    // reason 2 for 3000, the network that lies behind peer B.
    let cases: [(&str, &[u8]); 3] = [
        ("no DNET", &[0x01, 0x80, 0x03, 0x02, 0x0B, 0xB8]),
        (
            "DNET 2000, DADR 02: the arrival port",
            &[
                0x01, 0xA0, 0x07, 0xD0, 0x01, 0x02, 0xFF, 0x03, 0x02, 0x0B, 0xB8,
            ],
        ),
        (
            "DNET 1000, DADR 01: the other port",
            &[
                0x01, 0xA0, 0x03, 0xE8, 0x01, 0x01, 0xFF, 0x03, 0x02, 0x0B, 0xB8,
            ],
        ),
    ];
    for (case, reject) in cases {
        let (mut fixture, mut controls) = RouterFixture::start_with_network_control().await;
        fixture.send_from_b(reject).await;

        let control = recv(&mut controls).await;
        let sent = decode_npdu(Bytes::copy_from_slice(reject)).unwrap();
        assert_eq!(control.npdu, sent, "{case}");
        assert_eq!(control.source_mac.as_slice(), [0x0B], "{case}");
        assert_eq!(control.ingress_sequence, 1, "{case}");
        assert_eq!(fixture.router.network_control_ingress_sequence(), 1);
        let now = fixture.router.now();
        assert_eq!(
            fixture.router.table().effective_reachability(REMOTE, now),
            Some(Reachability::Busy),
            "{case}: the table still learns from it"
        );
        // A reject for device 0C on 1000 is the next frame on port A, so
        // nothing was relayed there before it.
        fixture
            .send_from_b(&[
                0x01, 0xA0, 0x03, 0xE8, 0x01, 0x0C, 0xFF, 0x03, 0x01, 0x13, 0x88,
            ])
            .await;
        let (relayed, _) = next_frame_from_router(&mut fixture.from_router_a).await;
        assert_eq!(
            relayed[..],
            [0x01, 0x88, 0x07, 0xD0, 0x01, 0x0B, 0x03, 0x01, 0x13, 0x88],
            "{case}"
        );
        assert!(controls.try_recv().is_err(), "{case}");
        fixture.stop().await;
    }
}

#[tokio::test]
async fn chained_routers_carry_a_reject_back_to_the_originating_device() {
    // device 0A --[1000]-- 01 R1 02 --[2000]-- 03 R2 04 --[3000]-- 0C
    let (mut r1_a, mut device) = LoopbackTransport::pair(vec![0x01], vec![0x0A]);
    let (r1_b, r2_a) = LoopbackTransport::pair(vec![0x02], vec![0x03]);
    let (r2_b, mut far) = LoopbackTransport::pair(vec![0x04], vec![0x0C]);
    let mut to_device = FromRouter::start(&mut r1_a, &mut device).await;
    let _to_far = far.start().await.unwrap();
    let port = |transport, network_number| RouterPort {
        transport,
        network_number,
    };
    let StartedRouter {
        router: mut r1,
        apdus: _r1_local,
        ..
    } = BACnetRouter::start(
        vec![port(r1_a, 1000), port(r1_b, 2000)],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    let StartedRouter {
        router: mut r2,
        apdus: _r2_local,
        ..
    } = BACnetRouter::start(
        vec![port(r2_a, 2000), port(r2_b, 3000)],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    // R1 sends traffic for 5000 to R2, which has no route there.
    let now = r1.now();
    r1.table()
        .add_learned(UNKNOWN, PortId(1), Mac::from_array([0x03]), now);

    device
        .send_unicast(&wire(Some((UNKNOWN, 0)), None, None), &[0x01])
        .await
        .unwrap();

    // R2 rejects toward 1000/0A, the SNET/SADR R1 put on the NPDU, through
    // R1; R1 relays it by that DNET to the device, from 2000/03.
    let (reject, to) = next_frame_from_router(&mut to_device).await;
    assert_eq!(to.as_deref(), Some(&[0x0A][..]));
    assert_eq!(
        reject[..],
        [0x01, 0x88, 0x07, 0xD0, 0x01, 0x03, 0x03, 0x01, 0x13, 0x88]
    );

    r1.stop().await;
    r2.stop().await;
    device.stop().await.unwrap();
    far.stop().await.unwrap();
}
