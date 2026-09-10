use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};

use bacnet_transport::sc_hub::ScHub;
use futures_util::{SinkExt, StreamExt};
use rcgen::{CertificateParams, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub type Peer = WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;
pub const HUB_VMAC: [u8; 6] = [0x10; 6];
pub const HUB_UUID: [u8; 16] = [0x10; 16];

pub struct Identity {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}

pub struct Fixture {
    pub ca: CertificateDer<'static>,
    pub server: Identity,
    pub good: Identity,
    pub wrong: Identity,
    pub expired: Identity,
    pub future: Identity,
}

impl Fixture {
    pub fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap().der().clone();
        let issuer = Issuer::from_params(&params, &key);
        let foreign_key = KeyPair::generate().unwrap();
        let mut foreign_params = params.clone();
        foreign_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "foreign CA");
        let foreign = Issuer::from_params(&foreign_params, &foreign_key);
        let leaf = |issuer: &Issuer<'_, &KeyPair>, dates: Option<(i32, i32)>| {
            let mut params = CertificateParams::new(vec!["localhost".into()]).unwrap();
            if let Some((start, end)) = dates {
                params.not_before = rcgen::date_time_ymd(start, 1, 1);
                params.not_after = rcgen::date_time_ymd(end, 1, 1);
            }
            let key = KeyPair::generate().unwrap();
            Identity {
                cert: params.signed_by(&key, issuer).unwrap().der().clone(),
                key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            }
        };
        Self {
            ca,
            server: leaf(&issuer, None),
            good: leaf(&issuer, None),
            wrong: leaf(&foreign, None),
            expired: leaf(&issuer, Some((2000, 2001))),
            future: leaf(&issuer, Some((2099, 2100))),
        }
    }

    pub fn client(
        &self,
        identity: Option<&Identity>,
        version: &'static rustls::SupportedProtocolVersion,
    ) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.ca.clone()).unwrap();
        let builder = rustls::ClientConfig::builder_with_protocol_versions(&[version])
            .with_root_certificates(roots);
        Arc::new(match identity {
            Some(id) => builder
                .with_client_auth_cert(vec![id.cert.clone()], id.key.clone_key())
                .unwrap(),
            None => builder.with_no_client_auth(),
        })
    }
}

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("owned loopback operation exceeded 3 seconds")
}

pub async fn websocket(address: SocketAddr, config: Arc<rustls::ClientConfig>) -> Peer {
    let tcp = bounded(TcpStream::connect(address)).await.unwrap();
    let tls = bounded(
        TlsConnector::from(config).connect(ServerName::try_from("localhost").unwrap(), tcp),
    )
    .await
    .unwrap();
    let request = tokio_tungstenite::tungstenite::client::ClientRequestBuilder::new(
        format!("wss://localhost:{}", address.port())
            .parse()
            .unwrap(),
    )
    .with_sub_protocol("hub.bsc.bacnet.org");
    bounded(tokio_tungstenite::client_async(request, tls))
        .await
        .unwrap()
        .0
}

pub async fn connect(peer: &mut Peer, id: u8, uuid: [u8; 16]) {
    connect_with_hub_identity(peer, id, HUB_VMAC, uuid).await;
}

pub async fn connect_with_hub_identity(peer: &mut Peer, id: u8, vmac: [u8; 6], uuid: [u8; 16]) {
    // Independent AB.2.10/11 wire vector: fixed 26-byte Connect payload.
    let mut request = vec![6, 0, 0x22, id];
    request.extend_from_slice(&[id; 6]);
    request.extend_from_slice(&[id; 16]);
    request.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
    bounded(peer.send(Message::Binary(request.into())))
        .await
        .unwrap();
    let Message::Binary(accepted) = bounded(peer.next()).await.unwrap().unwrap() else {
        panic!("expected SC Connect-Accept");
    };
    assert_eq!(&accepted[..4], &[7, 0, 0x22, id]);
    assert_eq!(&accepted[4..10], &vmac);
    assert_eq!(&accepted[10..26], &uuid);
    assert_eq!(accepted.len(), 30);
}

pub async fn relay(sender: &mut Peer, recipient: &mut Peer, id: u8) {
    // Unicast NPDU: the hub replaces destination VMAC with originating VMAC.
    let mut sent = vec![1, 4, 0x33, id];
    sent.extend_from_slice(&[2; 6]);
    sent.extend_from_slice(&[1, 0, 0x10, 8]);
    bounded(sender.send(Message::Binary(sent.into())))
        .await
        .unwrap();
    let received = bounded(recipient.next()).await.unwrap().unwrap();
    let mut expected = vec![1, 8, 0x33, id];
    expected.extend_from_slice(&[1; 6]);
    expected.extend_from_slice(&[1, 0, 0x10, 8]);
    assert_eq!(received, Message::Binary(expected.into()));
}

// The body is caught by each caller so explicit stop/join also runs on assertion
// failure; Drop is only the last-resort fallback for a broken cleanup itself.
pub async fn finish(mut hub: ScHub, outcome: std::thread::Result<()>) {
    let address = hub.local_addr().unwrap();
    bounded(hub.stop()).await;
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    drop(listener);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
