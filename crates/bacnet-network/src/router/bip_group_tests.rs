//! A routed NPDU from an MS/TP port whose six-octet DADR is one of a B/IP
//! port's group addresses: the subnet broadcast, the limited broadcast or a
//! multicast address. Anything but an Unconfirmed-Request is dropped and
//! counted before it reaches the B/IP port (#1504); an Unconfirmed-Request
//! goes out there as a local broadcast (#1677), since B/IP refuses a
//! unicast to a group address. A group at another UDP port belongs to
//! another B/IP network, so an Unconfirmed-Request for one is dropped and
//! counted too.

use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::TransportPort;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use std::net::Ipv4Addr;
use tokio::time::{Duration, timeout};

use super::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

/// The MS/TP network, and the B/IP network behind the router.
const MSTP_NET: u16 = 100;
const BIP_NET: u16 = 300;

/// The router's B/IP port. It records every send the router asks of it.
/// A unicast goes on to the real transport, which refuses a group
/// address; a broadcast stops here, so no test datagram leaves the host.
fn bip_port(inner: BipTransport) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Record)
        .build()
}

/// The router's MS/TP port, over a transport that loops back its serial line.
fn mstp_port(serial: LoopbackSerial, config: MstpConfig) -> TestTransport {
    TestTransport::builder()
        .inner(MstpTransport::new(serial, config))
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .build()
}

fn station(station: u8) -> MstpConfig {
    MstpConfig {
        this_station: station,
        max_master: 1,
        baud_rate: 38_400,
        ..MstpConfig::default()
    }
}

/// An NPDU for DNET `BIP_NET`, DADR `dadr`, carrying `apdu`.
fn routed(dadr: [u8; 6], apdu: &[u8]) -> Vec<u8> {
    let mut npdu = vec![0x01, 0x20];
    npdu.extend_from_slice(&BIP_NET.to_be_bytes());
    npdu.push(6);
    npdu.extend_from_slice(&dadr);
    npdu.push(0xFF);
    npdu.extend_from_slice(apdu);
    npdu
}

/// Nothing but an Unconfirmed-Request for a group reaches the B/IP port,
/// and that only as a broadcast, never as a unicast to the group.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew, where it is
/// `mstp_six_byte_bip_group_destinations_never_reach_the_sink`.
#[tokio::test(start_paused = true)]
async fn mstp_six_byte_bip_group_destinations_reach_the_bip_port_only_as_broadcasts() {
    let (router_serial, peer_serial) = LoopbackSerial::pair();
    let bip = bip_port(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::new(192, 0, 2, 255),
    ));
    let sent = bip.sent();
    let ports = vec![
        RouterPort {
            transport: mstp_port(router_serial, station(0)),
            network_number: MSTP_NET,
        },
        RouterPort {
            transport: bip,
            network_number: BIP_NET,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    // The port the OS gave the B/IP port, so the groups at it and at another
    // can be named.
    let (_, port) = bacnet_transport::bvll::decode_bip_mac(router.ports()[1].local_mac()).unwrap();
    let other = port.checked_add(1).unwrap_or(port - 1);
    let mut peer = MstpTransport::new(peer_serial, station(1));
    let _from_router = peer.start().await.unwrap();
    // The router's own I-Am-Router-To-Network on the B/IP port.
    timeout(Duration::from_secs(10), async {
        while sent.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the router announces itself");
    sent.clear();

    let at = |ip: [u8; 4], port: u16| {
        let [high, low] = port.to_be_bytes();
        [ip[0], ip[1], ip[2], ip[3], high, low]
    };
    let groups = [
        at([192, 0, 2, 255], port),
        at([255, 255, 255, 255], port),
        at([239, 1, 2, 3], port),
    ];
    let elsewhere = [at([192, 0, 2, 255], other), at([255, 255, 255, 255], other)];
    // An Error PDU for each group, then a Who-Is for each, and a Who-Is for
    // groups at another port.
    let not_unconfirmed = [0x50, 0x01, 0x0C, 0x91, 0x02, 0x91, 0x1F];
    let who_is = [0x10, 0x08];
    for apdu in [&not_unconfirmed[..], &who_is[..]] {
        for dadr in groups {
            peer.send_unicast(&routed(dadr, apdu), &[0]).await.unwrap();
        }
    }
    for dadr in elsewhere {
        peer.send_unicast(&routed(dadr, &who_is), &[0])
            .await
            .unwrap();
    }
    timeout(Duration::from_secs(10), async {
        while sent.len() < groups.len() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the Who-Is requests go out");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let sent = sent.frames();
    assert_eq!(sent.len(), groups.len(), "{sent:?}");
    for send in sent {
        assert!(send.broadcast, "a unicast reached the B/IP port: {send:?}");
        let npdu = decode_npdu(send.npdu).unwrap();
        assert_eq!(npdu.destination, None, "network 300 is the port's own");
        let source = npdu.source.expect("the router names the origin");
        assert_eq!(
            (source.network, source.mac_address.as_slice()),
            (MSTP_NET, &[1][..])
        );
        assert_eq!(npdu.payload.as_ref(), who_is);
    }
    assert_eq!(
        router.group_dadr_drops(),
        (groups.len() + elsewhere.len()) as u64
    );
    router.stop().await.unwrap();
    peer.stop().await.unwrap();
}
