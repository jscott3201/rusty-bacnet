//! Routed path limits in the TUI (#2667): `:path-limit` goes to the worker
//! at every level, since it sends nothing, and its answer flashes; a device
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
use crate::tui::routed_paths::{PathLimit, hint_text};

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
                router: "10.0.0.9:47808".into(),
                network: 2,
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
    let path = bacnet_ops::RoutedPath::new(AddressStyle::Bip, &[10, 0, 0, 9, 0xBA, 0xC0], 2);
    let browser = site.app.browser.as_mut().unwrap();
    assert!(matches!(browser.list, ListState::Failed(_)));
    browser.path_hint = Some(hint_text(&path));
    let text = screen(&mut site.app, 100, 30);
    assert!(
        text.contains("The router at 10.0.0.9:47808 refused"),
        "{text}"
    );
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
    browser.focus = Pane::Properties;
    let text = screen(&mut site.app, 100, 30);
    assert!(text.contains("Read failed: request timed out"), "{text}");
    assert!(text.contains(":path-limit 10.0.0.9:47808/2=SIZE"), "{text}");
    site.stop().await;
}
