//! The one-shot write guard (#972), from real CLI processes: a mutating
//! command without `--allow-write`, or device management without `--yes`
//! when stdin is not a terminal, exits with status 4 and sends nothing.
//! With the flags, the same commands reach the lab device. A finite
//! `subscribe` (the default) is cancelled when the command is stopped.
#[allow(dead_code)]
mod support;

use std::io::ErrorKind;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_server::server::{BACnetServer, DccPolicy};
use bacnet_transport::bip::BipTransport;
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};
use support::run;

/// Every mutating one-shot, aliases included, as arguments after the target.
const MUTATING: &[(&str, &[&str])] = &[
    ("write", &["av:1", "pv", "42"]),
    ("wp", &["av:1", "pv", "42@8"]),
    ("writem", &["av:1", "pv=42,on=Zone"]),
    ("wpm", &["av:1", "pv=42@8", "bv:1", "pv=active"]),
    ("create-object", &["av:100"]),
    ("delete-object", &["av:1"]),
    ("file-write", &["1", "no-such-input.bin"]),
    (
        "ack-alarm",
        &[
            "ai:1",
            "--state",
            "1",
            "--timestamp",
            "sequence:1",
            "--ack-time",
            "sequence:2",
        ],
    ),
    ("time-sync", &[]),
    ("ts", &["--utc"]),
    ("unregister", &[]),
    ("control", &["enable"]),
    ("dcc", &["disable-initiation", "--duration", "5"]),
    ("reinit", &["warmstart"]),
    ("subscribe", &["ai:1", "--lifetime", "0"]),
    ("cov", &["ai:1", "--confirmed", "--lifetime", "0"]),
];

/// The class each refused command reports.
fn class(command: &str) -> &'static str {
    match command {
        "control" | "dcc" | "reinit" => "device-management",
        _ => "write",
    }
}

fn canonical(command: &str) -> &str {
    match command {
        "wp" => "write",
        "wpm" => "writem",
        "ts" => "time-sync",
        "dcc" => "control",
        "cov" => "subscribe",
        other => other,
    }
}

/// A loopback "device" that only counts what reaches it.
fn listener() -> (UdpSocket, String) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket.set_nonblocking(true).unwrap();
    let target = format!("127.0.0.1:{}", socket.local_addr().unwrap().port());
    (socket, target)
}

/// Nothing has arrived. The CLI process has exited, so anything it sent
/// over loopback is already queued on the socket.
fn assert_nothing_sent(socket: &UdpSocket, what: &str) {
    let mut buf = [0u8; 1500];
    match socket.recv_from(&mut buf) {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        other => panic!("{what}: a datagram reached the device: {other:?}"),
    }
}

fn args<'a>(
    globals: &[&'a str],
    command: &'a str,
    target: &'a str,
    rest: &[&'a str],
) -> Vec<&'a str> {
    let mut args = vec!["--interface", "127.0.0.1", "--port", "0"];
    args.extend_from_slice(globals);
    args.push(command);
    args.push(target);
    args.extend_from_slice(rest);
    args
}

#[tokio::test]
async fn a_mutating_one_shot_without_allow_write_sends_nothing() {
    let (socket, target) = listener();
    for &(command, rest) in MUTATING {
        // Table errors name the missing flag.
        let output = run(args(&["--format", "table"], command, &target, rest)).await;
        assert_eq!(output.status.code(), Some(4), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("needs --allow-write"),
            "{command}: {stderr}"
        );
        assert!(stderr.contains("nothing was sent"), "{command}: {stderr}");
        assert_nothing_sent(&socket, command);

        // JSON: the stable shape, on stderr.
        let output = run(args(&["--json"], command, &target, rest)).await;
        assert_eq!(output.status.code(), Some(4), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
        let missing = match class(command) {
            // Stdin is /dev/null here, not a terminal.
            "device-management" => json!(["--allow-write", "--yes"]),
            _ => json!(["--allow-write"]),
        };
        assert_eq!(refusal["error"], "refused", "{command}");
        assert_eq!(refusal["reason"], "allow-write-required", "{command}");
        assert_eq!(refusal["command"], canonical(command), "{command}");
        assert_eq!(refusal["class"], class(command), "{command}");
        assert_eq!(refusal["missing"], missing, "{command}");
        assert_nothing_sent(&socket, command);
    }
}

#[tokio::test]
async fn device_management_off_a_terminal_needs_yes_as_well() {
    let (socket, target) = listener();
    for &(command, rest) in MUTATING {
        if class(command) != "device-management" {
            continue;
        }
        let output = run(args(&["--json", "--allow-write"], command, &target, rest)).await;
        assert_eq!(output.status.code(), Some(4), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(refusal["reason"], "yes-required", "{command}");
        assert_eq!(refusal["missing"], json!(["--yes"]), "{command}");
        assert_nothing_sent(&socket, command);
    }
}

async fn lab(reinitialized: Arc<AtomicBool>) -> (BACnetServer<BipTransport>, String) {
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(lab_database(1234))
        .dcc_policy(DccPolicy::RequirePassword)
        .dcc_password("lab-pass")
        .on_reinitialize(move |_, _| {
            reinitialized.store(true, Ordering::SeqCst);
            Ok(())
        })
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (server, target)
}

/// With the flags given, on either side of the subcommand, the commands
/// run; reads never needed them.
#[tokio::test]
async fn with_the_flags_the_commands_reach_the_device() {
    let reinitialized = Arc::new(AtomicBool::new(false));
    let (mut server, target) = lab(reinitialized.clone()).await;
    for (globals, command, rest) in [
        (&["--allow-write"][..], "write", &["av:1", "pv", "42.5"][..]),
        (&[], "write", &["av:1", "pv", "43.5", "--allow-write"]),
        (&["--allow-write"], "writem", &["av:1", "pv=44.5"]),
        (
            &["--allow-write", "--yes"],
            "control",
            &["enable", "--password", "lab-pass"],
        ),
        (&["--allow-write"], "reinit", &["warmstart", "--yes"]),
    ] {
        let output = run(args(
            &[&["--json"], globals].concat(),
            command,
            &target,
            rest,
        ))
        .await;
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = run(args(&["--json"], "read", &target, &["av:1", "pv"])).await;
    server.stop().await.unwrap();
    assert!(output.status.success());
    let read: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(read["value"], "44.5", "{read}");
    assert!(reinitialized.load(Ordering::SeqCst));
}

/// The lab device's Active_COV_Subscriptions, as the CLI reads it. Only the
/// Unix signal test uses it.
#[cfg(unix)]
async fn active_subscriptions(target: &str) -> String {
    let output = run(args(
        &["--json"],
        "read",
        target,
        &["device:1234", "active-cov-subscriptions"],
    ))
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let read: Value = serde_json::from_slice(&output.stdout).unwrap();
    read["value"].as_str().unwrap().to_string()
}

/// `subscribe` without `--lifetime` asks for a finite subscription, needs
/// no flag, and cancels it when stopped by Ctrl+C (SIGINT) or SIGTERM.
#[cfg(unix)]
#[tokio::test]
async fn a_default_subscription_is_finite_and_cancelled_when_stopped() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let (mut server, target) = lab(Arc::new(AtomicBool::new(false))).await;
    let empty = active_subscriptions(&target).await;
    for signal in ["-INT", "-TERM"] {
        let mut child = support::command()
            .args(args(
                &["--format", "table"],
                "subscribe",
                &target,
                &["ai:2"],
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Wait for the confirmation line: the subscription is in place.
        // The reader stays open so later notifications have somewhere to go.
        let stdout = child.stdout.take().unwrap();
        let (first, _reader) = support::bounded(tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            (line, reader)
        }))
        .await
        .unwrap();
        assert!(first.contains("lifetime 300s"), "{first}");
        let active = active_subscriptions(&target).await;
        assert_ne!(active, empty, "no subscription after {first}");

        let pid = child.id().to_string();
        let killed = std::process::Command::new("kill")
            .args([signal, &pid])
            .status()
            .unwrap();
        assert!(killed.success());
        let status = support::bounded(tokio::task::spawn_blocking(move || child.wait()))
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{signal}: {status:?}");
        assert_eq!(
            active_subscriptions(&target).await,
            empty,
            "{signal} left the subscription"
        );
    }
    server.stop().await.unwrap();
}
