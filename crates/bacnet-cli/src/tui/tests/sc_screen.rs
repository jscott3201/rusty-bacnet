//! The SC screen from synthetic reports: the state history, the doctor's
//! reading, the duplicate-VMAC history, the gap marker, keys, and frames of
//! the status and doctor views at 120x40.

use std::time::Duration;

use bacnet_transport::sc::{
    ScActiveHub, ScCertificateRejection, ScConnectionState, ScTlsFailure, ScWebSocketErrorKind,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use insta::assert_snapshot;

use super::*;
use crate::tui::app::sc::View;
use crate::tui::app::watch::Screen;
use crate::tui::sc::doctor::Phase;
use crate::tui::sc::{
    CertSummary, Fault, LinkEvent, LinkSnapshot, ScFailure, ScIdentity, ScReport,
};

pub(super) const PRIMARY_VMAC: [u8; 6] = [0x10; 6];
const FAILOVER_VMAC: [u8; 6] = [0x20; 6];

/// An SC app before connecting, without colour.
pub(super) fn sc_app() -> App {
    App::new(
        AppConfig {
            transport: "SC",
            style: AddressStyle::Hex,
            color: false,
            picker: None,
            join: None,
            passive: false,
            now: t0(),
        },
        LogRing::new(100),
    )
}

pub(super) fn identity() -> ScIdentity {
    ScIdentity {
        primary: "wss://hub.site.example:443/bacnet".into(),
        failover: Some("wss://hub2.site.example:443/bacnet".into()),
        vmac: [0x22, 0x10, 0x20, 0x30, 0x40, 0x50],
        uuid: [
            0x6f, 0x1c, 0x2a, 0x9e, 0x44, 0x07, 0x4b, 0x3d, 0x9a, 0x51, 0x0e, 0x7c, 0x88, 0x21,
            0xd4, 0x0b,
        ],
    }
}

pub(super) fn sc(app: &mut App, report: ScReport) {
    worker(app, WorkerEvent::Sc(report));
}

pub(super) fn connected(hub: ScActiveHub, vmac: [u8; 6]) -> LinkSnapshot {
    LinkSnapshot {
        state: ScConnectionState::Connected,
        hub: Some(hub),
        hub_vmac: Some(vmac),
        heartbeat_age: Some(Duration::from_secs(3)),
        heartbeat_rtt: Some(Duration::from_millis(12)),
    }
}

pub(super) fn hub_chain() -> Vec<CertSummary> {
    vec![
        CertSummary {
            subject: "CN=primary hub".into(),
            issuer: "CN=site CA".into(),
            not_after_unix: 1_924_992_000,
            not_after: "2031-01-01T00:00:00Z (Unix 1924992000)".into(),
            signature_algorithm: "1.2.840.10045.4.3.2".into(),
        },
        CertSummary {
            subject: "CN=site CA".into(),
            issuer: "CN=site CA".into(),
            not_after_unix: 2_240_524_800,
            not_after: "2041-01-01T00:00:00Z (Unix 2240524800)".into(),
            signature_algorithm: "1.2.840.10045.4.3.2".into(),
        },
    ]
}

fn states(app: &App) -> Vec<(ScConnectionState, Option<ScActiveHub>)> {
    app.sc
        .transitions
        .iter()
        .map(|t| (t.state, t.hub))
        .collect()
}

/// An app connected to the primary hub, with a failover, a reconnect and
/// the primary's return on the timeline.
fn after_a_failover() -> App {
    let mut app = sc_app();
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "22:10:20:30:40:50".into(),
        },
    );
    sc(&mut app, ScReport::Dialing(identity()));
    tick(&mut app, at(1));
    sc(
        &mut app,
        ScReport::Status(connected(ScActiveHub::Primary, PRIMARY_VMAC)),
    );
    sc(&mut app, ScReport::Chain(hub_chain()));
    tick(&mut app, at(60));
    let lost = Fault::WebSocket(ScWebSocketErrorKind::Receive);
    let events = [
        LinkEvent::Disconnected {
            hub: ScActiveHub::Primary,
            fault: lost,
        },
        LinkEvent::Reconnecting {
            hub: ScActiveHub::Primary,
            attempt: 1,
            backoff: Duration::from_millis(10_000),
        },
        LinkEvent::ConnectFailed {
            hub: ScActiveHub::Primary,
            fault: Fault::WebSocket(ScWebSocketErrorKind::TcpDial),
        },
        LinkEvent::FailoverEngaged,
        LinkEvent::Connected {
            hub: ScActiveHub::Failover,
            hub_vmac: FAILOVER_VMAC,
        },
    ];
    for event in events {
        sc(&mut app, ScReport::Event(event));
    }
    tick(&mut app, at(90));
    sc(&mut app, ScReport::Event(LinkEvent::PrimaryRestored));
    sc(
        &mut app,
        ScReport::Event(LinkEvent::Connected {
            hub: ScActiveHub::Primary,
            hub_vmac: PRIMARY_VMAC,
        }),
    );
    sc(
        &mut app,
        ScReport::Status(connected(ScActiveHub::Primary, PRIMARY_VMAC)),
    );
    sc(&mut app, ScReport::Chain(hub_chain()));
    tick(&mut app, at(95));
    app
}

#[test]
fn events_record_the_failover_and_the_return_in_order() {
    let app = after_a_failover();
    use ScActiveHub::{Failover, Primary};
    use ScConnectionState::{Connected, Connecting, Disconnected};
    assert_eq!(
        states(&app),
        [
            (Connecting, Some(Primary)),
            (Connected, Some(Primary)),
            (Disconnected, Some(Primary)),
            (Connecting, Some(Primary)),
            (Connecting, Some(Failover)),
            (Connected, Some(Failover)),
            (Connected, Some(Primary)),
        ]
    );
    // The last dial failure stays for the doctor; the attempt clears on connect.
    let last = app.sc.failure.as_ref().unwrap();
    assert_eq!(last.diagnosis.failed, Some(Phase::Tcp));
    assert_eq!(app.sc.attempt, None);
    let text: Vec<&str> = app.sc.timeline.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(text[0], "lost the primary hub: the hub ended the exchange");
    assert_eq!(
        text[3],
        "primary hub unreachable: switching to the failover hub"
    );
    assert_eq!(
        text[4],
        "connected to the failover hub (hub VMAC 20:20:20:20:20:20)"
    );
    assert_eq!(text[5], "primary hub is back: leaving the failover hub");
}

/// A failed restore probe is a timeline line (#1828); the failover link it
/// leaves in place keeps its state, and the doctor keeps the last dial
/// failure.
#[test]
fn a_failed_restore_probe_is_a_timeline_line_only() {
    let mut app = after_a_failover();
    let before = (states(&app), app.sc.failure.clone(), app.sc.link);
    sc(
        &mut app,
        ScReport::Event(LinkEvent::PrimaryRestoreFailed {
            fault: Fault::WebSocket(ScWebSocketErrorKind::TcpDial),
        }),
    );
    let last = app.sc.timeline.back().unwrap();
    assert_eq!(last.severity, crate::tui::app::sc::Severity::Warn);
    assert_eq!(
        last.text,
        "primary hub still unreachable: nothing accepted the TCP connection"
    );
    assert_eq!((states(&app), app.sc.failure.clone(), app.sc.link), before);
}

/// Failed restore probes in a row with the same cause share one line,
/// with a count and the latest time, so they can't push the loss and
/// failover lines out; a different cause starts a new line.
#[test]
fn repeated_restore_failures_share_one_timeline_line() {
    let mut app = after_a_failover();
    let lines = app.sc.timeline.len();
    let probe = |fault| {
        ScReport::Event(LinkEvent::PrimaryRestoreFailed {
            fault: Fault::WebSocket(fault),
        })
    };
    sc(&mut app, probe(ScWebSocketErrorKind::TcpDial));
    let first_at = app.sc.timeline.back().unwrap().at;
    let later = app.now + Duration::from_secs(30);
    tick(&mut app, later);
    sc(&mut app, probe(ScWebSocketErrorKind::TcpDial));
    sc(&mut app, probe(ScWebSocketErrorKind::TcpDial));
    assert_eq!(app.sc.timeline.len(), lines + 1);
    let last = app.sc.timeline.back().unwrap();
    assert_eq!(last.repeats, 3);
    assert!(last.at > first_at, "the line keeps the latest time");
    app.screen = Screen::Sc;
    assert!(
        screen(&mut app, 120, 40).contains("nothing accepted the TCP connection (3 times)"),
        "{}",
        screen(&mut app, 120, 40)
    );
    sc(&mut app, probe(ScWebSocketErrorKind::TlsHandshake));
    assert_eq!(app.sc.timeline.len(), lines + 2);
    assert_eq!(app.sc.timeline.back().unwrap().repeats, 1);
}

#[test]
fn a_status_poll_resyncs_and_records_only_changes() {
    let mut app = sc_app();
    for _ in 0..3 {
        sc(
            &mut app,
            ScReport::Status(connected(ScActiveHub::Primary, PRIMARY_VMAC)),
        );
    }
    sc(&mut app, ScReport::Gap { missed: 7 });
    sc(
        &mut app,
        ScReport::Status(connected(ScActiveHub::Failover, FAILOVER_VMAC)),
    );
    assert_eq!(
        states(&app),
        [
            (ScConnectionState::Connected, Some(ScActiveHub::Primary)),
            (ScConnectionState::Connected, Some(ScActiveHub::Failover)),
        ]
    );
    assert_eq!(app.sc.missed, 7);
    let gap = app.sc.timeline.back().unwrap();
    assert_eq!(gap.text, "... 7 events missed; status resynced");
    assert!(screen(
        &mut {
            app.screen = Screen::Sc;
            app
        },
        120,
        40
    )
    .contains("Events (7 missed)"));
}

#[test]
fn duplicate_vmac_naks_are_kept_with_the_reseed() {
    let mut app = sc_app();
    sc(&mut app, ScReport::Dialing(identity()));
    let duplicate = |reseeded| Fault::Nak {
        class: ErrorClass::COMMUNICATION,
        code: ErrorCode::NODE_DUPLICATE_VMAC,
        reseeded,
    };
    sc(
        &mut app,
        ScReport::Event(LinkEvent::ConnectFailed {
            hub: ScActiveHub::Primary,
            fault: duplicate(true),
        }),
    );
    let failure = ScFailure {
        fault: duplicate(false),
        detail: "NAK".into(),
        presented: Vec::new(),
    };
    sc(&mut app, ScReport::Failed(Box::new(failure)));
    let history: Vec<_> = app
        .sc
        .duplicates
        .iter()
        .map(|d| (d.hub, d.reseeded))
        .collect();
    assert_eq!(history, [(Some(ScActiveHub::Primary), true), (None, false)]);
    // A failed build opens the doctor on the Connect phase.
    assert_eq!(app.sc.view, View::Doctor);
    assert_eq!(
        app.sc.failure.as_ref().unwrap().diagnosis.failed,
        Some(Phase::Connect)
    );
    app.screen = Screen::Sc;
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("2 NAK(s), last 0s ago"), "{text}");
    assert!(text.contains("[FAIL] Connect   duplicate VMAC"), "{text}");
}

#[test]
fn keys_reach_the_screen_and_switch_views() {
    let mut app = sc_app();
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(app.screen, Screen::Sc);
    assert_eq!(app.sc.view, View::Status);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.sc.view, View::Doctor);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.sc.view, View::Status);
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.screen, Screen::Network);
    press(&mut app, KeyCode::Char('5'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
    // On B/IP the screen says what it is for.
    let mut bip = connected_app();
    press(&mut bip, KeyCode::Char('5'));
    assert!(screen(&mut bip, 120, 40).contains("SC only"));
}

#[test]
fn the_status_bar_names_the_hub_in_use() {
    let mut app = after_a_failover();
    assert!(screen(&mut app, 120, 40).contains("| hub primary |"));
    sc(&mut app, ScReport::Event(LinkEvent::FailoverEngaged));
    assert!(screen(&mut app, 120, 40).contains("| hub connecting |"));
}

#[test]
fn sc_status_frame_at_120x40() {
    let mut app = after_a_failover();
    app.screen = Screen::Sc;
    let text = screen(&mut app, 120, 40);
    // The chain shows the fixture's subject and expiry as given.
    assert!(text.contains("subject CN=primary hub"));
    assert!(text.contains("expires 2031-01-01T00:00:00Z (Unix 1924992000)"));
    assert_snapshot!("sc_status_120x40", text);
}

/// A session whose dial the hub's expired certificate refused.
pub(super) fn refused() -> App {
    let mut app = sc_app();
    sc(&mut app, ScReport::Dialing(identity()));
    tick(&mut app, at(5));
    let failure = ScFailure {
        fault: Fault::Tls(ScTlsFailure::PeerCertificate(
            ScCertificateRejection::Expired {
                by: Some(Duration::from_secs(400 * 86_400)),
            },
        )),
        detail: "BACnet/SC WebSocket tls-handshake failure: WebSocket TLS handshake with \
                 hub.site.example:443 failed: invalid peer certificate: certificate expired"
            .into(),
        presented: vec![CertSummary {
            subject: "CN=primary hub".into(),
            issuer: "CN=site CA".into(),
            not_after_unix: 1_735_689_600,
            not_after: "2025-01-01T00:00:00Z (Unix 1735689600)".into(),
            signature_algorithm: "1.2.840.10045.4.3.2".into(),
        }],
    };
    sc(&mut app, ScReport::Failed(Box::new(failure)));
    worker(
        &mut app,
        WorkerEvent::ConnectFailed {
            error: "TLS handshake failed".into(),
        },
    );
    tick(&mut app, at(8));
    press(&mut app, KeyCode::Char('5'));
    app
}

#[test]
fn sc_doctor_frame_at_120x40() {
    let mut app = refused();
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("[FAIL] TLS       hub certificate expired 400 days ago"),
        "{text}"
    );
    assert!(text.contains("[ -- ] WebSocket"), "{text}");
    assert_snapshot!("sc_doctor_120x40", text);
}
