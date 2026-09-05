use super::heartbeat::HeartbeatIo;
use super::*;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Error};

type ClientWs = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type AckHook = Arc<std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>>;

pub(super) struct LiveClient {
    pub ws: ClientWs,
    pub clients: Clients,
    pub vmac: Vmac,
    pub ack_observed: Arc<Notify>,
    pub after_ack: AckHook,
    pub reader: JoinHandle<()>,
}

impl Drop for LiveClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl LiveClient {
    pub async fn open(clients: Clients, vmac: Vmac) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der());
        let server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key.into())
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
        let request = ClientRequestBuilder::new(url.parse().unwrap())
            .with_sub_protocol(crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL);
        let accept = async {
            let (tcp, addr) = listener.accept().await.unwrap();
            let tls = TlsAcceptor::from(Arc::new(server))
                .accept(tcp)
                .await
                .unwrap();
            // Tungstenite requires the concrete (unboxed) handshake error response.
            #[allow(clippy::result_large_err)]
            let ws = tokio_tungstenite::accept_hdr_async(tls, |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            }).await.unwrap();
            (ws, addr)
        };
        let dial = tokio_tungstenite::connect_async_tls_with_config(
            request,
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(Arc::new(client))),
        );
        let ((server, peer_addr), client) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(accept, dial) })
                .await
                .unwrap();
        let (write, read) = server.split();
        let sink = Arc::new(Mutex::new(write));
        let ack_observed = Arc::new(Notify::new());
        let after_ack: AckHook = Arc::new(std::sync::Mutex::new(None));
        let reader = tokio::spawn({
            let clients = clients.clone();
            let sink = sink.clone();
            let observed = ack_observed.clone();
            let after_ack = after_ack.clone();
            async move {
                handle_client_observed(
                    peer_addr,
                    [0x10; 6],
                    [0x10; 16],
                    read,
                    sink,
                    clients,
                    move || {
                        if let Some(hook) = after_ack.lock().unwrap().take() {
                            hook();
                        }
                        observed.notify_one();
                    },
                )
                .await;
            }
        });
        Self {
            ws: client.unwrap().0,
            clients,
            vmac,
            ack_observed,
            after_ack,
            reader,
        }
    }

    pub async fn connect(clients: Clients, vmac: Vmac) -> Self {
        let mut live = Self::open(clients, vmac).await;
        let mut request = frame(ScFunction::ConnectRequest, 1);
        let mut payload = Vec::from(vmac);
        payload.extend_from_slice(&[vmac[0]; 16]);
        payload.extend_from_slice(&1476u16.to_be_bytes());
        payload.extend_from_slice(&1476u16.to_be_bytes());
        request.payload = Bytes::from(payload);
        live.send(request).await;
        assert_eq!(live.recv().await.function, ScFunction::ConnectAccept);
        live
    }

    pub async fn send(&mut self, message: ScMessage) {
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &message);
        self.ws
            .send(Message::Binary(buf.to_vec().into()))
            .await
            .unwrap();
    }

    pub async fn recv(&mut self) -> ScMessage {
        match tokio::time::timeout(Duration::from_secs(2), self.ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Binary(data) => decode_sc_message(&data).unwrap(),
            other => panic!("expected binary frame, got {other:?}"),
        }
    }

    pub async fn ack(&mut self, id: u16) {
        self.send(frame(ScFunction::HeartbeatAck, id)).await;
        tokio::time::timeout(Duration::from_secs(2), self.ack_observed.notified())
            .await
            .unwrap();
    }

    pub async fn idle(&self) {
        self.clients
            .lock()
            .await
            .get(&self.vmac)
            .unwrap()
            .last_activity
            .store(0, Ordering::Release);
    }

    pub async fn sink(&self) -> Arc<Mutex<WsSink>> {
        self.clients
            .lock()
            .await
            .get(&self.vmac)
            .unwrap()
            .sink
            .clone()
    }

    pub async fn expect_closed(&mut self) {
        let result = tokio::time::timeout(Duration::from_secs(2), self.ws.next())
            .await
            .expect("reader must release socket");
        assert!(
            matches!(result, None | Some(Err(_)) | Some(Ok(Message::Close(_)))),
            "{result:?}"
        );
    }
}

pub(super) fn clients() -> Clients {
    Arc::new(Mutex::new(HashMap::new()))
}

pub(super) fn frame(function: ScFunction, message_id: u16) -> ScMessage {
    ScMessage {
        function,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    }
}

pub(super) struct ClockIo(pub AtomicU64);

impl HeartbeatIo for ClockIo {
    fn now_secs(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
    async fn send(&self, sink: &mut WsSink, frame: Message) -> Result<(), Error> {
        sink.send(frame).await
    }
}

pub(super) struct GatedIo {
    pub now: AtomicU64,
    pub sent: Notify,
    pub release: Notify,
    pub fail: AtomicBool,
}

impl GatedIo {
    pub fn new(now: u64) -> Arc<Self> {
        Arc::new(Self {
            now: AtomicU64::new(now),
            sent: Notify::new(),
            release: Notify::new(),
            fail: AtomicBool::new(false),
        })
    }
}

impl HeartbeatIo for GatedIo {
    fn now_secs(&self) -> u64 {
        self.now.load(Ordering::Acquire)
    }
    async fn send(&self, sink: &mut WsSink, frame: Message) -> Result<(), Error> {
        sink.send(frame).await?;
        self.sent.notify_one();
        self.release.notified().await;
        if self.fail.load(Ordering::Acquire) {
            Err(Error::ConnectionClosed)
        } else {
            Ok(())
        }
    }
}
