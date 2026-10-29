//! Real TLS admission; only scheduling between listener and network is synthetic.
use bacnet_encoding::{
    apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu},
    npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu},
};
use bacnet_objects::{database::ObjectDatabase, value_types::CharacterStringValueObject};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::{
    port::{ReceivedNpdu, TransportPort},
    sc::{ScConnection, WebSocketPort},
    sc_frame::{decode_sc_message, encode_sc_message},
    sc_tls::{DirectAcceptConfig, DirectListener, ScNodeTlsConfig, TlsWebSocket},
};
use bacnet_types::{
    enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier},
    error::Error,
    primitives::ObjectIdentifier,
};
use bytes::{Bytes, BytesMut};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub const PEER_MAC: [u8; 6] = [0x22; 6];
pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("fixture made no progress")
}
pub struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    der: CertificateDer<'static>,
}
impl TestCa {
    pub fn new() -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let der = params.self_signed(&key).unwrap().der().clone();
        Self { params, key, der }
    }
    pub fn server_tls(&self) -> Arc<rustls::ServerConfig> {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .signed_by(&key, &rcgen::Issuer::from_params(&self.params, &self.key))
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.der.clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .unwrap();
        Arc::new(
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_client_cert_verifier(verifier)
                .with_single_cert(
                    vec![cert.der().clone()],
                    PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap(),
        )
    }
    pub fn tls(&self, name: &str) -> ScNodeTlsConfig {
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
pub struct QueuedPort {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    generic: mpsc::UnboundedSender<Npdu>,
    gate: Option<Arc<tokio::sync::Semaphore>>,
    attributes: Arc<std::sync::Mutex<Vec<Vec<bacnet_transport::port::DataAttribute>>>>,
}
impl TransportPort for QueuedPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.incoming.take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, bytes: &[u8], _: &[u8]) -> Result<(), Error> {
        self.generic
            .send(decode_npdu(Bytes::copy_from_slice(bytes)).unwrap())
            .unwrap();
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        Ok(())
    }
    async fn send_broadcast(&self, bytes: &[u8]) -> Result<(), Error> {
        self.send_unicast(bytes, &[]).await
    }
    async fn send_unicast_with_data_attributes(
        &self,
        bytes: &[u8],
        mac: &[u8],
        attributes: &[bacnet_transport::port::DataAttribute],
    ) -> Result<(), Error> {
        self.attributes.lock().unwrap().push(attributes.to_vec());
        self.send_unicast(bytes, mac).await
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[0xaa; 6]
    }
}
pub struct Fixture {
    pub listener: DirectListener,
    admitted: mpsc::Receiver<ReceivedNpdu>,
    pub incoming: mpsc::Sender<ReceivedNpdu>,
    pub generic: mpsc::UnboundedReceiver<Npdu>,
    pub attributes: Arc<std::sync::Mutex<Vec<Vec<bacnet_transport::port::DataAttribute>>>>,
}
impl Fixture {
    pub async fn new(ca: &TestCa) -> (Self, QueuedPort) {
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
        let (generic, observed) = mpsc::unbounded_channel();
        let attributes = Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Self {
                listener,
                admitted,
                incoming,
                generic: observed,
                attributes: attributes.clone(),
            },
            QueuedPort {
                incoming: Some(rx),
                generic,
                gate: None,
                attributes,
            },
        )
    }
    pub async fn peer(&self, tls: ScNodeTlsConfig, limits: Option<(u16, u16)>) -> Peer {
        let url = format!(
            "wss://localhost:{}/.bacnet/sc",
            self.listener.local_addr().port()
        );
        let ws = bounded(TlsWebSocket::connect_direct(&url, tls))
            .await
            .unwrap();
        let mut connection = ScConnection::new(PEER_MAC, [7; 16]);
        if let Some((npdu, bvlc)) = limits {
            connection.max_apdu_length = npdu;
            connection.max_bvlc_length = bvlc;
        }
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &connection.build_connect_request());
        ws.send(&wire).await.unwrap();
        let accept = decode_sc_message(&bounded(ws.recv()).await.unwrap()).unwrap();
        assert!(connection.handle_connect_accept(&accept));
        Peer {
            ws: Arc::new(ws),
            connection,
        }
    }
    pub async fn capture(
        &mut self,
        peer: &mut Peer,
        apdu: &Apdu,
        source: Option<NpduAddress>,
    ) -> ReceivedNpdu {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, apdu).unwrap();
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
        assert!(admitted.provenance.direct_sc_identity().is_some());
        assert!(admitted.direct_response.is_some());
        admitted
    }
    pub async fn feed(&self, admitted: ReceivedNpdu) {
        self.incoming.send(admitted).await.unwrap();
    }
    pub async fn response(&mut self, peer: &Peer) -> (Npdu, Apdu) {
        let wire = bounded(async { tokio::select! {
            wire = peer.ws.recv() => wire.unwrap(),
            generic = self.generic.recv() => panic!("direct reply used ordinary egress: {generic:?}"),
        }}).await;
        let frame = decode_sc_message(&wire).unwrap();
        assert_eq!(
            frame.function,
            bacnet_transport::sc_frame::ScFunction::EncapsulatedNpdu
        );
        assert_eq!(frame.originating_vmac, None);
        assert_eq!(frame.destination_vmac, None);
        assert!(frame.data_options.is_empty());
        let npdu = decode_npdu(frame.payload).unwrap();
        let apdu = decode_apdu(npdu.payload.clone()).unwrap();
        (npdu, apdu)
    }
}
pub struct Peer {
    pub ws: Arc<TlsWebSocket>,
    connection: ScConnection,
}
pub fn confirmed(service: ConfirmedServiceChoice, invoke_id: u8, data: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(2),
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: data,
    })
}
pub fn unsupported(invoke: u8) -> Apdu {
    confirmed(ConfirmedServiceChoice::from_raw(254), invoke, Bytes::new())
}
pub fn csv_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap()
}
pub fn database(name: &str) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(CharacterStringValueObject::new(1, name).unwrap()))
        .unwrap();
    db
}
pub fn read_name(invoke: u8) -> Apdu {
    let mut payload = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut payload);
    confirmed(
        ConfirmedServiceChoice::READ_PROPERTY,
        invoke,
        payload.freeze(),
    )
}

impl Fixture {
    /// Both dispatchers finish prior inbound work before this response. Any stale
    /// reply on B or generic egress is observed before the barrier can succeed.
    pub async fn barrier(&mut self, peer: &mut Peer, invoke: u8) {
        let envelope = self.capture(peer, &unsupported(invoke), None).await;
        self.feed(envelope).await;
        let reply = self.response(peer).await.1;
        assert!(
            matches!(&reply, Apdu::Reject(r) if r.invoke_id == invoke),
            "unexpected reply before barrier {invoke}: {reply:?}"
        );
        assert!(self.generic.try_recv().is_err());
    }
}
pub enum Consumer {
    Client(Box<bacnet_client::client::BACnetClient<QueuedPort>>),
    Endpoint(Box<bacnet_endpoint::session::EndpointSession<QueuedPort>>),
}
impl Consumer {
    pub async fn start(client: bool, port: QueuedPort) -> Self {
        if client {
            Self::Client(Box::new(
                bacnet_client::client::BACnetClient::start(Default::default(), port)
                    .await
                    .unwrap(),
            ))
        } else {
            let mut session = bacnet_endpoint::session::EndpointSession::new(
                port,
                bacnet_endpoint::session::SessionRole::Both,
                Default::default(),
            )
            .unwrap()
            .with_database(database("original"));
            session.start().await.unwrap();
            Self::Endpoint(Box::new(session))
        }
    }
    pub async fn stop(&mut self) {
        match self {
            Self::Client(client) => client.stop().await.unwrap(),
            Self::Endpoint(session) => {
                session.stop().await.unwrap();
            }
        }
    }
}
pub fn routed() -> Option<NpduAddress> {
    Some(NpduAddress {
        network: 123,
        mac_address: bacnet_types::MacAddr::from_slice(&[3, 4, 5, 6, 7, 8]),
    })
}

impl QueuedPort {
    pub fn hold_sends(&mut self) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        self.gate = Some(gate.clone());
        gate
    }
}
pub fn received_apdu(envelope: ReceivedNpdu) -> bacnet_network::layer::ReceivedApdu {
    let npdu = decode_npdu(envelope.npdu).unwrap();
    bacnet_network::layer::ReceivedApdu {
        apdu: npdu.payload,
        source_mac: envelope.source_mac,
        source_network: npdu.source,
        ingress_network: None,
        link_layer_group: envelope.link_layer_group,
        is_group: envelope.link_layer_group,
        global_broadcast: false,
        data_attributes: envelope.data_attributes,
        provenance: envelope.provenance,
        router_response: None,
        direct_response: envelope.direct_response,
        reply_tx: envelope.reply_tx,
    }
}
