//! Real built-in TLS builder/redial boundaries; no test-only production bypass.
use bacnet_benchmarks::sc_helpers::*;
use bacnet_client::client::BACnetClient;
use bacnet_objects::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_server::server::BACnetServer;
use bacnet_transport::{
    sc::{ScReconnectConfig, ScTransport},
    sc_hub::ScHub,
    sc_tls::TlsWebSocket,
};
use bacnet_types::{
    enums::{ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use futures_util::FutureExt;
use std::{future::Future, net::SocketAddr, panic::AssertUnwindSafe, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};

type Client = BACnetClient<ScTransport<TlsWebSocket>>;
type Server = BACnetServer<ScTransport<TlsWebSocket>>;
const SERVER: [u8; 6] = [2, 0, 0, 0, 0, 1];
const SERVER_UUID: [u8; 16] = [
    0x8e, 0x62, 0xac, 0x46, 0xd7, 0x08, 0x42, 0x26, 0x91, 0x37, 0x76, 0xa3, 0x2b, 0x61, 0x93, 0x15,
];
const CLIENT_UUID: [u8; 16] = [
    0x95, 0xdf, 0xe4, 0xef, 0x97, 0xf6, 0x49, 0x0d, 0x9a, 0x2c, 0xf2, 0xb4, 0xb0, 0xc0, 0xe6, 0x82,
];

async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .expect("node TLS barrier timed out")
}

fn reconnect() -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 10,
        max_delay_ms: 10,
        max_retries: 1,
    }
}

// An opaque TCP forwarder cuts only the selected node's socket. It never
// terminates TLS, chooses policy, rewrites SC, or performs application reads.
struct Proxy {
    url: String,
    cut: mpsc::Sender<()>,
    accepted: mpsc::Receiver<usize>,
    task: JoinHandle<()>,
}

impl Proxy {
    async fn new(targets: Vec<SocketAddr>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
        let (cut, mut cuts) = mpsc::channel(1);
        let (events, accepted) = mpsc::channel(3);
        let task = tokio::spawn(async move {
            for (round, target) in targets.into_iter().enumerate() {
                let (mut node, _) = listener.accept().await.unwrap();
                let mut hub = TcpStream::connect(target).await.unwrap();
                events.send(round).await.unwrap();
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut node, &mut hub) => {},
                    _ = cuts.recv() => {},
                }
                // Both sockets drop before accepting the fresh connection.
            }
        });
        Self {
            url,
            cut,
            accepted,
            task,
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct Fixture {
    hubs: Vec<ScHub>,
    client: Option<Client>,
    server: Option<Server>,
    proxy: Option<Proxy>,
    bad_server: Option<JoinHandle<std::io::Error>>,
}

impl Fixture {
    async fn hub(&mut self, certs: &CertMaterial, id: u8) -> String {
        let hub = bounded(ScHub::start_with_uuid(
            "127.0.0.1:0",
            try_make_hub_tls_config(certs).unwrap(),
            [id; 6],
            [id; 16],
        ))
        .await
        .unwrap();
        let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
        self.hubs.push(hub);
        url
    }

    async fn server(&mut self, url: &str, certs: &CertMaterial) {
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
                BACnetServer::sc_builder()
                    .hub_url(url)
                    .tls_config(try_make_node_tls_config(certs).unwrap())
                    .vmac(SERVER)
                    .device_uuid(SERVER_UUID)
                    .database(db)
                    .reconnect(reconnect())
                    .build(),
            )
            .await
            .unwrap(),
        );
    }

    async fn client(&mut self, url: &str, certs: &CertMaterial) {
        self.client = Some(
            bounded(
                BACnetClient::sc_builder()
                    .hub_url(url)
                    .tls_config(try_make_node_tls_config(certs).unwrap())
                    .vmac([2; 6])
                    .device_uuid(CLIENT_UUID)
                    .apdu_timeout_ms(100)
                    .apdu_retries(0)
                    .reconnect(reconnect())
                    .build(),
            )
            .await
            .unwrap(),
        );
    }

    async fn read(&self) -> Result<(), bacnet_types::error::Error> {
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap();
        let ack = self
            .client
            .as_ref()
            .unwrap()
            .read_property(&SERVER, oid, PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .await?;
        assert_eq!(ack.object_identifier, oid);
        assert_eq!(ack.property_value, vec![0xc4, 0x02, 0, 0, 123]);
        Ok(())
    }

    async fn reads_after_reconnect(&self) {
        bounded(async {
            loop {
                if self.read().await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    async fn stop(&mut self) {
        if let Some(client) = &mut self.client {
            bounded(client.stop()).await.unwrap();
        }
        self.client = None;
        if let Some(server) = &mut self.server {
            bounded(server.stop()).await.unwrap();
        }
        self.server = None;
        if let Some(mut proxy) = self.proxy.take() {
            proxy.task.abort();
            let result = bounded(&mut proxy.task).await;
            assert!(result.is_ok() || result.unwrap_err().is_cancelled());
        }
        if let Some(task) = self.bad_server.take() {
            task.abort();
            let result = bounded(task).await;
            assert!(result.is_ok() || result.unwrap_err().is_cancelled());
        }
        for hub in &mut self.hubs {
            bounded(hub.stop()).await;
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.bad_server {
            task.abort();
        }
    }
}

async fn run(test: impl AsyncFnOnce(&mut Fixture)) {
    let mut f = Fixture::default();
    let result = AssertUnwindSafe(test(&mut f)).catch_unwind().await;
    f.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn builder_redial(f: &mut Fixture, client_subject: bool) {
    let certs = generate_test_certs();
    let url = f.hub(&certs, 9).await;
    let good = f.hubs[0].local_addr().unwrap();
    let wrong = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad = listener.local_addr().unwrap();
    f.bad_server = Some(tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tokio_rustls::TlsAcceptor::from(make_server_tls_config_mtls(&wrong))
            .accept(tcp)
            .await
            .expect_err("node must reject the unrelated server CA")
    }));
    f.proxy = Some(Proxy::new(vec![good, good, bad]).await);
    let proxy_url = f.proxy.as_ref().unwrap().url.clone();
    // Independently provisioned node UUIDs survive each builder's redial.
    f.server(if client_subject { &url } else { &proxy_url }, &certs)
        .await;
    f.client(if client_subject { &proxy_url } else { &url }, &certs)
        .await;
    assert_eq!(
        bounded(f.proxy.as_mut().unwrap().accepted.recv()).await,
        Some(0)
    );
    bounded(f.read()).await.unwrap();
    f.proxy.as_ref().unwrap().cut.send(()).await.unwrap();
    assert_eq!(
        bounded(f.proxy.as_mut().unwrap().accepted.recv()).await,
        Some(1)
    );
    f.reads_after_reconnect().await; // fresh TLS + SC Connect + application read
    f.proxy.as_ref().unwrap().cut.send(()).await.unwrap();
    assert_eq!(
        bounded(f.proxy.as_mut().unwrap().accepted.recv()).await,
        Some(2)
    );
    let error = bounded(f.bad_server.as_mut().unwrap()).await.unwrap();
    f.bad_server = None;
    // Generated CAs share a subject, not a signing key: BadSignature/DecryptError.
    assert!(format!("{error:?}").contains("DecryptError"), "{error:?}");
    assert!(
        bounded(f.read()).await.is_err(),
        "auth-failing redial must not downgrade"
    );
}

#[tokio::test]
async fn node_tls_client_builder_redials_with_same_policy_and_rejects_wrong_hub() {
    run(async |f| builder_redial(f, true).await).await;
}

#[tokio::test]
async fn node_tls_client_builder_preflight_keeps_existing_error_precedence() {
    let tls = try_make_node_tls_config(&generate_test_certs()).unwrap();
    for case in 0..3 {
        let builder = BACnetClient::sc_builder()
            .hub_url("not-a-websocket-url")
            .tls_config(tls.clone())
            .vmac([2; 6])
            .device_uuid(CLIENT_UUID);
        let (builder, expected) = match case {
            0 => (builder.vmac([0; 6]), "unknown VMAC"),
            1 => (builder.max_segments(Some(1)), "max-segments-accepted"),
            _ => (
                builder.reconnect(ScReconnectConfig {
                    initial_delay_ms: 0,
                    ..reconnect()
                }),
                "reconnect",
            ),
        };
        let error = builder
            .build()
            .await
            .err()
            .expect("invalid local options must fail");
        assert!(error.to_string().contains(expected), "{error:?}");
    }
}

#[tokio::test]
async fn node_tls_server_builder_redials_with_same_policy_and_rejects_wrong_hub() {
    run(async |f| builder_redial(f, false).await).await;
}

#[tokio::test]
async fn node_tls_reconnect_fixture_joins_endpoints_and_proxy_on_panic() {
    let result = AssertUnwindSafe(run(async |f| {
        let certs = generate_test_certs();
        let url = f.hub(&certs, 9).await;
        f.proxy = Some(Proxy::new(vec![f.hubs[0].local_addr().unwrap(); 2]).await);
        let proxy_url = f.proxy.as_ref().unwrap().url.clone();
        f.server(&url, &certs).await;
        f.client(&proxy_url, &certs).await;
        bounded(f.read()).await.unwrap();
        panic!("injected node fixture failure");
    }))
    .catch_unwind()
    .await;
    let panic = result.expect_err("panic must propagate only after joined cleanup");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"injected node fixture failure")
    );
}

#[tokio::test]
async fn node_tls_generic_failover_connector_cannot_bypass_typed_policy() {
    for trusted in [true, false] {
        run(async |f| {
            let certs = generate_test_certs();
            let primary = f.hub(&certs, 9).await;
            let wrong = generate_test_certs();
            let secondary_certs = if trusted { &certs } else { &wrong };
            let secondary = f.hub(secondary_certs, 8).await;
            f.server(&secondary, secondary_certs).await;
            let node = try_make_node_tls_config(&certs).unwrap();
            let ws = bounded(TlsWebSocket::connect(&primary, node.clone()))
                .await
                .unwrap();
            let (events, mut outcomes) = mpsc::unbounded_channel();
            let transport = ScTransport::new(ws, [2; 6])
                .with_device_uuid(CLIENT_UUID)
                .with_reconnect(ScReconnectConfig {
                    max_retries: 0,
                    ..reconnect()
                })
                .with_failover_connector(move || {
                    let url = secondary.clone();
                    let node = node.clone();
                    let events = events.clone();
                    async move {
                        let result = TlsWebSocket::connect(&url, node).await;
                        events
                            .send(result.as_ref().err().map(|e| e.to_string()))
                            .unwrap();
                        result
                    }
                });
            f.client = Some(
                bounded(
                    BACnetClient::generic_builder()
                        .transport(transport)
                        .apdu_timeout_ms(100)
                        .apdu_retries(0)
                        .build(),
                )
                .await
                .unwrap(),
            );
            bounded(f.hubs[0].stop()).await;
            let result = bounded(outcomes.recv())
                .await
                .expect("failover dial must execute");
            if trusted {
                assert_eq!(result, None);
                f.reads_after_reconnect().await;
            } else {
                let error = result.expect("untrusted failover must be rejected");
                assert!(
                    error.contains("TLS handshake") && error.contains("BadSignature"),
                    "{error}"
                );
                assert!(bounded(f.read()).await.is_err());
            }
        })
        .await;
    }
}
