//! Attached agents' writes, approved here (#2185), continued: the dialog's
//! keys at each tier and its guard against typing ahead, a filter being
//! typed when a request arrives, what the desk refuses after a confirm (a
//! stale preview, a session no longer armed), reverts, and a cancelled
//! call. On paused time, against the in-process lab.

use bacnet_objects::command_source::CommandOrigin;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

use super::mcp_approve::{armed_site, asking, shown, write};
use super::mcp_attach::{LAB, attached};
use super::site::Site;
use super::writes::{ao1, modal, slot, writes_sent};
use super::*;
use crate::tui::app::safety::{CONFIRM_GUARD, Modal};
use crate::tui::safety::SafetyCommand;

/// What the dialog on screen has typed toward a T3 confirmation.
fn typed(app: &App) -> String {
    match modal(app) {
        Some(Modal::Confirm { typed, .. }) => typed.clone(),
        other => panic!("expected the dialog, got {other:?}"),
    }
}

/// The structured result's refusal reason.
fn reason(answer: &Value) -> &Value {
    &answer["result"]["structuredContent"]["reason"]
}

/// `bacnet_revert` of everything, as request `id`.
fn revert_all(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": "bacnet_revert", "arguments": { "all": true } } })
}

/// Confirm the dialog on screen with `y`, once its guard has passed.
async fn confirm_t2(site: &mut Site) {
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
}

/// A T3 write takes the device instance typed out: keys typed before the
/// guard passes are dropped, digits included, and a wrong instance is
/// refused with nothing sent.
#[tokio::test(start_paused = true)]
async fn t3_takes_the_instance_and_drops_typing_ahead() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 2)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    screen(&mut site.app, 120, 40);
    type_text(&mut site.app, "100");
    site.press(KeyCode::Enter).await;
    assert_eq!(typed(&site.app), "", "typed ahead, so dropped");
    advance(&mut site.app, CONFIRM_GUARD);
    type_text(&mut site.app, "999");
    site.press(KeyCode::Enter).await;
    let Some(Modal::Confirm { error, .. }) = modal(&site.app) else {
        panic!("the dialog stays");
    };
    assert!(error.as_ref().unwrap().contains("100"), "{error:?}");
    assert_eq!(writes_sent(&site), 0);
    type_text(&mut site.app, "100");
    site.press(KeyCode::Enter).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    assert_eq!(slot(&site, 2).await, PropertyValue::Real(42.0));
    site.quit().await;
    site.stop().await;
}

/// The guard runs from the first frame that shows the dialog, however
/// long it sat undrawn.
#[tokio::test(start_paused = true)]
async fn the_guard_starts_when_the_dialog_is_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 12)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    // Time passes with no frame: the dialog was never seen.
    site.app.now += CONFIRM_GUARD * 4;
    site.press(KeyCode::Enter).await;
    assert!(shown(&site.app).is_some(), "not yet drawn, so too soon");
    screen(&mut site.app, 120, 40);
    site.press(KeyCode::Enter).await;
    assert!(shown(&site.app).is_some(), "drawn just now");
    // T1 takes Enter once it has been on screen a moment.
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Enter).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    assert_eq!(slot(&site, 12).await, PropertyValue::Real(42.0));
    site.quit().await;
    site.stop().await;
}

/// A request that arrives while a filter is typed waits: the filter keeps
/// its keys, and the dialog opens once the filter is done.
#[tokio::test(start_paused = true)]
async fn a_filter_being_typed_keeps_its_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    site.press(KeyCode::Char('/')).await;
    assert!(site.app.filter_editing);
    let mut call = asking(dir.path(), write(2, 42, 2)).await;
    site.pump_until(|app| app.safety.agents.held() == 1).await;
    advance(&mut site.app, CONFIRM_GUARD);
    type_text(&mut site.app, "100");
    assert!(
        modal(&site.app).is_none(),
        "no dialog while the filter is typed"
    );
    assert_eq!(site.app.filter_text(), "100");
    site.press(KeyCode::Enter).await;
    assert!(!site.app.filter_editing);
    advance(&mut site.app, Duration::from_secs(1));
    assert!(
        shown(&site.app).is_some(),
        "it opens once the filter is done"
    );
    assert_eq!(writes_sent(&site), 0);
    site.press(KeyCode::Esc).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(reason(&answer), "declined", "{answer}");
    site.quit().await;
    site.stop().await;
}

/// The desk reads the targets again after the confirm: a slot another
/// commander took meanwhile makes the preview stale, and nothing goes.
#[tokio::test(start_paused = true)]
async fn a_preview_gone_stale_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    {
        let mut db = site.server(0).database().write().await;
        let operator = CommandOrigin::Local {
            owner_device: ObjectIdentifier::new(ObjectType::DEVICE, LAB).unwrap(),
            initiating_object: None,
        };
        db.get_mut(&ao1())
            .unwrap()
            .write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(50.0),
                Some(8),
                &operator,
            )
            .unwrap();
    }
    confirm_t2(&mut site).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(reason(&answer), "stale", "{answer}");
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(slot(&site, 8).await, PropertyValue::Real(50.0));
    site.quit().await;
    site.stop().await;
}

/// A write session that ended between the confirm and the send refuses
/// the write at the desk, as the TUI's own; nothing goes.
#[tokio::test(start_paused = true)]
async fn a_session_disarmed_before_the_send_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    // The worker's session expires; the app hasn't heard yet.
    let disarm = SafetyCommand::Disarm { expired: true };
    site.send(vec![Command::Safety(disarm)]).await;
    // Let the worker take it, without the app hearing of it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    confirm_t2(&mut site).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(true), "{answer}");
    let outcome = answer["result"]["structuredContent"]["results"][0]["outcome"].to_string();
    assert!(outcome.contains("arm"), "{answer}");
    assert_eq!(writes_sent(&site), 0);
    site.quit().await;
    site.stop().await;
}

/// A revert is confirmed here too, at the tier of the write it undoes,
/// and the desk refuses it unless this session may write: refused unshown
/// while disarmed, and at the desk when the session ended after the
/// confirm.
#[tokio::test(start_paused = true)]
async fn a_revert_needs_a_confirm_and_an_armed_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    confirm_t2(&mut site).await;
    let (mut agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");

    // Confirmed here, but the worker's session ended first.
    let mut revert = tokio::spawn(async move {
        let answer = agent.request(revert_all(3)).await;
        (agent, answer)
    });
    site.pump_until(|app| shown(app).is_some()).await;
    assert!(shown(&site.app).unwrap().contains("to revert"));
    let disarm = SafetyCommand::Disarm { expired: true };
    site.send(vec![Command::Safety(disarm)]).await;
    // Let the worker take it, without the app hearing of it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    confirm_t2(&mut site).await;
    let (mut agent, answer) = site.pump_while(&mut revert).await;
    assert_eq!(reason(&answer), "tui-refused", "{answer}");
    assert_eq!(slot(&site, 8).await, PropertyValue::Real(42.0));

    // Disarmed here as well: refused without a dialog.
    site.press(KeyCode::Char('W')).await;
    site.pump_until(|app| app.safety.armed.is_none()).await;
    let mut revert = tokio::spawn(async move {
        let answer = agent.request(revert_all(4)).await;
        (agent, answer)
    });
    let (_agent, answer) = site.pump_while(&mut revert).await;
    assert_eq!(reason(&answer), "not-armed", "{answer}");
    assert_eq!(slot(&site, 8).await, PropertyValue::Real(42.0));
    site.quit().await;
    site.stop().await;
}

/// An agent's `notifications/cancelled` withdraws its request: the dialog
/// closes and nothing is sent.
#[tokio::test(start_paused = true)]
async fn a_cancelled_call_is_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut agent = attached(&bacnet_mcp::attach::socket_path(dir.path()).unwrap()).await;
    let call = format!("{}\n", write(2, 42, 8));
    agent.write.write_all(call.as_bytes()).await.unwrap();
    site.pump_until(|app| shown(app).is_some()).await;
    let cancel = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": { "requestId": 2, "reason": "changed my mind" } });
    let cancel = format!("{cancel}\n");
    agent.write.write_all(cancel.as_bytes()).await.unwrap();
    site.run_for(Duration::from_millis(100)).await;
    advance(&mut site.app, Duration::from_secs(1));
    assert_eq!(site.app.safety.agents.held(), 0);
    assert!(modal(&site.app).is_none(), "its dialog closed");
    assert_eq!(writes_sent(&site), 0);
    site.quit().await;
    site.stop().await;
}
