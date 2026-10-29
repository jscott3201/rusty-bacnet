//! `update` without a terminal: synthetic keys and worker events in, state
//! and commands out.

use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::tui::app::devices::{Movement, SortKey};
use crate::tui::app::{Link, OpState, Overlay};
use crate::tui::message::{DirectedTo, OpOutcome, WhoIsScope, WhoIsSpec};

fn five_devices(app: &mut App) {
    for (i, vendor) in [260, 5, 260, 8, 260].into_iter().enumerate() {
        let instance = 100 + i as u32;
        let address = format!("10.0.1.{}:47808", i + 1);
        worker(
            app,
            WorkerEvent::Discovered(row(instance, &address, vendor, 0)),
        );
    }
}

#[test]
fn five_discovered_events_then_a_filter_leave_exactly_the_matching_rows() {
    let mut app = connected_app();
    five_devices(&mut app);
    assert_eq!(visible(&mut app), [100, 101, 102, 103, 104]);

    press(&mut app, KeyCode::Char('/'));
    assert!(app.filter_editing);
    type_text(&mut app, "260");
    assert_eq!(visible(&mut app), [100, 102, 104], "vendor 260 only");
    press(&mut app, KeyCode::Enter);
    assert!(!app.filter_editing);
    assert_eq!(app.devices.filter(), "260");
    assert_eq!(
        visible(&mut app),
        [100, 102, 104],
        "the filter stays after Enter"
    );

    // An address match, case-insensitively, then Esc clears it.
    press(&mut app, KeyCode::Char('/'));
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "10.0.1.4");
    assert_eq!(visible(&mut app), [103]);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.devices.filter(), "");
    assert_eq!(visible(&mut app).len(), 5);
}

#[test]
fn q_types_in_the_filter_but_quits_from_the_table() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert_eq!(app.devices.filter(), "q");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn sort_cycles_columns_and_reverses() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(3, "10.0.0.9:47808", 1, 5)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1, "10.0.0.7:47808", 9, 1)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(2, "10.0.0.8:47808", 5, 9)),
    );
    assert_eq!(visible(&mut app), [1, 2, 3]);
    press(&mut app, KeyCode::Char('S'));
    assert_eq!(visible(&mut app), [3, 2, 1]);
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.devices.sort(), (SortKey::Address, false));
    assert_eq!(visible(&mut app), [1, 2, 3]);
    for _ in 0..2 {
        press(&mut app, KeyCode::Char('s'));
    }
    assert_eq!(app.devices.sort().0, SortKey::Vendor);
    assert_eq!(visible(&mut app), [3, 2, 1]);
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.devices.sort().0, SortKey::LastSeen);
    assert_eq!(visible(&mut app), [2, 3, 1], "most recently seen first");
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.devices.sort().0, SortKey::Name);
    assert_eq!(visible(&mut app), [1, 2, 3], "no names yet: by instance");
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.devices.sort().0, SortKey::Instance);
}

#[test]
fn selection_follows_the_device_when_rows_reorder() {
    let mut app = connected_app();
    five_devices(&mut app);
    visible(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.devices.selected_index(), Some(2));
    press(&mut app, KeyCode::Char('S'));
    app.settle();
    assert_eq!(
        app.devices.selected_index(),
        Some(2),
        "102 is still the middle row"
    );
    act(&mut app, Action::Move(Movement::End));
    assert_eq!(app.devices.selected_index(), Some(4));
    worker(
        &mut app,
        WorkerEvent::Lost(row(100, "10.0.1.1:47808", 260, 0)),
    );
    assert_eq!(visible(&mut app), [104, 103, 102, 101]);
    assert_eq!(
        app.devices.selected_index(),
        Some(0),
        "fell back to the first row"
    );
}

#[test]
fn who_is_flow_warns_sends_listens_and_counts_replies() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(7, "10.0.0.7:47808", 1, 0)),
    );
    press(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.overlay, Overlay::WhoIs(_)));
    // Unbounded local Who-Is: the first Enter only warns.
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    let Overlay::WhoIs(form) = &app.overlay else {
        panic!("form closed");
    };
    assert!(
        form.warning
            .as_deref()
            .unwrap()
            .contains("(1 device known)")
    );
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::WhoIs { op, spec }] = commands.as_slice() else {
        panic!("expected one WhoIs, got {commands:?}");
    };
    assert_eq!(spec.scope, WhoIsScope::Local);
    assert_eq!(spec.range, None);
    assert_eq!(app.overlay, Overlay::None);

    worker(&mut app, WorkerEvent::WhoIsSent { op: *op });
    assert!(matches!(
        app.op.as_ref().unwrap().state,
        OpState::Listening { .. }
    ));
    worker(
        &mut app,
        WorkerEvent::Updated(row(7, "10.0.0.7:47808", 1, 1)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(8, "10.0.0.8:47808", 1, 1)),
    );
    worker(
        &mut app,
        WorkerEvent::OpFinished {
            op: *op,
            outcome: OpOutcome::Completed,
        },
    );
    let finished = app.op.as_ref().unwrap();
    assert_eq!((finished.replies, &finished.state), (2, &OpState::Done));

    // The form reopens with the last request.
    press(&mut app, KeyCode::Char('d'));
    let Overlay::WhoIs(form) = &app.overlay else {
        panic!("form closed");
    };
    assert_eq!(form.validate(AddressStyle::Bip).unwrap(), *spec);
}

#[test]
fn who_is_form_waits_for_the_link() {
    let mut app = new_app(None);
    assert_eq!(app.link, Link::Connecting);
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(app.flash.as_ref().unwrap().text, "Not connected yet.");
}

fn start_who_is(app: &mut App) -> u64 {
    press(app, KeyCode::Char('d'));
    press(app, KeyCode::Tab);
    // The form keeps the last request's range; replace it.
    for _ in 0..8 {
        press(app, KeyCode::Backspace);
    }
    type_text(app, "1-9");
    let commands = press(app, KeyCode::Enter);
    let [Command::WhoIs { op, .. }] = commands.as_slice() else {
        panic!("expected one WhoIs, got {commands:?}");
    };
    worker(app, WorkerEvent::WhoIsSent { op: *op });
    *op
}

#[test]
fn ctrl_c_cancels_the_operation_and_a_second_press_quits() {
    let mut app = connected_app();
    let op = start_who_is(&mut app);
    assert!(app.op_running());

    let commands = press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(commands, [Command::Cancel { op }]);
    assert!(!app.op_running() && !app.quit);
    assert_eq!(app.op.as_ref().unwrap().state, OpState::Cancelled);
    assert!(app.flash.as_ref().unwrap().text.contains("Ctrl-C again"));

    // The worker's late confirmation of the cancel changes nothing.
    worker(
        &mut app,
        WorkerEvent::OpFinished {
            op,
            outcome: OpOutcome::Completed,
        },
    );
    assert_eq!(app.op.as_ref().unwrap().state, OpState::Cancelled);

    advance(&mut app, Duration::from_millis(500));
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.quit, "second Ctrl-C within the window quits");
}

#[test]
fn ctrl_c_window_expires_and_ctrl_c_closes_modals_first() {
    let mut app = connected_app();
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.quit);
    advance(&mut app, Duration::from_secs(3));
    assert_eq!(app.flash, None, "the hint expired with the window");
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.overlay, Overlay::Help);
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        app.overlay,
        Overlay::None,
        "closed help instead of quitting"
    );
    assert!(!app.quit);
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.quit);
}

#[test]
fn a_newer_who_is_supersedes_the_running_one() {
    let mut app = connected_app();
    let first = start_who_is(&mut app);
    let second = start_who_is(&mut app);
    assert_ne!(first, second);
    worker(
        &mut app,
        WorkerEvent::OpFinished {
            op: first,
            outcome: OpOutcome::Cancelled,
        },
    );
    let op = app.op.as_ref().unwrap();
    assert_eq!(op.id, second);
    assert!(op.running(), "the stale result did not end the new request");
}

#[test]
fn collisions_record_every_address_for_the_banner() {
    let mut app = connected_app();
    let retained = row(200, "10.0.0.20:47808", 1, 0);
    let incoming = row(200, "10.0.0.21:47808", 1, 0);
    worker(
        &mut app,
        WorkerEvent::Collision {
            retained: retained.clone(),
            incoming,
        },
    );
    worker(
        &mut app,
        WorkerEvent::Collision {
            retained,
            incoming: row(200, "10.0.0.22:47808", 1, 0),
        },
    );
    assert_eq!(
        app.duplicates.lines(),
        ["instance 200 claimed by 10.0.0.20:47808, 10.0.0.21:47808 and 10.0.0.22:47808"]
    );
}

#[test]
fn snapshot_replaces_rows_and_lost_removes_one() {
    let mut app = connected_app();
    five_devices(&mut app);
    worker(
        &mut app,
        WorkerEvent::Snapshot(vec![
            row(9, "10.0.0.9:47808", 1, 0),
            row(4, "10.0.0.4:47808", 1, 0),
        ]),
    );
    assert_eq!(visible(&mut app), [4, 9]);
    worker(&mut app, WorkerEvent::Lost(row(9, "10.0.0.9:47808", 1, 0)));
    assert_eq!(visible(&mut app), [4]);
}

#[test]
fn connect_failure_keeps_the_ui_available_for_recovery() {
    let mut app = new_app(None);
    worker(
        &mut app,
        WorkerEvent::ConnectFailed {
            error: "Address in use".into(),
        },
    );
    assert!(!app.quit);
    assert_eq!(app.link, Link::Failed("Address in use".into()));
    assert!(app.exit_error.is_none());
}

#[test]
fn picker_choice_connects_and_esc_quits() {
    let mut app = new_app(Some(Picker::new(interfaces())));
    assert_eq!(app.link, Link::Picking);
    press(&mut app, KeyCode::Down);
    let commands = press(&mut app, KeyCode::Enter);
    assert_eq!(
        commands,
        [Command::Connect {
            interface: Ipv4Addr::new(192, 168, 50, 20),
            broadcast: Ipv4Addr::new(192, 168, 50, 255),
        }]
    );
    assert_eq!(app.link, Link::Connecting);
    assert_eq!(app.overlay, Overlay::None);

    let mut app = new_app(Some(Picker::new(interfaces())));
    assert_eq!(
        press(&mut app, KeyCode::Char('1')),
        [Command::Connect {
            interface: Ipv4Addr::new(10, 0, 1, 5),
            broadcast: Ipv4Addr::new(10, 0, 1, 255),
        }]
    );

    let mut app = new_app(Some(Picker::new(interfaces())));
    press(&mut app, KeyCode::Esc);
    assert!(app.quit);
}

#[test]
fn ticks_mark_dirty_only_when_something_visible_changed() {
    let mut app = connected_app();
    app.dirty = false;
    tick(&mut app, at(0) + Duration::from_millis(400));
    assert!(!app.dirty, "nothing to age yet");
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1, "10.0.0.1:47808", 1, 0)),
    );
    app.dirty = false;
    tick(&mut app, at(0) + Duration::from_millis(800));
    assert!(!app.dirty, "same second: ages unchanged");
    tick(&mut app, at(1) + Duration::from_millis(100));
    assert!(app.dirty, "ages moved to the next second");

    app.dirty = false;
    let (now, log_generation) = (app.now, app.log.generation());
    update(
        &mut app,
        Action::Tick {
            now,
            dropped: 3,
            log_generation,
        },
    );
    assert!(app.dirty && app.dropped == 3, "the dropped counter changed");

    app.dirty = false;
    app.log.info("hidden");
    advance(&mut app, Duration::ZERO);
    assert!(!app.dirty, "log pane hidden");
    press(&mut app, KeyCode::Char('L'));
    app.dirty = false;
    app.log.info("shown");
    advance(&mut app, Duration::ZERO);
    assert!(app.dirty, "log pane shown");
}

#[test]
fn describe_marks_remote_and_directed_scopes() {
    let spec = WhoIsSpec {
        scope: WhoIsScope::Directed {
            to: DirectedTo::Mac(vec![1, 2, 3, 4, 0xBA, 0xC0]),
            label: "1.2.3.4".into(),
        },
        range: Some(bacnet_services::who_is::DeviceInstanceRange::single(5).unwrap()),
        listen: Duration::from_secs(1),
    };
    assert_eq!(spec.describe(), "to 1.2.3.4 5");
}
