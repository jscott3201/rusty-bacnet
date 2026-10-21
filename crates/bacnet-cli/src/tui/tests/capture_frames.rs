//! The Capture screen's frames at 120x40 and 80x24 (#971).

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::capture::{capturing, deliver, mixed};
use super::*;
use crate::tui::capture::CaptureEvent;

/// The mixed session, with the tap reported running.
fn session() -> App {
    let mut app = capturing();
    let started = CaptureEvent::Started {
        what: "BIP tap at 10.0.0.1:47808".into(),
        session: true,
    };
    worker(&mut app, WorkerEvent::Capture(started));
    deliver(&mut app, mixed(), 3);
    app
}

#[test]
fn following_the_newest_frame() {
    let mut app = session();
    assert_snapshot!("capture_live_120x40", screen(&mut app, 120, 40));
    assert_snapshot!("capture_live_80x24", screen(&mut app, 80, 24));
}

#[test]
fn a_read_property_request_with_its_invoke_id_marked() {
    let mut app = session();
    press(&mut app, KeyCode::Home);
    app.settle();
    press(&mut app, KeyCode::Tab);
    for _ in 0..10 {
        press(&mut app, KeyCode::Down);
    }
    assert_snapshot!("capture_request_120x40", screen(&mut app, 120, 40));
    assert_snapshot!("capture_request_80x24", screen(&mut app, 80, 24));
    press(&mut app, KeyCode::Tab);
    assert_snapshot!("capture_hex_80x24", screen(&mut app, 80, 24));
}

#[test]
fn a_filter_on_a_paused_list() {
    let mut app = session();
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "svc:ReadProperty errors");
    press(&mut app, KeyCode::Enter);
    deliver(&mut app, mixed(), 0);
    assert_snapshot!("capture_filtered_80x24", screen(&mut app, 80, 24));
}

#[test]
fn before_any_frame_and_the_help() {
    let mut app = new_app(None);
    press(&mut app, KeyCode::Char('7'));
    assert_snapshot!("capture_empty_80x24", screen(&mut app, 80, 24));
    press(&mut app, KeyCode::Char('?'));
    assert_snapshot!("capture_help_80x24", screen(&mut app, 80, 24));
}
