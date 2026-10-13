//! Frames on a `TestBackend` at the minimum (80x24) and full (120x40)
//! layouts, as insta snapshots. Times are fixed offsets from `t0()`, so ages
//! render the same on every run.

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use tracing::Level;

use super::*;
use crate::tui::log_layer::LogLine;
use crate::tui::message::{OpOutcome, WorkerEvent};

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

/// Five devices, one behind a router, after a finished Who-Is.
fn populated() -> App {
    const LOCAL: [(u32, &str, u16, u64); 4] = [
        (1001, "10.0.1.21:47808", 260, 2),
        (1002, "10.0.1.22:47808", 5, 9),
        (1003, "10.0.1.23:47808", 260, 11),
        (1004, "10.0.1.24:47809", 8, 12),
    ];
    let mut app = connected_app();
    for (instance, address, vendor, seen) in LOCAL {
        worker(
            &mut app,
            WorkerEvent::Discovered(row(instance, address, vendor, seen)),
        );
    }
    let mut routed = row(2001, "05 via 10.0.1.1:47808", 24, 7);
    routed.network = Some(2);
    routed.max_apdu = 480;
    routed.segmentation = bacnet_types::enums::Segmentation::BOTH;
    worker(&mut app, WorkerEvent::Discovered(routed));

    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "1000-2999");
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::WhoIs { op, .. }] = commands.as_slice() else {
        panic!("expected one WhoIs, got {commands:?}");
    };
    worker(&mut app, WorkerEvent::WhoIsSent { op: *op });
    // Three answer the Who-Is at 12 s.
    for (instance, address, vendor, _) in &LOCAL[..3] {
        worker(
            &mut app,
            WorkerEvent::Updated(row(*instance, address, *vendor, 12)),
        );
    }
    worker(
        &mut app,
        WorkerEvent::OpFinished {
            op: *op,
            outcome: OpOutcome::Completed,
        },
    );
    tick(&mut app, at(14));
    app
}

#[test]
fn devices_table() {
    for (w, h) in SIZES {
        let mut app = populated();
        assert_snapshot!(format!("devices_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn devices_table_filtered_and_sorted() {
    for (w, h) in SIZES {
        let mut app = populated();
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "10.0.1.2");
        assert_snapshot!(format!("filtered_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn duplicate_instance_banner() {
    for (w, h) in SIZES {
        let mut app = populated();
        worker(
            &mut app,
            WorkerEvent::Collision {
                retained: row(1002, "10.0.1.22:47808", 5, 9),
                incoming: row(1002, "10.0.1.99:47808", 5, 13),
            },
        );
        assert_snapshot!(format!("duplicate_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn who_is_form_with_the_unbounded_warning() {
    for (w, h) in SIZES {
        let mut app = connected_app();
        worker(
            &mut app,
            WorkerEvent::Discovered(row(7, "10.0.0.7:47808", 1, 0)),
        );
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(format!("who_is_warning_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn who_is_form_directed_with_an_error() {
    for (w, h) in SIZES {
        let mut app = connected_app();
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "2:1234");
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(format!("who_is_error_{w}x{h}"), screen(&mut app, w, h));
    }
}

/// On B/IPv6 the target hint names the VMAC forms and the IPv6 address the
/// worker resolves, and the status bar shows the VMAC as the table would.
#[test]
fn who_is_form_on_bip6_hints_vmac_targets() {
    let mut app = App::new(
        AppConfig {
            transport: "BIP6",
            style: AddressStyle::Bip6,
            color: false,
            picker: None,
            now: t0(),
        },
        LogRing::new(100),
    );
    worker(
        &mut app,
        WorkerEvent::Connected {
            local: "bip6:0x40ABCD ([::1]:47808)".into(),
        },
    );
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("Directed (one address)"), "{text}");
    assert!(
        text.contains("bip6:N, bip6:0xVMAC or [IPv6]:port"),
        "{text}"
    );
    assert!(!text.contains("IP or IP:port"), "{text}");
}

#[test]
fn help_overlay() {
    for (w, h) in SIZES {
        let mut app = populated();
        press(&mut app, KeyCode::Char('?'));
        assert_snapshot!(format!("help_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn interface_picker() {
    for (w, h) in SIZES {
        let mut app = new_app(Some(Picker::new(interfaces())));
        press(&mut app, KeyCode::Down);
        assert_snapshot!(format!("picker_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn log_pane() {
    for (w, h) in SIZES {
        let mut app = populated();
        for (time, level, text) in [
            (
                "12:00:01",
                Level::INFO,
                "connected; local address 10.0.0.1:47808",
            ),
            ("12:00:02", Level::INFO, "Who-Is 1 sent (local 1000-2999)"),
            (
                "12:00:03",
                Level::WARN,
                "worker command not sent: channel full",
            ),
        ] {
            app.log.push(LogLine {
                time: time.into(),
                level,
                text: text.into(),
            });
        }
        press(&mut app, KeyCode::Char('L'));
        assert_snapshot!(format!("log_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn connecting_and_empty_states() {
    let mut app = new_app(None);
    assert_snapshot!("connecting_80x24", screen(&mut app, 80, 24));
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1, "10.0.0.2:47808", 1, 0)),
    );
    app.dropped = 42;
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "nothing");
    assert_snapshot!("no_match_80x24", screen(&mut app, 80, 24));
}

#[test]
fn below_the_minimum_size_only_a_notice_is_drawn() {
    for (w, h) in [(79, 24), (80, 23)] {
        let mut app = populated();
        assert_snapshot!(format!("too_small_{w}x{h}"), screen(&mut app, w, h));
    }
}

#[test]
fn colour_is_used_unless_no_color_is_set() {
    use ratatui::style::Color;
    let mut app = populated();
    let plain = render(&mut app, 80, 24);
    app.color = true;
    let colored = render(&mut app, 80, 24);
    let has_color = |t: &Terminal<TestBackend>| {
        t.backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.fg != Color::Reset || cell.bg != Color::Reset)
    };
    assert!(!has_color(&plain), "NO_COLOR frames use modifiers only");
    assert!(has_color(&colored));
    assert_eq!(
        plain.backend().to_string(),
        colored.backend().to_string(),
        "colour never carries information the text lacks"
    );
}
