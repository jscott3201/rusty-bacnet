//! Attached agents' reverts, approved here (#2185): the desk sends exactly
//! the revert that was confirmed, refusing it as stale when the entries the
//! dialog showed changed since (an id another write took, a write merged
//! into an entry), and undoing a Relinquish_Default write is T3, as the
//! write was. On paused time, against the in-process lab.

use bacnet_types::primitives::PropertyValue;
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::mcp_approve::{armed_site, asking, shown, write};
use super::mcp_attach::Agent;
use super::site::Site;
use super::writes::{modal, slot};
use super::*;
use crate::tui::app::safety::{CONFIRM_GUARD, Modal};
use crate::tui::safety::{Change, Gate, SafetyCommand, Tier};

/// `bacnet_revert` of the entries `ids`, as request `id`.
fn revert(id: u64, ids: &[u64]) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": "bacnet_revert", "arguments": { "entries": ids } } })
}

/// Confirm the T2 dialog on screen once its guard has passed.
async fn confirm_t2(site: &mut Site) {
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
}

/// The reason of a refused result.
fn reason(answer: &Value) -> &Value {
    &answer["result"]["structuredContent"]["reason"]
}

/// An agent's confirmed write of AO:1 = `value` at `priority`.
async fn written(site: &mut Site, dir: &std::path::Path, value: u32, priority: u8) {
    let mut call = asking(dir, write(2, value, priority)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    confirm_t2(site).await;
    let (_agent, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
}

/// A revert of entry 1 waiting behind a write of AO:1 = 43 at `priority`
/// on screen: (the write's call, the revert's call).
async fn revert_behind_a_write(
    site: &mut Site,
    dir: &std::path::Path,
    priority: u8,
) -> (JoinHandle<(Agent, Value)>, JoinHandle<(Agent, Value)>) {
    let write_call = asking(dir, write(2, 43, priority)).await;
    site.pump_until(|app| app.safety.agents.held() == 1).await;
    let revert_call = asking(dir, revert(3, &[1])).await;
    site.pump_until(|app| app.safety.agents.held() == 2).await;
    (write_call, revert_call)
}

/// Entry 1 is reverted and a new write to another slot takes its id
/// before the revert is confirmed: the confirmed revert is refused as
/// stale, and the new write stays.
#[tokio::test(start_paused = true)]
async fn a_reused_id_makes_the_revert_stale() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    written(&mut site, dir.path(), 42, 8).await;
    let (mut write_call, mut revert_call) = revert_behind_a_write(&mut site, dir.path(), 7).await;
    // The person reverts everything here first; the ledger is empty.
    site.send(vec![Command::Safety(SafetyCommand::Revert { req: 99 })])
        .await;
    site.pump_until(|app| app.safety.ledger.is_empty()).await;
    confirm_t2(&mut site).await;
    let (_a, answer) = site.pump_while(&mut write_call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    let entries = crate::tui::safety::ledger::Ledger::open(dir.path()).unwrap();
    assert_eq!(entries.entries()[0].id, 1, "the new write took id 1");
    site.pump_until(|app| shown(app).is_some_and(|who| who.contains("to revert")))
        .await;
    confirm_t2(&mut site).await;
    let (_b, answer) = site.pump_while(&mut revert_call).await;
    assert_eq!(reason(&answer), "stale", "{answer}");
    assert_eq!(
        slot(&site, 7).await,
        PropertyValue::Real(43.0),
        "not undone"
    );
    site.quit().await;
    site.stop().await;
}

/// A second write to the slot merges into entry 1 before the revert is
/// confirmed: what the dialog showed would put back the wrong value, so
/// it is refused as stale.
#[tokio::test(start_paused = true)]
async fn a_merged_entry_makes_the_revert_stale() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    written(&mut site, dir.path(), 42, 8).await;
    let (mut write_call, mut revert_call) = revert_behind_a_write(&mut site, dir.path(), 8).await;
    confirm_t2(&mut site).await;
    let (_a, answer) = site.pump_while(&mut write_call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    site.pump_until(|app| shown(app).is_some_and(|who| who.contains("to revert")))
        .await;
    confirm_t2(&mut site).await;
    let (_b, answer) = site.pump_while(&mut revert_call).await;
    assert_eq!(reason(&answer), "stale", "{answer}");
    assert_eq!(
        slot(&site, 8).await,
        PropertyValue::Real(43.0),
        "not undone"
    );
    site.quit().await;
    site.stop().await;
}

/// Undoing a Relinquish_Default write is T3 over MCP, as the write was.
#[tokio::test(start_paused = true)]
async fn undoing_relinquish_default_is_t3() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let fallback = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "bacnet_write", "arguments": { "device": super::mcp_attach::LAB,
        "writes": [{ "object": "ao:1", "property": "relinquish-default", "value": 10 }] } } });
    let mut call = asking(dir.path(), fallback).await;
    site.pump_until(|app| shown(app).is_some()).await;
    advance(&mut site.app, CONFIRM_GUARD);
    type_text(&mut site.app, "100");
    site.press(KeyCode::Enter).await;
    let (_a, answer) = site.pump_while(&mut call).await;
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    let mut undo = asking(dir.path(), revert(3, &[1])).await;
    site.pump_until(|app| shown(app).is_some()).await;
    let Some(Modal::Confirm {
        change: Change::Write(preview),
        ..
    }) = modal(&site.app)
    else {
        panic!("the revert's dialog");
    };
    assert_eq!(preview.gate, Gate::Allowed(Tier::T3));
    site.press(KeyCode::Esc).await;
    let (_b, answer) = site.pump_while(&mut undo).await;
    assert_eq!(reason(&answer), "declined", "{answer}");
    site.quit().await;
    site.stop().await;
}

/// A revert of an entry with nothing to put back asks nothing and sends
/// nothing, even when the entry that id names by then has a pre-image:
/// the ledger as read says entry 1 is unknown, the TUI's own holds a write
/// to slot 8 under that id, and slot 8 keeps it.
#[tokio::test(start_paused = true)]
async fn nothing_to_put_back_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    written(&mut site, dir.path(), 42, 8).await;
    // The file the call reads now says entry 1's old value is unknown.
    let path = dir.path().join("ledger.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["entries"][0]["before"] = Value::Null;
    file["entries"][0]["commandable"] = json!(false);
    std::fs::write(&path, file.to_string()).unwrap();
    let mut undo = asking(dir.path(), revert(3, &[1])).await;
    let (_agent, answer) = site.pump_while(&mut undo).await;
    assert!(shown(&site.app).is_none(), "nothing to confirm");
    let outcome = answer["result"]["structuredContent"]["results"][0]["outcome"].to_string();
    assert!(outcome.contains("old value"), "{answer}");
    assert_eq!(
        slot(&site, 8).await,
        PropertyValue::Real(42.0),
        "not undone"
    );
    assert_eq!(site.app.safety.ledger.len(), 1, "the entry stays");
    site.quit().await;
    site.stop().await;
}
