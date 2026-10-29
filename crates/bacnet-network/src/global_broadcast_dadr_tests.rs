//! An inbound NPDU whose DNET is 0xFFFF and that also carries a DADR
//! (#1379), on the wire.
//!
//! DNET 0xFFFF already names every device on every network (Clauses 6.2.2
//! and 6.3.2), so a DADR beside it contradicts it. Hand-built NPDU bytes go
//! in through loopback peers, as an APDU and as a network message, with DADRs
//! of 1, 6 and 18 octets. The non-router [`NetworkLayer`] hands neither to
//! its receivers, and the router forwards, delivers, answers and rejects
//! nothing. Each counts every such NPDU in `global_broadcast_dadr_drops`.
//! Frames sent right after the refused ones are the first to come out, which
//! shows the refused ones went nowhere, and a global broadcast with DLEN 0
//! still goes everywhere it should.

use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;

use crate::layer::NetworkLayer;
use crate::loopback_fixture::{APDU, RouterFixture, next_from_router, pump, recv, reject_of, wire};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::RejectMessageToNetwork;

/// DADR lengths beside DNET 0xFFFF: an MS/TP station, a B/IP or BACnet/SC
/// address, and the longest an NPDU address may be.
const DADR_LENGTHS: [u8; 3] = [1, 6, bacnet_npdu::MAX_MAC_LEN as u8];

/// A global broadcast of the fixture APDU, or of a `control` network message
/// carrying network 3000, with a `dlen`-octet DADR.
fn global(dlen: u8, control: Option<NetworkMessageType>) -> Vec<u8> {
    wire(Some((0xFFFF, dlen)), None, control)
}

#[tokio::test]
async fn non_router_drops_and_counts_a_global_broadcast_that_names_a_device() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut controls = network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start().await.unwrap();
    let control = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);

    for dlen in DADR_LENGTHS {
        peer.send_unicast(&global(dlen, None), &[0x01])
            .await
            .unwrap();
        peer.send_broadcast(&global(dlen, control)).await.unwrap();
    }
    // A local APDU, then a global broadcast with DLEN 0, each as an APDU and
    // as a network message.
    for bytes in [wire(None, None, None), global(0, None)] {
        peer.send_unicast(&bytes, &[0x01]).await.unwrap();
    }
    for bytes in [wire(None, None, control), global(0, control)] {
        peer.send_unicast(&bytes, &[0x01]).await.unwrap();
    }

    let local = recv(&mut apdus).await;
    assert_eq!(local.apdu, APDU[..]);
    assert!(!local.global_broadcast, "the local APDU comes out first");
    let delivered = recv(&mut apdus).await;
    assert!(delivered.global_broadcast && delivered.is_group);
    // The controls behind the last APDU reach their receiver once the
    // APDU receiver is polled again (#1713).
    pump(&mut apdus);

    let local = recv(&mut controls).await;
    assert_eq!(
        local.npdu.destination, None,
        "the local control comes first"
    );
    let delivered = recv(&mut controls).await;
    let destination = delivered.npdu.destination.expect("a global broadcast");
    assert_eq!(
        (destination.network, destination.mac_address.len()),
        (0xFFFF, 0)
    );

    assert_eq!(
        network.global_broadcast_dadr_drops(),
        2 * DADR_LENGTHS.len() as u64
    );
    assert_eq!(network.address_length_drops(), 0, "the lengths were fine");

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn router_neither_forwards_nor_delivers_nor_answers_a_global_broadcast_that_names_a_device() {
    let mut fixture = RouterFixture::start().await;
    // Network 3000 lies behind port B. Taken as plain global broadcasts, the
    // route query for 3000 would be answered on port A, and the APDU would go
    // out on port B and to the local queue.
    let who_is_router = Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK);
    for dlen in DADR_LENGTHS {
        fixture.send_from_a(&global(dlen, None)).await;
        fixture
            .peer_a
            .send_broadcast(&global(dlen, who_is_router))
            .await
            .unwrap();
    }

    // For an unknown DNET the router rejects with reason 1 on port A and
    // asks for a route on port B. Each is the first frame on its port, so no
    // refused NPDU was answered, rejected or forwarded.
    fixture
        .send_from_a(&wire(Some((5000, 0)), None, None))
        .await;
    let (frame, to) = fixture.from_router_a.recv().await;
    assert_eq!(to.as_deref(), Some(&[0x0A][..]));
    assert_eq!(
        reject_of(&decode_npdu(frame.npdu).unwrap()),
        RejectMessageToNetwork {
            reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            network: 5000,
        }
    );
    let (solicit, _) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(solicit.message_type, who_is_router);
    assert_eq!(solicit.payload[..], 5000u16.to_be_bytes());

    // Nor was any delivered locally: a local APDU is the first to arrive.
    fixture.send_from_a(&wire(None, None, None)).await;
    let local = recv(&mut fixture.local).await;
    assert!(!local.global_broadcast, "the local APDU comes out first");

    // A global broadcast with DLEN 0 still goes out on port B with DLEN 0,
    // and to the local queue.
    fixture.send_from_a(&global(0, None)).await;
    let (forwarded, to) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(to, None, "a data-link broadcast");
    assert_eq!(forwarded.payload, APDU[..]);
    let destination = forwarded.destination.expect("still a global broadcast");
    assert_eq!(
        (destination.network, destination.mac_address.len()),
        (0xFFFF, 0)
    );
    assert_eq!(forwarded.source.expect("SNET added").network, 1000);
    assert!(recv(&mut fixture.local).await.global_broadcast);

    assert_eq!(
        fixture.router.global_broadcast_dadr_drops(),
        2 * DADR_LENGTHS.len() as u64
    );
    assert_eq!(
        fixture.router.address_length_drops(),
        0,
        "the lengths were fine"
    );
    fixture.stop().await;
}
