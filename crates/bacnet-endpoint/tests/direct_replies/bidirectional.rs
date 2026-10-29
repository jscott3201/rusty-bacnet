//! Real transport -> network -> public consumer intake on an outbound socket.
use super::*;
use bacnet_encoding::{
    apdu::{ComplexAck, decode_apdu, encode_apdu},
    npdu::{Npdu, decode_npdu, encode_npdu},
};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::{
    port::{DirectResponse, DirectResponseScope, ReceivedNpdu, TransportPort},
    sc::{LoopbackWebSocket, ScTransport, WebSocketPort},
    sc_frame::{ScFunction, ScMessage, decode_sc_message, encode_sc_message},
    sc_tls::{DirectAcceptConfig, DirectListener, ScNodeTlsConfig},
};
use bacnet_types::{
    enums::{ConfirmedServiceChoice, PropertyIdentifier},
    error::Error,
};
use bytes::{Bytes, BytesMut};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::mpsc;
const LOCAL: [u8; 6] = [0xaa; 6];
const REMOTE: [u8; 6] = [0x22; 6];

// Startup alone is adapted: every tested ingress still traverses the real
// ScTransport intake, NetworkLayer, and unmodified public consumer dispatcher.
struct StartedSc {
    transport: ScTransport<LoopbackWebSocket>,
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    ordinary_sends: Arc<AtomicUsize>,
}
impl TransportPort for StartedSc {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.rx.take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.transport.stop().await
    }
    fn abort(&mut self) {
        self.transport.abort();
    }
    async fn send_unicast(&self, bytes: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.ordinary_sends.fetch_add(1, Ordering::AcqRel);
        self.transport.send_unicast(bytes, mac).await
    }
    async fn send_broadcast(&self, bytes: &[u8]) -> Result<(), Error> {
        self.ordinary_sends.fetch_add(1, Ordering::AcqRel);
        self.transport.send_broadcast(bytes).await
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.transport.local_receive_apdu_capacity()
    }

    fn local_mac(&self) -> &[u8] {
        self.transport.local_mac()
    }
    fn egress_apdu_limit(&self) -> u16 {
        self.transport.egress_apdu_limit()
    }
}
struct Peer {
    listener: DirectListener,
    local: DirectListener,
    received: mpsc::Receiver<ReceivedNpdu>,
    route: DirectResponse,
    hub: LoopbackWebSocket,
    tls: ScNodeTlsConfig,
}
async fn hub_accept(hub: &LoopbackWebSocket) {
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    // Current Hub capacities: 16-byte envelope + 4192 options + 1497 NPDU.
    let mut payload = Vec::new();
    payload.extend_from_slice(&[0x33; 6]);
    payload.extend_from_slice(&[3; 16]);
    payload.extend_from_slice(&5705u16.to_be_bytes());
    payload.extend_from_slice(&1497u16.to_be_bytes());
    let mut wire = BytesMut::new();
    encode_sc_message(
        &mut wire,
        &ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(payload),
        },
    );
    hub.send(&wire).await.unwrap();
}
fn wire(apdu: &Apdu) -> Vec<u8> {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.to_vec()
}
fn ack(invoke: u8) -> Apdu {
    let mut payload = BytesMut::new();
    ReadPropertyACK {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: vec![0x72, 0, b'x'],
    }
    .encode(&mut payload);
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: invoke,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: payload.freeze(),
    })
}
impl Peer {
    async fn start(ca: &TestCa) -> (Self, StartedSc) {
        let tls = ca.tls("localhost");
        let config =
            DirectAcceptConfig::new("127.0.0.1:0".parse().unwrap(), REMOTE, [2; 16], tls.clone());
        let (listener, mut received) = DirectListener::start(config).await.unwrap();
        let uri = format!(
            "wss://localhost:{}/.bacnet/sc",
            listener.local_addr().port()
        );
        let (port, local, hub) = start_port(ca, uri).await;
        let route = bounded(received.recv())
            .await
            .unwrap()
            .direct_response
            .unwrap();
        (
            Self {
                listener,
                local,
                received,
                route,
                hub,
                tls,
            },
            port,
        )
    }
    async fn send(&self, apdu: &Apdu) {
        self.route
            .send(&wire(apdu), &DirectResponseScope::default())
            .await
            .unwrap();
    }
    async fn receive(&mut self) -> Apdu {
        let incoming=bounded(async { tokio::select! {
            incoming=self.received.recv()=>incoming.unwrap(),
            hub=self.hub.recv()=>panic!("original direct response used Hub: {:?}",decode_sc_message(&hub.unwrap()).unwrap().function),
        }}).await;
        let npdu = decode_npdu(incoming.npdu).unwrap();
        decode_apdu(npdu.payload).unwrap()
    }
    async fn hub_reply(&self, apdu: &Apdu) {
        let mut bytes = BytesMut::new();
        encode_sc_message(
            &mut bytes,
            &ScMessage {
                function: ScFunction::EncapsulatedNpdu,
                message_id: 101,
                originating_vmac: Some(REMOTE),
                destination_vmac: None,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::from(wire(apdu)),
            },
        );
        self.hub.send(&bytes).await.unwrap();
    }
    async fn stop(&mut self) {
        self.listener.stop().await;
        self.local.stop().await;
    }
}
async fn start_port(ca: &TestCa, uri: String) -> (StartedSc, DirectListener, LoopbackWebSocket) {
    let (ws, hub) = LoopbackWebSocket::pair();
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        LOCAL,
        [1; 16],
        ca.tls("localhost"),
    );
    let (mut transport, local) = ScTransport::new(ws, LOCAL)
        .with_device_uuid([1; 16])
        .with_direct_tls(ca.tls("caller"))
        .with_direct_listener(config)
        .await
        .unwrap();
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub));
    let rx = started.unwrap();
    let (sent, ()) = tokio::join!(transport.send_unicast(&[1, 0, 0x10, 8], &REMOTE), async {
        let ar = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
        assert_eq!(ar.function, ScFunction::AddressResolution);
        let mut wire = BytesMut::new();
        encode_sc_message(
            &mut wire,
            &ScMessage {
                function: ScFunction::AddressResolutionAck,
                message_id: ar.message_id,
                originating_vmac: Some(REMOTE),
                destination_vmac: None,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::from(uri),
            },
        );
        hub.send(&wire).await.unwrap();
    });
    sent.unwrap();
    (
        StartedSc {
            transport,
            rx: Some(rx),
            ordinary_sends: Arc::new(AtomicUsize::new(0)),
        },
        local,
        hub,
    )
}
#[tokio::test]
async fn outbound_tls_public_client_notification_and_reject_use_original_socket() {
    let ca = TestCa::new();
    let (mut peer, port) = Peer::start(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let mut notifications = client.cov_notifications();
    peer.send(&super::notifications::cov(71)).await;
    assert_eq!(
        bounded(notifications.recv())
            .await
            .unwrap()
            .notification
            .subscriber_process_identifier,
        71
    );
    assert!(matches!(peer.receive().await,Apdu::SimpleAck(a) if a.invoke_id==71));
    peer.send(&unsupported(72)).await;
    assert!(matches!(peer.receive().await,Apdu::Reject(a) if a.invoke_id==72));
    client.stop().await.unwrap();
    peer.stop().await;
}
#[tokio::test]
async fn outbound_tls_live_endpoint_and_server_return_data_on_original_socket() {
    for endpoint in [false, true] {
        let ca = TestCa::new();
        let (mut peer, port) = Peer::start(&ca).await;
        if endpoint {
            let mut session =
                EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
                    .unwrap()
                    .with_database(database("outbound"));
            session.start().await.unwrap();
            peer.send(&read_name(73)).await;
            assert!(matches!(peer.receive().await,Apdu::ComplexAck(a) if a.invoke_id==73));
            session.stop().await.unwrap();
        } else {
            let config = bacnet_server::server::ServerConfig::default();
            let mut server = bacnet_server::server::BACnetServer::start_clockless(
                config,
                database("outbound"),
                port,
            )
            .await
            .unwrap();
            peer.send(&read_name(74)).await;
            assert!(matches!(peer.receive().await,Apdu::ComplexAck(a) if a.invoke_id==74));
            server.stop().await.unwrap();
        }
        peer.stop().await;
    }
}
#[tokio::test]
async fn standard_client_direct_request_accepts_matching_hub_response() {
    let ca = TestCa::new();
    let (mut peer, port) = Peer::start(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let request = client.read_property(&REMOTE, csv_oid(), PropertyIdentifier::OBJECT_NAME, None);
    let remote = async {
        let Apdu::ConfirmedRequest(request) = peer.receive().await else {
            panic!("ReadProperty request")
        };
        peer.hub_reply(&ack(request.invoke_id)).await;
    };
    let (response, ()) = tokio::join!(request, remote);
    assert_eq!(response.unwrap().property_value, vec![0x72, 0, b'x']);
    client.stop().await.unwrap();
    peer.stop().await;
}
#[tokio::test]
async fn standard_client_pending_direct_request_accepts_replacement_peer_terminal() {
    use bacnet_transport::sc_tls::TlsWebSocket;
    let ca = TestCa::new();
    let (mut peer, port) = Peer::start(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let request = client.read_property(&REMOTE, csv_oid(), PropertyIdentifier::OBJECT_NAME, None);
    let remote = async {
        let Apdu::ConfirmedRequest(request) = peer.receive().await else {
            panic!("ReadProperty request")
        };
        let ws = TlsWebSocket::connect_direct(
            &format!(
                "wss://localhost:{}/.bacnet/sc",
                peer.local.local_addr().port()
            ),
            ca.tls("replacement"),
        )
        .await
        .unwrap();
        let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
        let mut bytes = BytesMut::new();
        encode_sc_message(&mut bytes, &connection.build_connect_request());
        ws.send(&bytes).await.unwrap();
        assert_eq!(
            decode_sc_message(&ws.recv().await.unwrap())
                .unwrap()
                .function,
            ScFunction::ConnectAccept
        );
        bytes.clear();
        encode_sc_message(
            &mut bytes,
            &connection
                .build_direct_encapsulated_npdu(&wire(&ack(request.invoke_id)), &[])
                .unwrap(),
        );
        ws.send(&bytes).await.unwrap();
        ws // Hold replacement alive until transaction completion.
    };
    let (response, _replacement) = tokio::join!(request, remote);
    assert_eq!(response.unwrap().property_value, vec![0x72, 0, b'x']);
    client.stop().await.unwrap();
    peer.stop().await;
}

#[path = "outbound_budget.rs"]
mod budget;

#[path = "outbound_segments.rs"]
mod segments;

#[tokio::test]
async fn standard_client_hub_request_accepts_matching_direct_response() {
    use bacnet_transport::sc_tls::TlsWebSocket;
    let ca = TestCa::new();
    let (mut peer, mut port) = Peer::start(&ca).await;
    // Retire the priming outbound socket. The independently registered local
    // listener continues accepting, while discovery is now disabled.
    port.transport = port.transport.with_direct_discovery(false);
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let request = client.read_property(&REMOTE, csv_oid(), PropertyIdentifier::OBJECT_NAME, None);
    let remote = async {
        let frame = decode_sc_message(&bounded(peer.hub.recv()).await.unwrap()).unwrap();
        assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(frame.destination_vmac, Some(REMOTE));
        let npdu = decode_npdu(frame.payload).unwrap();
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("ReadProperty request")
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY
        );
        let ws = TlsWebSocket::connect_direct(
            &format!(
                "wss://localhost:{}/.bacnet/sc",
                peer.local.local_addr().port()
            ),
            ca.tls("responder"),
        )
        .await
        .unwrap();
        let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
        let mut bytes = BytesMut::new();
        encode_sc_message(&mut bytes, &connection.build_connect_request());
        ws.send(&bytes).await.unwrap();
        assert_eq!(
            decode_sc_message(&ws.recv().await.unwrap())
                .unwrap()
                .function,
            ScFunction::ConnectAccept
        );
        bytes.clear();
        encode_sc_message(
            &mut bytes,
            &connection
                .build_direct_encapsulated_npdu(&wire(&ack(request.invoke_id)), &[])
                .unwrap(),
        );
        ws.send(&bytes).await.unwrap();
        ws
    };
    let (response, _responder) = tokio::join!(request, remote);
    assert_eq!(response.unwrap().property_value, vec![0x72, 0, b'x']);
    client.stop().await.unwrap();
    peer.stop().await;
}

#[path = "outbound_notifications.rs"]
mod native_notifications;

#[path = "outbound_replacement.rs"]
mod replacement;
