//! The ledger's edges on the lab site: a WritePropertyMultiple that fails
//! part-way, a batch naming one property twice, the exit guard while a
//! write is in flight, keeping entries with no connected session, a typed
//! `null`, a stale preview, two sessions on one profile, a send a crash
//! cut short, and a revert after a restart. All on paused time.

use std::path::Path;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::writes::*;
use super::*;
use crate::tui::app::safety::Modal;
use crate::tui::profile::Safety;
use crate::tui::safety::ledger::{Entry, Ledger, State};
use crate::tui::safety::{Outcome, SafetyCommand, SafetyEvent, Stored};
use crate::tui::worker::write::STALE;
use crate::tui::worker::{EventSink, EVENT_CHANNEL_CAPACITY};

/// A ledger file holding `entries`, as an earlier run left it.
fn leave_ledger(dir: &Path, entries: &[Entry]) {
    let file = serde_json::json!({ "version": 1, "entries": entries });
    std::fs::write(dir.join("ledger.json"), file.to_string()).unwrap();
}

/// AO:1 commanded to 42 at priority 8 by an earlier run, in `state`.
fn earlier(state: State) -> Entry {
    Entry {
        id: 1,
        run: "earlier-run".into(),
        at: "2026-10-18T12:00:00.000Z".into(),
        device: LAB,
        object_type: ObjectType::ANALOG_OUTPUT.to_raw(),
        instance: 1,
        property: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        index: None,
        priority: Some(8),
        commandable: true,
        before: None,
        written: Stored {
            bytes: vec![0x44, 0x42, 0x28, 0x00, 0x00],
            text: "42".into(),
        },
        state,
        kept: false,
        seq: 1,
    }
}

fn events(dir: &Path) -> Vec<String> {
    audit(dir)
        .iter()
        .map(|r| r["event"].as_str().unwrap().to_string())
        .collect()
}

/// The device refuses the second write of three: the first is applied and
/// tracked, the second isn't, the third was never tried (Clause 15.10.2).
#[tokio::test(start_paused = true)]
async fn a_batch_failing_part_way_tracks_only_the_writes_before_the_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    site.press(KeyCode::Char(':')).await;
    // A string is the wrong datatype for AV:1's Present_Value.
    type_text(
        &mut site.app,
        "writem 100 ao:1 pv 41@12 av:1 pv \"x\"@12 bv:1 pv active@12",
    );
    site.press(KeyCode::Enter).await;
    previewed(&mut site).await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    let outcomes = outcomes(&site.app);
    assert_eq!(outcomes[0], Outcome::Ack, "{outcomes:?}");
    assert!(matches!(outcomes[1], Outcome::Error { .. }), "{outcomes:?}");
    assert!(matches!(outcomes[2], Outcome::NotSent(_)), "{outcomes:?}");
    assert_eq!(slot(&site, 12).await, PropertyValue::Real(41.0));
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.object(), Some(ao1()));
    assert_eq!(entry.state, State::Applied);
    site.stop().await;
}

/// A batch naming one property twice is refused by the worker even when
/// the preview claims it is fine.
#[tokio::test(start_paused = true)]
async fn the_worker_refuses_a_batch_naming_one_property_twice() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "1", "12").await;
    let mut forged = preview(&site.app).clone();
    let mut second = forged.items[0].clone();
    second.priority = Some(13);
    second.spec.priority = Some(13);
    forged.items.push(second);
    forged.batch = true;
    let execute = SafetyCommand::Execute {
        req: 9,
        preview: Box::new(forged),
    };
    site.send(vec![Command::Safety(execute)]).await;
    site.pump_events_until(|event| {
        matches!(event, WorkerEvent::Safety(SafetyEvent::Refused { req: Some(9), reason })
            if reason.contains("appears twice"))
    })
    .await;
    assert_eq!(writes_sent(&site), 0);
    let records = audit(dir.path());
    let refused = records.iter().find(|r| r["event"] == "write").unwrap();
    assert_eq!(refused["outcome"]["result"], "refused");
    assert_eq!(
        refused["items"].as_array().unwrap().len(),
        2,
        "names its targets"
    );
    site.stop().await;
}

/// Quitting while the first write is still out goes through the exit
/// guard: the ledger already holds it.
#[tokio::test(start_paused = true)]
async fn the_exit_guard_counts_a_write_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server {
        quirks: Quirks {
            swallow_writes: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = lab(dir.path(), Safety::default(), server).await;
    arm(&mut site).await;
    write(&mut site, "42", "12").await;
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| !app.safety.ledger.is_empty()).await;
    assert_eq!(site.app.safety.ledger[0].state, State::Sending);
    // Close the "Writing..." view, then quit.
    site.press(KeyCode::Esc).await;
    site.press(KeyCode::Char('q')).await;
    assert!(!site.app.quit);
    assert!(matches!(modal(&site.app), Some(Modal::ExitGuard(_))));
    assert!(events(dir.path()).contains(&"sending".to_string()));
    site.stop().await;
}

/// The exit guard also waits on a request with nothing in the ledger yet.
#[test]
fn the_exit_guard_waits_for_a_pending_request() {
    let mut app = connected_app();
    app.safety.pending = Some(3);
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert!(matches!(modal(&app), Some(Modal::ExitGuard(_))));
    let commands = press(&mut app, KeyCode::Char('k'));
    assert_eq!(commands, [Command::Safety(SafetyCommand::Keep)]);
    assert!(app.quit);
}

/// Keep in a passive session flags the entries and records it.
#[tokio::test(start_paused = true)]
async fn keep_in_a_passive_session_flags_and_records() {
    use crate::tui::worker::session::{spawn, ConnectPlan, TransportKind};
    let dir = tempfile::tempdir().unwrap();
    leave_ledger(dir.path(), &[earlier(State::Applied)]);
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args: crate::tui::profile::Profile::default()
            .transport_args()
            .unwrap(),
        await_interface: false,
    };
    let (sink, mut events_rx) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (commands, rx) = tokio::sync::mpsc::channel(16);
    let task = spawn(plan, sink, rx, options(dir.path(), Safety::default(), true));
    assert_eq!(events_rx.recv().await, Some(WorkerEvent::Passive));
    commands
        .send(Command::Safety(SafetyCommand::Keep))
        .await
        .unwrap();
    drop(commands);
    super::site::bounded(task).await.unwrap();
    assert!(Ledger::open(dir.path()).unwrap().entries()[0].kept);
    let records = audit(dir.path());
    assert_eq!(records[0]["event"], "kept");
    assert_eq!(records[0]["flagged"], true);
    assert_eq!(records[0]["level"], "passive");
}

/// Keep with no session at all (disconnected): the coordinator does it.
#[tokio::test(start_paused = true)]
async fn keep_with_no_session_is_done_by_the_coordinator() {
    let dir = tempfile::tempdir().unwrap();
    leave_ledger(dir.path(), &[earlier(State::Applied)]);
    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (commands, rx) = tokio::sync::mpsc::channel(16);
    let task = crate::tui::worker::coordinator::spawn(
        None,
        None,
        sink,
        rx,
        options(dir.path(), Safety::default(), false),
    );
    commands
        .send(Command::Safety(SafetyCommand::Keep))
        .await
        .unwrap();
    drop(commands);
    super::site::bounded(task).await.unwrap();
    assert!(Ledger::open(dir.path()).unwrap().entries()[0].kept);
    assert_eq!(events(dir.path()), ["kept"]);
}

/// `null` typed as a value is a relinquish: its tier, and its record.
#[tokio::test(start_paused = true)]
async fn a_typed_null_is_relinquished_and_recorded_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "null", "10").await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(slot(&site, 10).await, PropertyValue::Null);
    let records = audit(dir.path());
    let relinquish = records.iter().find(|r| r["event"] == "relinquish").unwrap();
    assert_eq!(relinquish["tier"], "T2");
    assert_eq!(relinquish["pre_image"]["value"], "55");
    site.stop().await;
}

/// A slot that changed between the preview and the confirmation is not
/// written: the preview no longer says what the write would do.
#[tokio::test(start_paused = true)]
async fn a_preview_gone_stale_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "42", "12").await;
    {
        let mut db = site.server(0).database().write().await;
        // Another commander takes slot 12.
        let operator = bacnet_objects::command_source::CommandOrigin::Local {
            owner_device: bacnet_types::primitives::ObjectIdentifier::new(ObjectType::DEVICE, LAB)
                .unwrap(),
            initiating_object: None,
        };
        db.get_mut(&ao1())
            .unwrap()
            .write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(50.0),
                Some(12),
                &operator,
            )
            .unwrap();
    }
    site.press(KeyCode::Enter).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::NotSent(STALE.into())]);
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(slot(&site, 12).await, PropertyValue::Real(50.0));
    let records = audit(dir.path());
    let declined = records.iter().find(|r| r["event"] == "declined").unwrap();
    assert_eq!(declined["why"], "stale");
    site.stop().await;
}

/// Two sessions on one profile: the second can't arm while the first
/// holds the workspace, says who does, and records the refusal.
#[tokio::test(start_paused = true)]
async fn a_second_session_on_the_profile_cannot_arm() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut first).await;
    let options = options(dir.path(), Safety::default(), false);
    let mut second = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    second.pump_until(|app| app.safety.policy.is_some()).await;
    let arm_request = SafetyCommand::Arm {
        reason: String::new(),
        minutes: 15,
    };
    second.send(vec![Command::Safety(arm_request)]).await;
    let mut why = String::new();
    second
        .pump_events_until(|event| match event {
            WorkerEvent::Safety(SafetyEvent::Refused { req: None, reason }) => {
                why = reason.clone();
                true
            }
            _ => false,
        })
        .await;
    assert!(why.contains("in use"), "{why}");
    // Windows does not let another handle read a locked file, so only Unix
    // can name the holder (see `safety::lock`).
    #[cfg(unix)]
    assert!(why.contains(&std::process::id().to_string()), "{why}");
    assert!(second.app.safety.armed.is_none());
    let refused = audit(dir.path())
        .into_iter()
        .filter(|r| r["event"] == "arm")
        .find(|r| r["outcome"]["result"] == "refused")
        .expect("the refusal is recorded");
    assert!(refused["outcome"]["why"]
        .as_str()
        .unwrap()
        .contains("in use"));
    second.stop().await;
    first.stop().await;
}

/// A write a crash cut short (still `sending`) becomes `unknown`, and the
/// audit log says the session ended.
#[tokio::test(start_paused = true)]
async fn a_send_cut_short_by_a_crash_is_recorded_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    leave_ledger(dir.path(), &[earlier(State::Sending)]);
    let options = options(dir.path(), Safety::default(), false);
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    assert_eq!(site.app.safety.ledger[0].state, State::Unknown);
    let records = audit(dir.path());
    assert_eq!(records[0]["event"], "unknown");
    assert_eq!(records[0]["outcome"]["why"], "session ended");
    site.stop().await;
}

/// After a restart nothing has been discovered yet: revert finds the
/// device first rather than failing.
#[tokio::test(start_paused = true)]
async fn revert_after_a_restart_finds_the_device_first() {
    let dir = tempfile::tempdir().unwrap();
    leave_ledger(dir.path(), &[earlier(State::Applied)]);
    let options = options(dir.path(), Safety::default(), false);
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    assert!(matches!(modal(&site.app), Some(Modal::Ledger { .. })));
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.ledger.is_empty() && app.safety.pending.is_none())
        .await;
    assert_eq!(writes_sent(&site), 1);
    assert_eq!(slot(&site, 8).await, PropertyValue::Null);
    site.stop().await;
}
