use crate::server::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::EnableDisable;
use futures_util::FutureExt;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use tokio_rustls::rustls::{self, pki_types::PrivatePkcs8KeyDer};

pub(super) const SERVER: [u8; 6] = [2, 0, 0, 0, 0, 1];
pub(super) const PEERS: [[u8; 6]; 2] = [[2, 0, 0, 0, 0, 2], [2, 0, 0, 0, 0, 3]];
pub(super) const PASSWORD: &str = "test-only-dcc";
type Transport = ScTransport<TlsWebSocket>;
type Server = BACnetServer<Transport>;

pub(super) async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("loopback operation exceeded its deadline")
}

// All keys stay in memory. Each endpoint has its own key/certificate, including
// the BACnet server (a TLS client of the hub). No permissive verifier is used.
pub(super) struct Certificates {
    pub hub: ScHubTlsConfig,
    pub clients: Vec<ScNodeTlsConfig>,
    pub missing: Arc<rustls::ClientConfig>,
    pub untrusted: Arc<rustls::ClientConfig>,
    pub wrong_server_trust: Arc<rustls::ClientConfig>,
}

impl Certificates {
    pub fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&params, &ca_key);
        let mut roots = rustls::RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        let leaf = |name: &str, usage| {
            let mut params = CertificateParams::new(vec![name.into()]).unwrap();
            params.extended_key_usages = vec![usage];
            let key = KeyPair::generate().unwrap();
            let cert = params.signed_by(&key, &issuer).unwrap();
            (
                cert.der().clone(),
                PrivatePkcs8KeyDer::from(key.serialize_der()),
            )
        };
        let (hub_cert, hub_key) = leaf("127.0.0.1", ExtendedKeyUsagePurpose::ServerAuth);
        let hub = ScHubTlsConfig::from_der(
            vec![ca.der().clone()],
            vec![hub_cert.clone()],
            hub_key.into(),
        )
        .unwrap();
        let mut certs = vec![hub_cert];
        let clients = (0..3)
            .map(|index| {
                let (cert, key) = leaf(
                    &format!("peer-{index}"),
                    ExtendedKeyUsagePurpose::ClientAuth,
                );
                assert!(!certs.contains(&cert), "endpoint certificates must differ");
                certs.push(cert.clone());
                ScNodeTlsConfig::from_der(vec![ca.der().clone()], vec![cert], key.into()).unwrap()
            })
            .collect();
        let missing =
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_root_certificates(roots.clone())
                .with_no_client_auth();
        let rogue_key = KeyPair::generate().unwrap();
        let mut rogue_params = CertificateParams::new(vec!["untrusted-peer".into()]).unwrap();
        rogue_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let rogue = rogue_params.self_signed(&rogue_key).unwrap();
        let untrusted =
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_root_certificates(roots)
                .with_client_auth_cert(
                    vec![rogue.der().clone()],
                    PrivatePkcs8KeyDer::from(rogue_key.serialize_der()).into(),
                )
                .unwrap();
        let (cert, key) = leaf("wrong-server-trust", ExtendedKeyUsagePurpose::ClientAuth);
        let wrong_server_trust =
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_client_auth_cert(vec![cert], key.into())
                .unwrap();
        Self {
            hub,
            clients,
            missing: Arc::new(missing),
            untrusted: Arc::new(untrusted),
            wrong_server_trust: Arc::new(wrong_server_trust),
        }
    }
}

pub(super) struct Peer {
    transport: Transport,
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    next_invoke: u8,
}

#[derive(Default)]
pub(super) struct Fixture {
    hub: Option<ScHub>,
    pub server: Option<Server>,
    peers: Vec<Peer>,
    pub url: String,
}

impl Fixture {
    pub fn server(&self) -> &Server {
        self.server.as_ref().unwrap()
    }

    pub async fn hub(&mut self, certs: &Certificates) {
        let hub = bounded(ScHub::start_with_tls_config(
            "127.0.0.1:0",
            certs.hub.clone(),
            [2, 0, 0, 0, 0, 9],
            [0; 16],
            ScHubHandshakeTimeouts::default(),
        ))
        .await
        .unwrap();
        self.url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
        self.hub = Some(hub);
    }

    pub async fn start(&mut self, certs: &Certificates, builder: sc_builder::ScServerBuilder) {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 123,
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        self.server = Some(
            bounded(
                builder
                    .hub_url(&self.url)
                    .tls_config(certs.clients[0].clone())
                    .vmac(SERVER)
                    .device_uuid(sc_builder::TEST_DEVICE_UUID)
                    .database(db)
                    .build(),
            )
            .await
            .unwrap(),
        );
        for (index, vmac) in PEERS.into_iter().enumerate() {
            let ws = bounded(TlsWebSocket::connect(
                &self.url,
                certs.clients[index + 1].clone(),
            ))
            .await
            .unwrap();
            // Register ownership before the cancellable SC handshake.
            self.peers.push(Peer {
                transport: ScTransport::new(ws, vmac).with_device_uuid([index as u8 + 1; 16]),
                receiver: None,
                next_invoke: 1,
            });
            let peer = self.peers.last_mut().unwrap();
            peer.receiver = Some(bounded(peer.transport.start()).await.unwrap());
            // start() awaits ConnectAccept; ReadProperty proves end-to-end service
            // dispatch, not merely TLS admission or a timeout interpreted as denial.
            self.read_property(index).await;
        }
    }

    pub async fn exchange(
        &mut self,
        peer: usize,
        service: ConfirmedServiceChoice,
        data: Bytes,
        source: Option<NpduAddress>,
    ) -> (u8, Apdu) {
        let peer = &mut self.peers[peer];
        let invoke = peer.next_invoke;
        peer.next_invoke = invoke.checked_add(1).unwrap();
        let mut payload = BytesMut::new();
        encode_apdu(
            &mut payload,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 480,
                invoke_id: invoke,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: service,
                service_request: data,
            }),
        )
        .unwrap();
        let mut wire = BytesMut::new();
        encode_npdu(
            &mut wire,
            &Npdu {
                expecting_reply: true,
                source: source.clone(),
                payload: payload.freeze(),
                ..Default::default()
            },
        )
        .unwrap();
        bounded(peer.transport.send_unicast(&wire, &SERVER))
            .await
            .unwrap();
        let response = bounded(async {
            loop {
                let incoming = peer
                    .receiver
                    .as_mut()
                    .unwrap()
                    .recv()
                    .await
                    .expect("SC receive closed");
                let npdu = decode_npdu(incoming.npdu).unwrap();
                let apdu = decode_apdu(npdu.payload).unwrap();
                if matches!(apdu, Apdu::UnconfirmedRequest(_)) {
                    continue;
                }
                assert_eq!(incoming.source_mac.as_slice(), SERVER);
                assert!(!incoming.link_layer_group);
                assert_eq!(
                    npdu.destination, source,
                    "routed reply must retain final destination"
                );
                return apdu;
            }
        })
        .await;
        (invoke, response)
    }

    pub async fn read_property(&mut self, peer: usize) {
        let object_identifier = ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap();
        let mut data = BytesMut::new();
        ReadPropertyRequest {
            object_identifier,
            property_identifier: PropertyIdentifier::OBJECT_IDENTIFIER,
            property_array_index: None,
        }
        .encode(&mut data);
        let (id, response) = self
            .exchange(
                peer,
                ConfirmedServiceChoice::READ_PROPERTY,
                data.freeze(),
                None,
            )
            .await;
        let Apdu::ComplexAck(ack) = response else {
            panic!("expected ReadProperty ACK, got {response:?}")
        };
        assert_eq!(ack.invoke_id, id);
        assert_eq!(ack.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
        let value = ReadPropertyACK::decode(&ack.service_ack).unwrap();
        assert_eq!(value.object_identifier, object_identifier);
        assert_eq!(
            value.property_identifier,
            PropertyIdentifier::OBJECT_IDENTIFIER
        );
        assert_eq!(value.property_array_index, None);
        // Independent application ObjectIdentifier tag + device(8), instance123.
        assert_eq!(value.property_value, [0xc4, 0x02, 0x00, 0x00, 0x7b]);
    }

    pub async fn dcc(
        &mut self,
        peer: usize,
        mode: EnableDisable,
        duration: Option<u16>,
        password: Option<&str>,
        source: Option<NpduAddress>,
        expected: Outcome,
    ) {
        let mut data = BytesMut::new();
        DeviceCommunicationControlRequest {
            time_duration: duration,
            enable_disable: mode,
            password: password.map(str::to_owned),
        }
        .encode(&mut data)
        .unwrap();
        let before = self.server().dcc_outcome_counters();
        let (id, response) = self
            .exchange(
                peer,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                data.freeze(),
                source,
            )
            .await;
        let mut after = before;
        let error = match expected {
            Outcome::Accepted => {
                after.accepted_total += 1;
                None
            }
            Outcome::Policy => {
                after.policy_denied_total += 1;
                Some((ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED))
            }
            Outcome::Password => {
                after.password_failure_total += 1;
                Some((ErrorClass::SECURITY, ErrorCode::PASSWORD_FAILURE))
            }
            Outcome::Deprecated => {
                after.deprecated_denied_total += 1;
                Some((ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED))
            }
        };
        if let Some((class, code)) = error {
            let Apdu::Error(pdu) = response else {
                panic!("expected DCC Error, got {response:?}")
            };
            assert_eq!(pdu.invoke_id, id);
            assert_eq!(
                pdu.service_choice,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
            );
            assert_eq!((pdu.error_class, pdu.error_code), (class, code));
        } else {
            let Apdu::SimpleAck(pdu) = response else {
                panic!("expected DCC SimpleACK, got {response:?}")
            };
            assert_eq!(pdu.invoke_id, id);
            assert_eq!(
                pdu.service_choice,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
            );
        }
        assert_eq!(self.server().dcc_outcome_counters(), after);
    }

    async fn stop(&mut self) -> bool {
        // Attempt every owner's joined stop even when another cleanup fails.
        let mut clean = true;
        for peer in &mut self.peers {
            clean &= matches!(
                tokio::time::timeout(Duration::from_secs(5), peer.transport.stop()).await,
                Ok(Ok(()))
            );
        }
        if let Some(server) = &mut self.server {
            clean &= matches!(
                tokio::time::timeout(Duration::from_secs(5), server.stop()).await,
                Ok(Ok(()))
            );
        }
        if let Some(hub) = &mut self.hub {
            clean &= tokio::time::timeout(Duration::from_secs(5), hub.stop())
                .await
                .is_ok();
            // Successful stop releases the listener before returning.
            clean &= tokio::net::TcpListener::bind(hub.local_addr().unwrap())
                .await
                .is_ok();
        }
        clean
    }
}

pub(super) enum Outcome {
    Accepted,
    Policy,
    Password,
    Deprecated,
}

pub(super) async fn run(test: impl AsyncFnOnce(&mut Fixture)) {
    let mut fixture = Fixture::default();
    let result = AssertUnwindSafe(test(&mut fixture)).catch_unwind().await;
    let clean = fixture.stop().await;
    assert!(clean, "SC fixture failed joined cleanup");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
