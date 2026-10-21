//! The footer and the status bar (#1894): notices that go with the screen
//! or dialog they were about, hints fitted by priority so `? help` and
//! `q quit` always stay, a failed connection's short summary, the
//! registration state behind `FD`, and keys listed only where they work.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};
use bacnet_types::error::Error;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::network::Registration;
use crate::tui::app::watch::Screen;
use crate::tui::app::{Link, Overlay};
use crate::tui::message::Failure;
use crate::tui::network::{Foreign, NetworkEvent};

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 50)];

/// The status bar and the footer of a `width` x `height` frame.
fn chrome(app: &mut App, width: u16, height: u16) -> (String, String) {
    let text = screen(app, width, height);
    let rows: Vec<&str> = text.lines().map(|row| row.trim_matches('"')).collect();
    (rows[0].to_string(), rows[rows.len() - 1].to_string())
}

/// Both rows at each size, for a snapshot.
fn chrome_at_sizes(app: &mut App) -> String {
    let mut out = String::new();
    for (w, h) in SIZES {
        let (status, footer) = chrome(app, w, h);
        out.push_str(&format!("{w}x{h}\n{status}\n{footer}\n"));
    }
    out
}

fn bbmd() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 1), 47808)
}

/// A session registering with 10.0.1.1, on the Network screen.
fn foreign_app() -> App {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::ForeignDevice(Foreign {
            bbmd: bbmd(),
            ttl: 60,
            renewal: Duration::from_secs(30),
        }),
    );
    press(&mut app, KeyCode::Char('4'));
    app
}

/// A change to a registration snapshot.
type Edit = fn(&mut ForeignRegistrationSnapshot);

fn registration(app: &mut App, edit: impl FnOnce(&mut ForeignRegistrationSnapshot)) {
    let mut snapshot = ForeignRegistrationSnapshot::default();
    snapshot.last_bbmd = Some(bbmd());
    snapshot.last_ttl = Some(60);
    edit(&mut snapshot);
    worker(
        app,
        WorkerEvent::Network(NetworkEvent::Registration(snapshot)),
    );
}

/// A flash from a mistyped command.
fn flash(app: &mut App) {
    press(app, KeyCode::Char(':'));
    type_text(app, "bogus");
    press(app, KeyCode::Enter);
    assert!(app.flash.is_some(), "the command bar flashed its usage");
}

#[test]
fn a_notice_goes_when_the_screen_changes() {
    let mut app = connected_app();
    flash(&mut app);
    // Moving on the same screen keeps it.
    press(&mut app, KeyCode::Down);
    assert!(app.flash.is_some());
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.screen, Screen::Watch);
    assert_eq!(app.flash, None);
    flash(&mut app);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.flash, None, "Esc back to the devices drops it too");
}

#[test]
fn a_notice_goes_when_a_dialog_opens_but_one_the_dialog_sets_stays() {
    let mut app = connected_app();
    flash(&mut app);
    press(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.overlay, Overlay::WhoIs(_)));
    assert_eq!(
        app.flash, None,
        "the Who-Is form opened over a stale notice"
    );
    // Ctrl-C closes the form and says so; that notice is this action's.
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(app.overlay, Overlay::None);
    assert!(app
        .flash
        .as_ref()
        .is_some_and(|f| f.text.starts_with("Closed")));
    // The help overlay is a dialog too.
    flash(&mut app);
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.flash, None);
}

#[test]
fn a_notice_sits_beside_the_hints_and_keeps_help_and_quit() {
    let mut app = foreign_app();
    flash(&mut app);
    for (w, h) in SIZES {
        let (_, footer) = chrome(&mut app, w, h);
        assert!(footer.starts_with(" unknown command 'bogus'"), "{footer}");
        assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
    }
    // At 80 columns the notice is cut, not the keys that always stay.
    let (_, footer) = chrome(&mut app, 80, 24);
    assert!(footer.contains('…'), "{footer}");
    assert!(footer.contains("Esc back  ? help  q quit"), "{footer}");
    assert_snapshot!("footer_flash", chrome_at_sizes(&mut app));
}

/// A notice `len` characters long, ending in `|` so a cut one shows.
fn long_notice(app: &mut App, len: usize) -> String {
    let text = format!("{}|", "n".repeat(len - 1));
    app.flash = Some(crate::tui::app::Flash {
        text: text.clone(),
        until: app.now + Duration::from_secs(4),
    });
    text
}

/// From 160 columns a notice shows in full (#1927): the hints give way
/// to it, the lowest priority first, and only `? help` and `q quit` are
/// kept clear of it. Below that the way back is kept too.
#[test]
fn a_wide_footer_shows_a_long_notice_in_full() {
    let mut app = foreign_app();
    let text = long_notice(&mut app, 120);
    let (_, footer) = chrome(&mut app, 200, 50);
    assert!(footer.starts_with(&format!(" {text}")), "{footer}");
    assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
    // The hints that fit beside it stay; `:bbmd` (the lowest) went first.
    assert!(footer.contains("r read tables"), "{footer}");
    assert!(footer.contains("Esc back"), "{footer}");
    assert!(!footer.contains(":bbmd"), "{footer}");
    // Below 160 columns the same notice is cut and the way back stays.
    let (_, footer) = chrome(&mut app, 120, 40);
    assert!(!footer.contains(&text) && footer.contains('…'), "{footer}");
    assert!(footer.contains("Esc back  ? help  q quit"), "{footer}");

    // A notice that leaves no room for the way back pushes it out too.
    let text = long_notice(&mut app, 180);
    let (_, footer) = chrome(&mut app, 200, 50);
    assert!(footer.starts_with(&format!(" {text}")), "{footer}");
    assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
    assert!(!footer.contains("Esc back"), "{footer}");

    // One too long to show whole even then is cut, and the way back stays:
    // it gives way only when that shows the whole notice.
    let text = long_notice(&mut app, 190);
    let (_, footer) = chrome(&mut app, 200, 50);
    assert!(!footer.contains(&text) && footer.contains('…'), "{footer}");
    assert!(footer.contains("Esc back  ? help  q quit"), "{footer}");
}

/// The wide footer starts at exactly 160 columns: a notice that fits whole
/// beside `? help` and `q quit` there (but not beside the way back too) is
/// cut at 159.
#[test]
fn the_wide_footer_starts_at_160_columns() {
    let mut app = foreign_app();
    let text = long_notice(&mut app, 140);
    let (_, footer) = chrome(&mut app, 160, 40);
    assert!(footer.starts_with(&format!(" {text}")), "{footer}");
    assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
    assert!(!footer.contains("Esc back"), "{footer}");
    let (_, footer) = chrome(&mut app, 159, 40);
    assert!(!footer.contains(&text) && footer.contains('…'), "{footer}");
    assert!(footer.contains("Esc back  ? help  q quit"), "{footer}");
}

/// While the link has failed, its recovery key stays at any width, even
/// where giving it up would show the whole summary.
#[test]
fn a_failed_link_keeps_its_recovery_key_on_a_wide_footer() {
    let mut app = new_app(None);
    let error = Error::Transport(std::io::Error::from_raw_os_error(2));
    worker(
        &mut app,
        WorkerEvent::ConnectFailed {
            error: Failure::of(&error),
        },
    );
    // 150 characters: too long either way; 120: whole without `c retry`.
    for len in [150, 120] {
        if let Link::Failed(failure) = &mut app.link {
            failure.summary = "s".repeat(len);
        }
        for (w, h) in [(160, 40), (200, 50)] {
            let (_, footer) = chrome(&mut app, w, h);
            assert!(footer.starts_with(" Connection failed: s"), "{footer}");
            assert!(footer.contains("c retry"), "{len} at {w}: {footer}");
            assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
        }
    }
}

/// Every screen keeps `? help` and `q quit` at 80 columns; the Network
/// screen keeps its way back too.
#[test]
fn every_footer_keeps_help_and_quit_at_80_columns() {
    let mut app = foreign_app();
    for key in ['1', '3', '4', '5', '6', '7'] {
        press(&mut app, KeyCode::Char(key));
        let (_, footer) = chrome(&mut app, 80, 24);
        assert!(footer.contains("? help  q quit"), "screen {key}: {footer}");
    }
    press(&mut app, KeyCode::Char('4'));
    let (_, footer) = chrome(&mut app, 80, 24);
    assert!(footer.contains("Esc back  ? help  q quit"), "{footer}");
    for view in 0..4 {
        let (_, footer) = chrome(&mut app, 80, 24);
        assert!(footer.contains("? help  q quit"), "view {view}: {footer}");
        press(&mut app, KeyCode::Tab);
    }
}

#[test]
fn after_another_bbmd_the_footer_says_how_to_go_back() {
    let mut app = foreign_app();
    let (_, footer) = chrome(&mut app, 80, 24);
    assert!(!footer.contains(":bbmd own"), "{footer}");
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.7.1");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.network.target, Some("10.0.7.1:47808".parse().unwrap()));
    for (w, h) in SIZES {
        let (_, footer) = chrome(&mut app, w, h);
        assert!(footer.contains(":bbmd own BBMD"), "{w}: {footer}");
    }
    assert_snapshot!("footer_other_bbmd", chrome_at_sizes(&mut app));
    // `:bbmd` alone goes back.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.network.target, Some(bbmd()));
    let (_, footer) = chrome(&mut app, 80, 24);
    assert!(!footer.contains(":bbmd own"), "{footer}");
}

#[test]
fn bbmd_alone_without_a_registration_says_what_it_needs() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.network.target, None);
    assert!(app.flash.as_ref().unwrap().text.contains(":bbmd ADDRESS"));
}

/// The last row of an 80x24 frame.
fn footer_of(app: &mut App) -> String {
    let text = screen(app, 80, 24);
    text.lines().last().unwrap().trim_matches('"').to_string()
}

#[test]
fn a_failed_connection_shows_its_summary_and_the_recovery_keys() {
    let mut app = new_app(None);
    // ENOENT on Unix, ERROR_FILE_NOT_FOUND on Windows: NotFound either way.
    let error = Error::Transport(std::io::Error::from_raw_os_error(2));
    worker(
        &mut app,
        WorkerEvent::ConnectFailed {
            error: Failure::of(&error),
        },
    );
    // Wide enough for the whole OS message: Windows' text for error 2 is
    // longer than Unix's, and 80 columns would cut the `(os error 2)`.
    let text = screen(&mut app, 200, 50);
    let footer = footer_of(&mut app);
    assert!(
        footer.starts_with(" Connection failed: entity not found"),
        "{footer}"
    );
    // Without profiles `c` retries; there is no `p settings` to offer.
    assert!(footer.contains("c retry"), "{footer}");
    assert!(!footer.contains("settings"), "{footer}");
    assert!(footer.contains("? help  q quit"), "{footer}");
    assert!(!footer.contains("os error"), "{footer}");
    // The device screen keeps the whole error.
    assert!(text.contains("transport error: "), "{text}");
    assert!(text.contains("(os error 2)"), "{text}");
    assert_snapshot!("footer_failed", chrome_at_sizes(&mut app));
}

#[test]
fn failures_keep_what_identifies_them() {
    // An OS error by its kind.
    let os = Failure::of(&Error::Transport(std::io::Error::from_raw_os_error(2)));
    assert_eq!(os.summary, "entity not found");
    // A message the stack wrote keeps its words, whatever its kind.
    let own = Failure::of(&Error::Transport(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        "own address 10.0.9.5 not local",
    )));
    assert_eq!(own.summary, "own address 10.0.9.5 not local");
    let timeout = Failure::of(&Error::Timeout(Duration::from_secs(3)));
    assert_eq!(timeout.summary, "timed out");
    let port = Failure::within(
        "/dev/ttyUSB0",
        &Error::Transport(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
    );
    assert_eq!(port.summary, "/dev/ttyUSB0: permission denied");
    // Text keeps both clauses; the footer shortens it to fit.
    let long = "profile 'plant-a-long-name': field 'Interface': no interface named \
                enp0s31f6 on this host; pick one from the list";
    // A profile error from the service keeps its clauses but not the OS
    // error number.
    let os = Failure::text("profile 'plant': interface en9: No such device (os error 19)");
    assert_eq!(os.summary, "profile 'plant': interface en9: No such device");
    assert!(os.detail.ends_with("(os error 19)"));
    let mut app = new_app(None);
    worker(&mut app, WorkerEvent::ConnectFailed { error: os });
    let (_, footer) = chrome(&mut app, 120, 40);
    assert!(!footer.contains("(os error 19)"), "{footer}");
    assert!(footer.contains("No such device"), "{footer}");
    let text = Failure::text(long);
    assert_eq!(text.summary, long);
    let mut app = new_app(None);
    worker(&mut app, WorkerEvent::ConnectFailed { error: text });
    let footer = footer_of(&mut app);
    assert!(
        footer.starts_with(" Connection failed: profile 'plant-a-long-name': field"),
        "{footer}"
    );
    assert!(footer.contains('…'), "{footer}");
    assert!(footer.contains("c retry"), "{footer}");
    assert!(footer.trim_end().ends_with("? help  q quit"), "{footer}");
}

/// Esc on a write preview declines it, and its notice stays in the
/// footer: it was set by that action.
#[test]
fn a_declined_preview_says_so_in_the_footer() {
    use crate::tui::app::safety::Modal;
    use crate::tui::safety::{Gate, Preview, Tier};
    let mut app = connected_app();
    app.overlay = Overlay::Safety(Box::new(Modal::Confirm {
        req: 1,
        preview: Box::new(Preview {
            device: 100,
            items: Vec::new(),
            batch: false,
            gate: Gate::Allowed(Tier::T1),
        }),
        typed: String::new(),
        error: None,
        opened: app.now,
    }));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Overlay::None);
    let footer = footer_of(&mut app);
    assert!(
        footer.starts_with(" Declined: nothing was sent."),
        "{footer}"
    );
}

/// `FD` reads the same state the Network screen's banner does.
#[test]
fn fd_shows_the_registration_state_the_banner_reads() {
    let mut app = connected_app();
    let (status, _) = chrome(&mut app, 120, 40);
    assert!(!status.contains("FD"), "no registration, no FD: {status}");

    let mut app = foreign_app();
    let cases: [(&str, Registration, Edit); 5] = [
        ("pending", Registration::Pending, |s| {
            s.attempts = 1;
            s.last_outcome = Some(ForeignRegistrationOutcome::Pending);
        }),
        ("held", Registration::Held, |s| {
            s.attempts = 1;
            s.accepted = 1;
            s.registered = true;
            s.last_outcome = Some(ForeignRegistrationOutcome::Accepted);
        }),
        ("refused", Registration::Refused, |s| {
            s.attempts = 1;
            s.refused = 1;
            s.last_outcome = Some(ForeignRegistrationOutcome::Refused);
        }),
        ("no answer", Registration::NoAnswer { unanswered: 2 }, |s| {
            s.attempts = 2;
            s.last_outcome = Some(ForeignRegistrationOutcome::TimedOut);
        }),
        ("lapsed", Registration::Lapsed, |s| {
            s.attempts = 3;
            s.accepted = 1;
            s.lapses = 1;
            s.last_outcome = Some(ForeignRegistrationOutcome::TimedOut);
        }),
    ];
    let (status, _) = chrome(&mut app, 120, 40);
    assert!(status.contains("FD pending"), "before any report: {status}");
    for (label, state, edit) in cases {
        registration(&mut app, edit);
        assert_eq!(app.network.registration_state(), Some(state));
        let (status, _) = chrome(&mut app, 120, 40);
        assert!(status.contains(&format!("FD {label}")), "{label}: {status}");
        let banner = app.network.registration_banner().is_some();
        let alarming = !matches!(state, Registration::Held | Registration::Pending);
        assert_eq!(banner, alarming, "{label}: the banner agrees");
    }
}

#[test]
fn the_status_bar_keeps_the_state_the_level_and_help_at_80_columns() {
    let mut app = foreign_app();
    registration(&mut app, |s| {
        s.attempts = 2;
        s.last_outcome = Some(ForeignRegistrationOutcome::TimedOut);
    });
    app.dropped = 12;
    for (w, h) in SIZES {
        let (status, _) = chrome(&mut app, w, h);
        for part in ["up", "READ-ONLY", "drop 12", "? help"] {
            assert!(status.contains(part), "{w}: {part}: {status}");
        }
    }
    let (status, _) = chrome(&mut app, 200, 50);
    assert!(status.contains("BIP 10.0.0.1:47808"), "{status}");
    assert!(status.contains("FD no answer"), "{status}");
    assert_snapshot!("status_no_answer", chrome_at_sizes(&mut app));
}

/// Before a profile connects, the status bar names no transport.
#[test]
fn before_a_profile_connects_no_transport_is_named() {
    let mut app = new_app(None);
    app.transport = "";
    app.link = Link::Down;
    let (status, _) = chrome(&mut app, 80, 24);
    assert!(status.contains("no profile | disconnected"), "{status}");
    assert!(!status.contains("BIP"), "{status}");
}

/// The SC and MS/TP screens of a session on another transport list only
/// the keys that do something there.
#[test]
fn empty_sc_and_mstp_screens_list_only_working_keys() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('5'));
    let (_, sc) = chrome(&mut app, 80, 24);
    assert!(!sc.contains("Tab"), "{sc}");
    assert!(!sc.contains("disconnect"), "{sc}");
    assert!(sc.contains("Esc back"), "{sc}");
    press(&mut app, KeyCode::Char('6'));
    let (_, mstp) = chrome(&mut app, 80, 24);
    for key in ["z ", "Space", "disconnect"] {
        assert!(!mstp.contains(key), "{key}: {mstp}");
    }
    assert!(mstp.contains("Esc back"), "{mstp}");
    let mut out = format!("SC\n{sc}\nMS/TP\n{mstp}\n");
    // The same screens on their own transports list their keys.
    let mut app = super::sc_screen::sc_app();
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "22:10:20:30:40:50".into(),
        },
    );
    press(&mut app, KeyCode::Char('5'));
    let (_, sc) = chrome(&mut app, 120, 40);
    assert!(sc.contains("Tab certificates/doctor/credentials"), "{sc}");
    out.push_str(&format!("SC session\n{sc}\n"));
    assert_snapshot!("footer_empty_screens", out);
}
