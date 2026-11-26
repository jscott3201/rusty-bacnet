#![allow(clippy::print_stderr)] // the peer helpers trace wire frames to diagnose failing tests
use super::tls;
use bacnet_client::client::{BACnetClient, ClientConfig, ClientOptions};
use bacnet_transport::{
    sc::{ScConnection, ScConnectionState, ScTransport, WebSocketPort},
    sc_frame::{BROADCAST_VMAC, ScFunction, ScMessage, decode_sc_message, encode_sc_message},
    sc_hub::ScHub,
    sc_tls::{DirectAcceptConfig, DirectListener, TlsWebSocket},
};
use bytes::BytesMut;
use std::time::Duration;
pub const NODE: [u8; 6] = [0x20; 6];
pub const PEER: [u8; 6] = [0x30; 6];
pub const QUERY: &[u8] = &[1, 0x80, 0x12];
pub type Client = BACnetClient<ScTransport<TlsWebSocket>>;
/// The window the client's question opens here (#2826).
const WINDOW_MS: u64 = 1_000;
pub async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("client SC fixture made no progress")
}
pub struct Peer {
    pub ws: TlsWebSocket,
    connection: ScConnection,
}
impl Peer {
    async fn connect(ws: TlsWebSocket, vmac: [u8; 6], uuid: [u8; 16]) -> Self {
        let mut connection = ScConnection::new(vmac, uuid);
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &connection.build_connect_request());
        bounded(ws.send(&wire)).await.unwrap();
        assert!(connection.handle_connect_accept(
            &decode_sc_message(&bounded(ws.recv()).await.unwrap()).unwrap()
        ));
        Self { ws, connection }
    }
    pub async fn send(&mut self, destination: [u8; 6], npdu: &[u8]) {
        let frame = self.connection.build_encapsulated_npdu(destination, npdu);
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &frame);
        eprintln!("Hub peer send: {wire:02x?}");
        bounded(self.ws.send(&wire)).await.unwrap();
    }
    pub async fn direct_query(&mut self) {
        let frame = self
            .connection
            .build_direct_encapsulated_npdu(QUERY, &[])
            .unwrap();
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &frame);
        eprintln!("direct peer send: {wire:02x?}");
        bounded(self.ws.send(&wire)).await.unwrap();
    }
    pub async fn receive(&self) -> ScMessage {
        let wire = bounded(self.ws.recv()).await.unwrap();
        eprintln!("Hub peer receive: {wire:02x?}");
        let frame = decode_sc_message(&wire).unwrap();
        assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(frame.originating_vmac, Some(NODE));
        frame
    }
    pub async fn number(&self, expected: u16) {
        let frame = self.receive().await;
        assert_eq!(frame.destination_vmac, Some(BROADCAST_VMAC));
        assert_eq!(
            frame.payload.as_ref(),
            [1, 0x80, 0x13, (expected >> 8) as u8, expected as u8, 0]
        );
    }
}
pub struct Fixture {
    pub client: Option<Client>,
    hub: ScHub,
    listener: DirectListener,
    states: tokio::sync::watch::Receiver<ScConnectionState>,
    pub peer: Peer,
    pub direct: Peer,
}
impl Fixture {
    pub async fn start() -> Self {
        let ca = tls::test_ca();
        let hub = ScHub::start_with_uuid("127.0.0.1:0", ca.hub_tls, [0x10; 6], [0x11; 16])
            .await
            .unwrap();
        let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
        let ws = bounded(TlsWebSocket::connect(&url, ca.node_tls.clone()))
            .await
            .unwrap();
        let (transport, listener) = ScTransport::new(ws, NODE)
            .with_device_uuid([0x21; 16])
            .with_direct_listener(DirectAcceptConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                NODE,
                [0x21; 16],
                ca.node_tls.clone(),
            ))
            .await
            .unwrap();
        let states = transport.connection_state_changes();
        // The client takes a number only from the answers to its own
        // question, within a window after it (#2826): a short one, through
        // the test hook, keeps this wall-clock test quick.
        let options = ClientOptions::default().with_number_window(Duration::from_millis(WINDOW_MS));
        let client = bounded(BACnetClient::start_with_options(
            ClientConfig::default(),
            transport,
            options,
        ))
        .await
        .unwrap();
        let peer = Peer::connect(
            bounded(TlsWebSocket::connect(&url, ca.node_tls.clone()))
                .await
                .unwrap(),
            PEER,
            [0x31; 16],
        )
        .await;
        let direct_url = format!(
            "wss://localhost:{}/.bacnet/sc",
            listener.local_addr().port()
        );
        let direct = Peer::connect(
            bounded(TlsWebSocket::connect_direct(&direct_url, ca.node_tls))
                .await
                .unwrap(),
            [0x40; 6],
            [0x41; 16],
        )
        .await;
        assert_eq!(hub.status().await.client_count, 2);
        assert_eq!(listener.active_connections(), 1);
        Self {
            client: Some(client),
            hub,
            listener,
            states,
            peer,
            direct,
        }
    }
    /// Have the client ask the peer alone, take its question off the peer's
    /// connection, send `npdu` from the peer to `destination` inside the
    /// window the question opens, and wait for the window to close. The
    /// client takes a number only from such answers (#2826): one it admits
    /// here decides, one it refuses decides nothing.
    pub async fn in_a_window(&mut self, destination: [u8; 6], npdu: &[u8]) {
        let client = self.client.as_ref().unwrap();
        bounded(client.what_is_network_number(Some(&PEER)))
            .await
            .unwrap();
        let question = self.peer.receive().await;
        assert_eq!(question.destination_vmac, None, "to the peer alone");
        assert_eq!(question.payload.as_ref(), QUERY);
        self.peer.send(destination, npdu).await;
        tokio::time::sleep(Duration::from_millis(WINDOW_MS + 200)).await;
    }

    /// Stops the client (or drops it bare), waits until the hub and direct
    /// listener see it gone, then stops both listeners and binds each address
    /// again to prove it was released. Both are stopped whatever the first
    /// bind returns; the first failed bind goes back to
    /// [`rerun_on_lost_port`](crate::port_retry::rerun_on_lost_port), since
    /// another process can take a port between a stop and its bind (#1070).
    pub async fn shutdown(mut self, bare_drop: bool) -> std::io::Result<()> {
        let mut client = self.client.take().unwrap();
        if !bare_drop {
            bounded(client.stop()).await.unwrap();
        }
        drop(client);
        // Drop requests cleanup; positive state/physical-owner observations,
        // rather than a quiet wire interval, prove eventual retirement.
        bounded(async {
            while *self.states.borrow_and_update() != ScConnectionState::Disconnected {
                self.states.changed().await.unwrap();
            }
            while self.hub.status().await.client_count != 1
                || self.listener.active_connections() != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await;
        let direct_address = self.listener.local_addr();
        // The external listener owns its join and bind independently of client.
        bounded(self.listener.stop()).await;
        let direct_rebound = tokio::net::TcpListener::bind(direct_address)
            .await
            .map(drop);
        let hub_address = self.hub.local_addr().unwrap();
        bounded(self.hub.stop()).await;
        let hub_rebound = tokio::net::TcpListener::bind(hub_address).await.map(drop);
        direct_rebound.and(hub_rebound)
    }
}
