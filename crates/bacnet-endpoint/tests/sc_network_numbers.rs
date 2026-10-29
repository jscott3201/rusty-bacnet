//! Actual constrained TLS Hub/peer evidence for both local nonrouter owners.
#![cfg(feature = "sc-tls")]
#[path = "sc_network_numbers/tls.rs"]
mod tls;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_objects::{
    database::ObjectDatabase,
    network_port::{BipPortConfig, NetworkPortObject},
};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::{
    any::AnyTransport,
    sc::{ScConnection, ScConnectionState, ScTransport, WebSocketPort},
    sc_frame::{BROADCAST_VMAC, ScFunction, decode_sc_message, encode_sc_message},
    sc_hub::ScHub,
    sc_tls::{DirectAcceptConfig, TlsWebSocket},
};
use bytes::BytesMut;
use std::time::Duration;

const NODE: [u8; 6] = [0x20; 6];
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("SC fixture made no progress")
}
fn database() -> ObjectDatabase {
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
    db
}
enum Owner {
    Server(Box<BACnetServer>),
    Endpoint(Box<EndpointSession>),
}
impl Owner {
    async fn start(server: bool, transport: ScTransport<TlsWebSocket>) -> Self {
        // Exercise the same erasure/delegation used by in-repo frontends.
        let transport = AnyTransport::Sc(Box::new(transport));
        if server {
            Self::Server(Box::new(
                bounded(BACnetServer::start(
                    ServerConfig::default(),
                    database(),
                    transport,
                ))
                .await
                .unwrap(),
            ))
        } else {
            let mut endpoint =
                EndpointSession::new(transport, SessionRole::Both, Default::default())
                    .unwrap()
                    .with_database(database());
            bounded(endpoint.start()).await.unwrap();
            Self::Endpoint(Box::new(endpoint))
        }
    }
    async fn stop(&mut self) {
        match self {
            Self::Server(owner) => bounded(owner.stop()).await.unwrap(),
            Self::Endpoint(owner) => {
                bounded(owner.stop()).await.unwrap();
            }
        }
    }
}
struct Peer {
    ws: TlsWebSocket,
    connection: ScConnection,
}
impl Peer {
    async fn send(&mut self, destination: [u8; 6], npdu: &[u8]) {
        let frame = self.connection.build_encapsulated_npdu(destination, npdu);
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &frame);
        bounded(self.ws.send(&wire)).await.unwrap();
    }
    async fn number(&self, expected: u16) {
        let bytes = bounded(self.ws.recv()).await.unwrap();
        let frame = decode_sc_message(&bytes).unwrap();
        assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(frame.originating_vmac, Some(NODE));
        assert_eq!(frame.destination_vmac, Some(BROADCAST_VMAC));
        // Independent fixed local NPDU: version1, network message, type19,
        // two network-number octets, learned flag0 (even learned-configured).
        assert_eq!(
            frame.payload.as_ref(),
            [1, 0x80, 0x13, (expected >> 8) as u8, expected as u8, 0]
        );
    }
}
async fn real_tls(server: bool) {
    let ca = tls::test_ca();
    let mut hub = ScHub::start_with_uuid("127.0.0.1:0", ca.hub_tls, [0x10; 6], [0x11; 16])
        .await
        .unwrap();
    let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
    let ws = bounded(TlsWebSocket::connect(&url, ca.node_tls.clone()))
        .await
        .unwrap();
    let (transport, mut listener) = ScTransport::new(ws, NODE)
        .with_device_uuid([0x21; 16])
        .with_direct_listener(DirectAcceptConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            NODE,
            [0x21; 16],
            ca.node_tls.clone(),
        ))
        .await
        .unwrap();
    let mut states = transport.connection_state_changes();
    let mut owner = Owner::start(server, transport).await;
    let ws = bounded(TlsWebSocket::connect(&url, ca.node_tls.clone()))
        .await
        .unwrap();
    let mut connection = ScConnection::new([0x30; 6], [0x31; 16]);
    let mut connect = BytesMut::new();
    encode_sc_message(&mut connect, &connection.build_connect_request());
    bounded(ws.send(&connect)).await.unwrap();
    assert!(
        connection
            .handle_connect_accept(&decode_sc_message(&bounded(ws.recv()).await.unwrap()).unwrap())
    );
    let mut peer = Peer { ws, connection };
    // UNKNOWN has no configured SC authority, even with unrelated B/IP999.
    peer.send(NODE, &[1, 0x80, 0x13, 3, 0xe7, 1]).await;
    peer.send(NODE, &[1, 0x80, 0x12]).await;
    peer.send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, 77, 0]).await;
    peer.send(BROADCAST_VMAC, &[1, 0x80, 0x12]).await;
    peer.number(77).await;
    // Clause 6.4.19 explicitly permits local unicast What-Is-Network-Number.
    peer.send(NODE, &[1, 0x80, 0x12]).await;
    peer.number(77).await;
    // Learned values update until configured evidence takes precedence.
    for (number, flag, expected) in [(78, 0, 78), (79, 1, 79), (80, 0, 79), (81, 1, 81)] {
        peer.send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, number, flag])
            .await;
        peer.send(NODE, &[1, 0x80, 0x12]).await;
        peer.number(expected).await;
    }
    for (dest, invalid) in [
        (NODE, vec![1, 0x80, 0x13, 0, 200, 1]), // unicast cannot teach
        (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 0, 1]),
        (BROADCAST_VMAC, vec![1, 0x80, 0x13, 255, 255, 1]),
        (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200, 2]),
        (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200]),
        (BROADCAST_VMAC, vec![1, 0x80, 0x13, 0, 200, 1, 0]),
        (BROADCAST_VMAC, vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 200, 1]),
        (
            BROADCAST_VMAC,
            vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x13, 0, 200, 1],
        ),
    ] {
        peer.send(dest, &invalid).await;
        peer.send(NODE, &[1, 0x80, 0x12]).await;
        peer.number(81).await;
    }
    // Any forbidden response here would carry the old number and be observed
    // before the positive new-number response on this same control FIFO.
    for (i, invalid) in [
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 0xff, 0xff, 0, 255, 0x12],
    ]
    .iter()
    .enumerate()
    {
        peer.send(NODE, invalid).await;
        let number = 82 + i as u8;
        peer.send(BROADCAST_VMAC, &[1, 0x80, 0x13, 0, number, 1])
            .await;
        peer.send(NODE, &[1, 0x80, 0x12]).await;
        peer.number(number.into()).await;
    }
    // A direct What-Is is valid, but NNI is a local broadcast through the Hub,
    // never an APDU reply on the original direct response capability.
    let direct_url = format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().port()
    );
    let direct = bounded(TlsWebSocket::connect_direct(&direct_url, ca.node_tls))
        .await
        .unwrap();
    let mut direct_connection = ScConnection::new([0x40; 6], [0x41; 16]);
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &direct_connection.build_connect_request());
    direct.send(&wire).await.unwrap();
    assert!(direct_connection.handle_connect_accept(
        &decode_sc_message(&bounded(direct.recv()).await.unwrap()).unwrap()
    ));
    wire.clear();
    encode_sc_message(
        &mut wire,
        &direct_connection
            .build_direct_encapsulated_npdu(&[1, 0x80, 0x12], &[])
            .unwrap(),
    );
    direct.send(&wire).await.unwrap();
    peer.number(84).await;
    // Remote Hub teardown is observed before owner shutdown. The still-live
    // direct peer cannot turn a Number reply into original-socket unicast.
    hub.stop().await;
    bounded(async {
        while *states.borrow_and_update() != ScConnectionState::Disconnected {
            states.changed().await.unwrap();
        }
    })
    .await;
    direct.send(&wire).await.unwrap();
    owner.stop().await;
    listener.stop().await;
    drop(direct);
    drop(peer);
}
#[tokio::test]
async fn sc_number_full_server_real_tls_broadcast_and_unicast_request() {
    real_tls(true).await;
}
#[tokio::test]
async fn sc_number_shared_endpoint_real_tls_broadcast_and_unicast_request() {
    real_tls(false).await;
}
