//! #1678 case 1, hosted: when `BACnetRouter` relays a Forwarded-NPDU its
//! B/IP port received, the SADR it adds names the forward's origin, not
//! the BBMD that sent it (Annex J.2.5), and the network priority survives.
//!
//! Ported, for the hosted router, from ACE IoT's bacnet-router-adapters
//! (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
//! `forwarded_origin_and_priority_survive_bip_to_mstp_composition`;
//! `bacnet-router-core`'s `tests/glue_bip_mstp.rs` runs it over the
//! portable B/IP and MS/TP glue. The egress here is an in-process port with
//! one-octet MACs standing in for MS/TP: what the router hands a port
//! doesn't depend on its kind.

use std::net::Ipv4Addr;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

/// The B/IP network, and the one behind the router.
const BIP_NET: u16 = 1;
const OTHER_NET: u16 = 2;

/// The forward's origin: 198.51.100.10, UDP port 0xBAC1.
const ORIGIN: [u8; 6] = [198, 51, 100, 10, 0xBA, 0xC1];

/// A router whose B/IP port is on loopback, its broadcasts to its own
/// address, at a UDP port picked here so the test can address it; its
/// other port's far end, the station at MAC 0x05; where the router's
/// unicasts there went; and the B/IP port.
///
/// The transport binds its own socket, so the pick has to let go of the
/// port before the router takes it, and another process can take it in
/// between: a start that fails is tried again on a fresh port.
async fn start() -> (
    BACnetRouter,
    LoopbackTransport,
    mpsc::UnboundedReceiver<MacAddr>,
    u16,
) {
    for _ in 0..8 {
        let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
        let (mut router_end, station) = LoopbackTransport::pair(vec![0x01], vec![0x05]);
        let unicast_to = router_end.record_unicast_destinations();
        let ports = vec![
            RouterPort {
                transport: AnyTransport::Bip(Box::new(bip)),
                network_number: BIP_NET,
            },
            RouterPort {
                transport: AnyTransport::Loopback(router_end),
                network_number: OTHER_NET,
            },
        ];
        if let Ok(StartedRouter { router, .. }) =
            BACnetRouter::start(ports, RouterOptions::new()).await
        {
            return (router, station, unicast_to, port);
        }
    }
    panic!("no loopback UDP port the router could bind");
}

#[tokio::test]
async fn forwarded_origin_and_priority_survive_routing() {
    let (mut router, mut station, mut unicast_to, port) = start().await;
    let mut from_router = station.start().await.unwrap();

    // A BBMD forwards a broadcast from the origin: an NPDU for station 5
    // on network 2, at life-safety priority, carrying a Who-Is.
    let npdu = [0x01, 0x23, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x08];
    let mut forwarded = vec![0x81, 0x04, 0x00, (4 + ORIGIN.len() + npdu.len()) as u8];
    forwarded.extend_from_slice(&ORIGIN);
    forwarded.extend_from_slice(&npdu);
    let bbmd = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    bbmd.send_to(&forwarded, (Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();

    // The relay: SNET 1 and the origin's six octets as SADR, priority 3,
    // the APDU unchanged, unicast to station 5.
    let relayed = loop {
        let received = timeout(Duration::from_secs(5), from_router.recv())
            .await
            .expect("the router relays it")
            .unwrap();
        // Skip the router's own network messages (its I-Am-Router).
        if received.npdu[1] & 0x80 == 0 {
            break received;
        }
    };
    let mut expected = vec![0x01, 0x0B, 0x00, 0x01, 0x06];
    expected.extend_from_slice(&ORIGIN);
    expected.extend_from_slice(&[0x10, 0x08]);
    assert_eq!(relayed.npdu.as_ref(), expected);
    assert_eq!(relayed.npdu[1] & 0x03, 3, "life-safety priority");
    assert!(!relayed.link_layer_group);
    let to = timeout(Duration::from_secs(5), unicast_to.recv())
        .await
        .expect("the relay's destination is recorded")
        .unwrap();
    assert_eq!(to.as_slice(), [0x05]);

    router.stop().await.unwrap();
    station.stop().await.unwrap();
}
