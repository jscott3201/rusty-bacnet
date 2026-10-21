//! A broadcast-addressed NPDU, global (DNET 0xFFFF) or remote (a DNET with
//! DLEN 0), that carries anything but an Unconfirmed-Request, on the wire
//! (#1491).
//!
//! A broadcast network address carries only an Unconfirmed-Request (Clause
//! 6.3); any other PDU belongs to one peer's transaction, and every device
//! reached would get a request or an answer that names no one. Hand-built
//! NPDUs go in through loopback peers with a Confirmed-Request, a ComplexACK,
//! every other PDU type and an empty APDU. The router forwards, delivers and
//! answers none of them, the non-router hands none to its application, and
//! each counts every one in `broadcast_pdu_type_drops`. An Unconfirmed-Request
//! sent right after still goes everywhere it went and is the first to come
//! out, which shows the refused ones went nowhere. Network messages, and an
//! APDU sent with no DNET, are not affected.

use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;

use crate::layer::NetworkLayer;
use crate::loopback_fixture::{
    next_from_router, recv, reject_of, wire, RouterFixture, APDU, REMOTE,
};
use bacnet_npdu::RejectMessageToNetwork;

/// A ReadProperty Confirmed-Request for Device 1's Object_Name.
const CONFIRMED_REQUEST: &[u8] = &[
    0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// Its ComplexACK, with the name "A".
const COMPLEX_ACK: &[u8] = &[
    0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x75, 0x02, 0x00, 0x41, 0x3F,
];

/// Every APDU a broadcast may not carry: each PDU type but
/// Unconfirmed-Request (SimpleACK, SegmentACK, Error, Reject and Abort beside
/// the two above), and an empty APDU, which names no type.
const REFUSED: [&[u8]; 8] = [
    CONFIRMED_REQUEST,
    COMPLEX_ACK,
    &[0x20, 0x01, 0x0F],
    &[0x40, 0x01, 0x00, 0x01],
    &[0x50, 0x01, 0x0C, 0x91, 0x01, 0x91, 0x1F],
    &[0x60, 0x01, 0x04],
    &[0x70, 0x01, 0x04],
    &[],
];

/// An NPDU for network `dnet` with DLEN 0, a broadcast there (0xFFFF for
/// every network), carrying `apdu`.
fn broadcast(dnet: u16, apdu: &[u8]) -> Vec<u8> {
    let mut bytes = wire(Some((dnet, 0)), None, None);
    bytes.truncate(bytes.len() - APDU.len());
    bytes.extend_from_slice(apdu);
    bytes
}

#[tokio::test]
async fn router_neither_forwards_nor_delivers_a_broadcast_that_isnt_an_unconfirmed_request() {
    let mut fixture = RouterFixture::start().await;
    // The global broadcast; a remote broadcast for network 3000 behind peer
    // B, for network 2000 on port B, and for network 5000, which the router
    // can't reach: forwarded or delivered, or rejected, if taken.
    let dnets = [0xFFFF, REMOTE, 2000, 5000];
    for dnet in dnets {
        for apdu in REFUSED {
            fixture.send_from_a(&broadcast(dnet, apdu)).await;
        }
    }

    // An Unconfirmed-Request still goes everywhere it went.
    for dnet in dnets {
        fixture.send_from_a(&broadcast(dnet, &APDU)).await;
    }
    // Port B: the global broadcast, the unicast to peer B for network 3000,
    // the local broadcast on network 2000, each with the Unconfirmed-Request.
    let (global, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to, None, "a data-link broadcast");
    assert_eq!(global.payload, APDU[..]);
    assert_eq!(global.destination.map(|to| to.network), Some(0xFFFF));
    let (remote, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to.as_deref(), Some(&[0x0B][..]));
    assert_eq!(remote.payload, APDU[..]);
    assert_eq!(remote.destination.map(|to| to.network), Some(REMOTE));
    let (local, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to, None, "a data-link broadcast");
    assert_eq!(local.payload, APDU[..]);
    assert_eq!(local.destination, None, "network 2000 is port B's own");
    // Port A: the reject for network 5000 is the first frame back, so no
    // refused NPDU drew one.
    let (reject, _) = next_from_router(&mut fixture.from_router_a).await;
    assert_eq!(
        reject_of(&reject),
        RejectMessageToNetwork {
            reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            network: 5000,
        }
    );
    // Locally: the global Unconfirmed-Request is the first delivery.
    let delivered = recv(&mut fixture.local).await;
    assert!(delivered.global_broadcast);
    assert_eq!(delivered.apdu, APDU[..]);

    assert_eq!(
        fixture.router.broadcast_pdu_type_drops(),
        (dnets.len() * REFUSED.len()) as u64
    );
    assert_eq!(fixture.router.global_broadcast_dadr_drops(), 0);
    fixture.stop().await;
}

/// A ComplexACK the router relays to one device, by DNET and DADR, is no
/// broadcast and still goes through.
#[tokio::test]
async fn router_still_relays_an_answer_to_one_device() {
    let mut fixture = RouterFixture::start().await;
    let mut bytes = wire(Some((REMOTE, 1)), None, None);
    bytes.truncate(bytes.len() - APDU.len());
    bytes.extend_from_slice(COMPLEX_ACK);
    fixture.send_from_a(&bytes).await;
    let (relayed, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to.as_deref(), Some(&[0x0B][..]));
    assert_eq!(relayed.payload, COMPLEX_ACK);
    assert_eq!(fixture.router.broadcast_pdu_type_drops(), 0);
    fixture.stop().await;
}

#[tokio::test]
async fn non_router_delivers_no_broadcast_that_isnt_an_unconfirmed_request() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut controls = network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start().await.unwrap();

    for apdu in REFUSED {
        peer.send_broadcast(&broadcast(0xFFFF, apdu)).await.unwrap();
        peer.send_broadcast(&broadcast(7, apdu)).await.unwrap();
    }
    // With no DNET, a Confirmed-Request is the application's to judge, even
    // by link broadcast; a global broadcast network message is a control.
    let mut local = wire(None, None, None);
    local.truncate(local.len() - APDU.len());
    local.extend_from_slice(CONFIRMED_REQUEST);
    peer.send_broadcast(&local).await.unwrap();
    let i_am_router = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);
    peer.send_broadcast(&wire(Some((0xFFFF, 0)), None, i_am_router))
        .await
        .unwrap();
    peer.send_broadcast(&broadcast(0xFFFF, &APDU))
        .await
        .unwrap();

    let first = recv(&mut apdus).await;
    assert_eq!(first.apdu, CONFIRMED_REQUEST, "no refused APDU came first");
    assert!(!first.global_broadcast);
    let global = recv(&mut apdus).await;
    assert_eq!(global.apdu, APDU[..]);
    assert!(global.global_broadcast && global.is_group);
    let control = recv(&mut controls).await;
    assert_eq!(control.npdu.message_type, i_am_router);

    assert_eq!(network.broadcast_pdu_type_drops(), 2 * REFUSED.len() as u64);
    assert!(decode_npdu(broadcast(0xFFFF, CONFIRMED_REQUEST).into()).is_ok());
    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}
