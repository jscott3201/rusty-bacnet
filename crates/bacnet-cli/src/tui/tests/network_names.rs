//! A foreign-device BBMD named by host name on the TUI (#2308): the status
//! bar, the banner and the Network screen's panel say the device waits for
//! the name, count its lookups and say why the newest failed; once the
//! name resolves, the panel shows the address and the tables follow it.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bbmd_lookup::BbmdLookupFailure;
use bacnet_transport::bip::{ForeignBbmd, ForeignRegistrationOutcome, ForeignRegistrationSnapshot};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::network::{Foreign, NetworkEvent};

fn named() -> ForeignBbmd {
    "bbmd.example".parse().unwrap()
}

const RESOLVED: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 4, 1), 47808);

/// A session whose BBMD's name hasn't resolved, on the Network screen.
fn waiting_app() -> App {
    let mut app = connected_app();
    let foreign = Foreign {
        configured: named(),
        bbmd: None,
        ttl: 60,
        renewal: Duration::from_secs(30),
        nat_global: None,
    };
    worker(&mut app, WorkerEvent::ForeignDevice(foreign));
    press(&mut app, KeyCode::Char('4'));
    app
}

/// The device still waits after `failures` failed lookups.
fn waiting(failures: u64) -> ForeignRegistrationSnapshot {
    let mut snapshot = ForeignRegistrationSnapshot::default();
    snapshot.waiting_for_bbmd = true;
    snapshot.bbmd_lookups = failures;
    snapshot.bbmd_lookup_failures = failures;
    snapshot.last_bbmd_lookup_failure = Some(BbmdLookupFailure::Error(io::ErrorKind::NotFound));
    snapshot
}

fn report(app: &mut App, snapshot: ForeignRegistrationSnapshot) {
    worker(
        app,
        WorkerEvent::Network(NetworkEvent::Registration(snapshot)),
    );
}

#[test]
fn a_device_waiting_for_its_bbmds_name_says_so() {
    let mut app = waiting_app();
    // Before the first report, and with the first lookup still out: no
    // banner yet.
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD waiting for name"), "{text}");
    assert!(
        text.contains("bbmd.example:47808 (not resolved yet)"),
        "{text}"
    );
    assert_eq!(app.network.registration_banner(), None);

    report(&mut app, waiting(2));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("waiting for the BBMD's host name to resolve"),
        "{text}"
    );
    assert!(
        text.contains("2 name lookups, 2 failed: the name did not resolve"),
        "{text}"
    );
    assert!(
        text.contains("Next registration: once the BBMD's host name resolves"),
        "{text}"
    );
    let banner = app.network.registration_banner().unwrap();
    assert!(
        banner.starts_with(
            " WAITING The BBMD's host name bbmd.example:47808 did not resolve (entity not found);"
        ),
        "{banner}"
    );
    assert!(
        banner.contains("trying again 30 s after each failure"),
        "{banner}"
    );
}

#[test]
fn a_name_that_resolves_shows_its_address_and_the_tables_follow_it() {
    let mut app = waiting_app();
    assert_eq!(app.network.target, None);
    report(&mut app, waiting(2));
    assert!(app.network.registration_banner().is_some());
    worker(&mut app, WorkerEvent::Network(NetworkEvent::Bbmd(RESOLVED)));
    assert_eq!(app.network.target, Some(RESOLVED));
    let mut snapshot = ForeignRegistrationSnapshot::default();
    snapshot.bbmd_lookups = 3;
    snapshot.bbmd_lookup_failures = 2;
    snapshot.last_bbmd = Some(RESOLVED);
    // The first attempt to the address is out.
    snapshot.attempts = 1;
    snapshot.last_outcome = Some(ForeignRegistrationOutcome::Pending);
    report(&mut app, snapshot);
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("bbmd.example:47808 (10.0.4.1:47808)"),
        "{text}"
    );
    assert!(text.contains("3 name lookups, 2 failed"), "{text}");
    assert!(!text.contains("the name did not resolve"), "{text}");
    assert!(!text.contains("FD waiting for name"), "{text}");
    // The WAITING banner went with the wait.
    assert_eq!(app.network.registration_banner(), None);

    // The name moves: the tables shown follow it, unless `:bbmd` chose
    // another BBMD's.
    let moved = SocketAddrV4::new(Ipv4Addr::new(10, 0, 5, 1), 47808);
    worker(&mut app, WorkerEvent::Network(NetworkEvent::Bbmd(moved)));
    assert_eq!(app.network.target, Some(moved));
    let other = SocketAddrV4::new(Ipv4Addr::new(10, 0, 6, 1), 47808);
    app.network.target = Some(other);
    worker(&mut app, WorkerEvent::Network(NetworkEvent::Bbmd(RESOLVED)));
    assert_eq!(app.network.target, Some(other));
    assert_eq!(
        app.network.foreign.as_ref().and_then(|f| f.bbmd),
        Some(RESOLVED)
    );
}
