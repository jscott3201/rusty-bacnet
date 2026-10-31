//! `bacnet mcp connect` as an agent host launches it (#2185): its stdin
//! and stdout bridged to the socket a TUI listens on in the profile's
//! workspace. Here the listener runs in the test, on a lab device's
//! engine, as the TUI runs it; stdout carries only the server's JSON-RPC.
#![cfg(all(unix, feature = "mcp"))]

#[allow(dead_code)] // Other integration tests use the rest.
#[path = "support/lab.rs"]
mod lab;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use bacnet_client::client::BACnetClient;
use bacnet_mcp::Policy;
use bacnet_mcp::attach::Listener;
use bacnet_mcp::policy::{ArmMode, Workspace, WriteMode, WritePolicy};
use bacnet_ops::engine::Engine;
use bacnet_ops::limits::RequestLimits;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use serde_json::{Value, json};

const LAB: u32 = 1201;
const LAB_MAC: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];

fn bacnet(config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bacnet"));
    command
        .env_remove("RUST_LOG")
        .env("BACNET_CONFIG_DIR", config);
    command
}

/// A config root with profile `site` and its private workspace.
fn config() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("profiles")).unwrap();
    std::fs::write(root.path().join("profiles/site.toml"), "version = 1\n").unwrap();
    let workspace = root.path().join("workspaces/site");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

/// `bacnet mcp connect` with `args` and nothing listening: its stderr,
/// after checking it failed and wrote nothing to stdout.
fn refused(config: &Path, args: &[&str]) -> String {
    let output = bacnet(config)
        .args(["mcp", "connect"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success(), "{args:?} ran");
    assert!(output.stdout.is_empty());
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// With no TUI listening, `connect` says so on stderr and fails.
#[test]
fn nothing_listening_is_an_error_on_stderr() {
    let root = config();
    let stderr = refused(root.path(), &["--profile", "site"]);
    assert!(stderr.contains("no TUI is listening"), "{stderr}");
    let stderr = refused(root.path(), &[]);
    assert!(stderr.contains("no TUI is listening"), "{stderr}");
}

/// The lab device on a loopback link, and an engine on its client.
async fn lab() -> (
    BACnetServer<LoopbackTransport>,
    Arc<Engine<LoopbackTransport>>,
) {
    let (client_port, server_port) =
        LoopbackTransport::pair(vec![10, 0, 0, 1, 0xBA, 0xC0], LAB_MAC.to_vec());
    let server = BACnetServer::generic_builder()
        .device(lab::lab_identity(LAB))
        .database(lab::lab_database(LAB))
        .transport(server_port)
        .build()
        .await
        .unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_port)
        .build()
        .await
        .unwrap();
    client.add_device(LAB, &LAB_MAC).await.unwrap();
    let engine = Engine::new(Arc::new(client), Arc::new(RequestLimits::new(20)));
    (server, Arc::new(engine))
}

/// Run `connect` with `args`, send `requests` one line each (waiting for
/// the answer to each with an id), and return its stdout lines; the child
/// is left running, its stdin open.
fn session(
    config: &Path,
    args: &[&str],
    requests: Vec<Value>,
) -> (std::process::Child, Vec<String>) {
    let mut child = bacnet(config)
        .args(["mcp", "connect"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut out = Vec::new();
    for request in &requests {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        if request.get("id").is_some() {
            loop {
                let line = lines.next().expect("an answer").unwrap();
                let done = serde_json::from_str::<Value>(line.trim_end_matches('\r'))
                    .is_ok_and(|v| v["id"] == request["id"]);
                out.push(line);
                if done {
                    break;
                }
            }
        }
    }
    // Stdin stays open: the host hasn't gone, only the TUI will.
    child.stdin = Some(stdin);
    (child, out)
}

fn requests() -> Vec<Value> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "test-host", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "bacnet_read", "arguments": { "device": LAB,
            "reads": [{ "object": "ai:2", "properties": ["present-value"] }] } } }),
    ]
}

/// Acceptance: through `connect` (with the profile named, then found as
/// the only one listening), an agent reads the lab on the listener's
/// engine, which its rate meter shows; stdout carries only JSON-RPC, and
/// the tools are read-only though the policy offers writes. When the TUI
/// shuts its listener, `connect` exits cleanly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_bridges_an_agent_to_the_listener() {
    let root = config();
    let workspace = root.path().join("workspaces/site");
    let (mut server, engine) = lab().await;
    for args in [&["--profile", "site"][..], &[]] {
        let policy = Policy {
            writes: WritePolicy {
                mode: WriteMode::Elicit,
                arm: ArmMode::Elicit,
                plan: None,
                workspace: None,
            },
            ..Policy::default()
        };
        let place = Workspace {
            dir: workspace.clone(),
            profile: Some("site".into()),
            transport: "BIP",
            local: "10.0.0.1:47808".into(),
        };
        let listener = Listener::bind(Arc::clone(&engine), policy, "tester", place)
            .await
            .unwrap();
        let before = engine.limits().recent();
        let config = root.path().to_path_buf();
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let (child, lines) = tokio::task::spawn_blocking(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            session(&config, &args, requests())
        })
        .await
        .unwrap();
        for line in &lines {
            let value: Value = serde_json::from_str(line.trim_end_matches('\r'))
                .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
            assert_eq!(value["jsonrpc"], json!("2.0"), "{line}");
        }
        let tools: Value = serde_json::from_str(lines[1].trim_end_matches('\r')).unwrap();
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 11, "{names:?}");
        assert!(!names.contains(&"bacnet_write") && !names.contains(&"bacnet_revert"));
        let read: Value = serde_json::from_str(lines[2].trim_end_matches('\r')).unwrap();
        assert_eq!(read["result"]["isError"], json!(false), "{read}");
        assert!(engine.limits().recent() > before, "the listener's meter");

        listener.shutdown().await;
        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(output.status.success(), "exit {:?}", output.status);
    }
    // Every session has let go of the engine, and with it the client.
    let engine = Arc::try_unwrap(engine).ok().expect("the listener let go");
    drop(engine);
    server.stop().await.unwrap();
}
