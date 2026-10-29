//! The SC screen against real mutual-TLS hubs on loopback, with an rcgen
//! PKI as in `tests/sc_handshake.rs` (#967): the status reaches Connected
//! on the primary with the hub's chain shown as issued, a failover and the
//! primary's return in order, and the doctor's reading of hubs whose
//! certificates this node must refuse, and of a hub refusing this node.
//!
//! Real TLS on a shared runner: every wait is generous (a 1 s bound flaked
//! in #1811); a passing run takes about a second.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::sc::{
    ScActiveHub, ScCertificateRejection, ScConnectionState, ScReconnectConfig, ScTlsAlert,
    ScTlsFailure,
};
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::ScNodeTlsConfig;
use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::sync::mpsc;

use super::sc_screen::sc_app;
use super::*;
use crate::transport::TransportArgs;
use crate::tui::app::Link;
use crate::tui::sc::Fault;
use crate::tui::sc::doctor::Phase;
use crate::tui::worker::session::{self, ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{EventSink, coordinator};

pub(super) const WAIT: Duration = Duration::from_secs(30);
/// A date as year, month, day.
type Ymd = (i32, u8, u8);
/// A link state and its hub.
type State = (ScConnectionState, Option<ScActiveHub>);
pub(super) const PRIMARY_VMAC: [u8; 6] = [0x10; 6];
pub(super) const FAILOVER_VMAC: [u8; 6] = [0x20; 6];
/// 2031-01-01T00:00:00Z: the primary hub certificate's not-after.
const HUB_NOT_AFTER: i64 = 1_924_992_000;

/// A leaf certificate and its key.
pub(super) struct Leaf {
    der: CertificateDer<'static>,
    pem: String,
    key: KeyPair,
}

/// A site CA that issues leaves.
pub(super) struct Site {
    params: CertificateParams,
    key: KeyPair,
    der: CertificateDer<'static>,
    pem: String,
}

impl Site {
    pub(super) fn new(name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, name);
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Self {
            der: cert.der().clone(),
            pem: cert.pem(),
            params,
            key,
        }
    }

    /// A leaf named `cn` for 127.0.0.1, signed by `signer` (this CA's key
    /// unless another is given: the key-mismatch case).
    pub(super) fn leaf(
        &self,
        cn: &str,
        usage: ExtendedKeyUsagePurpose,
        dates: Option<(Ymd, Ymd)>,
        signer: Option<&KeyPair>,
    ) -> Leaf {
        let mut params = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, cn);
        params.extended_key_usages = vec![usage];
        let ((y0, m0, d0), (y1, m1, d1)) = dates.unwrap_or(((2020, 1, 1), (2031, 1, 1)));
        params.not_before = rcgen::date_time_ymd(y0, m0, d0);
        params.not_after = rcgen::date_time_ymd(y1, m1, d1);
        let key = KeyPair::generate().unwrap();
        let issuer = Issuer::from_params(&self.params, signer.unwrap_or(&self.key));
        let cert = params.signed_by(&key, &issuer).unwrap();
        Leaf {
            der: cert.der().clone(),
            pem: cert.pem(),
            key,
        }
    }

    fn hub_config(&self, hub: &Leaf) -> ScHubTlsConfig {
        ScHubTlsConfig::from_der(
            vec![self.der.clone()],
            vec![hub.der.clone()],
            PrivatePkcs8KeyDer::from(hub.key.serialize_der()).into(),
        )
        .unwrap()
    }

    fn node_config(&self, node: &Leaf) -> ScNodeTlsConfig {
        ScNodeTlsConfig::from_der(
            vec![self.der.clone()],
            vec![node.der.clone()],
            PrivatePkcs8KeyDer::from(node.key.serialize_der()).into(),
        )
        .unwrap()
    }
}

pub(super) fn client_leaf(site: &Site) -> Leaf {
    site.leaf("tui node", ExtendedKeyUsagePurpose::ClientAuth, None, None)
}

/// Flags for an SC session trusting `site`, as `node`, dialling `url`.
pub(super) fn args(dir: &Path, site: &Site, node: &Leaf, url: &str) -> TransportArgs {
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    };
    TransportArgs {
        interface: std::net::Ipv4Addr::UNSPECIFIED,
        port: 0,
        broadcast: std::net::Ipv4Addr::BROADCAST,
        timeout_ms: 6000,
        min_interval_ms: 0,
        sc: true,
        sc_url: Some(url.into()),
        sc_failover_url: None,
        sc_reconnect: None,
        sc_connect_timeout_ms: None,
        sc_ca: Some(write("ca.pem", &site.pem)),
        sc_cert: Some(write("node.pem", &node.pem)),
        sc_key: Some(write("node.key", &node.key.serialize_pem())),
        sc_vmac: Some([0x02, 0, 0, 0, 0, 0x67]),
        sc_device_uuid: Some([0x67; 16]),
        ipv6: false,
        ipv6_interface: None,
        device_instance: None,
        foreign_device: None,
        multicast: None,
        mstp: None,
    }
}

pub(super) async fn hub(
    site: &Site,
    cert: &Leaf,
    addr: &str,
    vmac: [u8; 6],
) -> std::io::Result<ScHub> {
    ScHub::start(addr, site.hub_config(cert), vmac, [vmac[0]; 16])
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Start a hub on `addr`, retrying while the old listener's port lingers.
pub(super) async fn rebind(site: &Site, cert: &Leaf, addr: SocketAddr, vmac: [u8; 6]) -> ScHub {
    for _ in 0..100 {
        if let Ok(hub) = hub(site, cert, &addr.to_string(), vmac).await {
            return hub;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("could not bind a hub on {addr}");
}

/// Feed worker events into `app` until `done` holds.
pub(super) async fn until(
    app: &mut App,
    events: &mut mpsc::Receiver<WorkerEvent>,
    what: &str,
    done: impl Fn(&App) -> bool,
) {
    tokio::time::timeout(WAIT, async {
        while !done(app) {
            let event = events.recv().await.expect("worker stopped");
            worker(app, event);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} not seen in time; states {:?}", app.sc.transitions));
}

pub(super) fn is_on(app: &App, hub: ScActiveHub) -> bool {
    app.sc.link.as_ref().is_some_and(|(link, _)| {
        link.state == ScConnectionState::Connected && link.hub == Some(hub)
    }) && !app.sc.chain.is_empty()
}

/// Run a flags-only session through the coordinator until `done`, then quit.
async fn session(args: TransportArgs, done: impl Fn(&App) -> bool) -> App {
    let mut app = sc_app();
    let plan = ConnectPlan {
        kind: TransportKind::Sc,
        args,
        await_interface: false,
    };
    let (sink, mut events) = EventSink::channel(256);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, SessionOptions::default());
    until(&mut app, &mut events, "the expected state", done).await;
    drop((commands, events));
    tokio::time::timeout(WAIT, task)
        .await
        .expect("coordinator quit")
        .unwrap();
    app
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_reaches_connected_on_the_primary_with_its_chain() {
    let site = Site::new("tui site CA");
    let cert = site.leaf(
        "primary hub",
        ExtendedKeyUsagePurpose::ServerAuth,
        None,
        None,
    );
    let mut primary = hub(&site, &cert, "127.0.0.1:0", PRIMARY_VMAC)
        .await
        .unwrap();
    let url = format!("wss://{}", primary.local_addr().unwrap());
    let dir = tempfile::tempdir().unwrap();
    // The first connect reaches the timeline: the session subscribed to the
    // connector's events before the dial (#1890).
    let mut app = session(args(dir.path(), &site, &client_leaf(&site), &url), |app| {
        is_on(app, ScActiveHub::Primary)
            && app
                .sc
                .timeline
                .iter()
                .any(|e| e.text == "connected to the primary hub (hub VMAC 10:10:10:10:10:10)")
    })
    .await;
    let (link, _) = app.sc.link.unwrap();
    assert_eq!(link.hub_vmac, Some(PRIMARY_VMAC));
    let identity = app.sc.identity.as_ref().unwrap();
    assert_eq!(
        (identity.primary.as_str(), identity.uuid),
        (url.as_str(), [0x67; 16])
    );
    // The chain is the hub's certificate, shown exactly as issued.
    let leaf = &app.sc.chain[0];
    assert_eq!(leaf.subject, "CN=primary hub");
    assert_eq!(leaf.issuer, "CN=tui site CA");
    assert_eq!(leaf.not_after_unix, HUB_NOT_AFTER);
    assert_eq!(leaf.not_after, "2031-01-01T00:00:00Z (Unix 1924992000)");
    app.screen = crate::tui::app::watch::Screen::Sc;
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("subject CN=primary hub"), "{text}");
    assert!(
        text.contains("expires 2031-01-01T00:00:00Z (Unix 1924992000)"),
        "{text}"
    );
    primary.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_primary_fails_over_and_its_return_restores_it_in_order() {
    let site = Site::new("tui failover CA");
    let primary_cert = site.leaf(
        "primary hub",
        ExtendedKeyUsagePurpose::ServerAuth,
        None,
        None,
    );
    let failover_cert = site.leaf(
        "failover hub",
        ExtendedKeyUsagePurpose::ServerAuth,
        None,
        None,
    );
    let mut primary = hub(&site, &primary_cert, "127.0.0.1:0", PRIMARY_VMAC)
        .await
        .unwrap();
    let primary_addr = primary.local_addr().unwrap();
    let mut failover = hub(&site, &failover_cert, "127.0.0.1:0", FAILOVER_VMAC)
        .await
        .unwrap();
    // Built here for a short reconnect policy; the session's own SC worker
    // reports, as it does for a profile's client.
    let client = BACnetClient::sc_builder()
        .hub_url(&format!("wss://{primary_addr}"))
        .failover_hub_url(&format!("wss://{}", failover.local_addr().unwrap()))
        .reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100,
            max_retries: 1,
        })
        .tls_config(site.node_config(&client_leaf(&site)))
        .vmac([0x02, 0, 0, 0, 0, 0x68])
        .device_uuid([0x68; 16])
        .build()
        .await
        .unwrap();
    let mut app = sc_app();
    let (sink, mut events) = EventSink::channel(256);
    let (commands, rx) = mpsc::channel(16);
    let serving = tokio::spawn(Box::pin(session::serve(
        client,
        bacnet_ops::target::AddressStyle::Hex,
        sink,
        rx,
        SessionOptions::default(),
    )));
    until(&mut app, &mut events, "primary", |a| {
        is_on(a, ScActiveHub::Primary)
    })
    .await;
    assert_eq!(app.sc.chain[0].subject, "CN=primary hub");

    primary.stop().await;
    until(&mut app, &mut events, "failover", |a| {
        is_on(a, ScActiveHub::Failover)
    })
    .await;
    assert_eq!(app.sc.chain[0].subject, "CN=failover hub");
    let mut primary = rebind(&site, &primary_cert, primary_addr, PRIMARY_VMAC).await;
    until(&mut app, &mut events, "the primary's return", |a| {
        a.sc.timeline
            .iter()
            .any(|e| e.text.starts_with("primary hub is back"))
            && is_on(a, ScActiveHub::Primary)
    })
    .await;

    // The App's states, in order: on the primary, then connecting, then on
    // the failover, then on the primary again.
    let states: Vec<_> = app
        .sc
        .transitions
        .iter()
        .map(|t| (t.state, t.hub))
        .collect();
    let find = |from: usize, wanted: &dyn Fn(&State) -> bool| {
        from + states[from..]
            .iter()
            .position(wanted)
            .unwrap_or_else(|| panic!("missing state after {from} in {states:?}"))
    };
    use ScConnectionState::{Connected, Connecting};
    let on_primary = find(0, &|s| *s == (Connected, Some(ScActiveHub::Primary)));
    let connecting = find(on_primary, &|s| s.0 == Connecting);
    let on_failover = find(connecting, &|s| {
        *s == (Connected, Some(ScActiveHub::Failover))
    });
    let back = find(on_failover, &|s| {
        *s == (Connected, Some(ScActiveHub::Primary))
    });
    assert!(on_primary < connecting && connecting < on_failover && on_failover < back);
    let order: Vec<&str> = app.sc.timeline.iter().map(|e| e.text.as_str()).collect();
    let at = |prefix: &str| {
        order
            .iter()
            .position(|t| t.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix:?} in {order:?}"))
    };
    assert!(at("lost the primary hub") < at("primary hub unreachable"));
    assert!(at("primary hub unreachable") < at("connected to the failover hub"));
    assert!(at("connected to the failover hub") < at("primary hub is back"));

    drop((commands, events));
    tokio::time::timeout(WAIT, serving)
        .await
        .expect("session stops")
        .unwrap();
    primary.stop().await;
    failover.stop().await;
}

/// A TLS server on loopback presenting `cert`, for hubs this node refuses.
async fn presenting(cert: &Leaf) -> (String, tokio::task::JoinHandle<()>) {
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der.clone()],
            PrivatePkcs8KeyDer::from(cert.key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            // The client refuses the certificate; the error is expected.
            let _ = acceptor.accept(tcp).await;
        }
    });
    (url, task)
}

/// The doctor after a failed connect to `url`.
async fn refused(site: &Site, node: &Leaf, url: &str) -> App {
    let dir = tempfile::tempdir().unwrap();
    session(args(dir.path(), site, node, url), |app| {
        matches!(app.link, Link::Failed(_)) && app.sc.failure.is_some()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_hub_certificates_each_land_on_tls_with_their_own_hint() {
    let site = Site::new("tui doctor CA");
    let node = client_leaf(&site);
    let other = Site::new("some other CA");
    let impostor = KeyPair::generate().unwrap();
    let server = || ExtendedKeyUsagePurpose::ServerAuth;
    let cases = [
        (
            "rogue hub",
            other.leaf("rogue hub", server(), None, None),
            Fault::Tls(ScTlsFailure::PeerCertificate(
                ScCertificateRejection::UnknownIssuer,
            )),
        ),
        (
            "expired hub",
            site.leaf(
                "expired hub",
                server(),
                Some(((2000, 1, 1), (2001, 1, 1))),
                None,
            ),
            Fault::Tls(ScTlsFailure::PeerCertificate(
                ScCertificateRejection::Expired { by: None },
            )),
        ),
        (
            "future hub",
            site.leaf(
                "future hub",
                server(),
                Some(((4090, 1, 1), (4091, 1, 1))),
                None,
            ),
            Fault::Tls(ScTlsFailure::PeerCertificate(
                ScCertificateRejection::NotYetValid { by: None },
            )),
        ),
        (
            "re-keyed hub",
            site.leaf("re-keyed hub", server(), None, Some(&impostor)),
            Fault::Tls(ScTlsFailure::PeerCertificate(
                ScCertificateRejection::BadSignature,
            )),
        ),
    ];
    let mut hints = Vec::new();
    for (name, cert, expected) in cases {
        let (url, task) = presenting(&cert).await;
        let app = refused(&site, &node, &url).await;
        task.abort();
        let last = app.sc.failure.as_ref().unwrap();
        // rustls reports the context variants: how far off is known.
        if let Fault::Tls(ScTlsFailure::PeerCertificate(
            ScCertificateRejection::Expired { by } | ScCertificateRejection::NotYetValid { by },
        )) = &last.failure.fault
        {
            assert!(
                by.is_some(),
                "{name}: no distance in {:?}",
                last.failure.fault
            );
        }
        // The dates are measured from the run's clock, so compare the kind.
        let kind = |fault: &Fault| match fault {
            Fault::Tls(ScTlsFailure::PeerCertificate(ScCertificateRejection::Expired {
                ..
            })) => "expired".to_string(),
            Fault::Tls(ScTlsFailure::PeerCertificate(ScCertificateRejection::NotYetValid {
                ..
            })) => "future".to_string(),
            other => format!("{other:?}"),
        };
        assert_eq!(
            kind(&last.failure.fault),
            kind(&expected),
            "{}",
            last.failure.detail
        );
        assert_eq!(last.diagnosis.failed, Some(Phase::Tls), "{expected:?}");
        assert!(!matches!(
            last.failure.fault,
            Fault::DialTimeout(_) | Fault::ConnectAcceptTimeout
        ));
        // The refused hub's own certificate is shown, untrusted.
        let shown = &last.failure.presented[0];
        assert_eq!(shown.subject, format!("CN={name}"));
        hints.push(last.diagnosis.hint.clone());
    }
    let distinct: std::collections::HashSet<_> = hints.iter().collect();
    assert_eq!(distinct.len(), 4, "{hints:#?}");
}

/// A hub that refuses this node's certificate, from a CA it does not trust:
/// the alert comes back typed, after TLS 1.3 finished on this side, and
/// still lands on TLS. Locally that leaf is only a lint warning (no bundle
/// CA issued it directly), which the credentials view shows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hub_refusing_this_nodes_certificate_lands_on_tls() {
    use crate::tui::sc::credentials::Level;
    let site = Site::new("tui alert CA");
    let cert = site.leaf("alert hub", ExtendedKeyUsagePurpose::ServerAuth, None, None);
    let mut hub = hub(&site, &cert, "127.0.0.1:0", PRIMARY_VMAC)
        .await
        .unwrap();
    let url = format!("wss://{}", hub.local_addr().unwrap());
    let rogue = Site::new("rogue CA").leaf(
        "rogue node",
        ExtendedKeyUsagePurpose::ClientAuth,
        None,
        None,
    );
    let app = refused(&site, &rogue, &url).await;
    let last = app.sc.failure.as_ref().unwrap();
    assert_eq!(
        last.failure.fault,
        Fault::Tls(ScTlsFailure::AlertReceived(ScTlsAlert::UnknownCa)),
        "{}",
        last.failure.detail
    );
    assert_eq!(last.diagnosis.failed, Some(Phase::Tls));
    let Some(Ok(credentials)) = &app.sc.credentials else {
        panic!("no credentials view: {:?}", app.sc.credentials);
    };
    assert_eq!(credentials.issuer.0, Level::Warning);
    assert!(!app.sc.lint_refused());
    hub.stop().await;
}

/// An expired node certificate never reaches the hub: the lint refuses the
/// dial, and the screen opens on the credentials view saying why.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_node_certificate_is_refused_before_the_hub() {
    use crate::tui::app::sc::View;
    let site = Site::new("tui lint CA");
    // A listener that records accepts stands in for the hub.
    let hub = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    hub.set_nonblocking(true).unwrap();
    let url = format!("wss://{}", hub.local_addr().unwrap());
    let expired = site.leaf(
        "expired node",
        ExtendedKeyUsagePurpose::ClientAuth,
        Some(((2000, 1, 1), (2001, 1, 1))),
        None,
    );
    let app = refused(&site, &expired, &url).await;
    let last = app.sc.failure.as_ref().unwrap();
    assert!(
        last.failure
            .detail
            .contains("SC credential check refused the dial before connecting"),
        "{}",
        last.failure.detail
    );
    assert!(app.sc.lint_refused());
    assert_eq!(app.sc.view, View::Credentials);
    // The session has ended: a dial would have left a connection queued.
    assert_eq!(
        hub.accept().map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "the hub was dialled"
    );
}

/// The timeouts and a refused upgrade, from real sockets, as the doctor
/// reads them (#1839): a dial stalled in TLS lands on TLS, a hub that never
/// answers the Connect-Request (the transport's bare timeout) on Connect,
/// and a proxy answering the upgrade with 502 on WebSocket, its hint naming
/// the status.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalls_and_a_refusing_proxy_each_land_on_their_phase() {
    use crate::tui::sc::ScFailure;
    use crate::tui::sc::doctor::diagnose;
    use bacnet_transport::sc::{ScDialStage, dial_within};
    use bacnet_transport::sc_tls::TlsWebSocket;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const LIMIT: Duration = Duration::from_millis(300);
    let site = Site::new("tui timeout CA");
    let node = site.node_config(&client_leaf(&site));

    // TCP connects; nothing answers the ClientHello.
    let stalled = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", stalled.local_addr().unwrap());
    let holder = tokio::spawn(async move {
        let (_tcp, _) = stalled.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = dial_within(LIMIT, TlsWebSocket::connect(&url, node.clone()))
        .await
        .err()
        .expect("a stalled TLS handshake cannot connect");
    holder.abort();
    let stalled = ScFailure::from_error(&error);
    assert_eq!(stalled.fault, Fault::DialTimeout(Some(ScDialStage::Tls)));
    assert_eq!(diagnose(&stalled.fault).failed, Some(Phase::Tls));

    let silent = ScFailure::from_error(&bacnet_types::error::Error::Timeout(LIMIT));
    assert_eq!(silent.fault, Fault::ConnectAcceptTimeout);
    assert_eq!(diagnose(&silent.fault).failed, Some(Phase::Connect));

    // TLS completes; the "proxy" answers the upgrade with 502.
    let cert = site.leaf("proxy", ExtendedKeyUsagePurpose::ServerAuth, None, None);
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der.clone()],
            PrivatePkcs8KeyDer::from(cert.key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", proxy.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (tcp, _) = proxy.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "the upgrade request ended early");
            request.extend_from_slice(&buf[..n]);
        }
        tls.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        // The client may have closed first; the 502 is already sent.
        let _ = tls.shutdown().await;
    });
    let error = dial_within(WAIT, TlsWebSocket::connect(&url, node))
        .await
        .err()
        .expect("a refused upgrade cannot connect");
    task.await.unwrap();
    let refused = ScFailure::from_error(&error);
    assert_eq!(
        refused.fault,
        Fault::UpgradeRefused(502),
        "{}",
        refused.detail
    );
    let diagnosis = diagnose(&refused.fault);
    assert_eq!(diagnosis.failed, Some(Phase::WebSocket));
    assert!(diagnosis.hint.contains("HTTP 502"), "{}", diagnosis.hint);
    assert!(diagnosis.hint.contains("proxy"), "{}", diagnosis.hint);
}
