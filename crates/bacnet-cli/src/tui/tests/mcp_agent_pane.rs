//! The agent pane (#2185): screen 8 lists the attached agents' calls with
//! their outcome and cost, newest first, and the requests waiting here
//! with their ages; disarming refuses what waits at once; and
//! `bacnet://session` tells an attached agent whether the TUI is armed.
//! Frames from a synthetic board, and end to end over the socket against
//! the in-process lab, on paused time.

use std::time::{Duration, UNIX_EPOCH};

use bacnet_mcp::attach::{Board, CALLS, CallOutcome, CallRow, socket_path};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};

use super::mcp_approve::{armed_site, asking, shown, write};
use super::mcp_attach::{LAB, attached};
use super::site::Site;
use super::writes::writes_sent;
use super::*;
use crate::tui::app::safety::CONFIRM_GUARD;
use crate::tui::app::watch::Screen;
use crate::tui::message::McpListener;

/// A call answered `secs` after 12:00:00 UTC.
fn call(secs: u64, agent: u64, tool: &str, target: &str, outcome: CallOutcome) -> CallRow {
    CallRow {
        at: UNIX_EPOCH + Duration::from_secs(43_200 + secs),
        agent,
        client: Some("claude-code".into()),
        tool: tool.into(),
        target: target.into(),
        outcome,
        requests: u32::try_from(secs % 4).unwrap(),
        elapsed: Duration::from_millis(secs * 100),
    }
}

/// A connected app with a listener of `agents`, its board, on screen 8.
fn pane(agents: usize) -> (App, Board, tokio::sync::watch::Sender<usize>) {
    let mut app = connected_app();
    let (count, seen) = tokio::sync::watch::channel(agents);
    let board = Board::default();
    worker(
        &mut app,
        WorkerEvent::Mcp(McpListener::Board(board.clone())),
    );
    worker(&mut app, WorkerEvent::Mcp(McpListener::Agents(seen)));
    press(&mut app, KeyCode::Char('8'));
    assert_eq!(app.screen, Screen::Agents);
    (app, board, count)
}

/// The pane with nothing yet, then with calls of every outcome, newest
/// first, at 120x40 and 80x24.
#[test]
fn the_pane_lists_calls_newest_first() {
    let (mut app, board, _count) = pane(2);
    assert_snapshot!("agent_pane_empty_120x40", screen(&mut app, 120, 40));
    assert_snapshot!("agent_pane_empty_80x24", screen(&mut app, 80, 24));
    let rows = [
        call(1, 1, "bacnet_discover", "net 0", CallOutcome::Ok),
        call(2, 1, "bacnet_read", "dev 100 ai:1 +2", CallOutcome::Ok),
        call(
            3,
            2,
            "bacnet_write",
            "dev 100 ao:1",
            CallOutcome::Refused("not-armed".into()),
        ),
        call(
            5,
            2,
            "bacnet_write",
            "dev 100 ao:1",
            CallOutcome::Declined("declined".into()),
        ),
        call(6, 1, "bacnet_revert", "entries 4", CallOutcome::Stale),
        call(
            7,
            2,
            "bacnet_read",
            "dev 200",
            CallOutcome::Error("timeout".into()),
        ),
    ];
    for row in rows {
        board.record(row);
    }
    advance(&mut app, Duration::from_millis(100));
    assert_eq!(app.safety.agents.pane.calls.rows.len(), 6);
    let wide = screen(&mut app, 120, 40);
    assert!(
        wide.find("bacnet_read").unwrap() < wide.find("bacnet_discover").unwrap(),
        "newest first"
    );
    assert_snapshot!("agent_pane_calls_120x40", wide);
    assert_snapshot!("agent_pane_calls_80x24", screen(&mut app, 80, 24));
}

/// An agent's long claimed name and a seven-digit device never push the
/// countdown or the host warning off an 80-column screen: the name comes
/// last, cut to the room left.
#[test]
fn a_long_name_leaves_the_countdown_and_warning_in_view() {
    use crate::tui::app::agents::Held;
    let (mut app, _board, _count) = pane(1);
    app.safety.agents.staged.push(Held {
        agent: 7,
        client: Some("n".repeat(64)),
        what: "write",
        device: 4_194_302,
        age: Duration::from_secs(75),
        left: Some(Duration::from_secs(45)),
    });
    let narrow = screen(&mut app, 80, 24);
    let line = narrow.lines().find(|l| l.contains("4194302")).unwrap();
    assert!(
        line.contains("45 s left") && line.contains("(host may be gone)"),
        "{line}"
    );
    assert!(line.contains("agent 7"), "{line}");
    assert_snapshot!("agent_pane_long_name_80x24", narrow);
    let wide = screen(&mut app, 120, 40);
    let line = wide.lines().find(|l| l.contains("4194302")).unwrap();
    assert!(
        line.contains("declined in 45 s (its host may have stopped waiting)"),
        "{line}"
    );
}

/// The heading gives the agents' share of the request meter.
#[test]
fn the_heading_shows_the_agents_share() {
    let (mut app, board, _count) = pane(1);
    board.record(call(3, 1, "bacnet_read", "dev 100 ai:1", CallOutcome::Ok));
    app.watch.stats.recent_requests = 30;
    app.watch.stats.agent_requests = 12;
    advance(&mut app, Duration::from_millis(100));
    let wide = screen(&mut app, 120, 40);
    assert!(wide.contains("requests 1.2/s of 3.0/s"), "{wide}");
    assert_snapshot!("agent_pane_share_120x40", wide);
    assert!(screen(&mut app, 80, 24).contains("1.2/s of 3.0/s"));
}

/// Scrolled down, the view keeps the same calls as new ones arrive on top.
#[test]
fn a_scrolled_view_stays_on_its_calls() {
    let (mut app, board, _count) = pane(1);
    for n in 0..60 {
        board.record(call(
            n,
            1,
            "bacnet_read",
            &format!("dev {n}"),
            CallOutcome::Ok,
        ));
    }
    advance(&mut app, Duration::from_millis(100));
    for _ in 0..5 {
        press(&mut app, KeyCode::Down);
    }
    let top = |app: &mut App| {
        let text = screen(app, 120, 40);
        text.lines()
            .find(|l| l.contains("dev "))
            .unwrap()
            .to_string()
    };
    let before = top(&mut app);
    assert!(before.contains("dev 54"), "{before}");
    for n in 60..63 {
        board.record(call(
            n,
            1,
            "bacnet_read",
            &format!("dev {n}"),
            CallOutcome::Ok,
        ));
    }
    advance(&mut app, Duration::from_millis(100));
    assert_eq!(top(&mut app), before);
}

/// Past the ring's size the oldest calls go, and the heading counts them.
#[test]
fn the_heading_counts_the_calls_dropped() {
    let (mut app, board, _count) = pane(1);
    for n in 0..(CALLS as u64 + 7) {
        board.record(call(n, 1, "bacnet_read", "dev 100 ai:1", CallOutcome::Ok));
    }
    advance(&mut app, Duration::from_millis(100));
    assert_eq!(app.safety.agents.pane.calls.dropped, 7);
    let wide = screen(&mut app, 120, 40);
    assert!(wide.contains("7 older dropped"), "{wide}");
    assert_snapshot!("agent_pane_dropped_120x40", wide);
    assert_snapshot!("agent_pane_dropped_80x24", screen(&mut app, 80, 24));
}

/// Without a listener, 8 stays where it is and says why; the strip lists
/// Agents only with one.
#[test]
fn eight_needs_a_listener() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('8'));
    assert_eq!(app.screen, Screen::Main);
    assert!(app.flash.as_ref().unwrap().text.contains("No MCP listener"));
    assert!(!screen(&mut app, 120, 24).contains(" 8 "));
    let (mut app, _board, _count) = pane(0);
    assert!(screen(&mut app, 120, 24).contains("[8 Agents]"));
}

/// The arming here reaches the board as it changes.
#[test]
fn the_board_hears_the_arming() {
    let (mut app, board, _count) = pane(1);
    assert!(!board.armed());
    app.safety.armed = Some(crate::tui::app::safety::Armed {
        until: app.now + Duration::from_secs(60),
        reason: String::new(),
    });
    advance(&mut app, Duration::from_millis(100));
    assert!(board.armed());
    act(&mut app, Action::ToggleArm);
    assert!(!board.armed());
}

/// A request waiting here shows its age, and past 60 s that its host may
/// have stopped waiting; the one on screen shows when it is declined.
#[tokio::test(start_paused = true)]
async fn a_waiting_request_shows_its_age() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    // Held behind no dialog: one opens only once the filter closes.
    site.app.filter_editing = true;
    let mut waiting = asking(dir.path(), write(2, 43, 8)).await;
    site.pump_until(|app| app.safety.agents.held() == 1).await;
    site.app.filter_editing = false;
    site.app.screen = Screen::Agents;
    site.app.now += Duration::from_secs(65);
    let wide = screen(&mut site.app, 120, 40);
    assert!(wide.contains("waiting 65 s (its host may"), "{wide}");
    assert_snapshot!("agent_pane_waiting_120x40", wide);
    assert_snapshot!("agent_pane_waiting_80x24", screen(&mut site.app, 80, 24));
    // On screen, it counts down from the dialog's opening.
    advance(&mut site.app, Duration::from_secs(10));
    assert!(shown(&site.app).is_some());
    advance(&mut site.app, Duration::from_secs(20));
    let text = screen(&mut site.app, 120, 40);
    assert!(
        text.contains("waiting 95 s, on screen, declined in 100 s"),
        "{text}"
    );
    site.press(KeyCode::Esc).await;
    let (_agent, _declined) = site.pump_while(&mut waiting).await;
    site.quit().await;
    site.stop().await;
}

/// Tick the app and apply worker events, a tenth of a second at a time,
/// until `done` holds: the pane copies the board's calls on a tick.
async fn ticking_until(site: &mut Site, done: impl Fn(&App) -> bool) {
    for _ in 0..50 {
        advance(&mut site.app, Duration::from_millis(100));
        if done(&site.app) {
            return;
        }
        site.run_for(Duration::from_millis(100)).await;
    }
    panic!(
        "never held: calls {:?}, meter {:?}",
        site.app.safety.agents.pane.calls, site.app.watch.stats
    );
}

/// The requests of the result `answer`'s cost.
fn requests(answer: &Value) -> u32 {
    let n = answer["result"]["structuredContent"]["cost"]["requests"].as_u64();
    u32::try_from(n.unwrap()).unwrap()
}

/// Acceptance, end to end: a read, a confirmed write, a declined one and
/// one refused unarmed each show on the pane with their outcome and the
/// cost their answers carried, newest first.
#[tokio::test(start_paused = true)]
async fn calls_show_with_their_outcome_and_cost() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut agent = attached(&socket_path(dir.path()).unwrap()).await;
    let read = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "bacnet_read", "arguments": { "device": LAB,
        "reads": [{ "object": "ai:1", "properties": ["present-value"] }] } } });
    let mut reading = tokio::spawn(async move {
        let answer = agent.request(read).await;
        (agent, answer)
    });
    let (_agent, read) = site.pump_while(&mut reading).await;

    let mut confirmed = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
    let (_agent, wrote) = site.pump_while(&mut confirmed).await;
    assert_eq!(writes_sent(&site), 1);
    assert!(requests(&wrote) >= 2, "the re-read and the write: {wrote}");

    let mut declined = asking(dir.path(), write(3, 43, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    site.press(KeyCode::Esc).await;
    let (_agent, _declined) = site.pump_while(&mut declined).await;

    site.press(KeyCode::Char('W')).await;
    let mut refused = asking(dir.path(), write(4, 44, 8)).await;
    let (_agent, _refused) = site.pump_while(&mut refused).await;

    ticking_until(&mut site, |app| {
        app.safety.agents.pane.calls.rows.len() == 4
    })
    .await;
    let rows = &site.app.safety.agents.pane.calls.rows;
    let seen: Vec<(&str, &CallOutcome)> =
        rows.iter().map(|r| (r.tool.as_str(), &r.outcome)).collect();
    assert_eq!(
        seen,
        [
            ("bacnet_write", &CallOutcome::Refused("not-armed".into())),
            ("bacnet_write", &CallOutcome::Declined("declined".into())),
            ("bacnet_write", &CallOutcome::Ok),
            ("bacnet_read", &CallOutcome::Ok),
        ]
    );
    assert_eq!(rows[2].requests, requests(&wrote), "the write's cost");
    assert_eq!(rows[3].requests, requests(&read), "the read's cost");
    assert_eq!(rows[0].client.as_deref(), Some("agent"));
    assert!(
        rows[2].target.starts_with(&format!("dev {LAB} ao:1")),
        "{:?}",
        rows[2]
    );
    site.quit().await;
    site.stop().await;
}

/// The meter and the pane agree: the agent's requests, the re-read before
/// its write included, are the agents' share on the watch list's meter,
/// and the pane's rows sum to it.
#[tokio::test(start_paused = true)]
async fn the_meter_counts_the_agents_requests_as_the_pane_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut confirmed = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    advance(&mut site.app, CONFIRM_GUARD);
    site.press(KeyCode::Char('y')).await;
    let (_agent, wrote) = site.pump_while(&mut confirmed).await;
    let charged = usize::try_from(requests(&wrote)).unwrap();
    assert!(charged >= 2, "{wrote}");
    ticking_until(&mut site, |app| {
        app.safety.agents.pane.calls.rows.len() == 1 && app.watch.stats.agent_requests == charged
    })
    .await;
    let stats = &site.app.watch.stats;
    assert!(stats.recent_requests >= stats.agent_requests);
    site.quit().await;
    site.stop().await;
}

/// Disarming refuses every request held at once, `not-armed`, the one on
/// screen and the one behind it, and nothing is sent.
#[tokio::test(start_paused = true)]
async fn disarming_refuses_the_queue_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let mut first = asking(dir.path(), write(2, 42, 8)).await;
    site.pump_until(|app| shown(app).is_some()).await;
    let mut second = asking(dir.path(), write(3, 43, 7)).await;
    site.pump_until(|app| app.safety.agents.held() == 2).await;
    // W's action, as a write session times out with a dialog open.
    let commands = act(&mut site.app, Action::ToggleArm);
    assert_eq!(site.app.safety.agents.held(), 0, "refused at once");
    assert!(shown(&site.app).is_none(), "the dialog closed");
    assert!(
        site.app.flash.as_ref().unwrap().text.contains("Refused 2"),
        "{:?}",
        site.app.flash
    );
    site.send(commands).await;
    for call in [&mut first, &mut second] {
        let (_agent, answer) = site.pump_while(call).await;
        assert_eq!(
            answer["result"]["structuredContent"]["reason"], "not-armed",
            "{answer}"
        );
    }
    assert_eq!(writes_sent(&site), 0);
    site.quit().await;
    site.stop().await;
}

/// `bacnet://session` on an attached session tells the agent whether the
/// TUI is armed, and follows it.
#[tokio::test(start_paused = true)]
async fn the_session_resource_reports_the_tuis_arming() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = armed_site(dir.path()).await;
    let session = |id: u64| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "resources/read",
            "params": { "uri": "bacnet://session" } })
    };
    let armed = |answer: &Value| {
        let text = answer["result"]["contents"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["tui"]["armed"].clone()
    };
    let mut agent = attached(&socket_path(dir.path()).unwrap()).await;
    let mut asked = tokio::spawn(async move {
        let answer = agent.request(session(2)).await;
        (agent, answer)
    });
    let (mut agent, answer) = site.pump_while(&mut asked).await;
    assert_eq!(armed(&answer), json!(true), "{answer}");
    site.press(KeyCode::Char('W')).await;
    let mut asked = tokio::spawn(async move {
        let answer = agent.request(session(3)).await;
        (agent, answer)
    });
    let (_agent, answer) = site.pump_while(&mut asked).await;
    assert_eq!(armed(&answer), json!(false), "{answer}");
    site.quit().await;
    site.stop().await;
}
