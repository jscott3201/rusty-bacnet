//! Real accepted TLS -> queued transport -> NetworkLayer -> live dispatch.
//!
//! The queue is a deterministic scheduling seam: every direct envelope comes
//! unchanged from the actual listener. Live replies are decoded from the exact
//! accepted TLS socket; a separate generic-egress spy detects fallback.
use super::*;
use crate::server::test_transport::{SendMode, TestTransport};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyPointResetCommit};
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{DirectScIdentity, ReceivedNpdu};
use bacnet_transport::sc::{ScConnection, WebSocketPort};
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message};
use bacnet_transport::sc_tls::{DirectAcceptConfig, DirectListener, ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::LifeSafetyState;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

#[path = "direct_principal_complete_tests.rs"]
mod complete;
#[path = "direct_principal_context_tests.rs"]
mod contexts;
#[path = "direct_principal_policy_tests.rs"]
mod policy;
#[path = "direct_response_tests.rs"]
mod responses;
#[path = "direct_principal_router_tests.rs"]
mod router;
#[path = "direct_principal_segment_tests.rs"]
mod segments;
use super::warn_spy;

const PEER_MAC: [u8; 6] = [0x22; 6];
const PEER_UUID: [u8; 16] = [7; 16];

struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    der: CertificateDer<'static>,
}
impl TestCa {
    fn new() -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let der = params.self_signed(&key).unwrap().der().clone();
        Self { params, key, der }
    }
    fn tls(&self, name: &str) -> ScNodeTlsConfig {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![name.into()])
            .unwrap()
            .signed_by(&key, &rcgen::Issuer::from_params(&self.params, &self.key))
            .unwrap();
        ScNodeTlsConfig::from_der(
            vec![self.der.clone()],
            vec![cert.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    }
}

/// Feeds the server from `incoming` and decodes every generic-egress unicast
/// onto `responses`, the fallback spy. Broadcasts are dropped.
fn queued_port(
    incoming: mpsc::Receiver<ReceivedNpdu>,
    responses: mpsc::UnboundedSender<Apdu>,
) -> TestTransport {
    TestTransport::builder()
        .local_mac(&[0xaa; 6])
        .inbound(incoming)
        .broadcast(SendMode::Ignore)
        .on_send(move |frame| {
            responses.send(frame.apdu()).unwrap();
            std::future::ready(Ok(()))
        })
        .build()
}

struct Fixture {
    server: BACnetServer<TestTransport>,
    listener: DirectListener,
    admitted: mpsc::Receiver<ReceivedNpdu>,
    incoming: mpsc::Sender<ReceivedNpdu>,
    responses: mpsc::UnboundedReceiver<Apdu>,
    current_peer: Option<Arc<TlsWebSocket>>,
}
impl Fixture {
    async fn new(ca: &TestCa, config: ServerConfig, db: ObjectDatabase) -> Self {
        let (listener, admitted) = DirectListener::start(
            DirectAcceptConfig::new(
                "127.0.0.1:0".parse().unwrap(),
                [0xaa; 6],
                [9; 16],
                ca.tls("localhost"),
            )
            .with_max_established_peers(1),
        )
        .await
        .unwrap();
        let (incoming, rx) = mpsc::channel(32);
        let (responses, observed) = mpsc::unbounded_channel();
        let server = BACnetServer::start(config, db, queued_port(rx, responses))
            .await
            .unwrap();
        Self {
            server,
            listener,
            admitted,
            incoming,
            responses: observed,
            current_peer: None,
        }
    }
    async fn peer(&mut self, tls: ScNodeTlsConfig) -> Peer {
        self.peer_config(tls, |_| {}).await
    }
    async fn peer_config(
        &mut self,
        tls: ScNodeTlsConfig,
        configure: impl FnOnce(&mut ScConnection),
    ) -> Peer {
        let url = format!(
            "wss://localhost:{}/.bacnet/sc",
            self.listener.local_addr().port()
        );
        let ws = bounded(TlsWebSocket::connect_direct(&url, tls))
            .await
            .unwrap();
        let mut connection = ScConnection::new(PEER_MAC, PEER_UUID);
        configure(&mut connection);
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &connection.build_connect_request());
        ws.send(&wire).await.unwrap();
        let accept = decode_sc_message(&bounded(ws.recv()).await.unwrap()).unwrap();
        assert!(connection.handle_connect_accept(&accept));
        let ws = Arc::new(ws);
        self.current_peer = Some(ws.clone());
        Peer { ws, connection }
    }
    async fn capture(&mut self, peer: &mut Peer, request: &Apdu) -> ReceivedNpdu {
        self.capture_from(
            peer,
            request,
            Some(NpduAddress {
                network: 123,
                mac_address: MacAddr::from_slice(&[3]),
            }),
        )
        .await
    }
    async fn capture_from(
        &mut self,
        peer: &mut Peer,
        request: &Apdu,
        source: Option<NpduAddress>,
    ) -> ReceivedNpdu {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, request).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                source,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        let mut wire = BytesMut::new();
        encode_sc_message(
            &mut wire,
            &peer
                .connection
                .build_direct_encapsulated_npdu(&npdu, &[])
                .unwrap(),
        );
        peer.ws.send(&wire).await.unwrap();
        let admitted = bounded(self.admitted.recv()).await.unwrap();
        assert_eq!(admitted.source_mac.as_ref(), PEER_MAC);
        assert!(admitted.provenance.direct_sc_identity().is_some());
        admitted
    }
    async fn feed(&self, admitted: ReceivedNpdu) {
        self.incoming.send(admitted).await.unwrap();
    }
    async fn response(&mut self) -> Apdu {
        let ws = self.current_peer.as_ref().unwrap().clone();
        let wire = bounded(async {
            tokio::select! {
                wire = ws.recv() => wire.unwrap(),
                fallback = self.responses.recv() => panic!("direct response used generic egress: {fallback:?}"),
            }
        }).await;
        let frame = decode_sc_message(&wire).unwrap();
        assert_eq!(
            frame.function,
            bacnet_transport::sc_frame::ScFunction::EncapsulatedNpdu
        );
        assert_eq!(frame.originating_vmac, None);
        assert_eq!(frame.destination_vmac, None);
        let npdu = decode_npdu(frame.payload).unwrap();
        apdu::decode_apdu(npdu.payload).unwrap()
    }
    async fn dispatch_barrier(&mut self, peer: &mut Peer, invoke: u8) {
        assert!(self.barrier_responses(peer, invoke).await.is_empty());
    }
    async fn barrier_responses(&mut self, peer: &mut Peer, invoke: u8) -> Vec<Apdu> {
        let request = confirmed(ConfirmedServiceChoice::from_raw(254), invoke, Bytes::new());
        let envelope = self.capture(peer, &request).await;
        self.feed(envelope).await;
        let mut preceding = Vec::new();
        loop {
            let response = self.response().await;
            if matches!(&response, Apdu::Reject(r) if r.invoke_id == invoke) {
                return preceding;
            }
            preceding.push(response);
        }
    }
    async fn active(&self, expected: usize) {
        bounded(async {
            while self.server.request_tasks.counters().confirmed_active != expected {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    async fn stop(mut self) {
        self.listener.stop().await;
        self.server.stop().await.unwrap();
    }
}
struct Peer {
    ws: Arc<TlsWebSocket>,
    connection: ScConnection,
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("fixture made no progress")
}
fn csv_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap()
}
fn point_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}
fn database(executions: Arc<AtomicUsize>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(1, "value").unwrap(),
    ))
    .unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |_| {
        executions.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            ..Default::default()
        })
    }));
    db.add(Box::new(point)).unwrap();
    db
}
fn confirmed(service: ConfirmedServiceChoice, invoke_id: u8, data: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: data,
    })
}
fn write_payload(text: &str) -> Bytes {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString(text.into())).unwrap();
    let mut payload = BytesMut::new();
    WritePropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut payload)
    .unwrap();
    payload.freeze()
}
fn request(lso: bool, invoke: u8) -> Apdu {
    if !lso {
        return confirmed(
            ConfirmedServiceChoice::WRITE_PROPERTY,
            invoke,
            write_payload("A value"),
        );
    }
    let mut payload = BytesMut::new();
    LifeSafetyOperationRequest {
        requesting_process_identifier: 41,
        requesting_source: "private operator claim".into(),
        request: LifeSafetyOperation::RESET,
        object_identifier: Some(point_oid()),
    }
    .encode(&mut payload)
    .unwrap();
    confirmed(
        ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        invoke,
        payload.freeze(),
    )
}
fn denied(response: &Apdu) -> bool {
    matches!(response, Apdu::Error(e) if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED)
}
