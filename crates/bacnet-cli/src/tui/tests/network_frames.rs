//! The Network screen's frames at 80x24, 120x40 and 200x50, from synthetic
//! worker events at fixed times, and the app's side of it: keys, the
//! countdown, the registration banners and what a read asks the worker for.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};
use bacnet_transport::bvll::BvlcResultCode;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::network::View;
use crate::tui::app::watch::Screen;
use crate::tui::network::{BdtRow, FdtRow, Foreign, NetworkCommand, NetworkEvent};
use bacnet_ops::bbmd::Probe;

pub(super) fn at_ip(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
}

pub(super) const LOCAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 9, 5), 47808);

fn foreign() -> Foreign {
    Foreign {
        bbmd: at_ip(1),
        ttl: 60,
        renewal: Duration::from_secs(30),
    }
}

fn snapshot(outcome: ForeignRegistrationOutcome) -> ForeignRegistrationSnapshot {
    let mut s = ForeignRegistrationSnapshot::default();
    s.last_bbmd = Some(at_ip(1));
    s.last_ttl = Some(60);
    s.last_outcome = Some(outcome);
    s.attempts = 2;
    if outcome == ForeignRegistrationOutcome::Refused {
        s.refused = 2;
        s.last_result = Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
        s.next_attempt_in = Some(Duration::from_secs(30));
    } else {
        s.accepted = 2;
        s.last_result = Some(BvlcResultCode::SUCCESSFUL_COMPLETION);
        s.registered = true;
        s.lease_remaining = Some(Duration::from_secs(48));
        s.next_attempt_in = Some(Duration::from_secs(18));
    }
    s
}

pub(super) fn bdt_row(last: u8, mask: [u8; 4]) -> BdtRow {
    BdtRow {
        address: at_ip(last),
        mask: Ipv4Addr::from(mask),
    }
}

pub(super) fn fdt_row(address: SocketAddrV4, remaining: u16) -> FdtRow {
    FdtRow {
        address,
        ttl: 60,
        remaining,
    }
}

/// Registered with 10.0.1.1 at 10 s, its tables read at 12 s, viewed at
/// 15 s.
pub(super) fn registered() -> App {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    press(&mut app, KeyCode::Char('4'));
    tick(&mut app, at(10));
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Accepted));
    worker(&mut app, WorkerEvent::Network(event));
    tick(&mut app, at(12));
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(vec![
            bdt_row(1, [255; 4]),
            bdt_row(2, [255; 4]),
            bdt_row(3, [255, 255, 255, 0]),
        ]),
        fdt: Ok(vec![
            fdt_row(SocketAddrV4::new(Ipv4Addr::new(10, 0, 7, 9), 47808), 40),
            fdt_row(LOCAL, 88),
        ]),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    tick(&mut app, at(15));
    app
}

#[test]
fn foreign_device_panel_and_bdt_120x40() {
    let mut app = registered();
    assert_eq!(app.network.own, Some(1));
    // 18 s at 10 s, so 13 s at 15 s.
    assert_eq!(
        app.network.next_attempt(app.now),
        Some(Duration::from_secs(13))
    );
    assert_eq!(
        app.network.lease_left(app.now),
        Some(Duration::from_secs(43))
    );
    assert_snapshot!("network_fd_bdt_120x40", screen(&mut app, 120, 40));
}

#[test]
fn fdt_view_marks_our_entry_120x40() {
    let mut app = registered();
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.network.view, View::Fdt);
    let text = screen(&mut app, 120, 40);
    // 88 s left at 12 s, so 85 s at 15 s.
    assert!(
        text.contains("10.0.9.5:47808         60 s      85 s      this client"),
        "{text}"
    );
    assert_snapshot!("network_fdt_120x40", text);
}

#[test]
fn matrix_flags_the_missing_entry_120x40() {
    let mut app = registered();
    let commands = press(&mut app, KeyCode::Char('m'));
    assert_eq!(
        commands,
        [Command::Network(NetworkCommand::Survey { seed: at_ip(1) })]
    );
    let probes = vec![
        (
            at_ip(1),
            Probe::Table(vec![bdt_row(1, [255; 4]), bdt_row(2, [255; 4])]),
        ),
        (at_ip(2), Probe::Table(vec![bdt_row(2, [255; 4])])),
    ];
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
        not_followed: Vec::new(),
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("#2 10.0.2.1:47808         MISS  uni"),
        "{text}"
    );
    assert!(
        text.contains(
            "10.0.1.1:47808 lists 10.0.2.1:47808, but 10.0.2.1:47808 does not list 10.0.1.1:47808"
        ),
        "{text}"
    );
    assert_snapshot!("network_matrix_120x40", text);
}

/// A lone BBMD with an empty BDT (one serving only foreign devices) shows
/// the note, not the sentence that every BDT lists itself.
#[test]
fn an_empty_bdt_shows_its_note_120x40() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes: vec![(at_ip(1), Probe::Table(Vec::new()))],
        not_followed: Vec::new(),
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("10.0.1.1:47808 has an empty BDT: it forwards to no peer BBMD"),
        "{text}"
    );
    assert!(!text.contains("BDTs agree"), "{text}");
    assert_snapshot!("network_matrix_empty_120x40", text);
}

#[test]
fn a_refusal_shows_a_banner_on_every_screen() {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Refused));
    worker(&mut app, WorkerEvent::Network(event));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("REFUSED BBMD 10.0.1.1:47808 refused the foreign-device registration (register-foreign-device-nak)"),
        "{text}"
    );
    press(&mut app, KeyCode::Char('4'));
    assert_snapshot!("network_refused_120x40", screen(&mut app, 120, 40));
    // Accepted again: the banner goes.
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Accepted));
    worker(&mut app, WorkerEvent::Network(event));
    assert_eq!(app.network.registration_banner(), None);
}

#[test]
fn keys_read_the_target_and_need_a_bbmd() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.screen, Screen::Network);
    assert!(press(&mut app, KeyCode::Char('r')).is_empty());
    assert!(app.flash.as_ref().unwrap().text.contains(":bbmd ADDRESS"));
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    let commands = press(&mut app, KeyCode::Enter);
    assert_eq!(
        commands,
        [Command::Network(NetworkCommand::Tables { bbmd: at_ip(4) })]
    );
    assert!(app.network.reading);
    // An answer for another BBMD is not shown as this one's.
    let stale = NetworkEvent::Tables {
        bbmd: at_ip(5),
        local: LOCAL,
        bdt: Ok(Vec::new()),
        fdt: Err("unreachable: no answer".into()),
    };
    worker(&mut app, WorkerEvent::Network(stale));
    assert_eq!(app.network.bdt, None);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.network.view, View::Networks);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.network.view, View::Matrix);
    assert_eq!(
        press(&mut app, KeyCode::Char('r')),
        [Command::Network(NetworkCommand::Survey { seed: at_ip(4) })]
    );
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Main);
}

#[test]
fn other_transports_and_passive_sessions_read_nothing() {
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
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "[::1]:47808".into(),
        },
    );
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    assert!(app.flash.as_ref().unwrap().text.contains("BACnet/IP"));
    assert!(screen(&mut app, 120, 40).contains("B/IP only"));

    let mut passive = App::new(
        AppConfig {
            transport: "BIP",
            style: AddressStyle::Bip,
            color: false,
            picker: None,
            join: None,
            passive: true,
            now: t0(),
        },
        LogRing::new(10),
    );
    passive.network.target = Some(at_ip(1));
    press(&mut passive, KeyCode::Char('4'));
    assert!(press(&mut passive, KeyCode::Char('m')).is_empty());
}

#[test]
fn answers_for_an_earlier_target_are_dropped() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    assert!(app.network.surveying);
    // `:bbmd` moves to another BBMD while the survey from 10.0.1.1 runs.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    press(&mut app, KeyCode::Enter);
    assert!(app.network.reading);
    let late = NetworkEvent::Survey {
        seed: at_ip(1),
        probes: vec![(at_ip(1), Probe::Table(vec![bdt_row(1, [255; 4])]))],
        not_followed: Vec::new(),
    };
    worker(&mut app, WorkerEvent::Network(late));
    assert_eq!(app.network.matrix, None, "the old survey's matrix shows");
    // The automatic read of the registration's BBMD, or a refusal of an
    // earlier request, leaves the new read waiting.
    let auto = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(Vec::new()),
        fdt: Ok(Vec::new()),
    };
    worker(&mut app, WorkerEvent::Network(auto));
    let refused = NetworkEvent::Refused {
        command: NetworkCommand::Survey { seed: at_ip(1) },
        reason: "busy".into(),
    };
    worker(&mut app, WorkerEvent::Network(refused));
    assert!(app.network.reading);
    let refused = NetworkEvent::Refused {
        command: NetworkCommand::Tables { bbmd: at_ip(4) },
        reason: "busy".into(),
    };
    worker(&mut app, WorkerEvent::Network(refused));
    assert!(!app.network.reading);
}

#[test]
fn a_large_matrix_shows_what_fits_and_counts_the_rest() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let everyone: Vec<BdtRow> = (1..=20).map(|last| bdt_row(last, [255; 4])).collect();
    let probes = (1..=20)
        .map(|last| (at_ip(last), Probe::Table(everyone.clone())))
        .collect();
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
        not_followed: Vec::new(),
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("#14   +6 more"), "{text}");
    assert!(!text.contains("#15"), "{text}");
    assert!(text.contains("#14 10.0.14.1:47808"), "{text}");
    assert!(text.contains("All 20 BDTs agree"), "{text}");
}

#[test]
fn the_panel_and_the_matrix_legend_fit_80x24() {
    let mut app = registered();
    let text = screen(&mut app, 80, 24);
    // The counters take their own line when the status line is full.
    assert!(
        text.contains("Counts attempts 2  accepted 2  refused 0  lapses 0"),
        "{text}"
    );
    assert_snapshot!("network_fd_bdt_80x24", text);
    press(&mut app, KeyCode::Char('m'));
    let both = vec![bdt_row(1, [255; 4]), bdt_row(2, [255; 4])];
    let probes = vec![
        (at_ip(1), Probe::Table(both.clone())),
        (at_ip(2), Probe::Table(both)),
    ];
    let survey = NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
        not_followed: Vec::new(),
    };
    worker(&mut app, WorkerEvent::Network(survey));
    let text = screen(&mut app, 80, 24);
    // Every legend entry is whole, the last one included.
    assert!(text.contains("? = not read"), "{text}");
    assert_snapshot!("network_matrix_80x24", text);
}

/// After `:bbmd` names another BBMD, its FDT is not expected to hold us:
/// no NAT warning, and the panel says whose tables these are.
#[test]
fn another_bbmds_tables_do_not_judge_our_entry() {
    let mut app = registered();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    press(&mut app, KeyCode::Enter);
    let other = SocketAddrV4::new(Ipv4Addr::new(10, 0, 7, 9), 47808);
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(4),
        local: LOCAL,
        bdt: Ok(vec![bdt_row(4, [255; 4]), bdt_row(1, [255; 4])]),
        fdt: Ok(vec![fdt_row(other, 40)]),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    assert!(!app.network.tables_are_ours());
    let text = screen(&mut app, 120, 40);
    assert!(!text.contains("NAT"), "{text}");
    assert!(
        text.contains("Our FDT entry not checked: tables are from 10.0.4.1:47808, not our BBMD"),
        "{text}"
    );
    assert!(text.contains("| 10.0.4.1:47808, not our BBMD |"), "{text}");
    assert_snapshot!("network_other_bbmd_120x40", text);
    assert_snapshot!("network_other_bbmd_80x24", screen(&mut app, 80, 24));
    // Back on our BBMD, our entry is judged again.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.1.1");
    press(&mut app, KeyCode::Enter);
    assert!(app.network.tables_are_ours());
}

/// A BBMD that never answers: the banner names it and counts the attempts
/// that ended unanswered, not the one still out.
#[test]
fn an_unanswered_registration_shows_a_banner() {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    let mut s = snapshot(ForeignRegistrationOutcome::Pending);
    s.accepted = 0;
    s.registered = false;
    s.lease_remaining = None;
    s.last_result = None;
    // The first attempt is still out: nothing to warn about yet.
    s.attempts = 1;
    worker(
        &mut app,
        WorkerEvent::Network(NetworkEvent::Registration(s)),
    );
    assert_eq!(app.network.registration_banner(), None);
    s.attempts = 2;
    worker(
        &mut app,
        WorkerEvent::Network(NetworkEvent::Registration(s)),
    );
    let banner = app.network.registration_banner().unwrap();
    assert!(
        banner.contains(
            "NO ANSWER BBMD 10.0.1.1:47808 has not answered 1 foreign-device registration;"
        ),
        "{banner}"
    );
    assert_snapshot!("network_unanswered_120x40", screen(&mut app, 120, 40));
    press(&mut app, KeyCode::Char('4'));
    assert_snapshot!("network_unanswered_80x24", screen(&mut app, 80, 24));
    // Accepted: the banner goes.
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Accepted));
    worker(&mut app, WorkerEvent::Network(event));
    assert_eq!(app.network.registration_banner(), None);
    // Later the lease lapses with no newer success.
    let mut s = snapshot(ForeignRegistrationOutcome::TimedOut);
    s.registered = false;
    s.lease_remaining = None;
    s.lapses = 1;
    worker(
        &mut app,
        WorkerEvent::Network(NetworkEvent::Registration(s)),
    );
    let banner = app.network.registration_banner().unwrap();
    assert!(
        banner.contains("LAPSED The registration with BBMD 10.0.1.1:47808 lapsed"),
        "{banner}"
    );
    // A later success clears it; the lapse stays counted.
    let mut s = snapshot(ForeignRegistrationOutcome::Accepted);
    s.lapses = 1;
    worker(
        &mut app,
        WorkerEvent::Network(NetworkEvent::Registration(s)),
    );
    assert_eq!(app.network.registration_banner(), None);
}

/// From 160 columns the BDT and FDT sit side by side, with the matrix
/// beneath them; Tab moves the accent, not the layout.
#[test]
fn a_wide_terminal_shows_the_tables_side_by_side_200x50() {
    let mut app = registered();
    let text = screen(&mut app, 200, 50);
    let header = text
        .lines()
        .find(|line| line.contains("How peers forward"))
        .unwrap_or_default();
    assert!(header.contains("Foreign device"), "{text}");
    assert!(text.contains("┌ Matrix "), "{text}");
    assert!(text.contains("Tab focus pane/networks"), "{text}");
    assert_snapshot!("network_wide_200x50", text);
    assert!(pane_is_accented(&mut app, "BDT"));
    assert!(!pane_is_accented(&mut app, "FDT"));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.network.view, View::Fdt);
    assert!(side_by_side(&mut app, 200, 50));
    assert!(pane_is_accented(&mut app, "FDT"));
    assert!(!pane_is_accented(&mut app, "BDT"));
    // Below the breakpoint the tabs are back, and Tab switches them.
    let text = screen(&mut app, 159, 40);
    assert!(!text.contains("How peers forward"), "{text}");
    assert!(text.contains("Tab BDT/FDT/matrix/networks"), "{text}");
}

/// Whether the BDT's and FDT's header lines share a row.
fn side_by_side(app: &mut App, width: u16, height: u16) -> bool {
    screen(app, width, height)
        .lines()
        .any(|line| line.contains("How peers forward") && line.contains("Foreign device"))
}

/// Whether the wide layout's `name` pane has the accent (bold without
/// colour) on its title.
fn pane_is_accented(app: &mut App, name: &str) -> bool {
    let terminal = render(app, 200, 50);
    let buffer = terminal.backend().buffer();
    let title = format!("┌ {name} ");
    for y in 0..buffer.area.height {
        let row: Vec<&str> = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let joined = row.concat();
        if let Some(at) = joined.find(&title) {
            let x = joined[..at].chars().count() + 2;
            return buffer[(x as u16, y)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD);
        }
    }
    panic!("no {name} pane")
}

/// At 160 and 170 columns this BDT does not fit whole beside the FDT, so
/// the tables stay stacked and no row is cut.
#[test]
fn the_tables_stay_stacked_until_the_bdt_fits_beside_the_fdt() {
    let mut app = registered();
    for width in [160, 170] {
        assert!(!side_by_side(&mut app, width, 40), "{width}");
        let text = screen(&mut app, width, 40);
        for row in [
            "(this BBMD) peers unicast to it, and it rebroadcasts on its subnet (two-hop)",
            "peers send a directed broadcast to 10.0.3.255 (one-hop; routers must pass it)",
        ] {
            assert!(text.contains(row), "{width}: {text}");
        }
    }
    assert!(side_by_side(&mut app, 180, 40));
    // The matrix needs room beneath: at 30 rows the BDT and FDT still sit
    // side by side, but the matrix view shows the matrix alone.
    let text = screen(&mut app, 200, 30);
    assert!(side_by_side(&mut app, 200, 30));
    assert!(!text.contains("┌ Matrix "), "{text}");
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.network.view, View::Matrix);
    let text = screen(&mut app, 200, 30);
    assert!(!text.contains("How peers forward"), "{text}");
    assert!(text.contains("Tab BDT/FDT/matrix/networks"), "{text}");
}

/// A table longer than its pane says how many rows it hides, in the
/// tabbed and the wide layouts.
#[test]
fn a_long_table_counts_the_rows_it_hides() {
    let mut app = registered();
    let peer = |i: u16| {
        SocketAddrV4::new(
            Ipv4Addr::new(10, 1, (i / 200) as u8, (i % 200) as u8),
            47808,
        )
    };
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok((1..=40).map(|last| bdt_row(last, [255; 4])).collect()),
        fdt: Ok((0..30).map(|i| fdt_row(peer(i), 50)).collect()),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    // 29 rows inside the border at 120x40: the header, 27 entries and the
    // count of the other 13.
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("+13 more"), "{text}");
    assert!(text.contains("10.0.27.1:47808"), "{text}");
    assert!(!text.contains("10.0.28.1:47808"), "{text}");
    // Wide, each pane holds 17 rows: the header, 15 entries and the count.
    let text = screen(&mut app, 200, 50);
    assert!(text.contains("+25 more"), "BDT: {text}");
    assert!(text.contains("+15 more"), "FDT: {text}");
}

/// The banner wraps between words with its height counted from the
/// wrapped lines, so its end is never cut, and continuation lines keep a
/// one-space indent.
#[test]
fn the_refusal_banner_is_whole_at_every_width() {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    let event = NetworkEvent::Registration(snapshot(ForeignRegistrationOutcome::Refused));
    worker(&mut app, WorkerEvent::Network(event));
    for width in 80..=130 {
        let text = screen(&mut app, width, 24);
        assert!(text.contains("foreign-device policy."), "{width}: {text}");
        let second = text.lines().nth(2).unwrap_or_default();
        assert!(second.starts_with("\" "), "{width}: {text}");
    }
}

#[test]
fn local_failures_are_not_the_bbmds_silence() {
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::ForeignDevice(foreign()));
    for outcome in [
        ForeignRegistrationOutcome::LocalError,
        ForeignRegistrationOutcome::Cancelled,
    ] {
        let mut s = snapshot(outcome);
        s.accepted = 0;
        s.registered = false;
        s.lease_remaining = None;
        s.last_result = None;
        s.attempts = 1;
        let event = NetworkEvent::Registration(s);
        worker(&mut app, WorkerEvent::Network(event));
        assert_eq!(app.network.registration_banner(), None, "{outcome:?}");
    }
}
