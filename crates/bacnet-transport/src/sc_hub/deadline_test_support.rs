use super::*;
use rcgen::{CertificateParams, Issuer, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::time::Duration;

pub(super) type ClientWs = WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

// Keep a paused Tokio clock from auto-advancing while real loopback I/O waits
// for the reactor. The wall-clock bound diagnoses a broken barrier.
pub(super) async fn poll_io<F: std::future::Future>(future: F) -> F::Output {
    tokio::pin!(future);
    let started = std::time::Instant::now();
    loop {
        if let std::task::Poll::Ready(value) = futures_util::poll!(&mut future) {
            return value;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "loopback/barrier did not complete"
        );
        tokio::task::yield_now().await;
    }
}

pub(super) async fn until(predicate: impl Fn() -> bool) {
    poll_io(async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

pub(super) fn request(vmac: Vmac, uuid: DeviceUuid) -> Message {
    let mut wire = crate::sc_frame::connect_test_support::valid_connect(6, vmac);
    wire[10..26].copy_from_slice(&uuid);
    Message::Binary(wire.into())
}

pub(super) struct DeadlinePeer {
    pub ws: ClientWs,
    pub sink: Arc<Mutex<WsSink>>,
    pub deadline: Arc<super::deadlines::ConnectDeadline>,
    pub task: JoinHandle<()>,
    pub active: Arc<std::sync::atomic::AtomicUsize>,
}

impl DeadlinePeer {
    pub async fn new(clients: Clients, duration: Duration) -> Self {
        let (server, ws, address, accepted) = TestTls::new().pair().await;
        let (write, read) = server.split();
        let sink = Arc::new(Mutex::new(write));
        let deadline = Arc::new(super::deadlines::ConnectDeadline::new(accepted + duration));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let admission = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
        let operation = super::deadlines::serve(
            address,
            ([0x10; 6], [0x10; 16]),
            read,
            sink.clone(),
            clients,
            deadline.clone(),
            || {},
        );
        let task = tokio::spawn(async move {
            let _admission = admission;
            operation.await;
        });
        Self {
            ws,
            sink,
            deadline,
            task,
            active,
        }
    }

    pub async fn next(&mut self) -> Message {
        poll_io(self.ws.next()).await.unwrap().unwrap()
    }
}

impl Drop for DeadlinePeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) struct TestTls {
    pub acceptor: TlsAcceptor,
    pub client: Arc<rustls::ClientConfig>,
}

impl TestTls {
    pub async fn pair(
        &self,
    ) -> (
        WebSocketStream<TlsStream>,
        ClientWs,
        SocketAddr,
        tokio::time::Instant,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (tcp, peer) = listener.accept().await.unwrap();
            let tls = self.acceptor.accept(tcp).await.unwrap();
            #[allow(clippy::result_large_err)]
            let ws = tokio_tungstenite::accept_hdr_async(tls, |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            }).await.unwrap();
            (ws, peer, tokio::time::Instant::now())
        };
        let ((server, peer, accepted), client) = tokio::join!(server, self.websocket(address));
        (server, client, peer, accepted)
    }

    pub async fn websocket(
        &self,
        address: SocketAddr,
    ) -> WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
        let tls = self.connect_tls(tcp).await;
        let request = tokio_tungstenite::tungstenite::client::ClientRequestBuilder::new(
            format!("wss://localhost:{}", address.port())
                .parse()
                .unwrap(),
        )
        .with_sub_protocol(crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL);
        tokio_tungstenite::client_async(request, tls)
            .await
            .unwrap()
            .0
    }

    pub async fn connect_tls(
        &self,
        tcp: tokio::net::TcpStream,
    ) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
        tokio_rustls::TlsConnector::from(self.client.clone())
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                tcp,
            )
            .await
            .unwrap()
    }

    pub fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let server_key = KeyPair::generate().unwrap();
        let server_cert = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .signed_by(&server_key, &issuer)
            .unwrap();
        let client_key = KeyPair::generate().unwrap();
        let client_cert = CertificateParams::new(vec!["bacnet-client".into()])
            .unwrap()
            .signed_by(&client_key, &issuer)
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(ca_cert.der().clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots.clone()))
            .build()
            .unwrap();
        let server = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![server_cert.der().clone()],
                PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
            )
            .unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![client_cert.der().clone()],
                PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
            )
            .unwrap();
        Self {
            acceptor: TlsAcceptor::from(Arc::new(server)),
            client: Arc::new(client),
        }
    }
}
