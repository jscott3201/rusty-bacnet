//! Real loopback UDP; only the optional producer/stop gates replace scheduling.
use bacnet_objects::{
    analog::AnalogInputObject,
    database::ObjectDatabase,
    network_port::{BipPortConfig, NetworkPortObject},
};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::{
    any::AnyTransport,
    bip::BipTransport,
    bvll::decode_bip_mac,
    mstp::LoopbackSerial,
    port::{ReceivedNpdu, TransportPort},
};
use bacnet_types::error::Error;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, Semaphore},
};

pub const BROADCAST: Ipv4Addr = Ipv4Addr::new(127, 255, 255, 255);
pub const QUERY: &[u8] = &[1, 0x80, 0x12];
pub async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .expect("B/IP Number fixture made no progress")
}
pub async fn udp() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    socket.set_broadcast(true).unwrap();
    socket
}
pub fn frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x81, function];
    bytes.extend_from_slice(&((4 + payload.len()) as u16).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
pub fn number(value: u16, flag: u8) -> Vec<u8> {
    vec![1, 0x80, 0x13, (value >> 8) as u8, value as u8, flag]
}
pub fn forwarded(origin: SocketAddrV4, npdu: &[u8]) -> Vec<u8> {
    let mut payload = origin.ip().octets().to_vec();
    payload.extend_from_slice(&origin.port().to_be_bytes());
    payload.extend_from_slice(npdu);
    frame(4, &payload)
}
pub fn address(socket: &UdpSocket) -> SocketAddrV4 {
    let SocketAddr::V4(a) = socket.local_addr().unwrap() else {
        panic!("IPv4 socket")
    };
    a
}
pub async fn send(socket: &UdpSocket, target: SocketAddrV4, bytes: &[u8]) {
    assert!(target.ip().is_loopback());
    assert_eq!(
        bounded(socket.send_to(bytes, target)).await.unwrap(),
        bytes.len()
    );
}
pub async fn receive(socket: &UdpSocket) -> (Vec<u8>, SocketAddrV4) {
    let mut bytes = [0; 2048];
    let (n, source) = bounded(socket.recv_from(&mut bytes)).await.unwrap();
    let SocketAddr::V4(source) = source else {
        panic!("IPv4 source")
    };
    assert!(source.ip().is_loopback());
    let bytes = bytes[..n].to_vec();
    assert!(bytes.len() >= 4);
    assert_eq!(bytes[0], 0x81);
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
        bytes.len()
    );
    (bytes, source)
}
pub async fn expect_number(socket: &UdpSocket, source: SocketAddrV4, function: u8, value: u16) {
    bounded(async {
        loop {
            let (bytes, from) = receive(socket).await;
            // BBMD input broadcasts and ordinary fanout are not its own reply.
            if from != source || bytes[1] == 4 {
                continue;
            }
            assert_eq!(
                bytes,
                frame(function, &number(value, 0)),
                "exact local learned Number frame"
            );
            break;
        }
    })
    .await;
}
pub async fn fence(socket: &UdpSocket, target: SocketAddrV4) {
    // A management response observes the same UDP receive loop after prior
    // datagrams on this local path. It is not Number-worker completion; a later
    // exact Number response remains the state and control-FIFO oracle.
    send(socket, target, &frame(2, &[])).await;
    bounded(async {
        loop {
            let (bytes, source) = receive(socket).await;
            assert_eq!(source, target);
            if bytes[1] == 4 {
                continue;
            } // BBMD fanout to this BDT/FDT peer
            assert!(
                bytes[1] == 3 || bytes == frame(0, &[0, 0x20]),
                "Read-BDT response"
            );
            break;
        }
    })
    .await;
}

pub struct Gates {
    pub entered: Semaphore,
    pub dropped: Semaphore,
    pub stop_entered: Semaphore,
    pub stop_release: Semaphore,
}
impl Gates {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            dropped: Semaphore::new(0),
            stop_entered: Semaphore::new(0),
            stop_release: Semaphore::new(0),
        })
    }
}
struct Signal<'a>(&'a Semaphore);
impl Drop for Signal<'_> {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}
pub struct ObservedBip {
    inner: AnyTransport<LoopbackSerial>,
    gates: Option<Arc<Gates>>,
}
impl TransportPort for ObservedBip {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        self.inner.supports_local_nonrouter_number_controls()
    }
    fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        self.inner.bip_port()
    }
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        self.inner.bip_broadcast_endpoint()
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(gates) = &self.gates {
            gates.stop_entered.add_permits(1);
            gates.stop_release.acquire().await.unwrap().forget();
        }
        self.inner.stop().await
    }
    fn abort(&mut self) {
        self.inner.abort();
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        if let Some(gates) = &self.gates {
            if npdu.starts_with(&[1, 0x80, 0x13]) {
                let _drop = Signal(&gates.dropped);
                gates.entered.add_permits(1);
                std::future::pending::<()>().await;
            }
        }
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
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }
}
pub type Server = BACnetServer<ObservedBip>;
pub async fn start(transport: BipTransport, gates: Option<Arc<Gates>>) -> (Server, SocketAddrV4) {
    let inner = AnyTransport::Bip(Box::new(transport));
    // A BBMD or foreign device, and the server registers no Network Port.
    assert_ne!(
        inner.bip_port().unwrap().mode.ip_mode(),
        bacnet_types::enums::IPMode::NORMAL
    );
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            9,
            "unrelated configured BIP",
            BipPortConfig {
                network_number: 999,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let mut analog = AnalogInputObject::new(1, "Number progress", 0).unwrap();
    analog.set_present_value(42.0);
    db.add(Box::new(analog)).unwrap();
    let server = bounded(BACnetServer::start(
        ServerConfig::default(),
        db,
        ObservedBip { inner, gates },
    ))
    .await
    .unwrap();
    let (ip, port) = decode_bip_mac(server.local_mac()).unwrap();
    (server, SocketAddrV4::new(Ipv4Addr::from(ip), port))
}
/// A broadcast observer bound before the node under test, which then starts on
/// the returned port. Only an explicitly requested B/IP port sets SO_REUSEADDR
/// (#892), and both sockets need it to share the port.
#[cfg(target_os = "linux")]
pub fn observer() -> (UdpSocket, u16) {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .unwrap();
    socket.set_reuse_address(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket
        .bind(&SocketAddrV4::new(BROADCAST, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}
pub async fn stopped(mut server: Server, local: SocketAddrV4) {
    bounded(server.stop()).await.unwrap();
    drop(server);
    let _rebound = std::net::UdpSocket::bind(local).expect("server released wildcard socket");
}
