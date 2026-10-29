//! B/IPv6 nodes are named by their 3-octet VMAC above the data link (U.1.3,
//! #1637): a router carries a B/IPv6 node's VMAC as its SADR and takes a
//! VMAC as DADR, queueing rather than stalling on one it must resolve, and
//! a client's request to a VMAC the transport doesn't know waits for its
//! resolution (U.5) and fails as a transport timeout when nothing answers.
#![cfg(feature = "ipv6")]

use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::{Arc, OnceLock};

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_npdu::NpduLimits;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{DataAttribute, GroupDestinations, ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, timeout};

/// The router's MAC on network 1.
const ROUTER_ON_1: [u8; 6] = [0x0A, 0x00, 0x00, 0x01, 0xBA, 0xC0];
/// A node on network 1.
const NODE_ON_1: [u8; 6] = [0x0A, 0x00, 0x00, 0x05, 0xBA, 0xC0];
/// The B/IPv6 peer on network 2: device instance 1234.
const PEER_VMAC: [u8; 3] = [0x00, 0x04, 0xD2];
/// A confirmed ReadProperty of Device 1's Object_Name.
const REQUEST: [u8; 11] = [
    0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// A SimpleAck for invoke ID 1.
const REPLY: [u8; 3] = [0x20, 0x01, 0x0C];

/// A router port that, once started, reports its B/IPv6 transport's bound
/// address, so the transport can take an ephemeral port rather than one
/// guessed free beforehand.
struct Reporting {
    inner: AnyTransport,
    bound: Arc<OnceLock<SocketAddrV6>>,
}

impl TransportPort for Reporting {
    fn prepare_for_routing(&mut self) {
        self.inner.prepare_for_routing();
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let received = self.inner.start().await?;
        if let AnyTransport::Bip6(bip6) = &self.inner {
            let _ = self.bound.set(bip6.local_address().expect("started"));
        }
        Ok(received)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.inner
            .send_unicast_with_data_attributes(npdu, mac, attributes)
            .await
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

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.inner.is_group_destination(mac)
    }

    fn group_destinations(&self) -> GroupDestinations {
        self.inner.group_destinations()
    }
}

/// A Original-Unicast-NPDU from VMAC `source` to VMAC `destination`.
fn original_unicast(source: [u8; 3], destination: [u8; 3], npdu: &[u8]) -> Vec<u8> {
    let length = u16::try_from(10 + npdu.len()).unwrap().to_be_bytes();
    let mut frame = vec![0x82, 0x01, length[0], length[1]];
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(npdu);
    frame
}

/// The next NPDU from the router that isn't a network-layer message.
async fn next_apdu_npdu(from_router: &mut mpsc::Receiver<ReceivedNpdu>) -> Vec<u8> {
    loop {
        let npdu = timeout(Duration::from_secs(5), from_router.recv())
            .await
            .expect("the router forwards it")
            .unwrap();
        if npdu.npdu[1] & 0x80 == 0 {
            return npdu.npdu.to_vec();
        }
    }
}

/// The next datagram `socket` reads that is an Original-Unicast-NPDU.
async fn next_unicast(socket: &UdpSocket) -> Vec<u8> {
    let mut buf = [0u8; 1500];
    loop {
        let (len, _) = timeout(Duration::from_secs(5), socket.recv_from(&mut buf))
            .await
            .expect("the router sends it")
            .unwrap();
        if buf[1] == 0x01 {
            return buf[..len].to_vec();
        }
    }
}

/// A router between network 1 (in-process) and network 2 (B/IPv6 on ::1,
/// VMAC 00 00 0A), and the node on network 1.
async fn router() -> (BACnetRouter, LoopbackTransport, SocketAddrV6) {
    let bip6 = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let (router_end, node) = LoopbackTransport::pair(ROUTER_ON_1.to_vec(), NODE_ON_1.to_vec());
    let bound = Arc::new(OnceLock::new());
    let port = |inner, network_number| RouterPort {
        transport: Reporting {
            inner,
            bound: Arc::clone(&bound),
        },
        network_number,
    };
    let ports = vec![
        port(AnyTransport::Loopback(router_end), 1),
        port(AnyTransport::Bip6(bip6), 2),
    ];
    let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let address = *bound.get().expect("the B/IPv6 port started");
    (router, node, address)
}

/// The peer's request to the node on network 1 goes out there with the
/// peer's VMAC as a 3-octet SADR, and the node's reply to that SADR comes
/// back to the peer as an Original-Unicast-NPDU to its VMAC, the node's
/// 6-octet MAC as SADR. Byte for byte, both ways.
#[tokio::test]
async fn a_router_carries_a_bip6_nodes_vmac_as_sadr_and_dadr() {
    let (mut router, mut node, router_address) = router().await;
    let mut from_router = node.start().await.unwrap();
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();

    // To network 1, MAC 0A 00 00 05 BA C0, expecting a reply, hop count 255.
    let mut npdu = vec![0x01, 0x24, 0x00, 0x01, 0x06];
    npdu.extend_from_slice(&NODE_ON_1);
    npdu.push(0xFF);
    npdu.extend_from_slice(&REQUEST);
    let frame = original_unicast(PEER_VMAC, [0x00, 0x00, 0x0A], &npdu);
    peer.send_to(&frame, router_address).await.unwrap();

    let mut expected = vec![0x01, 0x0C, 0x00, 0x02, 0x03, 0x00, 0x04, 0xD2];
    expected.extend_from_slice(&REQUEST);
    assert_eq!(next_apdu_npdu(&mut from_router).await, expected);

    // The reply to network 2, DADR 00 04 D2, hop count 255.
    let mut reply = vec![0x01, 0x20, 0x00, 0x02, 0x03, 0x00, 0x04, 0xD2, 0xFF];
    reply.extend_from_slice(&REPLY);
    node.send_unicast(&reply, &ROUTER_ON_1).await.unwrap();
    let mut forwarded = vec![0x01, 0x08, 0x00, 0x01, 0x06];
    forwarded.extend_from_slice(&NODE_ON_1);
    forwarded.extend_from_slice(&REPLY);
    assert_eq!(
        next_unicast(&peer).await,
        original_unicast([0x00, 0x00, 0x0A], PEER_VMAC, &forwarded)
    );
    router.stop().await.unwrap();
    node.stop().await.unwrap();
}

/// The router's B/IPv6 port queues an NPDU for a VMAC it must resolve and
/// goes on: the next NPDU, for a known VMAC, isn't held behind it for the
/// three seconds a resolution may take.
#[tokio::test]
async fn a_routers_bip6_port_does_not_stall_behind_an_unknown_vmac() {
    let (mut router, mut node, router_address) = router().await;
    let _from_router = node.start().await.unwrap();
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();
    // The peer makes itself known with an unconfirmed Who-Is to network 1.
    let mut who_is = vec![0x01, 0x20, 0x00, 0x01, 0x00, 0xFF];
    who_is.extend_from_slice(&[0x10, 0x08]);
    let frame = original_unicast(PEER_VMAC, [0x00, 0x00, 0x0A], &who_is);
    peer.send_to(&frame, router_address).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let to = |vmac: [u8; 3]| {
        let mut npdu = vec![0x01, 0x20, 0x00, 0x02, 0x03];
        npdu.extend_from_slice(&vmac);
        npdu.extend_from_slice(&[0xFF, 0x10, 0x08]);
        npdu
    };
    let start = Instant::now();
    node.send_unicast(&to([0x00, 0x04, 0xD3]), &ROUTER_ON_1)
        .await
        .unwrap();
    node.send_unicast(&to(PEER_VMAC), &ROUTER_ON_1)
        .await
        .unwrap();
    let arrived = next_unicast(&peer).await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "held {:?} behind the unknown VMAC",
        start.elapsed()
    );
    assert_eq!(&arrived[7..10], PEER_VMAC);
    router.stop().await.unwrap();
    node.stop().await.unwrap();
}

const SERVER_INSTANCE: u32 = 1234;

async fn server() -> BACnetServer<Bip6Transport> {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(SERVER_INSTANCE, 0)
            .expect("valid Device identity")
            .with_name("B/IPv6 server"),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(SERVER_INSTANCE, 0).unwrap())
        .database(db)
        .transport(Bip6Transport::new(
            Ipv6Addr::LOCALHOST,
            0,
            Some(SERVER_INSTANCE),
        ))
        .build()
        .await
        .unwrap()
}

async fn client() -> BACnetClient<Bip6Transport> {
    BACnetClient::generic_builder()
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10)))
        .build()
        .await
        .unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, SERVER_INSTANCE).unwrap()
}

/// A request to a VMAC nobody answers fails with the transport's TimedOut
/// once the three-second resolution gives up, not with the transaction
/// manager's timeout, whose timer never started.
#[tokio::test(start_paused = true)]
async fn a_request_to_an_unknown_vmac_fails_with_a_transport_timeout() {
    let mut client = client().await;
    let start = Instant::now();
    let result = client
        .read_property(
            &[0x00, 0x04, 0xD3],
            device(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await;
    assert!(
        matches!(&result, Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::TimedOut),
        "{result:?}"
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_secs(3) && elapsed < Duration::from_millis(3500),
        "{elapsed:?}"
    );
    let counters = client.transport().resolution_counters();
    assert_eq!(
        (counters.resolutions_started, counters.resolutions_timed_out),
        (1, 1)
    );
    client.stop().await.unwrap();
}

/// A request to the server's VMAC waits on its resolution; a
/// Virtual-Address-Resolution of the server's address, run meanwhile,
/// teaches the client the VMAC, which releases the request, and the server
/// answers the client's VMAC, learned from the request.
#[tokio::test]
async fn a_request_waiting_on_resolution_goes_out_once_the_vmac_resolves() {
    let mut server = server().await;
    let server_address = server.transport().unwrap().local_address().unwrap();
    let server_vmac = server.local_mac().to_vec();
    assert_eq!(server_vmac, PEER_VMAC);
    let mut client = client().await;
    let read = client.read_property(
        &server_vmac,
        device(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    let resolve = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        client
            .transport()
            .resolve_virtual_address(server_address)
            .await
    };
    let (read, found) = timeout(Duration::from_secs(5), async {
        tokio::join!(read, resolve)
    })
    .await
    .expect("well inside the resolution timeout");
    assert_eq!(found.unwrap().octets(), PEER_VMAC);
    let ack = read.unwrap();
    assert_eq!(ack.object_identifier, device());
    let name = b"B/IPv6 server";
    assert!(ack.property_value.windows(name.len()).any(|w| w == name));
    let counters = client.transport().resolution_counters();
    assert_eq!(
        (counters.resolutions_started, counters.resolutions_resolved),
        (1, 1)
    );
    let client_address = client.transport().local_address().unwrap();
    let client_vmac = client.transport().local_vmac().unwrap();
    assert_eq!(
        server.transport().unwrap().lookup_vmac(client_vmac),
        Some(client_address)
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
