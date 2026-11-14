//! The TUI's MCP listener (#2185): an agent attached to the open
//! profile's session reads the lab device through the session's own
//! client and limits, so its requests show on the watch list's rate meter;
//! the status bar counts the agents; quitting ends the agent's session and
//! removes the socket. Its plan comes from the profile's `[mcp]` table.
//! On Unix through the socket, on Windows through the named pipe.
//! A bug an agent's request reaches ends only that agent's session.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_mcp::Policy;
use bacnet_mcp::attach::socket_path;
use bacnet_mcp::policy::Workspace;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader, Lines,
};

use super::site::{ClientLimits, Server, Site, bounded};
use super::*;
use crate::tui::attach::Plan;
use crate::tui::worker::session::SessionOptions;

pub(super) const LAB: u32 = 100;

/// An agent on the socket: newline-delimited JSON-RPC.
pub(super) struct Agent {
    pub(super) write: Box<dyn AsyncWrite + Send + Unpin>,
    lines: Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
}

impl Agent {
    /// Connect as the bridge does, checking the listener's user.
    pub(super) async fn connect(path: &Path) -> Self {
        let stream = bacnet_mcp::attach::connect(path).await.unwrap();
        let (read, write) = tokio::io::split(stream);
        let read: Box<dyn AsyncRead + Send + Unpin> = Box::new(read);
        Self {
            write: Box::new(write),
            lines: BufReader::new(read).lines(),
        }
    }

    /// Send `request` and return the answer with its id.
    pub(super) async fn request(&mut self, request: Value) -> Value {
        let line = format!("{request}\n");
        self.write.write_all(line.as_bytes()).await.unwrap();
        if request.get("id").is_none() {
            return Value::Null;
        }
        loop {
            let line = bounded(self.lines.next_line()).await.unwrap().unwrap();
            let message: Value = serde_json::from_str(line.trim_end_matches('\r')).unwrap();
            if message["id"] == request["id"] {
                return message;
            }
        }
    }
}

/// Set to make the next request an agent's session thread sends panic.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Panic, once armed, on an agent's session thread: the hub port calls it
/// as a request leaves, standing in for a bug an agent's request reaches.
pub(super) fn panic_if_armed() {
    let on_agents_thread = std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with(bacnet_mcp::attach::SESSION_THREADS));
    if on_agents_thread && ARMED.swap(false, Ordering::SeqCst) {
        panic!("an injected bug, reached by an agent's request");
    }
}

/// An agent on the socket at `path`, its session open.
pub(super) async fn attached(path: &Path) -> Agent {
    let mut agent = Agent::connect(path).await;
    let hello = agent
        .request(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "agent", "version": "1" } } }),
        )
        .await;
    assert!(hello.get("result").is_some(), "{hello}");
    agent
        .request(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    agent
}

/// A read of the lab's ai:1, as request `id`.
fn read(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": "bacnet_read", "arguments": { "device": LAB,
        "reads": [{ "object": "ai:1", "properties": ["present-value"] }] } } })
}

/// The agents the bar counts, while the listener runs.
pub(super) fn mcp_agents(app: &App) -> Option<usize> {
    app.mcp
        .as_ref()
        .and_then(crate::tui::message::McpListener::agents)
}

pub(super) fn plan(dir: &Path) -> Plan {
    Plan {
        policy: Policy::default(),
        workspace: Workspace {
            dir: dir.to_path_buf(),
            profile: Some("lab".into()),
            transport: "BIP",
            local: "10.0.0.1:47808".into(),
        },
        principal: "tester".into(),
        own_runtime: false,
    }
}

/// Acceptance: the agent's reads go through the session's client, on its
/// rate meter; the bar shows `MCP n`; quitting the TUI ends the agent's
/// connection, removes the socket, and leaves its open and close audited.
#[tokio::test(start_paused = true)]
async fn an_agent_reads_through_the_open_session() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = SessionOptions {
        attach: Some(plan(dir.path())),
        ..SessionOptions::default()
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    site.who_is("", 1).await;
    site.pump_until(|app| mcp_agents(app) == Some(0)).await;
    assert!(screen(&mut site.app, 120, 40).contains("MCP 0"));

    let path = socket_path(dir.path()).unwrap();
    let mut agent = Agent::connect(&path).await;
    let hello = agent
        .request(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "agent", "version": "1" } } }),
        )
        .await;
    assert!(hello.get("result").is_some(), "{hello}");
    agent
        .request(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    site.pump_until(|app| mcp_agents(app) == Some(1)).await;
    for width in [120, 80] {
        let text = screen(&mut site.app, width, 24);
        assert!(text.contains("MCP 1"), "{width} columns:\n{text}");
    }

    site.run_for(Duration::from_secs(11)).await;
    let before = site.app.watch.stats.recent_requests;
    for id in 2..5 {
        let read = agent
            .request(
                json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
                "name": "bacnet_read", "arguments": { "device": LAB,
                "reads": [{ "object": "ai:1", "properties": ["present-value"] }] } } }),
            )
            .await;
        assert_eq!(read["result"]["isError"], json!(false), "{read}");
    }
    site.pump_until(|app| app.watch.stats.recent_requests >= before + 3)
        .await;

    site.quit().await;
    #[cfg(unix)]
    assert!(!path.exists(), "removed with the session");
    let end = bounded(agent.lines.next_line()).await.unwrap();
    assert_eq!(end, None, "the agent's connection ends");
    let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    let events: Vec<String> = audit
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["event"].to_string())
        .collect();
    assert_eq!(events, ["\"mcp-connect\"", "\"mcp-disconnect\""]);
    site.stop().await;
}

/// The listener a profile asks for: only once MCP is authorized and
/// `tui_listener` isn't false, never in a passive session, offering writes
/// as `mcp_writes` does, and refused when the command line changes the
/// transport the authorization covered.
#[test]
fn the_plan_follows_the_profile() {
    use crate::tui::attach::plan;
    use crate::tui::profile::mcp::McpChoice;
    use crate::tui::profile::service::Prepared;
    use crate::tui::profile::{McpSection, Profile};

    let mut profile = Profile::default();
    let mut section = McpSection {
        networks: Some(vec![0]),
        mcp_writes: McpChoice::Elicit,
        ..McpSection::default()
    };
    section.authorize(1, profile.link());
    profile.mcp = Some(Box::new(section));
    let args = profile.transport_args().unwrap();
    let dir = Path::new("/tmp/site");
    let prepared = |stored: &Profile, resolved: &Profile| Prepared {
        stored: stored.clone(),
        resolved: resolved.clone(),
    };
    let same = prepared(&profile, &profile);
    let planned = plan("site", &same, Some(dir), &args, false)
        .unwrap()
        .expect("a listener");
    assert_eq!(planned.policy.networks, Some([0].into()));
    // `mcp_writes` offers writes, approved here; nothing else of the
    // write policy reaches the listener.
    assert!(planned.policy.writes.offered());
    assert_eq!(
        planned.policy.writes.arm,
        bacnet_mcp::policy::ArmMode::Never
    );
    assert!(planned.policy.writes.plan.is_none());
    assert_eq!(
        planned
            .policy
            .writes
            .workspace
            .as_ref()
            .map(|w| w.dir.as_path()),
        Some(dir)
    );
    assert_eq!(planned.workspace.dir, dir);
    assert_eq!(planned.workspace.profile.as_deref(), Some("site"));
    assert!(
        plan("site", &same, Some(dir), &args, true)
            .unwrap()
            .is_none()
    );
    assert!(plan("site", &same, None, &args, false).is_err());

    let mut off = profile.clone();
    off.mcp.as_mut().unwrap().tui_listener = Some(false);
    let off = prepared(&off, &off);
    assert!(
        plan("site", &off, Some(dir), &args, false)
            .unwrap()
            .is_none()
    );
    let bare = Profile::default();
    let bare = prepared(&bare, &bare);
    assert!(
        plan("site", &bare, Some(dir), &args, false)
            .unwrap()
            .is_none()
    );

    let mut overridden = profile.clone();
    overridden.bip.as_mut().unwrap().port = 47809;
    let error = plan(
        "site",
        &prepared(&profile, &overridden),
        Some(dir),
        &args,
        false,
    )
    .unwrap_err();
    assert!(error.contains("transport"), "{error}");
}

/// The bar shows the listener's latest count without another event, and
/// `MCP off` when a listener the profile asked for couldn't start.
#[test]
fn the_bar_follows_the_listeners_count() {
    use crate::tui::message::McpListener;
    let mut app = connected_app();
    worker(&mut app, WorkerEvent::Mcp(McpListener::Off));
    assert!(screen(&mut app, 120, 24).contains("MCP off"));
    let (count, seen) = tokio::sync::watch::channel(2);
    worker(&mut app, WorkerEvent::Mcp(McpListener::Agents(seen)));
    assert!(screen(&mut app, 120, 24).contains("MCP 2"));
    count.send_replace(3);
    let text = screen(&mut app, 120, 24);
    assert!(text.contains("MCP 3") && !text.contains("MCP 2"), "{text}");
}

/// With the listener running, the watch list's hub keeps the client's
/// first range of subscriber process identifiers: its first SubscribeCOV
/// carries the hub's base, 0x4D43_0000, as it does without a listener.
#[tokio::test(start_paused = true)]
async fn the_watch_list_keeps_the_first_subscriber_ids() {
    use bacnet_services::cov::SubscribeCOVRequest;
    use bacnet_types::enums::ConfirmedServiceChoice;
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = SessionOptions {
        attach: Some(plan(dir.path())),
        ..SessionOptions::default()
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    site.who_is("", 1).await;
    site.pump_until(|app| mcp_agents(app) == Some(0)).await;
    super::watch::command(&mut site, "watch 100 ai:1").await;
    super::watch::settle(&mut site).await;
    let mac = [10, 0, 0, 100, 0xBA, 0xC0];
    let bodies = site
        .hub
        .bodies_to(&mac, ConfirmedServiceChoice::SUBSCRIBE_COV);
    let first = SubscribeCOVRequest::decode(&bodies[0]).unwrap();
    assert_eq!(first.subscriber_process_identifier, 0x4D43_0000);
    site.quit().await;
    site.stop().await;
}

/// Acceptance (#2203): a bug an attached agent's request reaches panics
/// on the listener's own threads, which the TUI's panic hook leaves alone:
/// the terminal isn't restored and nothing is printed over the screen.
/// That agent's session ends, audited with the panic; the other agent
/// reads on, and the TUI keeps drawing. On a real clock, as the
/// listener's own runtime doesn't share a paused one.
#[tokio::test]
async fn an_agents_panic_ends_its_session_and_the_tui_draws_on() {
    use crate::tui::terminal::chain_panic_hook;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let original = Arc::new(std::panic::take_hook());
    std::panic::set_hook(chain_panic_hook(
        {
            let (calls, original) = (Arc::clone(&calls), Arc::clone(&original));
            Box::new(move |info| {
                calls.lock().unwrap().push("previous hook");
                original(info);
            })
        },
        {
            let calls = Arc::clone(&calls);
            move || calls.lock().unwrap().push("restore")
        },
    ));
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = SessionOptions {
        attach: Some(Plan {
            own_runtime: true,
            ..plan(dir.path())
        }),
        ..SessionOptions::default()
    };
    let servers = vec![Server::lab(100, LAB)];
    let mut site = Site::start_with_options(servers, ClientLimits::default(), options).await;
    // Most of this test's time: the Who-Is window, on the real clock.
    site.who_is_briefly("", 1).await;
    site.pump_until(|app| mcp_agents(app) == Some(0)).await;
    let path = socket_path(dir.path()).unwrap();
    let mut first = attached(&path).await;
    let mut second = attached(&path).await;
    site.pump_until(|app| mcp_agents(app) == Some(2)).await;

    ARMED.store(true, Ordering::SeqCst);
    let line = format!("{}\n", read(2));
    first.write.write_all(line.as_bytes()).await.unwrap();
    // The SDK drains the answers of requests in flight as the session
    // ends, so the internal error gets out before the stream closes.
    let mut answered = false;
    while let Some(line) = bounded(first.lines.next_line()).await.unwrap() {
        let message: Value = serde_json::from_str(&line).unwrap();
        answered |= message["id"] == json!(2) && message["error"]["code"] == json!(-32603);
    }
    assert!(answered, "the read was answered with an internal error");
    assert!(
        !ARMED.load(Ordering::SeqCst),
        "the read ran on an agent's thread"
    );
    let other = second.request(read(2)).await;
    assert_eq!(other["result"]["isError"], json!(false), "{other}");
    site.pump_until(|app| mcp_agents(app) == Some(1)).await;
    let text = screen(&mut site.app, 120, 40);
    assert!(text.contains("MCP 1"), "the TUI draws on:\n{text}");
    let hooked = calls.lock().unwrap().clone();
    assert!(hooked.is_empty(), "the terminal was left alone: {hooked:?}");

    site.quit().await;
    drop(std::panic::take_hook());
    std::panic::set_hook(Box::new(move |info| original(info)));
    let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    let records: Vec<Value> = audit
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let first_close = records
        .iter()
        .find(|r| r["event"] == json!("mcp-disconnect") && r["connection"] == json!(1))
        .unwrap_or_else(|| panic!("{records:?}"));
    let why = first_close["error"].as_str().unwrap_or_default();
    assert!(why.contains("panicked"), "{first_close}");
    site.stop().await;
}
