//! Rejected outbound TLS attempts preserve evidence without changing rejection.
#![cfg(feature = "sc-tls")]

use std::{sync::Arc, time::Duration};

use bacnet_transport::sc::{ScConnectError, ScWebSocketErrorKind};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, ScUntrustedPeerInfo, TlsWebSocket};
use bacnet_types::error::Error;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;

struct Ca {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    der: CertificateDer<'static>,
}

impl Ca {
    fn new(name: &str) -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let key = rcgen::KeyPair::generate().unwrap();
        let der = params.self_signed(&key).unwrap().der().clone();
        Self { params, key, der }
    }

    fn leaf(&self, name: &str, expired: bool) -> (CertificateDer<'static>, rcgen::KeyPair) {
        let mut params = rcgen::CertificateParams::new(vec![name.into()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "diagnostic peer");
        if expired {
            params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let key = rcgen::KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        (params.signed_by(&key, &issuer).unwrap().der().clone(), key)
    }

    fn node(&self) -> ScNodeTlsConfig {
        let (der, key) = self.leaf("node", false);
        ScNodeTlsConfig::from_der(
            vec![self.der.clone()],
            vec![der],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    }
}

fn server(chain: Vec<CertificateDer<'static>>, key: &rcgen::KeyPair) -> Arc<rustls::ServerConfig> {
    Arc::new(
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(chain, PrivatePkcs8KeyDer::from(key.serialize_der()).into())
            .unwrap(),
    )
}

async fn reject(config: Arc<rustls::ServerConfig>, node: ScNodeTlsConfig, direct: bool) -> Error {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("wss://localhost:{}", address.port());
        let dial = async {
            if direct {
                TlsWebSocket::connect_direct(&url, node).await
            } else {
                TlsWebSocket::connect(&url, node).await
            }
            .err()
            .expect("normal TLS verification must reject")
        };
        let accept = async {
            let (tcp, _) = listener.accept().await.unwrap();
            assert!(tokio_rustls::TlsAcceptor::from(config)
                .accept(tcp)
                .await
                .is_err());
        };
        let (error, ()) = tokio::join!(dial, accept);
        let sc = ScConnectError::from_error(&error).unwrap();
        assert!(matches!(
            sc,
            ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            }
        ));
        let info = sc.untrusted_peer_info().unwrap();
        assert_eq!(info.peer_address(), address);
        assert_eq!(info.server_name(), "localhost");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err(),
            "inspection must not probe a second connection"
        );
        error
    })
    .await
    .expect("rejected handshake timed out")
}

fn info(error: &Error) -> &ScUntrustedPeerInfo {
    ScConnectError::from_error(error)
        .unwrap()
        .untrusted_peer_info()
        .unwrap()
}

#[tokio::test]
async fn wrong_ca_keeps_exact_leaf_extras_and_malformed_metadata() {
    let ca = Ca::new("untrusted fixture CA");
    let (leaf, key) = ca.leaf("localhost", false);
    let malformed = CertificateDer::from(vec![0xff, 0, 1]);
    let chain = vec![leaf, ca.der.clone(), malformed];
    let node = Ca::new("foreign fixture CA").node();
    let clean = reject(server(chain[..2].to_vec(), &key), node.clone(), false).await;
    assert!(clean.to_string().contains("UnknownIssuer"), "{clean}");
    assert_eq!(info(&clean).certificates()[0].der(), chain[0].as_ref());
    let error = reject(server(chain.clone(), &key), node, false).await;
    // WebPKI sees the malformed extra while trying to build this untrusted
    // chain. Display parsing must preserve that original BadEncoding reason.
    assert!(error.to_string().contains("BadEncoding"), "{error}");
    let diagnostic = info(&error).clone();
    assert_eq!(diagnostic.certificates().len(), chain.len());
    for (got, expected) in diagnostic.certificates().iter().zip(&chain) {
        assert_eq!(got.der(), expected.as_ref());
    }
    assert_eq!(
        diagnostic.certificates()[0].metadata().unwrap().subject(),
        "CN=diagnostic peer"
    );
    assert!(diagnostic.certificates()[2].metadata().is_none());
    let sc = ScConnectError::from_error(&error).unwrap();
    assert_eq!(*sc, sc.clone());
    for display in [
        format!("{sc:?}"),
        error.to_string(),
        format!("{diagnostic:?}"),
    ] {
        assert!(!display.contains("CN=diagnostic peer"));
        assert!(!display.contains("untrusted fixture CA"));
        assert!(!display.contains(&format!("{:?}", chain[0].as_ref())));
    }
    drop(error);
    assert_eq!(diagnostic.certificates()[0].der(), chain[0].as_ref());
}

#[tokio::test]
async fn concurrent_name_and_date_rejections_share_config_without_mixing_chains() {
    let ca = Ca::new("untrusted fixture CA");
    let node = ca.node();
    let (name_der, name_key) = ca.leaf("different.invalid", false);
    let (date_der, date_key) = ca.leaf("localhost", true);
    // Both futures are polled by one task: thread/task-wide mutable slots would mix them.
    let (name, date) = tokio::join!(
        reject(
            server(vec![name_der.clone()], &name_key),
            node.clone(),
            false
        ),
        reject(
            server(vec![date_der.clone()], &date_key),
            node.clone(),
            true
        ),
    );
    assert!(name.to_string().contains("not valid for name"), "{name}");
    assert!(date.to_string().contains("expired"), "{date}");
    assert_eq!(info(&name).certificates()[0].der(), name_der.as_ref());
    assert_eq!(info(&date).certificates()[0].der(), date_der.as_ref());
    assert_eq!(
        info(&date).certificates()[0]
            .metadata()
            .unwrap()
            .not_after_unix(),
        978_307_200
    );
    // A later pre-certificate failure must not inherit either rejected chain.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let accept = async {
        let (tcp, _) = listener.accept().await.unwrap();
        drop(tcp);
    };
    let (error, ()) = tokio::join!(TlsWebSocket::connect(&url, node), accept);
    let error = error.err().expect("closed socket must fail");
    let sc = ScConnectError::from_error(&error).unwrap();
    assert!(matches!(
        sc,
        ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::TlsHandshake,
            ..
        }
    ));
    assert!(sc.untrusted_peer_info().is_none());
}

#[derive(Debug)]
struct DeliberatelyInvalidIdentity(Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for DeliberatelyInvalidIdentity {
    fn resolve(
        &self,
        _: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

fn invalid_server(
    chain: Vec<CertificateDer<'static>>,
    key: rcgen::KeyPair,
) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let signing_key = provider
        .key_provider
        .load_private_key(PrivatePkcs8KeyDer::from(key.serialize_der()).into())
        .unwrap();
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(DeliberatelyInvalidIdentity(Arc::new(
                rustls::sign::CertifiedKey::new(chain, signing_key),
            )))),
    )
}

#[tokio::test]
async fn malformed_leaf_and_signature_rejections_preserve_original_failure() {
    let ca = Ca::new("untrusted fixture CA");
    let node = ca.node();
    let malformed = CertificateDer::from(vec![0xff, 0, 1]);
    let error = reject(
        invalid_server(vec![malformed.clone()], rcgen::KeyPair::generate().unwrap()),
        node.clone(),
        false,
    )
    .await;
    assert!(error.to_string().contains("BadEncoding"), "{error}");
    assert_eq!(info(&error).certificates()[0].der(), malformed.as_ref());
    assert!(info(&error).certificates()[0].metadata().is_none());

    // The certificate verifier accepts this trusted leaf; CertificateVerify
    // is signed by a different key and must still be rejected afterwards.
    let (leaf, _) = ca.leaf("localhost", false);
    let error = reject(
        invalid_server(vec![leaf.clone()], rcgen::KeyPair::generate().unwrap()),
        node,
        true,
    )
    .await;
    assert!(error.to_string().contains("BadSignature"), "{error}");
    assert_eq!(info(&error).certificates()[0].der(), leaf.as_ref());
}
