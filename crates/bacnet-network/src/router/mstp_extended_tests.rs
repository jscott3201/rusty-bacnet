//! A router's MS/TP ports take extended frames (#1611): Clause 9.3 requires
//! COBS-encoded frames of a routing node, so `BACnetRouter::start` turns
//! them on for every MS/TP port it routes, whatever its configuration says.
//! The router answers an NPDU too long for an MS/TP port with reject reason
//! 4, by the limit the port reports (#1665).

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use bacnet_npdu::NpduLimits;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::{GroupDestinations, ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use super::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use crate::router_table::PortId;
use bacnet_npdu::Mac;

/// A station of masters 0 and 1 at 38400 baud, without extended frames.
fn standard(station: u8) -> MstpConfig {
    MstpConfig {
        this_station: station,
        max_master: 1,
        baud_rate: 38_400,
        ..MstpConfig::default()
    }
}

/// The router's two MS/TP ports are configured without extended frames. A
/// peer on network 1 that takes them broadcasts a 1000-octet NPDU, which
/// goes out as a Type 33; the router's port takes it and delivers the APDU
/// locally. A port without extended frames would count the frame as invalid
/// and deliver nothing.
#[tokio::test(start_paused = true)]
async fn a_routers_mstp_ports_take_extended_frames() {
    let (router_serial_1, peer_serial_1) = LoopbackSerial::pair();
    let (router_serial_2, _peer_serial_2) = LoopbackSerial::pair();
    let ports = vec![
        RouterPort {
            transport: MstpTransport::new(router_serial_1, standard(0)),
            network_number: 1,
        },
        RouterPort {
            transport: MstpTransport::new(router_serial_2, standard(0)),
            network_number: 2,
        },
    ];
    let StartedRouter {
        mut router,
        mut apdus,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();

    let mut peer = MstpTransport::new(
        peer_serial_1,
        MstpConfig {
            extended_frames: true,
            ..standard(1)
        },
    );
    let _from_router = peer.start().await.unwrap();
    // A local broadcast Who-Is, padded past what a standard frame carries.
    let mut npdu = vec![0x01, 0x00, 0x10, 0x08];
    npdu.resize(1000, 0);
    peer.send_broadcast(&npdu).await.unwrap();

    let received = timeout(Duration::from_secs(10), apdus.recv())
        .await
        .expect("the router delivers the broadcast")
        .unwrap();
    assert_eq!(received.apdu.as_ref(), &npdu[2..]);
    assert_eq!(received.ingress_network, Some(1));
    assert_eq!(received.source_mac.as_slice(), &[1]);
    router.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// An MS/TP port that records the APDU limits it reports once started,
/// after the router has prepared it.
struct Watched {
    inner: MstpTransport<LoopbackSerial>,
    /// `local_receive_apdu_capacity` and `egress_apdu_limit` at start.
    limits: Arc<[AtomicU16; 2]>,
}

impl TransportPort for Watched {
    fn prepare_for_routing(&mut self) {
        self.inner.prepare_for_routing();
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let received = self.inner.start().await?;
        self.limits[0].store(self.inner.local_receive_apdu_capacity(), Ordering::SeqCst);
        self.limits[1].store(self.inner.egress_apdu_limit(), Ordering::SeqCst);
        Ok(received)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }

    fn egress_apdu_limit(&self) -> u16 {
        self.inner.egress_apdu_limit()
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
}

/// The router forwards a 1000-octet NPDU from network 2 to station 1 on
/// network 1. Its MS/TP port, configured without extended frames, sends it
/// in an extended frame, since no standard frame carries more than 501
/// octets, and both ports report the 1476-octet APDU limit once the router
/// has prepared them.
#[tokio::test(start_paused = true)]
async fn a_routers_mstp_port_sends_extended_frames() {
    let (router_serial_1, peer_serial_1) = LoopbackSerial::pair();
    let (router_serial_2, peer_serial_2) = LoopbackSerial::pair();
    let limits: Vec<Arc<[AtomicU16; 2]>> = (0..2)
        .map(|_| Arc::new([AtomicU16::new(0), AtomicU16::new(0)]))
        .collect();
    let ports = vec![
        RouterPort {
            transport: Watched {
                inner: MstpTransport::new(router_serial_1, standard(0)),
                limits: limits[0].clone(),
            },
            network_number: 1,
        },
        RouterPort {
            transport: Watched {
                inner: MstpTransport::new(router_serial_2, standard(0)),
                limits: limits[1].clone(),
            },
            network_number: 2,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    for port in &limits {
        assert_eq!(port[0].load(Ordering::SeqCst), 1476);
        assert_eq!(port[1].load(Ordering::SeqCst), 1476);
    }

    let extended = |station| MstpConfig {
        extended_frames: true,
        ..standard(station)
    };
    let mut destination = MstpTransport::new(peer_serial_1, extended(1));
    let mut from_router = destination.start().await.unwrap();
    let mut source = MstpTransport::new(peer_serial_2, extended(1));
    let _to_source = source.start().await.unwrap();

    // To DNET 1, DADR 1, hop count 255: a Who-Is padded past what a
    // standard frame carries.
    let mut apdu = vec![0x10, 0x08];
    apdu.resize(990, 0x5A);
    let mut npdu = vec![0x01, 0x20, 0x00, 0x01, 0x01, 0x01, 0xFF];
    npdu.extend_from_slice(&apdu);
    source.send_unicast(&npdu, &[0]).await.unwrap();

    // Skip the router's own network messages, such as I-Am-Router-To-Network.
    let forwarded = timeout(Duration::from_secs(10), async {
        loop {
            let received = from_router.recv().await.unwrap();
            if received
                .npdu
                .get(1)
                .is_some_and(|control| control & 0x80 == 0)
            {
                break received;
            }
        }
    })
    .await
    .expect("the router forwards the NPDU");
    assert!(forwarded.npdu.len() > 501, "{}", forwarded.npdu.len());
    assert!(forwarded.npdu.ends_with(&apdu));
    assert_eq!(forwarded.source_mac.as_slice(), &[0]);
    router.stop().await.unwrap();
    destination.stop().await.unwrap();
    source.stop().await.unwrap();
}

/// An MS/TP port that a router may prepare, or not: one it doesn't keeps
/// standard frames, so it carries NPDUs of at most 501 octets, as an MS/TP
/// port behind a wrapper that skips `prepare_for_routing` would.
struct Routed {
    inner: MstpTransport<LoopbackSerial>,
    prepare: bool,
}

impl TransportPort for Routed {
    fn prepare_for_routing(&mut self) {
        if self.prepare {
            self.inner.prepare_for_routing();
        }
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
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

    fn group_destinations(&self) -> GroupDestinations {
        self.inner.group_destinations()
    }
}

/// A started MS/TP peer and what it receives.
type Peer = (MstpTransport<LoopbackSerial>, mpsc::Receiver<ReceivedNpdu>);

/// A router on two MS/TP links, network 1 and network 2, at station 0 on
/// each, preparing the ports `prepare` says; and a started peer at station
/// 1 on each link, with extended frames where its link has them.
async fn two_links(prepare: [bool; 2]) -> (BACnetRouter, [Peer; 2]) {
    let (router_1, peer_1) = LoopbackSerial::pair();
    let (router_2, peer_2) = LoopbackSerial::pair();
    let ports = [(router_1, 1, prepare[0]), (router_2, 2, prepare[1])]
        .into_iter()
        .map(|(serial, network_number, prepare)| RouterPort {
            transport: Routed {
                inner: MstpTransport::new(serial, standard(0)),
                prepare,
            },
            network_number,
        })
        .collect();
    let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let start = |serial, extended_frames| {
        let config = MstpConfig {
            extended_frames,
            ..standard(1)
        };
        MstpTransport::new(serial, config)
    };
    let mut peer_1 = start(peer_1, prepare[0]);
    let mut peer_2 = start(peer_2, prepare[1]);
    let received_1 = peer_1.start().await.unwrap();
    let received_2 = peer_2.start().await.unwrap();
    (router, [(peer_1, received_1), (peer_2, received_2)])
}

/// The next NPDU `received` gets that isn't one of the router's startup
/// announcements (I-Am-Router, Network-Number-Is).
async fn next_npdu(received: &mut mpsc::Receiver<ReceivedNpdu>) -> Vec<u8> {
    timeout(Duration::from_secs(10), async {
        loop {
            let npdu = received.recv().await.unwrap().npdu;
            let message = npdu.get(1).is_some_and(|control| control & 0x80 != 0);
            if !(message && matches!(npdu.get(2), Some(0x01 | 0x13))) {
                break npdu.to_vec();
            }
        }
    })
    .await
    .expect("an NPDU arrives")
}

/// An NPDU of `len` octets for station `mac` on `network`, with no source
/// fields: a Who-Is, padded.
fn npdu_of(len: usize, network: u16, mac: u8) -> Vec<u8> {
    let mut npdu = vec![0x01, 0x20];
    npdu.extend_from_slice(&network.to_be_bytes());
    npdu.extend_from_slice(&[0x01, mac, 0xFF, 0x10, 0x08]);
    npdu.resize(len, 0x5A);
    npdu
}

/// The reason and network of a reject sent to a node on the link.
fn reject_of(npdu: &[u8]) -> (u8, u16) {
    let [0x01, 0x80, 0x03, reason, high, low] = *npdu else {
        panic!("expected a local reject, got {npdu:02x?}");
    };
    (reason, u16::from_be_bytes([high, low]))
}

/// #1665, on the wire: network 9 lies behind station 1 on network 1, and
/// the router's MS/TP ports carry 1,497 octets with the extended frames it
/// turned on. A request from network 2 whose rewrite comes to 1,498 octets
/// (it gains SNET, SLEN and a one-octet SADR) draws reject reason 4 for
/// network 9 and is counted, and only the next one, at 1,497, reaches the
/// next router.
#[tokio::test(start_paused = true)]
async fn an_npdu_too_long_for_an_mstp_port_draws_reason_4() {
    let (mut router, [(mut next_router, mut at_next), (mut source, mut at_source)]) =
        two_links([true, true]).await;
    {
        let now = router.now();
        let mut table = router.table();
        assert!(table.add_learned(9, PortId(0), Mac::from_array([1]), now));
    }

    let too_long = npdu_of(1494, 9, 5);
    source.send_unicast(&too_long, &[0]).await.unwrap();
    assert_eq!(reject_of(&next_npdu(&mut at_source).await), (4, 9));
    let counters = router.counters();
    assert_eq!((counters.egress_too_long, counters.rejects), (1, 1));

    let longest = npdu_of(1493, 9, 5);
    source.send_unicast(&longest, &[0]).await.unwrap();
    let forwarded = next_npdu(&mut at_next).await;
    assert_eq!(forwarded.len(), 1497, "the 1,498-octet rewrite never went");
    assert_eq!(forwarded[..6], [0x01, 0x28, 0x00, 0x09, 0x01, 0x05]);
    router.stop().await.unwrap();
    next_router.stop().await.unwrap();
    source.stop().await.unwrap();
}

/// #1665's second case: an MS/TP port with standard frames carries 501
/// octets, so an NPDU for station 1 there whose rewrite comes to 502 (five
/// octets of destination go, four of source come) draws reason 4 for
/// network 1, and one at 501 goes out.
#[tokio::test(start_paused = true)]
async fn a_standard_frame_mstp_port_refuses_502_octets() {
    let (
        mut router,
        [
            (mut destination, mut at_destination),
            (mut source, mut at_source),
        ],
    ) = two_links([false, true]).await;

    let too_long = npdu_of(503, 1, 1);
    source.send_unicast(&too_long, &[0]).await.unwrap();
    assert_eq!(reject_of(&next_npdu(&mut at_source).await), (4, 1));
    assert_eq!(router.counters().egress_too_long, 1);

    let longest = npdu_of(502, 1, 1);
    source.send_unicast(&longest, &[0]).await.unwrap();
    let delivered = next_npdu(&mut at_destination).await;
    assert_eq!(delivered.len(), 501, "the 502-octet rewrite never went");
    assert_eq!(delivered[..6], [0x01, 0x08, 0x00, 0x02, 0x01, 0x01]);
    router.stop().await.unwrap();
    destination.stop().await.unwrap();
    source.stop().await.unwrap();
}
