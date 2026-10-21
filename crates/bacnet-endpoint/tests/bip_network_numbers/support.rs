//! Independent byte/UDP helpers adapted from the full-server B/IP fixture.
#![allow(clippy::print_stderr)] // the fixture reports skipped setups outside a test function
use bacnet_endpoint::{bip::BipEndpointBuilder, session::EndpointSession};
use bacnet_objects::{
    analog::AnalogInputObject,
    database::ObjectDatabase,
    network_port::{BipPortConfig, NetworkPortObject},
};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::EngineeringUnits;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio::net::UdpSocket;
pub type Endpoint = EndpointSession<BipTransport>;
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
    eprintln!("send {} -> {target}: {bytes:02x?}", address(socket));
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
    eprintln!("recv {source} -> {}: {bytes:02x?}", address(socket));
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
pub async fn start(builder: BipEndpointBuilder) -> (Endpoint, SocketAddrV4) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            9,
            "unrelated declared BIP",
            BipPortConfig {
                network_number: 999,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let mut analog =
        AnalogInputObject::new(1, "Number progress", EngineeringUnits::SQUARE_METERS).unwrap();
    analog.set_present_value(42.0);
    db.add(Box::new(analog)).unwrap();
    let mut endpoint = builder.database(db).build_session().unwrap();
    bounded(endpoint.start()).await.unwrap();
    let local = endpoint.bip_local_address().unwrap();
    (endpoint, local)
}
pub fn builder() -> BipEndpointBuilder {
    builder_on(0)
}
pub fn builder_on(port: u16) -> BipEndpointBuilder {
    BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, port, BROADCAST).client_timers(2000, 0)
}
/// Stops the endpoint and binds its address, which only succeeds once the
/// endpoint has closed its wildcard socket. The bind's result goes back to
/// [`rerun_on_lost_port`](crate::port_retry::rerun_on_lost_port), since another
/// process can take the port between the stop and the bind (#1070).
pub async fn stopped(mut endpoint: Endpoint, local: SocketAddrV4) -> std::io::Result<()> {
    bounded(endpoint.stop()).await.unwrap();
    drop(endpoint);
    std::net::UdpSocket::bind(local).map(drop)
}
