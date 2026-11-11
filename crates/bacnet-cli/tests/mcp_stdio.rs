//! `bacnet mcp serve --stdio` as an agent host launches it: JSON-RPC on
//! stdin and stdout, logs on stderr only (#2066). A live server needs an
//! authorized profile; `--passive` alone needs none.
#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// A B/IP profile on loopback, as a person would write one.
const PROFILE: &str = "version = 1\ntransport = \"bip\"\n\n[bip]\ninterface = \"127.0.0.1\"\n\
     port = 0\n\n[safety]\nwrites = \"never\"\ndangerous = false\n\n[mcp]\nnetworks = [0]\n";

/// A profile store holding `site`, written as `text`.
fn store(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profiles")).unwrap();
    std::fs::write(dir.path().join("profiles/site.toml"), text).unwrap();
    dir
}

fn bacnet(config: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bacnet"));
    command.env_remove("RUST_LOG");
    if let Some(config) = config {
        command.env("BACNET_CONFIG_DIR", config);
    }
    command
}

/// Run the server with `args`, send `requests` (one JSON-RPC message a
/// line), close stdin, and return its stdout lines and stderr.
fn session(args: &[&str], config: Option<&Path>, requests: &[Value]) -> (Vec<String>, String) {
    let mut child = bacnet(config)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut lines = stdout.lines();
    let mut out = Vec::new();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        // A request with an id gets its answer before the next goes.
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
    drop(stdin);
    out.extend(lines.map(Result::unwrap));
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "exit {:?}", output.status);
    (out, String::from_utf8_lossy(&output.stderr).into_owned())
}

/// A command that must refuse: its stderr, after checking it failed and
/// wrote nothing to stdout.
fn refused(args: &[&str], config: Option<&Path>) -> String {
    let output = bacnet(config)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success(), "{args:?} ran");
    assert!(output.stdout.is_empty());
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn requests() -> Vec<Value> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "test-host", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "bacnet_discover", "arguments": { "send": false } } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "prompts/list" }),
    ]
}

/// Every stdout line is a JSON-RPC message.
fn assert_json_rpc_only(lines: &[String]) {
    assert!(lines.len() >= 4, "{lines:?}");
    for line in lines {
        let value: Value = serde_json::from_str(line.trim_end_matches('\r'))
            .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
        assert_eq!(value["jsonrpc"], json!("2.0"), "{line}");
    }
}

/// Passive, at the default log level: only JSON-RPC on stdout, and the
/// read log on stderr all the same.
#[test]
fn passive_stdout_carries_only_json_rpc() {
    let (lines, stderr) = session(&["mcp", "serve", "--stdio", "--passive"], None, &requests());
    assert_json_rpc_only(&lines);
    let tools: Value = serde_json::from_str(lines[1].trim_end_matches('\r')).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bacnet_discover", "bacnet_decode"]);
    assert!(
        stderr.contains("read call"),
        "the read log is on by default: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1b}'),
        "no colour off a terminal: {stderr}"
    );
}

#[test]
fn serve_needs_stdio() {
    refused(&["mcp", "serve"], None);
}

/// Live traffic needs a profile: transport flags alone are refused.
#[test]
fn a_live_server_needs_a_profile() {
    let stderr = refused(
        &[
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "mcp",
            "serve",
            "--stdio",
        ],
        None,
    );
    assert!(stderr.contains("--profile"), "{stderr}");
}

/// A profile nobody authorized for MCP doesn't serve, and transport flags
/// can't change a profile's transport.
#[test]
fn an_unauthorized_profile_or_extra_flags_are_refused() {
    let config = store(PROFILE);
    let stderr = refused(
        &["mcp", "serve", "--stdio", "--profile", "site"],
        Some(config.path()),
    );
    assert!(
        stderr.contains("bacnet mcp authorize --profile site"),
        "{stderr}"
    );
    let stderr = refused(
        &[
            "--port",
            "47809",
            "mcp",
            "serve",
            "--stdio",
            "--profile",
            "site",
        ],
        Some(config.path()),
    );
    assert!(stderr.contains("transport flags"), "{stderr}");
    // A profile that isn't there says where it was looked for and what
    // to run: an agent host's settings named it (#2304).
    let stderr = refused(
        &["mcp", "serve", "--stdio", "--profile", "other"],
        Some(config.path()),
    );
    assert!(
        stderr.contains("no profile named other")
            && stderr.contains("profiles")
            && stderr.contains("bacnet mcp authorize --profile other"),
        "{stderr}"
    );
    // The same for a passive server, which reads the profile's policy.
    let stderr = refused(
        &["mcp", "serve", "--stdio", "--passive", "--profile", "other"],
        Some(config.path()),
    );
    assert!(stderr.contains("no profile named other"), "{stderr}");
    // A bare consent time authorizes nothing.
    let bare = store(&format!("{PROFILE}authorized_at_unix = 1\n"));
    let stderr = refused(
        &["mcp", "serve", "--stdio", "--profile", "site"],
        Some(bare.path()),
    );
    assert!(stderr.contains("not authorized"), "{stderr}");
}

/// The whole path: a person authorizes the profile at a terminal, then
/// the agent host's server runs on it, live on loopback.
#[cfg(unix)]
#[test]
fn an_authorized_profile_serves_live() {
    use std::io::Read;
    let config = store(PROFILE);
    let pty = nix::pty::openpty(None, None).expect("openpty");
    let mut child = bacnet(Some(config.path()))
        .args(["mcp", "authorize", "--profile", "site"])
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave))
        .spawn()
        .unwrap();
    let mut master = std::fs::File::from(pty.master);
    master.write_all(b"site\n").unwrap();
    let reader = std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buffer = [0; 4096];
        // Linux reports EIO, not end of file, once the slave is closed.
        while let Ok(read @ 1..) = master.read(&mut buffer) {
            seen.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8_lossy(&seen).into_owned()
    });
    let status = child.wait().unwrap();
    let shown = reader.join().unwrap();
    assert!(status.success(), "authorize failed: {shown}");
    assert!(
        shown.contains("networks: [0]") && shown.contains("authorized"),
        "{shown}"
    );

    let (lines, stderr) = session(
        &["mcp", "serve", "--stdio", "--profile", "site"],
        Some(config.path()),
        &requests(),
    );
    assert_json_rpc_only(&lines);
    let tools: Value = serde_json::from_str(lines[1].trim_end_matches('\r')).unwrap();
    assert_eq!(
        tools["result"]["tools"].as_array().unwrap().len(),
        12,
        "{tools}"
    );
    let call: Value = serde_json::from_str(lines[2].trim_end_matches('\r')).unwrap();
    assert_eq!(call["result"]["isError"], json!(false), "{call}");
    assert!(stderr.contains("read call"), "{stderr}");
}

/// The release's MCP bundles are smoke-tested with an authorized profile
/// kept beside the packer, since a host can't run `bacnet mcp authorize`
/// (#2304). It must still be authorized, and its server must list exactly
/// the read tools the bundle's manifest names.
#[test]
fn the_bundle_smoke_profile_serves_the_manifests_tools() {
    let release = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release/mcpb");
    let profile = std::fs::read_to_string(release.join("smoke-profile.toml")).unwrap();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(release.join("manifest.json")).unwrap())
            .unwrap();
    let config = store(&profile);
    let (lines, _) = session(
        &["mcp", "serve", "--stdio", "--profile", "site"],
        Some(config.path()),
        &requests(),
    );
    assert_json_rpc_only(&lines);
    let hello: Value = serde_json::from_str(lines[0].trim_end_matches('\r')).unwrap();
    let title = hello["result"]["serverInfo"]["title"].as_str().unwrap();
    assert!(title.contains("read-only"), "{hello}");
    let tools: Value = serde_json::from_str(lines[1].trim_end_matches('\r')).unwrap();
    let mut served: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut listed: Vec<&str> = manifest["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    served.sort_unstable();
    listed.sort_unstable();
    assert_eq!(served, listed);
}

/// A refusal reads as plain text on stderr, as an agent host's log shows
/// it: no Debug quotes around the message and no doubled backslashes in a
/// Windows path, and the exit code is 1 (#2325).
#[test]
fn a_refusal_prints_plain_text_and_exits_1() {
    let config = store(PROFILE);
    let output = bacnet(Some(config.path()))
        .args(["mcp", "serve", "--stdio", "--profile", "other"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    let line = lines[0].trim_end_matches('\r');
    assert!(
        line.starts_with("Error: no profile named other in "),
        "{line}"
    );
    assert!(!line.ends_with('"'), "{line}");
    assert!(!line.contains("\\\\") && !line.contains("\\\""), "{line}");
    // The store's path as the OS spells it, separators unescaped.
    let profiles = config.path().join("profiles");
    assert!(line.contains(&*profiles.to_string_lossy()), "{line}");
}
