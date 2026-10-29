//! `:dcc` and `:reinit` in `update` alone, with the worker's reports
//! scripted (#1799): each refusal and its reason, the target rule, the
//! masked prompt's editing, and what Esc and a decline leave behind.

use bacnet_types::enums::{EnableDisable, ReinitializedState};
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::Overlay;
use crate::tui::app::command_bar::Line;
use crate::tui::app::monitor::MonitorState;
use crate::tui::app::safety::{ARM_HINT, CONFIRM_GUARD, Modal};
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::manage::{Action, DccTime, MAX_PASSWORD};
use crate::tui::safety::{Change, SafetyCommand, SafetyEvent};

const DANGEROUS: Safety = Safety {
    writes: Writes::Session,
    dangerous: true,
};

/// A connected app on a profile with `policy`, armed when `armed`, with
/// device 100 in the table.
fn app_with(policy: Safety, armed: bool) -> App {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Ready {
            policy,
            workspace: true,
            run: "this-run".into(),
            ledger: Vec::new(),
        }),
    );
    if armed {
        worker(
            &mut app,
            WorkerEvent::Safety(SafetyEvent::Armed { minutes: 15 }),
        );
    }
    worker(
        &mut app,
        WorkerEvent::Discovered(row(100, "10.0.0.100:47808", 260, 0)),
    );
    app.flash = None;
    app
}

/// Type `line` on the command bar and run it.
fn command(app: &mut App, line: &str) -> Vec<Command> {
    press(app, KeyCode::Char(':'));
    type_text(app, line);
    press(app, KeyCode::Enter)
}

fn flash(app: &App) -> &str {
    app.flash.as_ref().map_or("", |f| f.text.as_str())
}

fn modal(app: &App) -> Option<&Modal> {
    match &app.overlay {
        Overlay::Safety(modal) => Some(modal),
        _ => None,
    }
}

/// Each refusal says why, opens nothing and sends nothing.
#[test]
fn each_refusal_names_its_reason() {
    let refused = |mut app: App, want: &str| {
        for line in ["dcc disable 5", "reinit coldstart"] {
            app.flash = None;
            assert!(command(&mut app, line).is_empty(), "{line}");
            assert_eq!(app.overlay, Overlay::None, "{line}");
            assert!(flash(&app).contains(want), "{line}: {}", flash(&app));
        }
    };
    refused(app_with(Safety::default(), true), "dangerous = true");
    refused(app_with(DANGEROUS, false), ARM_HINT);
    let mut passive = app_with(DANGEROUS, true);
    passive.safety.passive = true;
    refused(passive, "Passive");
    let mut monitor = app_with(DANGEROUS, true);
    monitor.safety.passive = true;
    monitor.monitor = Some(Box::new(MonitorState::new(
        "/dev/ttyUSB0".into(),
        38_400,
        1,
        false,
    )));
    refused(monitor, "Monitor-only");
    // Nothing selected: an empty table, or a screen with no device on it.
    let mut empty = app_with(DANGEROUS, true);
    worker(
        &mut empty,
        WorkerEvent::Lost(row(100, "10.0.0.100:47808", 260, 0)),
    );
    refused(empty, "No device selected");
    let mut watch = app_with(DANGEROUS, true);
    press(&mut watch, KeyCode::Char('3'));
    refused(watch, "No device selected");
}

/// The selected row is the target; the prompt masks every character,
/// takes Backspace, stops at the length limit, and Esc drops it all.
#[test]
fn the_prompt_masks_edits_and_esc_clears() {
    let mut app = app_with(DANGEROUS, true);
    assert!(command(&mut app, "dcc disable-initiation 30").is_empty());
    let Some(Modal::Password { request, typed, .. }) = modal(&app) else {
        panic!("expected the password prompt: {}", flash(&app));
    };
    assert_eq!(request.device, 100);
    assert_eq!(
        request.action,
        Action::Dcc {
            state: EnableDisable::DISABLE_INITIATION,
            time: DccTime::Minutes(30)
        }
    );
    assert!(typed.is_empty());
    type_text(&mut app, "abcq");
    press(&mut app, KeyCode::Backspace);
    let Some(Modal::Password { typed, .. }) = modal(&app) else {
        panic!("still prompting");
    };
    assert_eq!(typed.expose(), "abc", "q is typed, not quit");
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("> Password  ***_"), "{text}");
    assert!(!text.contains("abc"), "{text}");
    type_text(&mut app, &"x".repeat(MAX_PASSWORD + 5));
    let Some(Modal::Password { typed, .. }) = modal(&app) else {
        panic!("still prompting");
    };
    assert_eq!(typed.chars(), MAX_PASSWORD);
    assert!(press(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.overlay, Overlay::None);
    assert!(
        flash(&app).contains("password was cleared"),
        "{}",
        flash(&app)
    );
}

/// A blank password sends none; a declined confirmation goes to the
/// worker without the password, and Enter after the guard on a typed
/// instance hands the request, password and all, to the worker.
#[test]
fn a_blank_password_sends_none_and_a_decline_drops_it() {
    let mut app = app_with(DANGEROUS, true);
    command(&mut app, "reinit warmstart");
    press(&mut app, KeyCode::Enter);
    let Some(Modal::Confirm {
        change: Change::Device(request),
        ..
    }) = modal(&app)
    else {
        panic!("expected the confirmation");
    };
    assert_eq!(request.password, None);
    assert_eq!(
        request.action,
        Action::Reinit {
            state: ReinitializedState::WARMSTART
        }
    );
    press(&mut app, KeyCode::Esc);

    command(&mut app, "dcc enable");
    type_text(&mut app, "s3cret");
    press(&mut app, KeyCode::Enter);
    let commands = press(&mut app, KeyCode::Esc);
    let [Command::Safety(SafetyCommand::Declined { change })] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    let Change::Device(declined) = change else {
        panic!("{change:?}");
    };
    assert_eq!(declined.password, None, "the password stays behind");

    command(&mut app, "dcc enable");
    type_text(&mut app, "s3cret");
    press(&mut app, KeyCode::Enter);
    advance(&mut app, CONFIRM_GUARD);
    // T3: Enter alone, or a wrong instance, sends nothing.
    for wrong in ["", "10"] {
        type_text(&mut app, wrong);
        assert!(press(&mut app, KeyCode::Enter).is_empty(), "{wrong:?}");
        let Some(Modal::Confirm { error, .. }) = modal(&app) else {
            panic!("still confirming");
        };
        assert!(error.as_deref().unwrap_or("").contains("100"), "{error:?}");
    }
    type_text(&mut app, "100");
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Safety(SafetyCommand::Manage { request, .. })] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    assert_eq!(
        request.password.as_ref().map(|p| p.expose()),
        Some("s3cret")
    );
    assert!(matches!(modal(&app), Some(Modal::Waiting { .. })));
}

/// `dcc` and `reinit` take the one-shot command's names, and `dcc` a
/// duration in whole minutes, 15 by default, or `forever` (#1799).
#[test]
fn dcc_and_reinit_take_the_one_shot_names() {
    let parse = |line: &str| crate::tui::app::command_bar::parse(line, &Default::default());
    let dcc = |state, time| Ok(Line::Manage(Action::Dcc { state, time }));
    assert_eq!(
        parse("dcc enable"),
        dcc(EnableDisable::ENABLE, DccTime::Forever)
    );
    assert_eq!(
        parse("control DISABLE_INITIATION 30"),
        dcc(EnableDisable::DISABLE_INITIATION, DccTime::Minutes(30))
    );
    assert_eq!(
        parse("dcc disable 65535"),
        dcc(EnableDisable::DISABLE, DccTime::Minutes(65535))
    );
    assert_eq!(
        parse("dcc disable-initiation"),
        dcc(EnableDisable::DISABLE_INITIATION, DccTime::Default)
    );
    assert_eq!(
        parse("dcc disable forever"),
        dcc(EnableDisable::DISABLE, DccTime::Forever)
    );
    for with in ["dcc enable 30", "dcc enable forever"] {
        let error = parse(with).unwrap_err();
        assert!(error.contains("takes no duration"), "{with}: {error}");
    }
    assert_eq!(
        parse("reinit start-backup"),
        Ok(Line::Manage(Action::Reinit {
            state: ReinitializedState::START_BACKUP
        }))
    );
    for bad in ["dcc disable 0", "dcc disable 65536", "dcc disable 1.5"] {
        let error = parse(bad).unwrap_err();
        assert!(
            error.contains("1 to 65535 whole minutes, or forever"),
            "{bad}: {error}"
        );
    }
    assert!(parse("dcc pause").unwrap_err().contains("unknown action"));
    assert!(
        parse("reinit reboot")
            .unwrap_err()
            .contains("unknown state")
    );
    for wrong in ["dcc", "dcc enable 5 more", "reinit", "reinit coldstart now"] {
        assert!(parse(wrong).unwrap_err().starts_with("usage: "), "{wrong}");
    }
}
