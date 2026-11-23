//! Routed path limits in the TUI (#2667): `:path-limit` goes to the worker
//! at every level, since it sends nothing, and its answer flashes; it names
//! the path by its router or by a device of the table (#2726). A device
//! behind a router that stops answering after a small request was refused
//! as too long shows why beside the failed read, with the command to set
//! the path's limit.

use bacnet_ops::target::AddressStyle;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::browser::open_device;
use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::browser::{ListState, Pane, ViewState};
use crate::tui::routed_paths::{PathLimit, PathTo, hint_text};

/// `:path-limit` sends the change to the worker, also on a read-only
/// session, and the worker's answer flashes.
#[test]
fn the_command_goes_to_the_worker_and_its_answer_flashes() {
    let mut app = connected_app();
    for (line, max_npdu) in [
        ("path-limit 10.0.0.9:47808/2=128", Some(128)),
        ("path-limit 10.0.0.9:47808/2 clear", None),
    ] {
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, line);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            [Command::PathLimit(PathLimit {
                path: PathTo::Router {
                    router: "10.0.0.9:47808".into(),
                    network: 2,
                },
                max_npdu,
            })]
        );
    }
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "path-limit 10.0.0.9:47808/2");
    assert_eq!(press(&mut app, KeyCode::Enter), []);
    assert!(
        app.flash
            .as_ref()
            .unwrap()
            .text
            .contains("is not ROUTER/NETWORK=MAX_NPDU")
    );

    let said = "Requests to network 2 through 10.0.0.9:47808 now go in at most 128 NPDU octets";
    worker(&mut app, WorkerEvent::PathLimit(said.into()));
    assert_eq!(app.flash.as_ref().unwrap().text, said);
}

/// The command bar for a line, and what it sent or flashed.
fn run(app: &mut App, line: &str) -> (Vec<Command>, Option<String>) {
    app.flash = None;
    press(app, KeyCode::Char(':'));
    type_text(app, line);
    let sent = press(app, KeyCode::Enter);
    (sent, app.flash.as_ref().map(|flash| flash.text.clone()))
}

/// The device form (#2726): a device of the table by any word `watch`
/// takes, or `.` for the selected one, goes to the worker with the
/// address its row holds. The forms can't be mistaken for each other: a
/// first word with a `/` is a path, a size goes after `=` there and is a
/// word of its own after a device, and no device word is taken alone.
#[test]
fn a_device_names_its_path_and_the_forms_stay_apart() {
    let mut app = connected_app();
    let routed = DeviceRow {
        instance: 77,
        at: bacnet_ops::DeviceAddress::new(2, &[5]).unwrap(),
        address: "2:0x05".into(),
        via: Some("10.0.0.9:47808".into()),
        network: Some(2),
        ..row(77, "10.0.0.77:47808", 260, 0)
    };
    worker(&mut app, WorkerEvent::Discovered(routed.clone()));
    let to = |max_npdu| {
        vec![Command::PathLimit(PathLimit {
            path: PathTo::Device {
                instance: 77,
                at: routed.at.clone(),
            },
            max_npdu,
        })]
    };
    for (line, max_npdu) in [
        ("path-limit 77 128", Some(128)),
        ("path-limit 2:77 128", Some(128)),
        ("path-limit 2:0x05 clear", None),
    ] {
        assert_eq!(run(&mut app, line), (to(max_npdu), None), "{line}");
    }
    // `.` is the selected device, the only row here; off the Devices
    // screen, with no browser open, none is selected and nothing goes.
    assert_eq!(run(&mut app, "path-limit . 480"), (to(Some(480)), None));
    app.screen = crate::tui::app::watch::Screen::Network;
    let (sent, flash) = run(&mut app, "path-limit . clear");
    assert_eq!(sent, []);
    assert!(flash.unwrap().starts_with("No device selected"));
    for (line, said) in [
        (
            "path-limit 77=128",
            "a device takes its size as a word of its own",
        ),
        (
            "path-limit 10.0.0.9:47808/2 128",
            "a path takes its size after '='",
        ),
        ("path-limit 77", "usage: path-limit DEVICE SIZE|clear"),
        ("path-limit 128", "usage: path-limit DEVICE SIZE|clear"),
        (
            "path-limit 78 128",
            "no device in the device table is that one",
        ),
        ("path-limit 77 big", "neither clear nor a number"),
        // A station at an address the one-station mark lifts is a target
        // (#2738); the table can't list one, and says where to go.
        (
            "path-limit 5:0xFF:one-station 128",
            "lists no device at a one-station address",
        ),
        ("path-limit 5:0xFF 128", "not a station"),
        (
            "path-limit 5:0x05:one-station 128",
            "':one-station' lifts nothing at MAC 0x05",
        ),
        (
            "path-limit 10.0.0.9:47808 128",
            "a path through a router is ROUTER/NETWORK=SIZE",
        ),
    ] {
        let (sent, flash) = run(&mut app, line);
        assert_eq!(sent, [], "{line}");
        let flash = flash.unwrap_or_default();
        assert!(flash.contains(said), "{line}: {flash}");
    }
}

/// The hint shows in the object list over its "failed", and under a failed
/// object read.
#[tokio::test(start_paused = true)]
async fn the_hint_shows_beside_a_failed_read() {
    let server = Server {
        quirks: Quirks {
            silent: true,
            ..Quirks::default()
        },
        ..Server::lab(100, 100)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    open_device(&mut site).await;
    let path =
        bacnet_ops::RoutedPath::new(AddressStyle::Bip, &[10, 0, 0, 9, 0xBA, 0xC0], 2).unwrap();
    let browser = site.app.browser.as_mut().unwrap();
    assert!(matches!(browser.list, ListState::Failed(_)));
    browser.list_hint = Some(hint_text("2:100", &path));
    let text = screen(&mut site.app, 100, 30);
    assert!(
        text.contains("The router at 10.0.0.9:47808 refused"),
        "{text}"
    );
    assert!(text.contains(":path-limit 2:100 SIZE"), "{text}");
    assert!(text.contains(":path-limit 10.0.0.9:47808/2=SIZE"), "{text}");
    assert!(text.contains(" failed: "), "{text}");
    assert_snapshot!("browser_routed_path_hint_100x30", text);

    let browser = site.app.browser.as_mut().unwrap();
    browser.view = ViewState::Failed(
        bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::ANALOG_INPUT,
            1,
        )
        .unwrap(),
        "request timed out after 6s".into(),
    );
    browser.view_hint = Some(hint_text("2:100", &path));
    browser.focus = Pane::Properties;
    let text = screen(&mut site.app, 100, 30);
    assert!(text.contains("Read failed: request timed out"), "{text}");
    assert!(text.contains(":path-limit 10.0.0.9:47808/2=SIZE"), "{text}");
    site.stop().await;
}
