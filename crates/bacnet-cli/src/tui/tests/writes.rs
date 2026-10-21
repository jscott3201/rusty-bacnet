//! Write sessions against the lab device on the in-memory site, AO:1
//! commanded to 55 at priority 10: arming, the preview and its tiers,
//! priority 6, SERVICE_REQUEST_DENIED, the ledger, revert-all through the
//! exit guard, expiry, `writes = "never"`, WritePropertyMultiple batches,
//! `--passive`, and the audit log. All on paused time.

use std::path::Path;
use std::time::Duration;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use serde_json::Value;

use super::browser::{open_device, open_object};
use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::safety::{Modal, ARM_HINT, CONFIRM_GUARD};
use crate::tui::app::Overlay;
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::ledger::{Ledger, State};
use crate::tui::safety::{
    Effect, Gate, Level, Outcome, SafetyCommand, SafetyEvent, Tier, WriteSpec,
};
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::write::SafetyOptions;

pub(super) const LAB: u32 = 100;
pub(super) const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];
const CLIENT_MAC: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const WRITE_PROPERTY: u8 = 15;
pub(super) const WRITE_PROPERTY_MULTIPLE: u8 = 16;

pub(super) fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

/// The session options of a profile `lab` with its workspace in `dir`.
pub(super) fn options(dir: &Path, policy: Safety, passive: bool) -> SessionOptions {
    SessionOptions {
        safety: SafetyOptions {
            workspace: Some(dir.to_owned()),
            profile: Some("lab".into()),
            policy,
            passive,
            transport: "BIP",
        },
        ..SessionOptions::default()
    }
}

/// The lab site with a profile workspace in `dir`, discovered, with AO:1
/// open in the browser.
pub(super) async fn lab(dir: &Path, policy: Safety, server: Server) -> Site {
    let options = options(dir, policy, false);
    let mut site = Site::start_with_options(vec![server], ClientLimits::default(), options).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    open_device(&mut site).await;
    open_object(&mut site, "analog-output").await;
    site
}

pub(super) fn modal(app: &App) -> Option<&Modal> {
    match &app.overlay {
        Overlay::Safety(modal) => Some(modal),
        _ => None,
    }
}

/// `W`, Enter on the arm form (15 minutes), and the worker's answer.
pub(super) async fn arm(site: &mut Site) {
    site.press(KeyCode::Char('W')).await;
    assert!(
        matches!(modal(&site.app), Some(Modal::Arm(_))),
        "{:?}",
        site.app.flash
    );
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| app.safety.armed.is_some()).await;
}

/// `w`, then `value` at `priority`, and wait for the preview.
pub(super) async fn write(site: &mut Site, value: &str, priority: &str) {
    site.press(KeyCode::Char('w')).await;
    type_text(&mut site.app, value);
    site.press(KeyCode::Tab).await;
    site.press(KeyCode::Backspace).await;
    site.press(KeyCode::Backspace).await;
    type_text(&mut site.app, priority);
    site.press(KeyCode::Enter).await;
    previewed(site).await;
}

/// Wait for the preview, then let the confirmation guard pass, as a
/// person reading it would.
pub(super) async fn previewed(site: &mut Site) {
    site.pump_until(|app| !matches!(modal(app), Some(Modal::Waiting { .. })))
        .await;
    advance(&mut site.app, CONFIRM_GUARD);
}

/// Wait for the writes to end.
pub(super) async fn finished(site: &mut Site) {
    site.pump_until(|app| matches!(modal(app), Some(Modal::Result(_))))
        .await;
}

pub(super) fn outcomes(app: &App) -> Vec<Outcome> {
    match modal(app) {
        Some(Modal::Result(outcomes)) => outcomes.iter().map(|(_, o)| o.clone()).collect(),
        other => panic!("expected a result, got {other:?}"),
    }
}

pub(super) fn preview(app: &App) -> &crate::tui::safety::Preview {
    match modal(app) {
        Some(Modal::Confirm { preview, .. }) => preview,
        other => panic!("expected a preview, got {other:?}"),
    }
}

pub(super) async fn slot(site: &Site, priority: u32) -> PropertyValue {
    let db = site.server(0).database().read().await;
    let object = db.get(&ao1()).unwrap();
    object
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
        .unwrap()
}

async fn present_value(site: &Site) -> PropertyValue {
    let db = site.server(0).database().read().await;
    let object = db.get(&ao1()).unwrap();
    object
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

/// Every record of `audit.jsonl`, each line parsed alone.
pub(super) fn audit(dir: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap_or_default();
    text.lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object a line"))
        .collect()
}

pub(super) fn writes_sent(site: &Site) -> usize {
    site.hub
        .services_to(&LAB_MAC)
        .iter()
        .filter(|s| [WRITE_PROPERTY, WRITE_PROPERTY_MULTIPLE].contains(s))
        .count()
}

pub(super) fn spec(value: Option<&str>, priority: u8) -> WriteSpec {
    WriteSpec {
        device: LAB,
        object: ao1(),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        value: value.map(str::to_string),
        priority: Some(priority),
    }
}

#[tokio::test(start_paused = true)]
async fn read_only_shows_the_arm_hint_and_sends_no_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    let commands = press(&mut site.app, KeyCode::Char('w'));
    assert!(commands.is_empty(), "{commands:?}");
    assert_eq!(site.app.flash.as_ref().unwrap().text, ARM_HINT);
    assert_eq!(site.app.level(), Level::ReadOnly);
    // The worker refuses on its own too: a request straight to it.
    let prepare = SafetyCommand::Prepare {
        req: 7,
        specs: vec![spec(Some("42"), 8)],
        batch: false,
    };
    site.send(vec![Command::Safety(prepare)]).await;
    site.pump_events_until(|event| {
        matches!(
            event,
            WorkerEvent::Safety(SafetyEvent::Refused { req: Some(7), .. })
        )
    })
    .await;
    assert_eq!(writes_sent(&site), 0);
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn priority_12_previews_that_present_value_will_not_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "42", "12").await;
    let planned = &preview(&site.app).items[0];
    assert_eq!(planned.effect, Effect::Unchanged { held_by: Some(10) });
    assert_eq!(preview(&site.app).gate, Gate::Allowed(Tier::T1));
    assert_snapshot!("write_preview_p12_120x40", screen(&mut site.app, 120, 40));
    assert_snapshot!("write_preview_p12_80x24", screen(&mut site.app, 80, 24));
    // Declining sends nothing and is recorded.
    site.press(KeyCode::Esc).await;
    site.run_for(Duration::from_millis(100)).await;
    assert_eq!(writes_sent(&site), 0);
    let events: Vec<String> = audit(dir.path())
        .iter()
        .map(|r| r["event"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(events, ["arm", "declined"]);
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn priority_8_needs_y_changes_the_present_value_and_is_audited() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "42", "8").await;
    assert_eq!(preview(&site.app).gate, Gate::Allowed(Tier::T2));
    // Enter isn't enough at T2.
    let commands = press(&mut site.app, KeyCode::Enter);
    assert!(commands.is_empty(), "{commands:?}");
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack]);
    assert_eq!(present_value(&site).await, PropertyValue::Real(42.0));
    assert_eq!(slot(&site, 8).await, PropertyValue::Real(42.0));
    // The ledger, on disk too, holds slot 8's pre-image: empty.
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!((entry.priority, &entry.before), (Some(8), &None));
    assert_eq!(entry.state, State::Applied);
    assert_eq!(site.app.safety.ledger, ledger.entries());
    let records = audit(dir.path());
    let write = records.iter().find(|r| r["event"] == "write").unwrap();
    assert_eq!(write["priority"], 8);
    assert_eq!(write["service"], "WriteProperty");
    assert_eq!(write["level"], "write-session");
    assert_eq!(write["outcome"]["result"], "ack");
    assert_eq!(write["pre_image"]["value"], Value::Null, "slot 8 was empty");
    assert_eq!(write["params"]["value"], "42");
    assert_eq!(write["target"]["object"], "analog-output 1");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn priority_6_is_refused_without_the_dangerous_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "1", "6").await;
    let blocked = preview(&site.app).clone();
    assert!(
        matches!(blocked.gate, Gate::Blocked(_)),
        "{:?}",
        blocked.gate
    );
    for key in [KeyCode::Enter, KeyCode::Char('y')] {
        assert!(press(&mut site.app, key).is_empty());
    }
    // The worker refuses it even when the preview claims a tier.
    let mut forged = blocked.clone();
    forged.gate = Gate::Allowed(Tier::T3);
    let execute = SafetyCommand::Execute {
        req: 9,
        preview: Box::new(forged),
    };
    site.send(vec![Command::Safety(execute)]).await;
    site.pump_events_until(|event| {
        matches!(
            event,
            WorkerEvent::Safety(SafetyEvent::Refused { req: Some(9), .. })
        )
    })
    .await;
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(slot(&site, 6).await, PropertyValue::Null);
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_dangerous_opt_in_allows_priority_6_at_t3() {
    let dir = tempfile::tempdir().unwrap();
    let policy = Safety {
        writes: Writes::Session,
        dangerous: true,
    };
    let mut site = lab(dir.path(), policy, Server::lab(100, LAB)).await;
    arm(&mut site).await;
    assert_eq!(site.app.level(), Level::Armed { dangerous: true });
    write(&mut site, "1", "6").await;
    assert_eq!(preview(&site.app).gate, Gate::Allowed(Tier::T3));
    type_text(&mut site.app, &LAB.to_string());
    site.press(KeyCode::Enter).await;
    finished(&mut site).await;
    assert_eq!(slot(&site, 6).await, PropertyValue::Real(1.0));
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn t3_needs_the_device_instance_typed() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "1", "5").await;
    assert_eq!(preview(&site.app).gate, Gate::Allowed(Tier::T3));
    assert_snapshot!("write_preview_t3_80x24", screen(&mut site.app, 80, 24));
    for wrong in ["", "99", "1000"] {
        type_text(&mut site.app, wrong);
        assert!(press(&mut site.app, KeyCode::Enter).is_empty(), "{wrong:?}");
        let Some(Modal::Confirm { error, .. }) = modal(&site.app) else {
            panic!("still confirming");
        };
        assert!(error.as_deref().unwrap().contains("100"));
    }
    assert_eq!(writes_sent(&site), 0);
    type_text(&mut site.app, "100");
    site.press(KeyCode::Enter).await;
    finished(&mut site).await;
    assert_eq!(slot(&site, 5).await, PropertyValue::Real(1.0));
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn quitting_lists_the_override_and_revert_all_relinquishes_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "42", "8").await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    site.press(KeyCode::Esc).await;
    site.press(KeyCode::Char('q')).await;
    assert!(!site.app.quit, "the exit guard comes first");
    assert!(matches!(modal(&site.app), Some(Modal::ExitGuard(None))));
    assert_snapshot!("exit_guard_120x40", screen(&mut site.app, 120, 40));
    let before = writes_sent(&site);
    site.press(KeyCode::Char('r')).await;
    site.pump_until(|app| app.quit).await;
    assert_eq!(writes_sent(&site), before + 1);
    assert_eq!(slot(&site, 8).await, PropertyValue::Null);
    assert_eq!(present_value(&site).await, PropertyValue::Real(55.0));
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    let records = audit(dir.path());
    let revert = records.iter().find(|r| r["event"] == "revert").unwrap();
    assert_eq!(revert["params"]["value"], "NULL");
    assert_eq!(revert["outcome"]["result"], "ack");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn revert_puts_back_another_commanders_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "60", "10").await;
    let planned = &preview(&site.app).items[0];
    assert_eq!(planned.before.as_ref().unwrap().text, "55", "the pre-image");
    site.press(KeyCode::Enter).await;
    finished(&mut site).await;
    assert_eq!(slot(&site, 10).await, PropertyValue::Real(60.0));
    site.press(KeyCode::Esc).await;
    site.press(KeyCode::Char('u')).await;
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.ledger.is_empty()).await;
    assert_eq!(slot(&site, 10).await, PropertyValue::Real(55.0));
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_writes_never_profile_says_why_and_arms_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let policy = Safety {
        writes: Writes::Never,
        dangerous: false,
    };
    let mut site = lab(dir.path(), policy, Server::lab(100, LAB)).await;
    let commands = press(&mut site.app, KeyCode::Char('W'));
    assert!(commands.is_empty(), "{commands:?}");
    assert!(site
        .app
        .flash
        .as_ref()
        .unwrap()
        .text
        .contains("writes = \"never\""));
    assert_eq!(site.app.overlay, Overlay::None);
    // Straight to the worker: refused, and the refusal recorded.
    let arm = SafetyCommand::Arm {
        reason: String::new(),
        minutes: 15,
    };
    site.send(vec![Command::Safety(arm)]).await;
    site.pump_events_until(|event| {
        matches!(
            event,
            WorkerEvent::Safety(SafetyEvent::Refused { req: None, .. })
        )
    })
    .await;
    assert_eq!(site.app.level(), Level::ReadOnly);
    let records = audit(dir.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["event"], "arm");
    assert_eq!(records[0]["outcome"]["result"], "refused");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_worker_disarms_at_expiry_on_its_own_clock() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    let arm = SafetyCommand::Arm {
        reason: "ticket 7".into(),
        minutes: 1,
    };
    site.send(vec![Command::Safety(arm)]).await;
    site.pump_until(|app| app.safety.armed.is_some()).await;
    site.run_for(Duration::from_secs(61)).await;
    let prepare = SafetyCommand::Prepare {
        req: 3,
        specs: vec![spec(Some("42"), 12)],
        batch: false,
    };
    site.send(vec![Command::Safety(prepare)]).await;
    site.pump_events_until(|event| {
        matches!(
            event,
            WorkerEvent::Safety(SafetyEvent::Refused { req: Some(3), .. })
        )
    })
    .await;
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(audit(dir.path())[0]["reason"], "ticket 7");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn service_request_denied_is_shown_apart_from_other_errors() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server {
        deny_writes: true,
        ..Server::lab(100, LAB)
    };
    let mut site = lab(dir.path(), Safety::default(), server).await;
    arm(&mut site).await;
    write(&mut site, "42", "12").await;
    site.press(KeyCode::Enter).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Denied]);
    assert_snapshot!("write_denied_80x24", screen(&mut site.app, 80, 24));
    assert!(site.app.safety.ledger.is_empty(), "nothing took effect");
    let records = audit(dir.path());
    let write = records.iter().find(|r| r["event"] == "write").unwrap();
    assert_eq!(write["outcome"]["denied"], true);
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_writem_batch_is_one_wpm_confirmed_with_y() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "writem 100 ao:1 pv 41@12 av:1 pv 7");
    site.press(KeyCode::Enter).await;
    previewed(&mut site).await;
    let batch = preview(&site.app);
    assert!(batch.batch);
    assert_eq!(batch.gate, Gate::Allowed(Tier::T2));
    assert_snapshot!("writem_preview_120x40", screen(&mut site.app, 120, 40));
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack, Outcome::Ack]);
    assert_eq!(
        site.hub
            .services_to(&LAB_MAC)
            .iter()
            .filter(|s| **s == WRITE_PROPERTY_MULTIPLE)
            .count(),
        1
    );
    assert_eq!(slot(&site, 12).await, PropertyValue::Real(41.0));
    assert_eq!(site.app.safety.ledger.len(), 2);
    site.stop().await;
}

/// A quoted value keeps its space: the preview and the write carry the
/// string `two words` (#1800).
#[tokio::test(start_paused = true)]
async fn a_quoted_writem_value_writes_the_string_with_its_space() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "writem 100 ao:1 description \"two words\"");
    site.press(KeyCode::Enter).await;
    previewed(&mut site).await;
    let batch = preview(&site.app);
    let shown = format!("{batch:?}");
    assert!(shown.contains("two words"), "{shown}");
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack]);
    let db = site.server(0).database().read().await;
    let description = db
        .get(&ao1())
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    drop(db);
    assert_eq!(
        description,
        PropertyValue::CharacterString("two words".into())
    );
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_write_in_flight_at_the_end_of_the_session_is_still_recorded() {
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
    // The UI goes away while the request is out.
    site.quit().await;
    let ledger = Ledger::open(dir.path()).unwrap();
    assert_eq!(ledger.entries().len(), 1);
    assert_eq!(ledger.entries()[0].state, State::Unknown);
    let records = audit(dir.path());
    let write = records.iter().find(|r| r["event"] == "write").unwrap();
    assert_eq!(write["outcome"]["result"], "timeout");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_passive_session_puts_nothing_on_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let options = options(dir.path(), Safety::default(), true);
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    site.app.safety.passive = true;
    // The keys go nowhere, and say why.
    for key in [KeyCode::Char('d'), KeyCode::Char('W'), KeyCode::Enter] {
        site.app.flash = None;
        assert!(press(&mut site.app, key).is_empty(), "{key:?}");
        assert_eq!(site.app.overlay, Overlay::None, "{key:?}");
        let flash = &site.app.flash.as_ref().unwrap().text;
        assert!(flash.starts_with("Passive"), "{key:?}: {flash}");
    }
    assert_eq!(site.app.level(), Level::Passive);
    // Straight to the worker: each one refused.
    let who_is = crate::tui::message::WhoIsSpec {
        scope: crate::tui::message::WhoIsScope::Global,
        range: None,
        listen: Duration::from_secs(1),
    };
    site.send(vec![
        Command::WhoIs {
            op: 1,
            spec: who_is,
        },
        Command::Browse {
            req: 1,
            device: LAB,
            read: crate::tui::point::BrowseRead::Device,
        },
        Command::Watch(crate::tui::watched::WatchCommand::Add {
            row: 1,
            point: crate::tui::watched::Point {
                device: LAB,
                network: None,
                object: ao1(),
                property: PropertyIdentifier::PRESENT_VALUE,
            },
        }),
        Command::Safety(SafetyCommand::Arm {
            reason: String::new(),
            minutes: 15,
        }),
        Command::Safety(SafetyCommand::Revert { req: 4 }),
    ])
    .await;
    site.pump_events_until(|event| {
        matches!(
            event,
            WorkerEvent::Safety(SafetyEvent::Refused { req: Some(4), .. })
        )
    })
    .await;
    site.run_for(Duration::from_secs(5)).await;
    assert_eq!(site.hub.frames_from(&CLIENT_MAC), 0);
    site.stop().await;
}

/// In production a passive session builds no client at all.
#[tokio::test(start_paused = true)]
async fn a_passive_session_builds_no_client() {
    use crate::tui::worker::session::{spawn, ConnectPlan, TransportKind};
    use crate::tui::worker::{EventSink, EVENT_CHANNEL_CAPACITY};
    let dir = tempfile::tempdir().unwrap();
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        // 47808 on every interface: binding it would be visible.
        args: crate::tui::profile::Profile::default()
            .transport_args()
            .unwrap(),
        await_interface: false,
    };
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (commands, rx) = tokio::sync::mpsc::channel(16);
    let task = spawn(plan, sink, rx, options(dir.path(), Safety::default(), true));
    assert_eq!(events.recv().await, Some(WorkerEvent::Passive));
    assert!(matches!(
        events.recv().await,
        Some(WorkerEvent::Safety(SafetyEvent::Ready {
            workspace: false,
            ..
        }))
    ));
    drop(commands);
    site_bounded(task).await.unwrap();
}

async fn site_bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    super::site::bounded(future).await
}
