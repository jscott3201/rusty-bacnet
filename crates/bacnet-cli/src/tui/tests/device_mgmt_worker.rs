//! The write worker's own checks on `:dcc` and `:reinit` (#1799): a
//! request sent straight to it while not armed, after the session expired,
//! or in a passive session is refused and nothing reaches the wire; the
//! first two are recorded in the audit log. And in the app, a session that
//! expires between the password prompt and the confirmation sends nothing.

use std::time::Duration;

use bacnet_types::enums::{ConfirmedServiceChoice, EnableDisable, ReinitializedState};
use ratatui::crossterm::event::KeyCode;

use super::device_mgmt::{DANGEROUS, PASSWORD, bodies, site};
use super::site::{ClientLimits, Server, Site};
use super::writes::{LAB, audit, options};
use super::*;
use crate::tui::app::Overlay;
use crate::tui::app::safety::CONFIRM_GUARD;
use crate::tui::safety::audit::Secret;
use crate::tui::safety::manage::{Action, DccTime, Request};
use crate::tui::safety::{Change, SafetyCommand, SafetyEvent};

fn manage(req: u64) -> Command {
    Command::Safety(SafetyCommand::Manage {
        req,
        request: Box::new(Request {
            device: LAB,
            action: Action::Dcc {
                state: EnableDisable::DISABLE_INITIATION,
                time: DccTime::Default,
            },
            password: Some(Secret::new(PASSWORD)),
        }),
    })
}

/// Send `manage(req)` straight to the worker and wait for its refusal;
/// returns the reason.
async fn refused(site: &mut Site, req: u64) -> String {
    site.send(vec![manage(req)]).await;
    let mut reason = String::new();
    site.pump_events_until(|event| match event {
        WorkerEvent::Safety(SafetyEvent::Refused {
            req: Some(r),
            reason: why,
        }) if *r == req => {
            reason = why.clone();
            true
        }
        _ => false,
    })
    .await;
    reason
}

fn nothing_sent(site: &Site) {
    for service in [
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
    ] {
        assert!(bodies(site, service).is_empty(), "{service:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn not_armed_or_expired_the_worker_refuses_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = site(dir.path(), DANGEROUS).await;
    let reason = refused(&mut site, 1).await;
    assert!(reason.contains("read-only"), "{reason}");
    // Armed for a minute, then past it on the worker's own clock.
    let arm = SafetyCommand::Arm {
        reason: String::new(),
        minutes: 1,
    };
    site.send(vec![Command::Safety(arm)]).await;
    site.pump_until(|app| app.safety.armed.is_some()).await;
    site.run_for(Duration::from_secs(61)).await;
    let reason = refused(&mut site, 2).await;
    assert!(reason.contains("read-only"), "{reason}");
    nothing_sent(&site);
    let records = audit(dir.path());
    let dcc: Vec<_> = records.iter().filter(|r| r["event"] == "dcc").collect();
    assert_eq!(dcc.len(), 2, "{records:?}");
    assert_eq!(dcc[0]["level"], "read-only");
    assert_eq!(dcc[1]["level"], "write-session (expired)");
    for record in dcc {
        assert_eq!(record["outcome"]["result"], "refused");
        assert_eq!(record["params"]["password"], "[redacted]");
    }
    site.stop().await;
}

/// A passive session refuses before the worker, as it does a write: there
/// is no client and no audit log to reach.
#[tokio::test(start_paused = true)]
async fn a_passive_session_refuses_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server {
        password: Some(PASSWORD),
        ..Server::lab(100, LAB)
    };
    let options = options(dir.path(), DANGEROUS, true);
    let mut site = Site::start_with_options(vec![server], ClientLimits::default(), options).await;
    site.app.safety.passive = true;
    let reason = refused(&mut site, 3).await;
    assert!(reason.contains("passive"), "{reason}");
    nothing_sent(&site);
    site.stop().await;
}

/// The session runs out while the password prompt is open: the
/// confirmation still opens, but confirming sends nothing, and the
/// decline it records carries no password.
#[test]
fn a_session_that_expires_before_the_confirm_sends_nothing() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Ready {
            policy: DANGEROUS,
            workspace: true,
            run: "this-run".into(),
            ledger: Vec::new(),
        }),
    );
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Armed { minutes: 1 }),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(LAB, "10.0.0.100:47808", 260, 0)),
    );
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "reinit coldstart");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "s3cret");
    let start = app.now;
    tick(&mut app, start + Duration::from_secs(61));
    assert!(app.safety.armed.is_none(), "expired");
    assert!(
        matches!(app.overlay, Overlay::Safety(_)),
        "the prompt stays"
    );
    press(&mut app, KeyCode::Enter);
    advance(&mut app, CONFIRM_GUARD);
    type_text(&mut app, &LAB.to_string());
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::Safety(SafetyCommand::Declined { change })] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    let Change::Device(request) = change else {
        panic!("{change:?}");
    };
    assert_eq!(request.password, None);
    assert_eq!(
        request.action,
        Action::Reinit {
            state: ReinitializedState::COLDSTART
        }
    );
    assert_eq!(app.overlay, Overlay::None);
    assert!(
        app.flash.as_ref().unwrap().text.contains("expired"),
        "{:?}",
        app.flash
    );
}

/// A password prompt left open as long as a preview may wait closes and
/// drops what was typed.
#[test]
fn a_password_prompt_left_open_expires() {
    use crate::tui::app::safety::PREVIEW_LIFETIME;
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Ready {
            policy: DANGEROUS,
            workspace: true,
            run: "this-run".into(),
            ledger: Vec::new(),
        }),
    );
    worker(
        &mut app,
        WorkerEvent::Safety(SafetyEvent::Armed { minutes: 15 }),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(LAB, "10.0.0.100:47808", 260, 0)),
    );
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "dcc enable");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "s3cret");
    let start = app.now;
    tick(&mut app, start + PREVIEW_LIFETIME - Duration::from_secs(1));
    assert!(matches!(app.overlay, Overlay::Safety(_)));
    tick(&mut app, start + PREVIEW_LIFETIME);
    assert_eq!(app.overlay, Overlay::None);
    assert!(
        app.flash
            .as_ref()
            .unwrap()
            .text
            .contains("password prompt expired"),
        "{:?}",
        app.flash
    );
}
