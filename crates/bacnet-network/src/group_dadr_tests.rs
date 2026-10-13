//! A routed NPDU whose DADR is a group destination on the port the router
//! delivers it on (#1504), on the wire.
//!
//! Delivered as one unicast, such an NPDU reaches every node in the group
//! without the broadcast network addresses #1491 filters, so the router
//! delivers only an Unconfirmed-Request there, and as that port's local
//! broadcast (#1677). The fixture's port B counts MAC 0xFF as its
//! broadcast. Hand-built NPDUs go in at peer A; an Unconfirmed-Request sent
//! right after the refused ones is the first frame out of port B, which
//! shows the refused ones went nowhere.

use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress, RejectMessageToNetwork};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};

use crate::loopback_fixture::{next_from_router, recv, reject_of, RouterFixture, APDU, REMOTE};

/// A group destination on port B (network 2000).
const GROUP: u8 = 0xFF;
/// A ReadProperty Confirmed-Request for Device 1's Object_Name.
const CONFIRMED_REQUEST: &[u8] = &[
    0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// Its ComplexACK, with the name "A".
const COMPLEX_ACK: &[u8] = &[
    0x30, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x75, 0x02, 0x00, 0x41, 0x3F,
];

/// An NPDU for `dadr` on network `dnet`, carrying `apdu`.
fn routed(dnet: u16, dadr: u8, apdu: &[u8]) -> Bytes {
    let npdu = Npdu {
        destination: Some(NpduAddress {
            network: dnet,
            mac_address: MacAddr::from_slice(&[dadr]),
        }),
        hop_count: 255,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &npdu).unwrap();
    bytes.freeze()
}

#[tokio::test]
async fn router_delivers_only_an_unconfirmed_request_to_a_group_dadr() {
    let mut fixture = RouterFixture::start_with_group_on_b(GROUP).await;
    // Refused: a request and an answer for the group on network 2000.
    fixture
        .send_from_a(&routed(2000, GROUP, CONFIRMED_REQUEST))
        .await;
    fixture.send_from_a(&routed(2000, GROUP, COMPLEX_ACK)).await;
    // Passed on: the same MAC on network 3000 behind peer B, which is the
    // next router's to judge; an Unconfirmed-Request for the group; and a
    // request for one device on network 2000.
    fixture
        .send_from_a(&routed(REMOTE, GROUP, CONFIRMED_REQUEST))
        .await;
    fixture.send_from_a(&routed(2000, GROUP, &APDU)).await;
    fixture
        .send_from_a(&routed(2000, 0x0B, CONFIRMED_REQUEST))
        .await;
    // No route: a reject comes back to A.
    fixture.send_from_a(&routed(5000, 0x0B, &APDU)).await;

    let (remote, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to.as_deref(), Some(&[0x0B][..]), "the next hop");
    assert_eq!(remote.payload, CONFIRMED_REQUEST);
    let dadr = remote.destination.map(|to| (to.network, to.mac_address));
    assert_eq!(dadr, Some((REMOTE, MacAddr::from_slice(&[GROUP]))));
    let (group, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to, None, "a local broadcast on port B");
    assert_eq!(group.payload, APDU[..]);
    assert_eq!(group.destination, None, "network 2000 is port B's own");
    let source = group.source.map(|from| (from.network, from.mac_address));
    assert_eq!(source, Some((1000, MacAddr::from_slice(&[0x0A]))));
    let (one, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to.as_deref(), Some(&[0x0B][..]));
    assert_eq!(one.payload, CONFIRMED_REQUEST);
    // The refused ones drew no reject: the one for network 5000 is first.
    let (reject, _) = next_from_router(&mut fixture.from_router_a).await;
    assert_eq!(
        reject_of(&reject),
        RejectMessageToNetwork {
            reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            dnet: 5000,
        }
    );

    assert_eq!(fixture.router.group_dadr_drops(), 2);
    assert_eq!(fixture.router.broadcast_pdu_type_drops(), 0);
    fixture.stop().await;
}

/// The same holds for delivery back out the port the NPDU arrived on: a
/// node on network 2000 addressing the group there through the router. The
/// broadcast reaches the router's own application too, as a DLEN 0
/// broadcast to network 2000 from there would.
#[tokio::test]
async fn router_delivers_back_out_the_arrival_port_only_an_unconfirmed_request() {
    let mut fixture = RouterFixture::start_with_group_on_b(GROUP).await;
    fixture
        .send_from_b(&routed(2000, GROUP, CONFIRMED_REQUEST))
        .await;
    fixture.send_from_b(&routed(2000, GROUP, &APDU)).await;

    let (back, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to, None, "a local broadcast on port B");
    assert_eq!(back.payload, APDU[..], "the refused request went nowhere");
    let local = recv(&mut fixture.local).await;
    assert_eq!(local.apdu.as_ref(), APDU);
    assert!(local.is_group, "a broadcast on network 2000");
    assert_eq!(local.ingress_network, Some(2000));
    assert!(fixture.local.try_recv().is_err(), "delivered once");
    assert_eq!(fixture.router.group_dadr_drops(), 1);
    fixture.stop().await;
}

/// A DLEN 0 broadcast for network 2000 from network 1000 isn't delivered to
/// the router's application, so neither is a group DADR's broadcast: only
/// the one that arrives on network 2000 itself is.
#[tokio::test]
async fn a_group_broadcast_for_another_port_stays_off_the_local_application() {
    let mut fixture = RouterFixture::start_with_group_on_b(GROUP).await;
    fixture.send_from_a(&routed(2000, GROUP, &APDU)).await;
    let (out, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!((to, out.payload.as_ref()), (None, &APDU[..]));
    fixture.send_from_b(&routed(2000, 0x02, &APDU)).await;
    let local = recv(&mut fixture.local).await;
    assert!(
        !local.is_group,
        "the marker, addressed to the router itself"
    );
    assert!(fixture.local.try_recv().is_err());
    fixture.stop().await;
}

/// A network message routed to a group DADR is dropped and counted, not
/// sent on as a broadcast to every router there (L23 on #1677).
#[tokio::test]
async fn a_network_message_for_a_group_dadr_is_dropped_and_counted() {
    let mut fixture = RouterFixture::start_with_group_on_b(GROUP).await;
    let to_group = Some(NpduAddress {
        network: 2000,
        mac_address: MacAddr::from_slice(&[GROUP]),
    });
    // Initialize-Routing-Table, a query with no ports; and a proprietary
    // message whose payload reads like a Who-Is, which a network message
    // never is.
    let controls = [
        Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
            destination: to_group.clone(),
            hop_count: 255,
            payload: Bytes::from_static(&[0x00]),
            ..Npdu::default()
        },
        Npdu {
            is_network_message: true,
            message_type: Some(0x80),
            vendor_id: Some(999),
            destination: to_group,
            hop_count: 255,
            payload: Bytes::from_static(&APDU),
            ..Npdu::default()
        },
    ];
    for control in &controls {
        let mut bytes = BytesMut::new();
        encode_npdu(&mut bytes, control).unwrap();
        fixture.send_from_a(&bytes).await;
    }
    fixture.send_from_a(&routed(2000, 0x0B, &APDU)).await;

    let (first, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(
        to.as_deref(),
        Some(&[0x0B][..]),
        "the controls went nowhere"
    );
    assert!(!first.is_network_message);
    assert_eq!(first.payload, APDU[..]);
    assert_eq!(fixture.router.group_dadr_drops(), 2);
    fixture.stop().await;
}

/// A Reject-Message-To-Network the router relays toward the node it names
/// isn't relayed to a group DADR either: it is dropped and counted.
#[tokio::test]
async fn a_relayed_reject_for_a_group_dadr_is_dropped_and_counted() {
    let mut fixture = RouterFixture::start_with_group_on_b(GROUP).await;
    // A reject about network 5000, which this router doesn't know, so the
    // routing table is left as it is.
    let reject = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&[GROUP]),
        }),
        hop_count: 255,
        payload: Bytes::from_static(&[0x01, 0x13, 0x88]),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &reject).unwrap();
    fixture.send_from_a(&bytes).await;
    fixture.send_from_a(&routed(2000, 0x0B, &APDU)).await;

    let (first, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to.as_deref(), Some(&[0x0B][..]), "the reject went nowhere");
    assert!(!first.is_network_message);
    assert_eq!(fixture.router.group_dadr_drops(), 1);
    fixture.stop().await;
}
