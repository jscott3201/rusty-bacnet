//! The error code a real SC failure reports, as the SC screen shows it
//! (#2438): a refused connection and a hub that answers the upgrade with
//! 503, each through a whole session. The doctor names the code below its
//! hint, and the timeline's line for the failure ends with it, as it does
//! for a connection the hub closes with a status.

use std::sync::Arc;

use bacnet_transport::sc::{ScActiveHub, ScWebSocketErrorKind};
use bacnet_types::enums::ErrorCode;
use rcgen::ExtendedKeyUsagePurpose;
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::sc_hub::{Leaf, Site, client_leaf, refused};
use super::sc_screen::{sc, sc_app};
use super::*;
use crate::tui::app::sc::View;
use crate::tui::app::watch::Screen;
use crate::tui::sc::doctor::Phase;
use crate::tui::sc::{Fault, LinkEvent, ScReport};

/// The doctor view, rendered.
fn doctor_view(app: &mut App) -> String {
    app.screen = Screen::Sc;
    app.sc.view = View::Doctor;
    screen(app, 120, 50)
}

/// The failure's timeline line, the doctor's code row and the failure
/// itself all name `code`.
fn shows(app: &mut App, code: ErrorCode) {
    let name = code.to_string();
    let line = app
        .sc
        .timeline
        .iter()
        .find(|entry| entry.text.starts_with("could not connect: "))
        .expect("a timeline line for the failure");
    assert!(line.text.ends_with(&format!("({name})")), "{}", line.text);
    assert_eq!(app.sc.failure.as_ref().unwrap().failure.code, Some(code));
    let shown = doctor_view(app);
    assert!(
        shown
            .lines()
            .any(|line| line.contains("Code") && line.contains(&name)),
        "{shown}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_dial_shows_tcp_connection_refused() {
    let site = Site::new("tui refused CA");
    let node = client_leaf(&site);
    let mut seen = Vec::new();
    for _ in 0..5 {
        // Freed so nothing listens; another process could take it first,
        // or Windows fail the connect on a local port collision: go again.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut app = refused(&site, &node, &format!("wss://{port}")).await;
        let last = app.sc.failure.as_ref().unwrap();
        if last.failure.fault != Fault::WebSocket(ScWebSocketErrorKind::TcpRefused) {
            seen.push(last.failure.detail.clone());
            continue;
        }
        assert_eq!(last.diagnosis.failed, Some(Phase::Tcp));
        shows(&mut app, ErrorCode::TCP_CONNECTION_REFUSED);
        return;
    }
    panic!("no dial was refused: {seen:#?}");
}

/// A TLS server presenting `cert` that answers the upgrade with 503.
async fn answering_503(cert: &Leaf) -> (String, tokio::task::JoinHandle<()>) {
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
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "the upgrade request ended early");
            request.extend_from_slice(&buf[..n]);
        }
        tls.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        // The client may have closed first; the 503 is already sent.
        let _ = tls.shutdown().await;
    });
    (url, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hub_answering_503_shows_http_temporary_unavailable() {
    let site = Site::new("tui busy CA");
    let node = client_leaf(&site);
    let busy = site.leaf("busy hub", ExtendedKeyUsagePurpose::ServerAuth, None, None);
    let (url, task) = answering_503(&busy).await;
    let mut app = refused(&site, &node, &url).await;
    task.await.unwrap();
    let last = app.sc.failure.as_ref().unwrap();
    assert_eq!(
        last.failure.fault,
        Fault::UpgradeRefused(503),
        "{}",
        last.failure.detail
    );
    assert_eq!(last.diagnosis.failed, Some(Phase::WebSocket));
    shows(&mut app, ErrorCode::HTTP_TEMPORARY_UNAVAILABLE);
}

/// A connection the hub closed with status 1001: its line and the doctor
/// name the code the event carried.
#[test]
fn a_lost_connection_shows_its_close_status_and_code() {
    let mut app = sc_app();
    sc(
        &mut app,
        ScReport::Event(LinkEvent::Disconnected {
            hub: ScActiveHub::Primary,
            fault: Fault::WebSocketClosed(1001),
            code: Some(ErrorCode::WEBSOCKET_ENDPOINT_LEAVES),
        }),
    );
    let line = &app.sc.timeline.back().unwrap().text;
    assert_eq!(
        line,
        "lost the primary hub: the hub closed the WebSocket with status 1001 \
         (WEBSOCKET_ENDPOINT_LEAVES)"
    );
    let shown = doctor_view(&mut app);
    assert!(
        shown
            .lines()
            .any(|line| line.contains("Code") && line.contains("WEBSOCKET_ENDPOINT_LEAVES")),
        "{shown}"
    );
}
