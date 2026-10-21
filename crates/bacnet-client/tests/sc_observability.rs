//! Hub-connector events through a built client's transport, against real
//! in-process primary and failover hubs (#959).
#![cfg(feature = "sc-tls")]

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::sc::{
    ScActiveHub, ScConnectError, ScConnectionState, ScEvent, ScReconnectConfig, ScTransport,
    ScWebSocketErrorKind,
};
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use tokio::sync::broadcast;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

const PRIMARY_VMAC: [u8; 6] = [0x10; 6];
const FAILOVER_VMAC: [u8; 6] = [0x20; 6];
/// Real TLS on a shared runner: waits are generous so load cannot fail them
/// (a 1 s bound flaked in #1811). A passing run takes well under a second.
const WAIT: Duration = Duration::from_secs(30);

struct Pki {
    ca: CertificateDer<'static>,
    node: ScNodeTlsConfig,
    hubs: Vec<(CertificateDer<'static>, Vec<u8>)>,
}

impl Pki {
    fn new() -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap().der().clone();
        let issuer = rcgen::Issuer::from_params(&params, &key);
        let leaf = |name: &str| {
            let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
            params.distinguished_name = rcgen::DistinguishedName::new();
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, name);
            let key = rcgen::KeyPair::generate().unwrap();
            (
                params.signed_by(&key, &issuer).unwrap().der().clone(),
                key.serialize_der(),
            )
        };
        let (node_der, node_key) = leaf("observed node");
        let node = ScNodeTlsConfig::from_der(
            vec![ca.clone()],
            vec![node_der],
            PrivatePkcs8KeyDer::from(node_key).into(),
        )
        .unwrap();
        let hubs = vec![leaf("primary hub"), leaf("failover hub")];
        Self { ca, node, hubs }
    }

    async fn hub(&self, index: usize, addr: &str, vmac: [u8; 6]) -> std::io::Result<ScHub> {
        let (der, key) = &self.hubs[index];
        let config = ScHubTlsConfig::from_der(
            vec![self.ca.clone()],
            vec![der.clone()],
            PrivatePkcs8KeyDer::from(key.clone()).into(),
        )
        .unwrap();
        ScHub::start(addr, config, vmac, [index as u8 + 1; 16])
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

fn url(hub: &ScHub) -> String {
    format!("wss://localhost:{}", hub.local_addr().unwrap().port())
}

/// Skip events until `wanted` matches, then return it; fail on a timeout.
async fn until_event(
    events: &mut broadcast::Receiver<ScEvent>,
    seen: &mut Vec<ScEvent>,
    wanted: impl Fn(&ScEvent) -> bool,
) {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.recv().await.expect("event stream ended or lagged");
            seen.push(event.clone());
            if wanted(&event) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("event not seen in time; saw {seen:?}"));
}

fn is_connected(event: &ScEvent, hub: ScActiveHub, vmac: [u8; 6]) -> bool {
    matches!(event, ScEvent::Connected { hub: h, hub_vmac, .. } if *h == hub && *hub_vmac == vmac)
}

async fn peer_leaf(transport: &ScTransport<TlsWebSocket>) -> Vec<Vec<u8>> {
    let info = transport.hub_peer_info().await.expect("connected hub");
    info.tls()
        .certificates()
        .iter()
        .map(|c| c.der().to_vec())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_and_restarting_the_primary_hub_reads_as_ordered_events() {
    let pki = Pki::new();
    let mut primary = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let primary_addr = primary.local_addr().unwrap();
    let mut failover = pki.hub(1, "127.0.0.1:0", FAILOVER_VMAC).await.unwrap();

    let mut client = BACnetClient::sc_builder()
        .hub_url(&url(&primary))
        .failover_hub_url(&url(&failover))
        .reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100,
            max_retries: 1,
        })
        .tls_config(pki.node.clone())
        .vmac([0x02, 0, 0, 0, 0, 0x22])
        .device_uuid([0x22; 16])
        .build()
        .await
        .unwrap();
    let transport = client.transport();
    // Built and started: the first Connected went out before anyone could
    // subscribe, so the status snapshot reports it.
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Primary));
    assert_eq!(status.hub_vmac, Some(PRIMARY_VMAC));
    // The chain captured during the handshake is exactly the hub's certificate.
    assert_eq!(peer_leaf(transport).await, vec![pki.hubs[0].0.to_vec()]);
    let mut events = transport.connection_events();
    let mut seen = Vec::new();

    primary.stop().await;
    until_event(&mut events, &mut seen, |e| {
        matches!(
            e,
            ScEvent::Disconnected {
                hub: ScActiveHub::Primary,
                ..
            }
        )
    })
    .await;
    until_event(&mut events, &mut seen, |e| {
        matches!(e, ScEvent::FailoverEngaged)
    })
    .await;
    // Nothing comes between failover engaged and connecting on the failover.
    let next = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        is_connected(&next, ScActiveHub::Failover, FAILOVER_VMAC),
        "{next:?} after {seen:?}"
    );
    seen.push(next);
    let status = transport.connection_status();
    assert_eq!(status.active_hub, Some(ScActiveHub::Failover));
    assert_eq!(status.hub_vmac, Some(FAILOVER_VMAC));
    assert_eq!(peer_leaf(transport).await, vec![pki.hubs[1].0.to_vec()]);

    // Restart the primary on its old address. A port can linger briefly
    // after the stop, so binding is retried.
    let mut restarted = None;
    for _ in 0..100 {
        match pki.hub(0, &primary_addr.to_string(), PRIMARY_VMAC).await {
            Ok(hub) => {
                restarted = Some(hub);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let mut primary = restarted.expect("rebind the primary hub address");
    until_event(&mut events, &mut seen, |e| {
        matches!(e, ScEvent::PrimaryRestored)
    })
    .await;
    let next = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        is_connected(&next, ScActiveHub::Primary, PRIMARY_VMAC),
        "{next:?} after {seen:?}"
    );
    seen.push(next);
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Primary));
    assert_eq!(peer_leaf(transport).await, vec![pki.hubs[0].0.to_vec()]);

    // The whole sequence, in order.
    let position = |wanted: &dyn Fn(&ScEvent) -> bool| {
        seen.iter()
            .position(wanted)
            .unwrap_or_else(|| panic!("missing event in {seen:?}"))
    };
    let lost = position(&|e| matches!(e, ScEvent::Disconnected { .. }));
    let engaged = position(&|e| matches!(e, ScEvent::FailoverEngaged));
    let on_failover = position(&|e| is_connected(e, ScActiveHub::Failover, FAILOVER_VMAC));
    let restored = position(&|e| matches!(e, ScEvent::PrimaryRestored));
    assert!(lost < engaged && engaged < on_failover && on_failover < restored);
    assert_eq!(restored + 1, seen.len() - 1);

    client.stop().await.unwrap();
    primary.stop().await;
    failover.stop().await;
}

/// Start the primary hub on `addr`, retrying while the port lingers.
async fn rebind(pki: &Pki, addr: &str) -> ScHub {
    for _ in 0..100 {
        if let Ok(hub) = pki.hub(0, addr, PRIMARY_VMAC).await {
            return hub;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("could not bind the primary hub on {addr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_primary_builds_on_the_failover_hub_and_restores_later() {
    let pki = Pki::new();
    // An address nobody listens on until the primary starts there.
    let primary_addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut failover = pki.hub(1, "127.0.0.1:0", FAILOVER_VMAC).await.unwrap();
    let mut client = BACnetClient::sc_builder()
        .hub_url(&format!("wss://localhost:{}", primary_addr.port()))
        .failover_hub_url(&url(&failover))
        .reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100,
            max_retries: 1,
        })
        .tls_config(pki.node.clone())
        .vmac([0x02, 0, 0, 0, 0, 0x23])
        .device_uuid([0x23; 16])
        .build()
        .await
        .unwrap();
    let transport = client.transport();
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Failover));
    assert_eq!(status.hub_vmac, Some(FAILOVER_VMAC));
    assert_eq!(peer_leaf(transport).await, vec![pki.hubs[1].0.to_vec()]);
    let mut events = transport.connection_events();
    let mut seen = Vec::new();

    let mut primary = rebind(&pki, &primary_addr.to_string()).await;
    until_event(&mut events, &mut seen, |e| {
        matches!(e, ScEvent::PrimaryRestored)
    })
    .await;
    let next = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        is_connected(&next, ScActiveHub::Primary, PRIMARY_VMAC),
        "{next:?}"
    );
    assert_eq!(
        transport.connection_status().active_hub,
        Some(ScActiveHub::Primary)
    );
    assert_eq!(peer_leaf(transport).await, vec![pki.hubs[0].0.to_vec()]);
    client.stop().await.unwrap();
    primary.stop().await;
    failover.stop().await;
}

/// A primary that accepts TCP and never answers TLS is given up on within the
/// connect timeout, and the client starts on the failover hub.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_primary_times_out_and_builds_on_the_failover_hub() {
    let pki = Pki::new();
    let stalled = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stalled_port = stalled.local_addr().unwrap().port();
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = stalled.accept().await {
            held.push(socket); // keep it open, say nothing
        }
    });
    let mut failover = pki.hub(1, "127.0.0.1:0", FAILOVER_VMAC).await.unwrap();
    let started = tokio::time::Instant::now();
    let mut client = tokio::time::timeout(
        WAIT,
        BACnetClient::sc_builder()
            .hub_url(&format!("wss://localhost:{stalled_port}"))
            .failover_hub_url(&url(&failover))
            .reconnect(ScReconnectConfig::default())
            .connect_timeout_ms(300)
            .tls_config(pki.node.clone())
            .vmac([0x02, 0, 0, 0, 0, 0x25])
            .device_uuid([0x25; 16])
            .build(),
    )
    .await
    .expect("build() must not hang on a stalled primary")
    .unwrap();
    // Well under the OS connect timeout; the bound is the 300 ms limit plus load.
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(
        client.transport().connection_status().active_hub,
        Some(ScActiveHub::Failover)
    );
    client.stop().await.unwrap();
    failover.stop().await;
    holder.abort();
}

/// `build_with_events` subscribes before the client starts, so its receiver
/// holds the first connect, which `connection_events()` after `build` misses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_with_events_sees_the_first_connect() {
    let pki = Pki::new();
    let mut primary = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let (mut client, mut events) = BACnetClient::sc_builder()
        .hub_url(&url(&primary))
        .tls_config(pki.node.clone())
        .vmac([0x02, 0, 0, 0, 0, 0x25])
        .device_uuid([0x25; 16])
        .build_with_events()
        .await
        .unwrap();
    let late = client.transport().connection_events();
    let first = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        is_connected(&first, ScActiveHub::Primary, PRIMARY_VMAC),
        "{first:?}"
    );
    // A receiver subscribed after the build holds nothing yet.
    assert!(late.is_empty());
    client.stop().await.unwrap();
    primary.stop().await;
}

/// The transport-erased build has the same subscribe-before-start step
/// (#1909): its receiver holds the first connect too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_any_with_events_sees_the_first_connect() {
    use bacnet_transport::any::AnyTransport;
    use bacnet_transport::mstp::NoSerial;
    let pki = Pki::new();
    let mut primary = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let (mut client, mut events) = BACnetClient::sc_builder()
        .hub_url(&url(&primary))
        .tls_config(pki.node.clone())
        .vmac([0x02, 0, 0, 0, 0, 0x26])
        .device_uuid([0x26; 16])
        .build_any_with_events::<NoSerial>()
        .await
        .unwrap();
    let AnyTransport::Sc(transport) = client.transport() else {
        panic!("an SC build yields the SC variant");
    };
    let late = transport.connection_events();
    let first = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        is_connected(&first, ScActiveHub::Primary, PRIMARY_VMAC),
        "{first:?}"
    );
    assert!(late.is_empty());
    client.stop().await.unwrap();
    primary.stop().await;
}

#[tokio::test]
async fn failover_settings_are_checked_before_any_dial() {
    let pki = Pki::new();
    let builder = || {
        BACnetClient::sc_builder()
            .hub_url("wss://localhost:1")
            .tls_config(pki.node.clone())
            .vmac([0x02, 0, 0, 0, 0, 0x24])
            .device_uuid([0x24; 16])
    };
    let error = builder()
        .failover_hub_url("wss://localhost:2")
        .build()
        .await
        .err()
        .expect("a failover hub needs reconnect");
    assert!(error.to_string().contains("requires reconnect"), "{error}");
    let error = builder()
        .failover_hub_url("https://localhost:2")
        .reconnect(ScReconnectConfig::default())
        .build()
        .await
        .err()
        .expect("a non-wss failover URL is refused");
    assert!(matches!(
        ScConnectError::from_error(&error),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::UnsupportedScheme,
            ..
        })
    ));
}
