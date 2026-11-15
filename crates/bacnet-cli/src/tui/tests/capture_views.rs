//! The Capture screen's routed stations, `dev:` against the device table
//! as it learns, pausing to read a frame, and what 80 columns keep (#971).

use bacnet_types::enums::ConfirmedServiceChoice;
use ratatui::crossterm::event::KeyCode;

use super::capture::{PEER, bip_frame, capturing, deliver, filter, mixed, routed_in, shown};
use super::*;
use crate::tui::app::capture::Focus;
use bacnet_ops::decode::testing;

#[test]
fn a_routed_frame_names_its_station_and_the_router_it_came_through() {
    let mut app = capturing();
    deliver(&mut app, vec![routed_in(0, &testing::i_am(4321))], 0);
    let frame = app.capture.ring.get(1).unwrap();
    assert_eq!(frame.source, "2:0x05");
    assert_eq!(frame.via.as_deref(), Some(PEER));
    let wide = screen(&mut app, 160, 40);
    assert!(wide.contains("Via"), "{wide}");
    let row = wide.lines().find(|l| l.contains("2:0x05")).unwrap();
    let (station, router) = (row.find("2:0x05").unwrap(), row.find(PEER).unwrap());
    assert!(station < router, "{row}");
    let narrow = screen(&mut app, 80, 24);
    assert!(narrow.contains("← 2:0x05"), "{narrow}");
}

/// `dev:N` finds a routed device by its routed station, and takes up a
/// device the table learns after the filter was applied.
#[test]
fn dev_follows_the_device_table_and_finds_routed_devices() {
    let mut app = capturing();
    let ack = routed_in(10, &testing::rp_ack(3));
    deliver(
        &mut app,
        vec![bip_frame(0, true, &testing::rp_request(3), false), ack],
        0,
    );
    filter(&mut app, "dev:4321");
    assert!(shown(&app).is_empty(), "nothing known of 4321 yet");
    let routed = routed_row(4321, (2, &[5]), PEER, 260, 0);
    worker(&mut app, WorkerEvent::Discovered(routed));
    assert_eq!(shown(&app), [2], "its routed station, 2:0x05");
}

#[test]
fn paused_counts_only_the_frames_the_filter_would_show() {
    let mut app = capturing();
    deliver(&mut app, mixed(), 0);
    filter(&mut app, "errors");
    press(&mut app, KeyCode::Char(' '));
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    let more = vec![
        bip_frame(2_000, true, &testing::rp_request(8), false),
        bip_frame(2_010, false, &testing::error(8, rp), false),
    ];
    deliver(&mut app, more, 0);
    assert_eq!(app.capture.held_back(), 1);
    assert!(screen(&mut app, 120, 40).contains("PAUSED +1"));
}

/// Going into the tree or the hex view pauses, so a new frame doesn't
/// replace the fields being read; the tree keeps its place.
#[test]
fn reading_a_frame_pauses_and_keeps_the_tree_where_it_was() {
    for key in [KeyCode::Tab, KeyCode::Enter] {
        let mut app = capturing();
        deliver(&mut app, mixed(), 0);
        app.settle();
        press(&mut app, key);
        assert_eq!(app.capture.focus, Focus::Tree);
        assert!(app.capture.paused.is_some(), "{key:?} pauses");
        press(&mut app, KeyCode::Down);
        let before = app.capture.tree.borrow().selected().to_vec();
        deliver(&mut app, mixed(), 0);
        app.settle();
        assert_eq!(app.capture.detail.as_ref().unwrap().number, 7);
        assert_eq!(app.capture.tree.borrow().selected(), before);
    }
}

/// At 80 columns a mark tells in from out, and the invoke ID leads the
/// Info column, so a cut keeps it.
#[test]
fn eighty_columns_keep_the_direction_and_the_invoke_id() {
    let mut app = capturing();
    deliver(&mut app, mixed(), 0);
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("→ 10.0.0.1:47808  10.0.0.2:47808  #1 conf-req"),
        "{text}"
    );
    assert!(
        text.contains("← 10.0.0.2:47808  10.0.0.1:47808  #1 cmplx-ack"),
        "{text}"
    );
}
