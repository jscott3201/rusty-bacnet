//! Write sessions in `update` alone, with the worker's reports scripted:
//! arming and its form, expiry on the app's clock, the flags-only refusal,
//! the exit guard on a double Ctrl-C, and the leftover ledger at launch.

use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::tui::app::safety::Modal;
use crate::tui::app::Overlay;
use crate::tui::profile::Safety;
use crate::tui::safety::ledger::{Entry, State};
use crate::tui::safety::{Level, SafetyCommand, SafetyEvent, Stored};

fn ready(app: &mut App, workspace: bool, ledger: Vec<Entry>) {
    worker(
        app,
        WorkerEvent::Safety(SafetyEvent::Ready {
            policy: Safety::default(),
            workspace,
            run: "this-run".into(),
            ledger,
        }),
    );
}

fn entry(run: &str) -> Entry {
    Entry {
        id: 1,
        run: run.into(),
        at: "2026-10-18T12:00:00.000Z".into(),
        device: 100,
        object_type: 1,
        instance: 1,
        property: 85,
        index: None,
        priority: Some(8),
        commandable: true,
        before: None,
        written: Stored {
            bytes: vec![0x44, 0x42, 0x28, 0x00, 0x00],
            text: "42".into(),
        },
        state: State::Applied,
        kept: false,
        seq: 1,
    }
}

#[test]
fn arming_shows_the_form_then_a_red_countdown() {
    let mut app = connected_app();
    ready(&mut app, true, Vec::new());
    press(&mut app, KeyCode::Char('W'));
    type_text(&mut app, "ticket 7");
    assert_snapshot!("arm_form_80x24", screen(&mut app, 80, 24));
    let commands = press(&mut app, KeyCode::Enter);
    assert_eq!(
        commands,
        [Command::Safety(SafetyCommand::Arm {
            reason: "ticket 7".into(),
            minutes: 15
        })]
    );
    assert_eq!(app.level(), Level::ReadOnly, "until the worker confirms");
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Armed { minutes: 15 }),
    );
    assert_eq!(app.level(), Level::Armed { dangerous: false });
    app.flash = None;
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("WRITE 15:00"), "{text}");
    // The bar is red, not only labelled.
    app.color = true;
    let terminal = render(&mut app, 80, 24);
    assert_eq!(
        terminal.backend().buffer()[(1, 0)].bg,
        ratatui::style::Color::Red
    );
}

#[test]
fn a_write_session_disarms_at_expiry() {
    let mut app = connected_app();
    ready(&mut app, true, Vec::new());
    press(&mut app, KeyCode::Char('W'));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "1");
    press(&mut app, KeyCode::Enter);
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Armed { minutes: 1 }),
    );
    let start = app.now;
    tick(&mut app, start + Duration::from_secs(59));
    assert_eq!(app.level(), Level::Armed { dangerous: false });
    let now = start + Duration::from_secs(60);
    let log_generation = app.log.generation();
    let commands = update(
        &mut app,
        Action::Tick {
            now,
            dropped: 0,
            log_generation,
        },
    );
    assert_eq!(
        commands,
        [Command::Safety(SafetyCommand::Disarm { expired: true })]
    );
    assert_eq!(app.level(), Level::ReadOnly);
    assert!(app.flash.as_ref().unwrap().text.contains("expired"));
}

#[test]
fn a_flags_only_session_cannot_arm() {
    let mut app = connected_app();
    ready(&mut app, false, Vec::new());
    assert!(press(&mut app, KeyCode::Char('W')).is_empty());
    assert_eq!(app.overlay, Overlay::None);
    assert!(app.flash.as_ref().unwrap().text.contains("need a profile"));
}

#[test]
fn a_double_ctrl_c_goes_through_the_exit_guard() {
    let mut app = connected_app();
    ready(&mut app, true, vec![entry("this-run")]);
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.quit);
    assert!(matches!(&app.overlay, Overlay::Safety(m) if matches!(**m, Modal::ExitGuard(_))));
    // Keep and log: the worker records it and the TUI quits.
    let commands = press(&mut app, KeyCode::Char('k'));
    assert_eq!(commands, [Command::Safety(SafetyCommand::Keep)]);
    assert!(app.quit);
}

#[test]
fn ctrl_c_on_a_preview_declines_it() {
    use crate::tui::safety::{Gate, Preview, Tier};
    let mut app = connected_app();
    ready(&mut app, true, Vec::new());
    let preview = Box::new(Preview {
        device: 100,
        items: Vec::new(),
        batch: false,
        gate: Gate::Allowed(Tier::T1),
    });
    app.overlay = Overlay::Safety(Box::new(Modal::Confirm {
        req: 1,
        preview: preview.clone(),
        typed: String::new(),
        error: None,
        opened: app.now,
    }));
    let commands = press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        commands,
        [Command::Safety(SafetyCommand::Declined { preview })]
    );
    assert_eq!(app.overlay, Overlay::None);
}

/// An armed app showing a T1 preview opened now.
fn confirming() -> App {
    use crate::tui::safety::{Gate, Preview, Tier};
    let mut app = connected_app();
    ready(&mut app, true, Vec::new());
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Armed { minutes: 15 }),
    );
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
    app
}

#[test]
fn enter_right_after_a_preview_opens_is_ignored() {
    use crate::tui::app::safety::CONFIRM_GUARD;
    let mut app = confirming();
    assert!(
        press(&mut app, KeyCode::Enter).is_empty(),
        "a repeating key"
    );
    advance(&mut app, CONFIRM_GUARD);
    let commands = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        commands.as_slice(),
        [Command::Safety(SafetyCommand::Execute { req: 1, .. })]
    ));
}

#[test]
fn a_preview_left_unconfirmed_expires_as_declined() {
    use crate::tui::app::safety::PREVIEW_LIFETIME;
    let mut app = confirming();
    let start = app.now;
    tick(&mut app, start + PREVIEW_LIFETIME - Duration::from_secs(1));
    assert!(matches!(&app.overlay, Overlay::Safety(_)));
    let log_generation = app.log.generation();
    let commands = update(
        &mut app,
        Action::Tick {
            now: start + PREVIEW_LIFETIME,
            dropped: 0,
            log_generation,
        },
    );
    assert!(matches!(
        commands.as_slice(),
        [Command::Safety(SafetyCommand::Declined { .. })]
    ));
    assert_eq!(app.overlay, Overlay::None);
}

#[test]
fn an_empty_ledger_quits_at_once() {
    let mut app = connected_app();
    ready(&mut app, true, Vec::new());
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn entries_from_an_earlier_run_open_the_ledger_at_launch() {
    let mut app = connected_app();
    let mut kept = entry("earlier-run");
    kept.id = 2;
    kept.kept = true;
    ready(
        &mut app,
        true,
        vec![entry("earlier-run"), kept, entry("this-run")],
    );
    assert_eq!(app.safety.leftovers(), 2);
    assert!(
        matches!(&app.overlay, Overlay::Safety(m) if matches!(**m, Modal::Ledger { notice: Some(_), .. }))
    );
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("[earlier run]"), "{text}");
    assert!(text.contains("[kept, earlier run]"), "{text}");
}
