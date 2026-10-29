//! A routed NPDU from an MS/TP port whose six-octet DADR is one of a B/IP
//! port's group addresses: the subnet broadcast, the limited broadcast or a
//! multicast address. Anything but an Unconfirmed-Request is dropped and
//! counted before it reaches the B/IP port (#1504); an Unconfirmed-Request
//! goes out there as a local broadcast (#1677), since B/IP refuses a
//! unicast to a group address. A group at another UDP port belongs to
//! another B/IP network, so an Unconfirmed-Request for one is dropped and
//! counted too.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::NpduLimits;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::{GroupDestinations, ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use super::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

/// The MS/TP network, and the B/IP network behind the router.
const MSTP_NET: u16 = 100;
const BIP_NET: u16 = 300;

/// A send the router asked the B/IP port for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sent {
    Unicast(Vec<u8>),
    Broadcast(Vec<u8>),
}

/// The router's B/IP port. It records every send the router asks of it.
/// A unicast goes on to the real transport, which refuses a group
/// address; a broadcast stops here, so no test datagram leaves the host.
struct Recorded {
    inner: BipTransport,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl TransportPort for Recorded {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent.lock().unwrap().push(Sent::Unicast(mac.to_vec()));
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.sent
            .lock()
            .unwrap()
            .push(Sent::Broadcast(npdu.to_vec()));
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }

    fn max_npdu(&self) -> NpduLimits {
        self.inner.max_npdu()
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }

    fn group_destinations(&self) -> GroupDestinations {
        self.inner.group_destinations()
    }

    fn local_broadcast_destinations(&self) -> GroupDestinations {
        self.inner.local_broadcast_destinations()
    }
}

/// Ports may hold one transport type, so both sides of the router are
/// this.
enum Port {
    Mstp(MstpTransport<LoopbackSerial>),
    Bip(Recorded),
}

impl TransportPort for Port {
    fn prepare_for_routing(&mut self) {
        if let Self::Mstp(mstp) = self {
            mstp.prepare_for_routing();
        }
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        match self {
            Self::Mstp(mstp) => mstp.start().await,
            Self::Bip(bip) => bip.start().await,
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match self {
            Self::Mstp(mstp) => mstp.stop().await,
            Self::Bip(bip) => bip.stop().await,
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        match self {
            Self::Mstp(mstp) => mstp.send_unicast(npdu, mac).await,
            Self::Bip(bip) => bip.send_unicast(npdu, mac).await,
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        match self {
            Self::Mstp(mstp) => mstp.send_broadcast(npdu).await,
            Self::Bip(bip) => bip.send_broadcast(npdu).await,
        }
    }

    fn local_mac(&self) -> &[u8] {
        match self {
            Self::Mstp(mstp) => mstp.local_mac(),
            Self::Bip(bip) => bip.local_mac(),
        }
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        match self {
            Self::Mstp(mstp) => mstp.local_receive_apdu_capacity(),
            Self::Bip(bip) => bip.local_receive_apdu_capacity(),
        }
    }

    fn max_npdu(&self) -> NpduLimits {
        match self {
            Self::Mstp(mstp) => mstp.max_npdu(),
            Self::Bip(bip) => bip.max_npdu(),
        }
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        match self {
            Self::Mstp(mstp) => mstp.is_broadcast_mac(mac),
            Self::Bip(bip) => bip.is_broadcast_mac(mac),
        }
    }

    fn group_destinations(&self) -> GroupDestinations {
        match self {
            Self::Mstp(mstp) => mstp.group_destinations(),
            Self::Bip(bip) => bip.group_destinations(),
        }
    }

    fn local_broadcast_destinations(&self) -> GroupDestinations {
        match self {
            Self::Mstp(mstp) => mstp.local_broadcast_destinations(),
            Self::Bip(bip) => bip.local_broadcast_destinations(),
        }
    }
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
    let sent = Arc::new(Mutex::new(Vec::new()));
    // A known port, so the groups at it and at another can be named.
    let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let other = port.checked_add(1).unwrap_or(port - 1);
    let bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::new(192, 0, 2, 255));
    let ports = vec![
        RouterPort {
            transport: Port::Mstp(MstpTransport::new(router_serial, station(0))),
            network_number: MSTP_NET,
        },
        RouterPort {
            transport: Port::Bip(Recorded {
                inner: bip,
                sent: Arc::clone(&sent),
            }),
            network_number: BIP_NET,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let mut peer = MstpTransport::new(peer_serial, station(1));
    let _from_router = peer.start().await.unwrap();
    // The router's own I-Am-Router-To-Network on the B/IP port.
    timeout(Duration::from_secs(10), async {
        while sent.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the router announces itself");
    sent.lock().unwrap().clear();

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
        while sent.lock().unwrap().len() < groups.len() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the Who-Is requests go out");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let sent = sent.lock().unwrap().clone();
    assert_eq!(sent.len(), groups.len(), "{sent:?}");
    for send in sent {
        let Sent::Broadcast(npdu) = send else {
            panic!("a unicast reached the B/IP port: {send:?}");
        };
        let npdu = decode_npdu(npdu.into()).unwrap();
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
