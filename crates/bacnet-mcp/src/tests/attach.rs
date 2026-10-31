//! TUI-attached mode (#2185): agents on the workspace's socket, through
//! the bridge, on the lab's shared engine; the socket's security checks;
//! and the listener's shutdown.

use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::engine::Engine;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::task::JoinHandle;

use super::{Agent, LAB, Lab};
use crate::attach::{self, AttachError, Listener, PeerId, socket_path};
use crate::policy::{ArmMode, Workspace, WriteMode, WritePolicy};
use crate::{McpServer, Policy};

const AI1: &str = "bacnet://device/1201/object/analog-input:1";

/// A workspace directory nobody else can enter.
pub(super) fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

pub(super) fn workspace(dir: &Path) -> Workspace {
    Workspace {
        dir: dir.to_path_buf(),
        profile: Some("lab".into()),
        transport: "BIP",
        local: "10.0.0.1:47808".into(),
    }
}

/// A policy that offers the write tools, as a profile with
/// `mcp_writes = "elicit"` does.
fn writes_offered(dir: &Path) -> Policy {
    Policy {
        writes: WritePolicy {
            mode: WriteMode::Elicit,
            arm: ArmMode::Elicit,
            plan: None,
            workspace: Some(workspace(dir)),
        },
        ..Policy::default()
    }
}

impl Lab {
    /// The lab client's engine, on the lab's limits: the TUI's shared one.
    pub(super) fn engine(&self) -> Arc<Engine<super::Recording>> {
        Arc::new(Engine::new(
            Arc::clone(&self.client),
            Arc::clone(&self.limits),
        ))
    }

    /// A listener on `dir` over the lab's engine.
    pub(super) async fn attach(&self, dir: &Path, policy: Policy) -> Result<Listener, AttachError> {
        Listener::bind(self.engine(), policy, "tester", workspace(dir)).await
    }
}

/// The lab, its own pipe's session started (it fails if never opened).
pub(super) async fn lab() -> Lab {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    lab
}

/// An agent through the bridge to the socket at `path`, and the bridge.
pub(super) fn bridged(path: &Path) -> (Agent, JoinHandle<Result<(), AttachError>>) {
    let (agent, bridge) = tokio::io::duplex(1 << 20);
    let path = path.to_path_buf();
    let bridge = tokio::spawn(async move {
        let (input, output) = tokio::io::split(bridge);
        attach::bridge(&path, input, output).await
    });
    (Agent::over(agent), bridge)
}

/// The workspace's audit records.
pub(super) fn audit(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("audit.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub(super) fn events(dir: &Path) -> Vec<String> {
    audit(dir)
        .iter()
        .map(|record| record["event"].as_str().unwrap().to_string())
        .collect()
}

fn tool_names(reply: &Value) -> Vec<String> {
    reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// Acceptance: an agent attached through the bridge surveys the lab on
/// the shared engine, so its reads show on the engine's rate meter; the
/// tool list has no write tool though the profile offers writes; the
/// connection's open and close are audited with the peer's uid.
#[tokio::test(start_paused = true)]
async fn an_attached_agent_reads_on_the_shared_engine() {
    let lab = lab().await;
    let dir = private_dir();
    let offered = writes_offered(dir.path());
    let direct = McpServer::new(Some(lab.engine()), offered.clone(), "x").unwrap();
    assert!(
        direct.tool_names().contains(&"bacnet_write"),
        "writes offered"
    );
    let listener = lab.attach(dir.path(), offered).await.unwrap();
    let path = socket_path(dir.path());
    assert_eq!(listener.path(), path);
    let mode = std::fs::symlink_metadata(&path).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o600, "{mode:o}");

    let (mut agent, bridge) = bridged(&path);
    let hello = agent.initialize().await;
    let instructions = hello["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("cannot write"), "{instructions}");
    let listed = tool_names(&agent.request("tools/list", json!({})).await);
    assert_eq!(listed, lab.mcp.tool_names(), "the read-only list");
    assert!(
        !listed
            .iter()
            .any(|name| name.contains("write") || name.contains("revert"))
    );
    assert_eq!(*listener.agents().borrow(), 1);

    let before = lab.limits.recent();
    let card = agent
        .call("bacnet_describe_device", json!({ "device": "1201" }))
        .await;
    assert_eq!(card["isError"], json!(false), "{card}");
    let read = agent
        .call(
            "bacnet_read",
            json!({ "device": LAB, "reads": [{ "object": "ai:1", "properties": ["present-value"] }] }),
        )
        .await;
    assert_eq!(read["isError"], json!(false), "{read}");
    let value = &read["structuredContent"]["results"][0]["value"]["typed"];
    assert_eq!(value, &json!(72.5), "{read}");
    assert!(lab.limits.recent() >= before + 2, "the shared meter");
    // A write is no tool at all over the socket.
    let refused = agent
        .request(
            "tools/call",
            json!({ "name": "bacnet_write", "arguments": { "device": "1201", "writes": [] } }),
        )
        .await;
    assert!(refused.get("error").is_some(), "{refused}");
    let refused = agent
        .request(
            "tools/call",
            json!({ "name": "bacnet_revert", "arguments": { "all": true } }),
        )
        .await;
    assert!(refused.get("error").is_some(), "{refused}");

    agent.close().await;
    bridge.await.unwrap().unwrap();
    let mut agents = listener.agents();
    agents.wait_for(|n| *n == 0).await.unwrap();
    listener.shutdown().await;
    assert!(!path.exists(), "removed at shutdown");
    let records = audit(dir.path());
    assert_eq!(events(dir.path()), ["mcp-connect", "mcp-disconnect"]);
    for record in &records {
        assert_eq!(record["front_end"], json!("mcp"));
        assert_eq!(record["principal"], json!("tester"));
        assert_eq!(record["via"], json!("local-socket"));
        assert_eq!(record["peer_uid"], json!(attach::own_uid()));
        assert_eq!(record["profile"], json!("lab"));
    }
    drop(direct);
    lab.stop().await;
}

/// A peer the credentials name as another user (a stand-in for the
/// kernel's answer, since a test can't connect as another uid) is closed
/// before any MCP traffic and audited with its uid and pid.
#[tokio::test(start_paused = true)]
async fn another_users_connection_is_refused_and_audited() {
    fn stranger(_: &tokio::net::UnixStream) -> std::io::Result<PeerId> {
        Ok(PeerId {
            uid: attach::own_uid().wrapping_add(1),
            pid: Some(4242),
        })
    }
    let lab = lab().await;
    let dir = private_dir();
    let listener = Listener::bind_with(
        lab.engine(),
        Policy::default(),
        "tester".into(),
        workspace(dir.path()),
        stranger,
    )
    .await
    .unwrap();
    let mut stream = tokio::net::UnixStream::connect(listener.path())
        .await
        .unwrap();
    let hello = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": { "name": "x", "version": "1" } } });
    // The listener may close before the request is written.
    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, format!("{hello}\n").as_bytes()).await;
    // Closed unanswered, not left waiting: a session would answer and
    // keep the connection open.
    let mut answer = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer));
    assert!(
        closed.await.is_ok(),
        "still open: {}",
        String::from_utf8_lossy(&answer)
    );
    assert!(answer.is_empty(), "{}", String::from_utf8_lossy(&answer));
    assert_eq!(*listener.agents().borrow(), 0);
    listener.shutdown().await;
    let records = audit(dir.path());
    assert_eq!(events(dir.path()), ["mcp-refused"], "{records:?}");
    assert_eq!(
        records[0]["peer_uid"],
        json!(attach::own_uid().wrapping_add(1))
    );
    assert_eq!(records[0]["peer_pid"], json!(4242));
    assert_eq!(records[0]["outcome"]["why"], json!("another-user"));
    lab.stop().await;
}

/// Acceptance (maintainer, 2026-10-31): a looser workspace of this user's
/// is narrowed to 0700 when the listener starts; one that is a symbolic
/// link, or isn't a directory, gets no socket and the refusal says why.
#[tokio::test(start_paused = true)]
async fn a_loose_workspace_is_narrowed_and_a_link_refused() {
    let lab = lab().await;
    let dir = private_dir();
    for mode in [0o755, 0o750, 0o705, 0o710, 0o701, 0o720] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
        let now = std::fs::metadata(dir.path()).unwrap().mode() & 0o777;
        assert_eq!(now, 0o700, "{mode:o}");
        listener.shutdown().await;
    }
    let link = dir.path().join("link");
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let error = lab.attach(&link, Policy::default()).await.unwrap_err();
    assert!(error.to_string().contains("symbolic link"), "{error}");
    assert!(!socket_path(&real).exists());
    let real_mode = std::fs::metadata(&real).unwrap().mode() & 0o777;
    assert_eq!(real_mode, 0o755, "nothing done through the link");
    let file = dir.path().join("file");
    std::fs::write(&file, "").unwrap();
    let error = lab.attach(&file, Policy::default()).await.unwrap_err();
    assert!(error.to_string().contains("not a directory"), "{error}");
    lab.stop().await;
}

/// A second listener on the workspace finds the first's lock and is
/// refused without touching the socket or opening a session, and the
/// first keeps serving; a socket with no lock holder (a TUI that crashed)
/// is cleared; a symbolic link or any other file at the path is neither
/// followed nor removed.
#[tokio::test(start_paused = true)]
async fn a_second_listener_is_refused_and_a_stale_socket_cleared() {
    let lab = lab().await;
    let dir = private_dir();
    let path = socket_path(dir.path());
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists(), "a stale socket");
    assert!(!attach::listening(dir.path()), "no lock held");
    let first = lab.attach(dir.path(), Policy::default()).await.unwrap();
    assert!(attach::listening(dir.path()));

    let error = lab.attach(dir.path(), Policy::default()).await.unwrap_err();
    assert!(
        error.to_string().contains("another TUI is listening"),
        "{error}"
    );
    assert_eq!(*first.agents().borrow(), 0, "no session was opened");
    assert!(events(dir.path()).is_empty(), "{:?}", events(dir.path()));
    let (mut agent, bridge) = bridged(&path);
    let hello = agent.initialize().await;
    assert!(
        hello.get("result").is_some(),
        "the first still serves: {hello}"
    );
    agent.close().await;
    bridge.await.unwrap().unwrap();
    first.shutdown().await;
    assert!(!path.exists());
    assert!(!attach::listening(dir.path()), "released at shutdown");

    let target = dir.path().join("target");
    std::fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let error = lab.attach(dir.path(), Policy::default()).await.unwrap_err();
    assert!(error.to_string().contains("symbolic link"), "{error}");
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    std::fs::remove_file(&path).unwrap();

    std::fs::write(&path, "not a socket").unwrap();
    let error = lab.attach(dir.path(), Policy::default()).await.unwrap_err();
    assert!(error.to_string().contains("not a socket"), "{error}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a socket");
    lab.stop().await;
}

/// Acceptance: shutting the listener down completes an attached agent's
/// listen stream, cancels its COV subscription through the session's
/// watch hub, ends the agent's connection and removes the socket; with
/// nothing listening, the bridge says so.
#[tokio::test(start_paused = true)]
async fn shutdown_ends_sessions_and_removes_the_socket() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let path = socket_path(dir.path());
    let (mut agent, bridge) = bridged(&path);
    let devices = agent
        .modern("resources/read", json!({ "uri": "bacnet://devices" }))
        .await;
    assert!(devices.get("result").is_some(), "{devices}");
    let (stream, accepted) = agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.cov_table().await.len(), 1);

    listener.shutdown().await;
    let done = agent.answer(stream).await;
    assert_eq!(done["result"]["resultType"], json!("complete"), "{done}");
    assert!(lab.cov_table().await.is_empty(), "cancelled at shutdown");
    bridge.await.unwrap().unwrap();
    assert!(!path.exists());

    let (_agent, bridge) = bridged(&path);
    let error = bridge.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("no TUI is listening"), "{error}");
    lab.stop().await;
}
