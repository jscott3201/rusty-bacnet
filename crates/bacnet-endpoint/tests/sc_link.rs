//! An endpoint session's BACnet/SC link through hub loss, failover, primary
//! restore and redial, against real in-process hubs, beside a client on the
//! same hubs (#1946).
#![cfg(feature = "sc-tls")]

use std::time::{Duration, Instant};

use bacnet_client::client::BACnetClient;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::EndpointSession;
use bacnet_transport::sc::{
    ScActiveHub, ScConnectError, ScConnectionState, ScDisconnectCause, ScEvent, ScHubDialer,
    ScLinkStatus, ScReconnectConfig, ScTransport, ScWebSocketErrorKind,
};
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation, ServiceSupported};
use bacnet_types::primitives::ObjectIdentifier;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::sync::broadcast;

const PRIMARY_VMAC: [u8; 6] = [0x10; 6];
const FAILOVER_VMAC: [u8; 6] = [0x20; 6];
/// Real TLS on a shared runner: waits are generous so load cannot fail them.
/// A passing run takes a few seconds.
const WAIT: Duration = Duration::from_secs(30);

type Session = EndpointSession<ScTransport<TlsWebSocket>>;
type Link = (ScConnectionState, Option<ScActiveHub>, Option<[u8; 6]>);

struct Pki {
    ca: CertificateDer<'static>,
    node: ScNodeTlsConfig,
    hubs: Vec<(CertificateDer<'static>, Vec<u8>)>,
}

impl Pki {
    fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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
        let (node_der, node_key) = leaf("endpoint node");
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

    /// Start hub `index` on `addr` again, retrying while the port lingers.
    async fn rebind(&self, index: usize, addr: &str, vmac: [u8; 6]) -> ScHub {
        for _ in 0..100 {
            if let Ok(hub) = self.hub(index, addr, vmac).await {
                return hub;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("could not bind hub {index} on {addr}");
    }
}

fn url(hub: &ScHub) -> String {
    format!("wss://localhost:{}", hub.local_addr().unwrap().port())
}

fn identity(vmac: [u8; 6]) -> DeviceIdentity {
    DeviceIdentity::new(instance(vmac), 42)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
        .with_device_uuid(uuid(vmac))
        .with_sc_port(2, 77, vmac)
        .unwrap()
}

fn instance(vmac: [u8; 6]) -> u32 {
    4000 + u32::from(vmac[5])
}

fn uuid(vmac: [u8; 6]) -> [u8; 16] {
    [vmac[5]; 16]
}

fn reconnect(max_retries: u32) -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 200,
        max_delay_ms: 200,
        max_retries,
    }
}

/// An unstarted session for `vmac`, connected through `hubs`.
async fn endpoint(
    hubs: ScHubDialer,
    vmac: [u8; 6],
    reconnect: Option<ScReconnectConfig>,
) -> Session {
    let mut builder = ScEndpointBuilder::new(vmac, uuid(vmac)).device(identity(vmac));
    if let Some(config) = reconnect {
        builder = builder.reconnect(config);
    }
    builder.connect_hub_session(hubs).await.unwrap()
}

async fn recv(events: &mut broadcast::Receiver<ScEvent>) -> ScEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("no event in time")
        .expect("event stream ended or lagged")
}

/// Skip events until `wanted` matches, keeping each in `seen`.
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

fn link(status: ScLinkStatus) -> Link {
    (status.state, status.active_hub, status.hub_vmac)
}

/// The client reads the endpoint's Device: the server role answers.
async fn read_device(client: &BACnetClient<ScTransport<TlsWebSocket>>, vmac: [u8; 6]) {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, instance(vmac)).unwrap();
    let ack = tokio::time::timeout(
        WAIT,
        client.read_property(&vmac, device, PropertyIdentifier::OBJECT_NAME, None),
    )
    .await
    .expect("read hung")
    .expect("the endpoint answers");
    assert_eq!(ack.object_identifier, device);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failover_and_primary_restore_read_as_the_clients_do() {
    let pki = Pki::new();
    let mut primary = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let primary_addr = primary.local_addr().unwrap();
    let mut failover = pki.hub(1, "127.0.0.1:0", FAILOVER_VMAC).await.unwrap();
    let hubs = ScHubDialer::new(&url(&primary), pki.node.clone()).failover_url(&url(&failover));

    let endpoint_vmac = [0x02, 0, 0, 0, 0, 0x31];
    let mut session = endpoint(hubs, endpoint_vmac, Some(reconnect(1))).await;
    let monitor = session.sc_link().expect("an SC session has a link monitor");
    assert_eq!(
        link(monitor.connection_status()),
        (ScConnectionState::Disconnected, None, None)
    );
    // Subscribed before start, so it holds the first connect.
    let mut events = monitor.connection_events();
    session.start().await.unwrap();
    let (mut client, mut client_events) = BACnetClient::sc_builder()
        .hub_url(&url(&primary))
        .failover_hub_url(&url(&failover))
        .reconnect(reconnect(1))
        .tls_config(pki.node.clone())
        .vmac([0x02, 0, 0, 0, 0, 0x32])
        .device_uuid([0x32; 16])
        .build_with_events()
        .await
        .unwrap();
    let on_primary = (
        ScConnectionState::Connected,
        Some(ScActiveHub::Primary),
        Some(PRIMARY_VMAC),
    );
    let mut seen = vec![recv(&mut events).await];
    assert!(
        is_connected(&seen[0], ScActiveHub::Primary, PRIMARY_VMAC),
        "{seen:?}"
    );
    let first = recv(&mut client_events).await;
    assert!(
        is_connected(&first, ScActiveHub::Primary, PRIMARY_VMAC),
        "{first:?}"
    );
    let mut client_seen = vec![first];
    assert_eq!(link(monitor.connection_status()), on_primary);
    assert_eq!(link(client.transport().connection_status()), on_primary);
    read_device(&client, endpoint_vmac).await;

    primary.stop().await;
    for (events, seen) in [
        (&mut events, &mut seen),
        (&mut client_events, &mut client_seen),
    ] {
        until_event(events, seen, |e| matches!(e, ScEvent::FailoverEngaged)).await;
        // Nothing comes between failover engaged and connecting to it.
        let next = recv(events).await;
        assert!(
            is_connected(&next, ScActiveHub::Failover, FAILOVER_VMAC),
            "{next:?} after {seen:?}"
        );
        seen.push(next);
    }
    let on_failover = (
        ScConnectionState::Connected,
        Some(ScActiveHub::Failover),
        Some(FAILOVER_VMAC),
    );
    assert_eq!(link(monitor.connection_status()), on_failover);
    assert_eq!(link(client.transport().connection_status()), on_failover);
    // The server role carries on through the failover hub.
    read_device(&client, endpoint_vmac).await;

    let mut primary = pki.rebind(0, &primary_addr.to_string(), PRIMARY_VMAC).await;
    for (events, seen) in [
        (&mut events, &mut seen),
        (&mut client_events, &mut client_seen),
    ] {
        until_event(events, seen, |e| matches!(e, ScEvent::PrimaryRestored)).await;
        let next = recv(events).await;
        assert!(
            is_connected(&next, ScActiveHub::Primary, PRIMARY_VMAC),
            "{next:?} after {seen:?}"
        );
        seen.push(next);
    }
    assert_eq!(link(monitor.connection_status()), on_primary);
    assert_eq!(link(client.transport().connection_status()), on_primary);
    read_device(&client, endpoint_vmac).await;

    // The same milestones, in the same order, for both.
    for seen in [&seen, &client_seen] {
        let position = |wanted: &dyn Fn(&ScEvent) -> bool| {
            seen.iter()
                .position(wanted)
                .unwrap_or_else(|| panic!("missing event in {seen:?}"))
        };
        let lost = position(&|e| {
            matches!(
                e,
                ScEvent::Disconnected {
                    hub: ScActiveHub::Primary,
                    ..
                }
            )
        });
        let engaged = position(&|e| matches!(e, ScEvent::FailoverEngaged));
        let on_failover = position(&|e| is_connected(e, ScActiveHub::Failover, FAILOVER_VMAC));
        let restored = position(&|e| matches!(e, ScEvent::PrimaryRestored));
        assert!(0 < lost && lost < engaged && engaged < on_failover && on_failover < restored);
        assert_eq!(restored + 2, seen.len());
    }

    session.stop().await.unwrap();
    client.stop().await.unwrap();
    primary.stop().await;
    failover.stop().await;
}

/// A primary that accepts TCP and never answers TLS is given up on within the
/// connect timeout, and the session starts on the failover hub.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_primary_times_out_and_starts_on_the_failover_hub() {
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
    let hubs = ScHubDialer::new(&format!("wss://localhost:{stalled_port}"), pki.node.clone())
        .failover_url(&url(&failover))
        .connect_timeout_ms(300);
    let started = Instant::now();
    let vmac = [0x02, 0, 0, 0, 0, 0x34];
    let mut session = tokio::time::timeout(WAIT, endpoint(hubs, vmac, Some(reconnect(1))))
        .await
        .expect("connect_hub_session must not hang on a stalled primary");
    // Well under the OS connect timeout; the bound is the 300 ms limit plus load.
    assert!(started.elapsed() < Duration::from_secs(20));
    let monitor = session.sc_link().unwrap();
    let mut events = monitor.connection_events();
    session.start().await.unwrap();
    let first = recv(&mut events).await;
    assert!(
        is_connected(&first, ScActiveHub::Failover, FAILOVER_VMAC),
        "{first:?}"
    );
    assert_eq!(
        link(monitor.connection_status()),
        (
            ScConnectionState::Connected,
            Some(ScActiveHub::Failover),
            Some(FAILOVER_VMAC)
        )
    );
    session.stop().await.unwrap();
    failover.stop().await;
    holder.abort();
}

/// After the hub drops, each redial is bounded by the connect timeout, and
/// the session reconnects once the hub is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_hub_is_redialled_within_the_connect_timeout() {
    let pki = Pki::new();
    let mut hub = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let addr = hub.local_addr().unwrap();
    let hubs = ScHubDialer::new(&url(&hub), pki.node.clone()).connect_timeout_ms(300);
    let mut session = endpoint(hubs, [0x02, 0, 0, 0, 0, 0x33], Some(reconnect(50))).await;
    let monitor = session.sc_link().unwrap();
    let mut events = monitor.connection_events();
    session.start().await.unwrap();
    let mut seen = Vec::new();
    until_event(&mut events, &mut seen, |e| {
        is_connected(e, ScActiveHub::Primary, PRIMARY_VMAC)
    })
    .await;

    hub.stop().await;
    // A listener on the hub's address that accepts and never answers: a
    // redial that reaches it stalls in TLS until the connect timeout.
    let mut stall = None;
    for _ in 0..100 {
        if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
            stall = Some(listener);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stall = stall.expect("rebind the hub address");
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = stall.accept().await {
            held.push(socket);
        }
    });
    let stalled_for = tokio::time::timeout(WAIT, async {
        let mut attempt_at = None;
        loop {
            let event = events.recv().await.expect("event stream ended or lagged");
            seen.push(event.clone());
            match event {
                ScEvent::ReconnectAttempt { .. } => attempt_at = Some(Instant::now()),
                ScEvent::ConnectFailed {
                    cause: ScDisconnectCause::DialTimeout { .. },
                    ..
                } => return attempt_at.expect("an attempt first").elapsed(),
                ScEvent::ReconnectGaveUp { .. } => panic!("gave up: {seen:?}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no dial timeout in time; saw {seen:?}"));
    // The 300 ms limit, not the 10 s default.
    assert!(
        (Duration::from_millis(250)..Duration::from_secs(5)).contains(&stalled_for),
        "{stalled_for:?}"
    );

    holder.abort();
    let _ = holder.await;
    let mut hub = pki.rebind(0, &addr.to_string(), PRIMARY_VMAC).await;
    until_event(&mut events, &mut seen, |e| {
        is_connected(e, ScActiveHub::Primary, PRIMARY_VMAC)
    })
    .await;
    assert_eq!(
        link(monitor.connection_status()),
        (
            ScConnectionState::Connected,
            Some(ScActiveHub::Primary),
            Some(PRIMARY_VMAC)
        )
    );
    session.stop().await.unwrap();
    hub.stop().await;
}

/// Without reconnect, both the caller-dialled build and the dialer build
/// stay disconnected after the hub drops, as before #1946.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_reconnect_a_lost_hub_stays_lost() {
    let pki = Pki::new();
    let mut hub = pki.hub(0, "127.0.0.1:0", PRIMARY_VMAC).await.unwrap();
    let dialled_vmac = [0x02, 0, 0, 0, 0, 0x36];
    let ws = TlsWebSocket::connect(&url(&hub), pki.node.clone())
        .await
        .unwrap();
    let dialled = ScEndpointBuilder::new(dialled_vmac, uuid(dialled_vmac))
        .device(identity(dialled_vmac))
        .build_hub_session(ws)
        .unwrap();
    let hubs = ScHubDialer::new(&url(&hub), pki.node.clone());
    let connected = endpoint(hubs, [0x02, 0, 0, 0, 0, 0x37], None).await;
    let mut sessions = Vec::new();
    for mut session in [dialled, connected] {
        let monitor = session.sc_link().unwrap();
        let mut events = monitor.connection_events();
        session.start().await.unwrap();
        let first = recv(&mut events).await;
        assert!(
            is_connected(&first, ScActiveHub::Primary, PRIMARY_VMAC),
            "{first:?}"
        );
        sessions.push((session, monitor, events));
    }

    hub.stop().await;
    for (_, monitor, events) in &mut sessions {
        let lost = recv(events).await;
        assert!(
            matches!(
                lost,
                ScEvent::Disconnected {
                    hub: ScActiveHub::Primary,
                    ..
                }
            ),
            "{lost:?}"
        );
        // No reconnect attempt follows.
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            panic!("unexpected {event:?}");
        }
        let status = monitor.connection_status();
        assert_eq!(status.state, ScConnectionState::Disconnected);
        assert_eq!(status.hub_vmac, None);
    }
    for (mut session, _, _) in sessions {
        let _ = session.stop().await;
    }
}

#[tokio::test]
async fn hub_settings_are_checked_before_any_dial() {
    let pki = Pki::new();
    let vmac = [0x02, 0, 0, 0, 0, 0x35];
    let builder = || ScEndpointBuilder::new(vmac, uuid(vmac));
    // Port 1 refuses: a dial would fail with a different error.
    let hubs = || ScHubDialer::new("wss://localhost:1", pki.node.clone());
    let error = builder()
        .connect_hub_session(hubs().failover_url("wss://localhost:2"))
        .await
        .err()
        .expect("a failover hub needs reconnect");
    assert!(error.to_string().contains("requires reconnect"), "{error}");
    let error = builder()
        .reconnect(reconnect(1))
        .connect_hub_session(hubs().failover_url("https://localhost:2"))
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
    let error = builder()
        .device(identity([0x02, 0, 0, 0, 0, 0x36]))
        .connect_hub_session(hubs())
        .await
        .err()
        .expect("another identity is refused");
    assert!(error.to_string().contains("must equal builder"), "{error}");
}
