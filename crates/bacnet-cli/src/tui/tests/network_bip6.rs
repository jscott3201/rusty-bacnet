//! A B/IPv6 foreign device on the TUI (#2378): the status bar's `FD`
//! field, the banners and the Network screen's panel show its registration
//! from the status its transport reports, as they show B/IP's: held,
//! refused or unanswered by the BBMD (#2388), waiting for the BBMD's host
//! name, or waiting after the name failed to resolve. B/IP's frames are
//! pinned in `network_frames` and `network_names`.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, UdpSocket};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::target::AddressStyle;
use bacnet_transport::bbmd_lookup::BbmdLookupFailure;
use bacnet_transport::bip6::{Bip6ForeignDeviceConfig, Bip6ForeignStatus, Bip6RegistrationAnswer};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;
use crate::tui::app::network::Registration;
use crate::tui::network::NetworkEvent;
use crate::tui::worker::EventSink;

const BBMD: SocketAddrV6 = SocketAddrV6::new(
    Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10),
    47808,
    0,
    0,
);

/// A B/IPv6 session, up, on the Network screen.
fn bip6_app() -> App {
    let mut app = App::new(
        AppConfig {
            transport: "BIP6",
            style: AddressStyle::Bip6,
            color: false,
            picker: None,
            join: None,
            passive: false,
            now: t0(),
        },
        LogRing::new(10),
    );
    let local = "[2001:db8::5]:47808".into();
    worker(&mut app, WorkerEvent::Connected { local });
    press(&mut app, KeyCode::Char('4'));
    app
}

/// A foreign device whose BBMD is `bbmd.example:47808`, as `edit` leaves it.
fn status(edit: impl FnOnce(&mut Bip6ForeignStatus)) -> Bip6ForeignStatus {
    let mut status = Bip6ForeignStatus::default();
    status.ttl = 60;
    status.bbmd_name = Some("bbmd.example".parse().unwrap());
    edit(&mut status);
    status
}

fn report(app: &mut App, status: Bip6ForeignStatus) {
    let event = NetworkEvent::Bip6Registration(status);
    worker(app, WorkerEvent::Network(event));
}

/// The name resolved and three registrations went there, the latest
/// accepted 12 s ago; one release of a BBMD left was answered.
fn registered() -> Bip6ForeignStatus {
    status(|s| {
        s.bbmd = Some(BBMD);
        s.registrations_sent = 3;
        s.last_answer = Some(Bip6RegistrationAnswer::Held);
        s.last_answer_age = Some(Duration::from_secs(12));
        s.lookups.lookups = 2;
        s.releases.releases = 1;
        s.releases.released = 1;
    })
}

/// The BBMD refused the latest of four registrations 4 s ago, and an
/// earlier one too.
fn refused() -> Bip6ForeignStatus {
    status(|s| {
        s.bbmd = Some(BBMD);
        s.registrations_sent = 4;
        s.registrations_refused = 2;
        s.last_answer = Some(Bip6RegistrationAnswer::Refused);
        s.last_answer_age = Some(Duration::from_secs(4));
        s.lookups.lookups = 2;
    })
}

/// Waiting for the name, its first lookup still out.
fn waiting() -> Bip6ForeignStatus {
    status(|s| s.waiting_for_bbmd = true)
}

/// Still waiting after two lookups found no IPv6 address, with four
/// broadcasts refused meanwhile.
fn failed() -> Bip6ForeignStatus {
    status(|s| {
        s.waiting_for_bbmd = true;
        s.lookups.lookups = 2;
        s.lookups.failures = 2;
        s.lookups.last_failure = Some(BbmdLookupFailure::NoAddress);
        s.broadcasts_refused = 4;
    })
}

/// The BBMD accepted the latest registration: `FD held`, no banner, and
/// the panel says how long ago, counting on from the report (#2388).
#[test]
fn a_registered_bip6_device_120x40() {
    let mut app = bip6_app();
    report(&mut app, registered());
    assert_eq!(app.network.registration_state(), Some(Registration::Held));
    assert_eq!(app.network.registration_banner(), None);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD held"), "{text}");
    assert!(
        text.contains("BBMD bbmd.example:47808 ([2001:db8::10]:47808)   TTL 60 s"),
        "{text}"
    );
    assert!(
        text.contains("Status registered, accepted 12 s ago   2 name lookups, 0 failed"),
        "{text}"
    );
    assert!(
        text.contains("Registrations sent 3, refused 0, to every BBMD this device has used"),
        "{text}"
    );
    assert!(text.contains("1 asked, 1 released"), "{text}");
    assert_snapshot!("network_bip6_registered_120x40", text);
    assert_snapshot!("network_bip6_registered_80x24", screen(&mut app, 80, 24));
    advance(&mut app, Duration::from_secs(3));
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("registered, accepted 15 s ago"), "{text}");
}

/// The BBMD refused the latest registration: `FD refused` and B/IP's red
/// banner, naming the B/IPv6 NAK, on every screen (#2388).
#[test]
fn a_refused_bip6_device_120x40() {
    let mut app = bip6_app();
    report(&mut app, refused());
    assert_eq!(
        app.network.registration_state(),
        Some(Registration::Refused)
    );
    let banner = app.network.registration_banner().unwrap();
    assert_eq!(
        banner,
        " REFUSED BBMD [2001:db8::10]:47808 refused the foreign-device registration \
         (register-foreign-device-nak); broadcasts through it fail until it accepts. \
         Check its foreign-device policy."
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD refused"), "{text}");
    assert!(text.contains("Status REFUSED 4 s ago"), "{text}");
    assert!(text.contains("Registrations sent 4, refused 2,"), "{text}");
    assert_snapshot!("network_bip6_refused_120x40", text);
    press(&mut app, KeyCode::Char('1'));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains(" REFUSED BBMD [2001:db8::10]:47808"),
        "{text}"
    );
}

/// The BBMD left the last two registrations unanswered: `FD no answer`
/// and B/IP's NO ANSWER banner, counting them (#2388).
#[test]
fn an_unanswered_bip6_device() {
    let mut app = bip6_app();
    report(
        &mut app,
        status(|s| {
            s.bbmd = Some(BBMD);
            s.registrations_sent = 2;
            s.last_answer = Some(Bip6RegistrationAnswer::Unanswered);
            s.last_answer_age = Some(Duration::ZERO);
            s.unanswered = 2;
        }),
    );
    assert_eq!(
        app.network.registration_state(),
        Some(Registration::NoAnswer { unanswered: 2 })
    );
    let banner = app.network.registration_banner().unwrap();
    assert!(
        banner.starts_with(
            " NO ANSWER BBMD [2001:db8::10]:47808 has not answered 2 foreign-device registrations;"
        ),
        "{banner}"
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD no answer"), "{text}");
    assert!(
        text.contains("Status no answer to the last 2 registrations"),
        "{text}"
    );
}

#[test]
fn a_waiting_bip6_device_120x40() {
    let mut app = bip6_app();
    report(&mut app, waiting());
    assert_eq!(
        app.network.registration_state(),
        Some(Registration::Waiting)
    );
    // The first lookup is still out: no banner yet, as on B/IP.
    assert_eq!(app.network.registration_banner(), None);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD waiting for name"), "{text}");
    assert!(
        text.contains("bbmd.example:47808 (not resolved yet)"),
        "{text}"
    );
    assert!(
        text.contains("Next registration: once the BBMD's host name resolves"),
        "{text}"
    );
    assert_snapshot!("network_bip6_waiting_120x40", text);
}

#[test]
fn a_bip6_device_whose_name_failed_120x40() {
    let mut app = bip6_app();
    report(&mut app, failed());
    let banner = app.network.registration_banner().unwrap();
    assert!(
        banner.starts_with(" WAITING The BBMD's host name bbmd.example:47808 has no IPv6 address;"),
        "{banner}"
    );
    assert!(
        banner.ends_with("broadcasts are refused meanwhile."),
        "{banner}"
    );
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("broadcasts refused 4  2 name lookups, 2 failed: the name has no IPv6"),
        "{text}"
    );
    assert_snapshot!("network_bip6_failed_120x40", text);

    // The name resolves: the banner goes, and the panel shows the address
    // and the registration waiting for its answer.
    report(
        &mut app,
        status(|s| {
            s.bbmd = Some(BBMD);
            s.registrations_sent = 1;
            s.lookups.lookups = 3;
            s.lookups.failures = 2;
        }),
    );
    assert_eq!(app.network.registration_banner(), None);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD pending"), "{text}");
    assert!(
        text.contains("Status registration sent, waiting for the BBMD's answer"),
        "{text}"
    );
    assert!(text.contains("([2001:db8::10]:47808)"), "{text}");
}

/// A BBMD known by address shows the address alone, and reads pending
/// until the first registration goes.
#[test]
fn a_bip6_bbmd_known_by_address() {
    let mut app = bip6_app();
    report(
        &mut app,
        status(|s| {
            s.bbmd_name = None;
            s.bbmd = Some(BBMD);
        }),
    );
    assert_eq!(
        app.network.registration_state(),
        Some(Registration::Pending)
    );
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("BBMD [2001:db8::10]:47808   TTL 60 s"),
        "{text}"
    );
    assert!(text.contains("FD pending"), "{text}");
    assert!(text.contains("first registration not sent yet"), "{text}");
}

/// A name that resolved and later failed to resolve: the device keeps
/// registering with the address it had, so no WAITING banner, and `FD`
/// still reads the BBMD's answer; the counts say why the newest lookup
/// failed.
#[test]
fn a_resolved_name_that_later_fails_keeps_its_address() {
    let mut app = bip6_app();
    report(
        &mut app,
        status(|s| {
            s.bbmd = Some(BBMD);
            s.registrations_sent = 2;
            s.last_answer = Some(Bip6RegistrationAnswer::Held);
            s.last_answer_age = Some(Duration::from_secs(1));
            s.lookups.lookups = 3;
            s.lookups.failures = 1;
            s.lookups.last_failure = Some(BbmdLookupFailure::Error(std::io::ErrorKind::NotFound));
        }),
    );
    assert_eq!(app.network.registration_state(), Some(Registration::Held));
    assert_eq!(app.network.registration_banner(), None);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("FD held"), "{text}");
    assert!(!text.contains("WAITING"), "{text}");
    assert!(
        text.contains("3 name lookups, 1 failed: the name did not resolve"),
        "{text}"
    );
}

/// A stopped device whose name never resolved has no address and no wait:
/// it reads as waiting for the name, without a banner until a lookup fails.
#[test]
fn a_stopped_bip6_device_without_an_address_reads_waiting() {
    let mut app = bip6_app();
    report(&mut app, status(|_| {}));
    assert_eq!(
        app.network.registration_state(),
        Some(Registration::Waiting)
    );
    assert_eq!(app.network.registration_banner(), None);
}

/// On B/IPv6 nothing reads BBMD tables: `:bbmd`, `r` and `m` say B/IP
/// only, even with a registration, and the footer offers none of them.
#[test]
fn bbmd_tables_are_b_ip_only_on_bip6() {
    let mut app = bip6_app();
    report(&mut app, registered());
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    let flash = app.flash.take().unwrap().text;
    assert_eq!(flash, "BBMD tables are read on BACnet/IP only.");
    for key in ['r', 'm'] {
        assert!(press(&mut app, KeyCode::Char(key)).is_empty(), "{key}");
        let flash = app.flash.take().unwrap().text;
        assert!(flash.contains("BACnet/IP only"), "{key}: {flash}");
    }
    let text = screen(&mut app, 120, 40);
    let footer = text.lines().last().unwrap();
    for gone in ["read tables", "survey", ":bbmd"] {
        assert!(!footer.contains(gone), "{gone}: {footer}");
    }
    assert!(footer.contains("Tab networks"), "{footer}");
}

/// The network worker polls a real B/IPv6 foreign device's status, the one
/// the CLI's one-shot wait reads, and sends it to the UI.
#[tokio::test]
async fn the_network_worker_reports_a_bip6_foreign_device() {
    let silent = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V6(bbmd) = silent.local_addr().unwrap() else {
        panic!("bound on ::1");
    };
    let mut client = BACnetClient::bip6_builder()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        .device_instance(4321)
        .foreign_device(Bip6ForeignDeviceConfig {
            bbmd: bbmd.into(),
            ttl: 60,
        })
        .build()
        .await
        .unwrap();
    let (sink, mut events) = EventSink::channel(16);
    let (commands, command_rx) = mpsc::channel(4);
    let reported = async {
        loop {
            let event = events.recv().await.expect("the worker is running");
            if let WorkerEvent::Network(NetworkEvent::Bip6Registration(status)) = event {
                return status;
            }
        }
    };
    let status = tokio::select! {
        () = crate::tui::worker::network::run(&client, &sink, command_rx) => {
            panic!("the worker ended")
        }
        status = timeout(Duration::from_secs(10), reported) => status.expect("a status in time"),
    };
    assert_eq!(status.bbmd, Some(bbmd));
    assert_eq!(status.ttl, 60);
    assert!(!status.waiting_for_bbmd);
    drop(commands);
    client.stop().await.unwrap();
}
