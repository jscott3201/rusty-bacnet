//! Attached agents' writes, approved here (#2185): an agent on the socket
//! asks to write the lab's AO:1 (commanded to 55 at priority 10), the
//! request opens the preview dialog under an "agent request" banner, and
//! only a confirmation sends it, through this session's desk and ledger,
//! audited as the TUI's approval. A decline, the dialog's lifetime and a
//! withdrawn request send nothing; requests wait one at a time, in order,
//! up to the cap. On paused time, against the in-process lab.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use bacnet_mcp::attach::{MAX_QUEUED, socket_path};
use bacnet_mcp::policy::{WriteMode, WritePolicy};
use bacnet_types::primitives::PropertyValue;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::mcp_attach::{Agent, LAB, attached, mcp_agents, plan};
use super::site::{ClientLimits, Server, Site};
use super::writes::{arm, audit, modal, slot, writes_sent};
use super::*;
use crate::tui::app::safety::{CONFIRM_GUARD, Modal, PREVIEW_LIFETIME};
use crate::tui::attach::Plan;
use crate::tui::profile::Safety;
use crate::tui::safety::ledger::Ledger;
use crate::tui::worker::session::SessionOptions;

/// The lab, a session armed for writes on a workspace in `dir`, and the
/// listener offering agents writes.
pub(super) async fn armed_site(dir: &Path) -> Site {
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut listening: Plan = plan(dir);
    listening.policy.writes = WritePolicy {
        mode: WriteMode::Elicit,
        workspace: Some(listening.workspace.clone()),
        ..WritePolicy::default()
    };
    let options = SessionOptions {
        attach: Some(listening),
        ..super::writes::options(dir, Safety::default(), false)
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    site.who_is("", 1).await;
    site.pump_until(|app| mcp_agents(app) == Some(0)).await;
    arm(&mut site).await;
    site
}

/// `bacnet_write` of AO:1 = `value` at `priority`, as request `id`.
pub(super) fn write(id: u64, value: u32, priority: u8) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": "bacnet_write", "arguments": { "device": LAB,
        "writes": [{ "object": "ao:1", "value": value, "priority": priority }] } } })
}

/// An agent attached at `dir`'s socket, its call `request` waiting on a
/// task of its own.
pub(super) async fn asking(dir: &Path, request: Value) -> JoinHandle<(Agent, Value)> {
    let mut agent = attached(&socket_path(dir).unwrap()).await;
    tokio::spawn(async move {
        let answer = agent.request(request).await;
        (agent, answer)
    })
}

/// The agent whose request the dialog shows, if it shows one.
pub(super) fn shown(app: &App) -> Option<String> {
    match modal(app) {
        Some(Modal::Confirm { req, .. }) => app.safety.agents.banner(*req).map(|(who, _)| who),
        _ => None,
    }
}

/// The audit records `event`, in order.
pub(super) fn events(dir: &Path) -> Vec<String> {
    audit(dir)
        .iter()
        .map(|r| r["event"].as_str().unwrap().to_string())
        .collect()
}

/// Acceptance: a T2 write goes only once the person here confirms it, after
/// the 300 ms guard; it is this agent's in the ledger, audited as the TUI's
/// approval by its OS user, and the agent's to revert.
#[tokio::test(start_paused = true)]
async fn a_t2_write_runs_only_after_a_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    assert_eq!(
        shown(&site.app).unwrap(),
        "AGENT REQUEST from agent 1 (its client says \"agent\")"
    );
    assert_snapshot!("agent_request_t2_120x40", screen(&mut site.app, 120, 40));
    assert_snapshot!("agent_request_t2_80x24", screen(&mut site.app, 80, 24));
    // Nothing goes while it waits, and a key at once is too soon.
    site.run_for(Duration::from_millis(100)).await;
    site.press(KeyCode::Char('y')).await;
    assert!(shown(&site.app).is_some(), "the guard held the dialog");
    assert_eq!(writes_sent(&site), 0);
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
    let (mut agent, answer) = site.pump_while(&mut call).await;
    let result = &answer["result"]["structuredContent"];
    assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    assert_eq!(result["approval"]["confirmed_via"], "tui");
    assert_eq!(result["approval"]["approved_by"], "tester");
    assert_eq!(slot(&site, 8).await, PropertyValue::Real(42.0));
    assert_eq!(writes_sent(&site), 1);
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(
        entry.origin.principal.as_deref(),
        Some("tester"),
        "the agent's to revert"
    );
    let sent = audit(dir.path())
        .into_iter()
        .find(|r| r["event"] == "write")
        .unwrap();
    assert_eq!(
        (
            &sent["front_end"],
            &sent["confirmed_via"],
            &sent["approved_by"]
        ),
        (&json!("mcp"), &json!("tui"), &json!("tester"))
    );
    site.pump_until(|app| app.safety.ledger.len() == 1).await;

    // The agent's revert goes through this session's desk too, once it is
    // confirmed here; its tools say who approves its changes.
    let mut revert = tokio::spawn(async move {
        let listed = agent
            .request(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }))
            .await;
        let reverted = agent
            .request(
                json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
                "name": "bacnet_revert", "arguments": { "all": true } } }),
            )
            .await;
        (listed, reverted)
    });
    site.pump_until(|app| shown(app).is_some()).await;
    assert!(shown(&site.app).unwrap().contains("to revert"));
    assert_eq!(
        slot(&site, 8).await,
        PropertyValue::Real(42.0),
        "not before the confirm"
    );
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
    let (listed, reverted) = site.pump_while(&mut revert).await;
    let write_tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "bacnet_write")
        .unwrap();
    assert!(
        write_tool["description"]
            .as_str()
            .unwrap()
            .contains("The person at the TUI")
    );
    assert_eq!(reverted["result"]["isError"], json!(false), "{reverted}");
    assert_eq!(slot(&site, 8).await, PropertyValue::Null);
    site.pump_until(|app| app.safety.ledger.is_empty()).await;
    site.quit().await;
    site.stop().await;
}

/// A decline and the dialog's 120 s each send nothing: the agent hears
/// `declined`, and both are audited.
#[tokio::test(start_paused = true)]
async fn a_decline_and_a_timeout_send_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    site.press(KeyCode::Esc).await;
    let (_first, declined) = site.pump_while(&mut call).await;
    assert_eq!(
        declined["result"]["structuredContent"]["reason"], "declined",
        "{declined}"
    );

    let mut call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    advance(&mut site.app, PREVIEW_LIFETIME - Duration::from_secs(1));
    assert!(shown(&site.app).is_some(), "still waiting");
    advance(&mut site.app, Duration::from_secs(1));
    assert!(shown(&site.app).is_none(), "expired");
    let (_second, expired) = site.pump_while(&mut call).await;
    let body = &expired["result"]["structuredContent"];
    assert_eq!(body["reason"], "declined", "{expired}");
    assert!(
        body["message"].as_str().unwrap().contains("120 s"),
        "{body}"
    );
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(
        events(dir.path()),
        ["arm", "mcp-connect", "declined", "mcp-connect", "declined"]
    );
    site.quit().await;
    site.stop().await;
}

/// Three agents' requests wait one at a time and go strictly in order,
/// each dialog guarded afresh; the banner and the bar show the queue.
#[tokio::test(start_paused = true)]
async fn queued_requests_go_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut calls = Vec::new();
    for (held, priority) in [(1, 8), (2, 7), (3, 4)] {
        calls.push(asking(dir.path(), write(2, 40 + u32::from(priority), priority)).await);
        site.pump_until(|app| app.safety.agents.held() == held)
            .await;
    }
    assert!(shown(&site.app).unwrap().contains("agent 1"));
    assert_snapshot!(
        "agent_request_queued_120x40",
        screen(&mut site.app, 120, 40)
    );
    for (n, call) in calls.iter_mut().enumerate().take(2) {
        assert!(
            shown(&site.app)
                .unwrap()
                .contains(&format!("agent {}", n + 1))
        );
        site.press(KeyCode::Char('y')).await;
        assert!(shown(&site.app).is_some(), "too soon after it opened");
        advance(&mut site.app, CONFIRM_GUARD);
        site.press(KeyCode::Char('y')).await;
        let (_agent, answer) = site.pump_while(call).await;
        assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
    }
    assert!(shown(&site.app).unwrap().contains("agent 3"));
    assert_eq!(
        slot(&site, 4).await,
        PropertyValue::Null,
        "the third waits its turn"
    );
    site.press(KeyCode::Esc).await;
    let (_agent, answer) = site.pump_while(&mut calls[2]).await;
    assert_eq!(
        answer["result"]["structuredContent"]["reason"], "declined",
        "{answer}"
    );
    let order: Vec<Option<u8>> = Ledger::open(dir.path())
        .unwrap()
        .entries()
        .iter()
        .map(|e| e.priority)
        .collect();
    assert_eq!(order, [Some(8), Some(7)]);
    site.quit().await;
    site.stop().await;
}

/// An agent that disconnects while its request waits has it withdrawn:
/// off the queue unshown, or its dialog closed; nothing is sent.
#[tokio::test(start_paused = true)]
async fn a_disconnect_withdraws_its_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let shown_call = asking(dir.path(), write(2, 41, 8)).await;
    site.pump_until(|app| app.safety.agents.held() == 1).await;
    let waiting_call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| app.safety.agents.held() == 2).await;
    waiting_call.abort();
    site.run_for(Duration::from_millis(100)).await;
    advance(&mut site.app, Duration::from_secs(1));
    assert_eq!(
        site.app.safety.agents.held(),
        1,
        "the waiting one left the queue"
    );
    assert!(shown(&site.app).unwrap().contains("agent 1"));
    shown_call.abort();
    site.run_for(Duration::from_millis(100)).await;
    advance(&mut site.app, Duration::from_secs(1));
    assert_eq!(site.app.safety.agents.held(), 0);
    assert!(modal(&site.app).is_none(), "its dialog closed");
    assert_eq!(writes_sent(&site), 0);
    site.quit().await;
    site.stop().await;
}

/// Past the cap, a request is refused at once, saying why; so is one while
/// no write session is armed here.
#[tokio::test(start_paused = true)]
async fn the_queue_cap_and_an_unarmed_session_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut calls = Vec::new();
    for held in 1..=MAX_QUEUED {
        calls.push(asking(dir.path(), write(2, 40, 8)).await);
        site.pump_until(|app| app.safety.agents.held() == held)
            .await;
    }
    let text = screen(&mut site.app, 120, 40);
    assert!(text.contains(&format!("{MAX_QUEUED} to confirm")), "{text}");
    let mut over = asking(dir.path(), write(2, 40, 8)).await;
    let (_agent, refused) = site.pump_while(&mut over).await;
    let body = &refused["result"]["structuredContent"];
    assert_eq!(body["reason"], "approval-queue-full", "{refused}");
    assert_eq!(site.app.safety.agents.held(), MAX_QUEUED);
    for call in calls {
        call.abort();
    }
    site.run_for(Duration::from_millis(100)).await;
    advance(&mut site.app, Duration::from_secs(1));
    assert_eq!(site.app.safety.agents.held(), 0, "all withdrawn");
    // Disarmed: refused unshown.
    site.press(KeyCode::Char('W')).await;
    site.pump_until(|app| app.safety.armed.is_none()).await;
    let mut unarmed = asking(dir.path(), write(2, 40, 8)).await;
    let (_agent, refused) = site.pump_while(&mut unarmed).await;
    assert_eq!(
        refused["result"]["structuredContent"]["reason"], "not-armed",
        "{refused}"
    );
    assert_eq!(writes_sent(&site), 0);
    site.quit().await;
    site.stop().await;
}

/// Quitting the TUI with a request on screen resolves the agent's call,
/// declined, audited, with nothing sent.
#[tokio::test(start_paused = true)]
async fn quitting_declines_what_waits() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let call = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    site.quit().await;
    let (_agent, answer) = super::site::bounded(call).await.unwrap();
    let body = &answer["result"]["structuredContent"];
    assert_eq!(body["reason"], "declined", "{answer}");
    assert_eq!(writes_sent(&site), 0);
    assert!(events(dir.path()).contains(&"declined".to_string()));
    site.stop().await;
}
