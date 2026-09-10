use super::{bounded, command, Files};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::{
    sc::ScTransport,
    sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig},
    sc_tls::{ScNodeTlsConfig, TlsWebSocket},
};
use futures_util::FutureExt;
use rcgen::{Certificate, CertificateParams, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::panic::AssertUnwindSafe;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_rustls::TlsAcceptor;

static IDENTITY: AtomicU64 = AtomicU64::new(32);

pub struct Leaf {
    pub cert: Certificate,
    pub key: KeyPair,
}

pub struct Site {
    pub ca: Certificate,
    params: CertificateParams,
    key: KeyPair,
}

impl Site {
    pub fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            format!(
                "ephemeral site {}",
                IDENTITY.fetch_add(1, Ordering::Relaxed)
            ),
        );
        let key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap();
        Self { ca, params, key }
    }

    pub fn leaf(
        &self,
        name: &str,
        usage: ExtendedKeyUsagePurpose,
        dates: Option<(i32, i32)>,
    ) -> Leaf {
        let mut params = CertificateParams::new(vec![name.into()]).unwrap();
        params.extended_key_usages = vec![usage];
        if let Some((start, end)) = dates {
            params.not_before = rcgen::date_time_ymd(start, 1, 1);
            params.not_after = rcgen::date_time_ymd(end, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let cert = params
            .signed_by(&key, &Issuer::from_params(&self.params, &self.key))
            .unwrap();
        Leaf { cert, key }
    }

    pub fn roots(&self) -> rustls::RootCertStore {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.ca.der().clone()).unwrap();
        roots
    }

    pub fn client(&self, leaf: &Leaf) -> ScNodeTlsConfig {
        ScNodeTlsConfig::from_der(
            vec![self.ca.der().clone()],
            vec![leaf.cert.der().clone()],
            PrivatePkcs8KeyDer::from(leaf.key.serialize_der()).into(),
        )
        .unwrap()
    }

    pub fn hub_tls_config(&self) -> ScHubTlsConfig {
        let hub = self.leaf("127.0.0.1", ExtendedKeyUsagePurpose::ServerAuth, None);
        ScHubTlsConfig::from_der(
            vec![self.ca.der().clone()],
            vec![hub.cert.der().clone()],
            PrivatePkcs8KeyDer::from(hub.key.serialize_der()).into(),
        )
        .unwrap()
    }

    // Independent raw TLS peer, including the TLS 1.2 negative oracle.
    pub fn acceptor(&self, version: &'static rustls::SupportedProtocolVersion) -> TlsAcceptor {
        let hub = self.leaf("127.0.0.1", ExtendedKeyUsagePurpose::ServerAuth, None);
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(self.roots()))
            .build()
            .unwrap();
        let config = rustls::ServerConfig::builder_with_protocol_versions(&[version])
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![hub.cert.der().clone()],
                PrivatePkcs8KeyDer::from(hub.key.serialize_der()).into(),
            )
            .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }
}

pub fn cli(files: &Files, url: &str, leaf: &Leaf) -> Command {
    let mut cmd = command();
    let identity = IDENTITY.fetch_add(1, Ordering::Relaxed);
    cmd.args([
        "--sc",
        "--json",
        "--timeout",
        "1000",
        "--sc-url",
        url,
        "--sc-cert",
    ])
    .arg(files.write("operational é.pem", leaf.cert.pem()))
    .arg("--sc-key")
    .arg(files.write("operational é.key", leaf.key.serialize_pem()))
    .arg("--sc-vmac")
    .arg(format!("02{identity:010x}"))
    .arg("--sc-device-uuid")
    .arg(format!("{identity:032x}"));
    cmd
}

pub fn read(cmd: &mut Command) -> &mut Command {
    // Existing CLI target parser encodes IP:port into six MAC bytes. On SC
    // these are the fixture server's VMAC, NOT an IP destination to dial.
    // Colon-hex VMAC target syntax is not currently accepted by that parser.
    cmd.args(["read", "2.0.0.0:1", "dev:123", "object-name"])
}

#[derive(Default)]
pub struct Fixture {
    pub hub: Option<ScHub>,
    pub server: Option<BACnetServer<ScTransport<TlsWebSocket>>>,
    pub url: String,
}

impl Fixture {
    pub async fn start(&mut self, site: &Site) {
        self.hub = Some(
            bounded(ScHub::start_with_tls_config(
                "127.0.0.1:0",
                site.hub_tls_config(),
                [2, 0, 0, 0, 0, 9],
                [9; 16],
                ScHubHandshakeTimeouts::default(),
            ))
            .await
            .unwrap(),
        );
        self.url = format!("wss://{}", self.hub.as_ref().unwrap().local_addr().unwrap());
        let leaf = site.leaf("server", ExtendedKeyUsagePurpose::ClientAuth, None);
        let ws = bounded(TlsWebSocket::connect(&self.url, site.client(&leaf)))
            .await
            .unwrap();
        let transport = ScTransport::new(ws, [2, 0, 0, 0, 0, 1]).with_device_uuid([1; 16]);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 123,
                name: "CLI site device".into(),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        // Build awaits SC ConnectAccept; subsequent CLI ReadProperty is the
        // application readiness barrier. Distinct hub/server/CLI VMACs AND UUIDs.
        self.server = Some(
            bounded(
                BACnetServer::generic_builder()
                    .transport(transport)
                    .database(db)
                    .build(),
            )
            .await
            .unwrap(),
        );
    }

    async fn stop(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(server) = &mut self.server {
            let result = tokio::time::timeout(Duration::from_secs(5), server.stop()).await;
            if !matches!(result, Ok(Ok(()))) {
                errors.push(format!("server stop: {result:?}"));
            }
        }
        // Server stop joins its producers/requests; dropping it also releases
        // its private NetworkLayer/transport before shutting down the hub.
        self.server = None;
        if let Some(hub) = &mut self.hub {
            if let Err(error) = tokio::time::timeout(Duration::from_secs(5), hub.stop()).await {
                errors.push(format!("hub stop: {error}"));
            }
            // A closed connection can leave the port in TIME_WAIT on macOS,
            // making immediate rebind fail without any live listener. Require
            // an actual connection refusal after the joined stop, not a delay
            // or timeout interpreted as cleanup success.
            let probe = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::TcpStream::connect(hub.local_addr().unwrap()),
            )
            .await;
            if !matches!(&probe, Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused)
            {
                errors.push(format!("hub listener still reachable: {probe:?}"));
            }
        }
        errors
    }
}

pub async fn with_fixture(test: impl AsyncFnOnce(&mut Fixture)) {
    let mut fixture = Fixture::default();
    let result = AssertUnwindSafe(bounded(test(&mut fixture)))
        .catch_unwind()
        .await;
    let errors = fixture.stop().await;
    assert!(
        errors.is_empty(),
        "joined endpoint cleanup failed: {errors:?}"
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
