//! `--format` and `--json` from real CLI processes (#2778): an unknown
//! format, or one the command doesn't print, is a usage error (status 2)
//! that sends nothing; each format a command takes is the format it prints.
//! The unit tests in `src/formats_tests.rs` hold every command's set; these
//! run the commands the lab device answers as it stands. The rest are left
//! to their own tests: the alarm commands need an event budget
//! (`cli_alarms`) and `ack-alarm` an alarm, the BBMD commands and
//! `routing-table` a BBMD or a router, the file commands a File object,
//! `sc-check` and `mstp-ports` reach no device, and `subscribe` and
//! `capture`, the two long-running commands, print JSON Lines rather than
//! one document (`cli_streams`, `cli_capture`).
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_server::server::{BACnetServer, DccPolicy};
use bacnet_transport::bip::BipTransport;
use serde_json::Value;
use support::lab::{lab_database, lab_identity};
use support::run;
use support::unsent::Unsent;

/// An unknown `--format`, or one the command doesn't print, on either side
/// of it: status 2, nothing on stdout, the reason on stderr, nothing sent.
#[tokio::test]
async fn a_format_the_command_does_not_print_is_refused_before_anything_is_sent() {
    let device = Unsent::new();
    let source = device.source_port().to_string();
    let target = format!("127.0.0.1:{}", device.port());
    let read = "bacnet read: --format csv is not available; this command supports table and json";
    let csv_exact = "bacnet inventory: --csv-exact needs --format csv";
    let cases: [(Vec<&str>, &str); 9] = [
        (
            vec!["--format", "jsn", "read", &target, "ai:1", "pv"],
            "[possible values: table, json, csv]",
        ),
        (vec!["read", &target, "ai:1", "pv", "--format", "csv"], read),
        (vec!["--format", "csv", "rp", &target, "ai:1", "pv"], read),
        (
            // Before the write guard, too.
            vec!["--format", "csv", "write", &target, "av:1", "pv", "1"],
            "bacnet write: --format csv is not available",
        ),
        (
            vec!["--json", "export", "223-experimental", &target],
            "bacnet export: --json is not available; this command takes no output format \
             (it writes Turtle)",
        ),
        (
            vec!["--json", "tui"],
            "bacnet tui: --json is not available; this command takes no output format",
        ),
        // The default through a pipe (JSON), a table, or JSON: not the CSV.
        (vec!["inventory", &target, "--csv-exact"], csv_exact),
        (
            vec!["inventory", &target, "--csv-exact", "--format", "table"],
            csv_exact,
        ),
        (
            vec!["--json", "inventory", &target, "--csv-exact"],
            "bacnet inventory: --csv-exact needs --format csv without --json",
        ),
    ];
    for (args, diagnostic) in cases {
        let mut line = vec!["--interface", "127.0.0.1", "--port", &source];
        line.extend(&args);
        let output = run(&line).await;
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        support::failure(&output, diagnostic);
        device.assert_nothing_sent(&format!("{args:?}"));
    }
}

/// The lab device on loopback, and its address as a target.
async fn lab() -> (BACnetServer<BipTransport>, String) {
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(lab_database(1234))
        .dcc_policy(DccPolicy::RequirePassword)
        .dcc_password("lab-pass")
        .on_reinitialize(|_, _| Ok(()))
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (server, target)
}

/// `stdout` is in `format`: JSON is one document (a command that prints
/// once prints one JSON value; only a long-running one streams, #2783), and
/// a table or CSV doesn't parse.
fn assert_prints(format: &str, stdout: &[u8], what: &str) {
    let text = String::from_utf8_lossy(stdout);
    assert!(!text.trim().is_empty(), "{what}: nothing on stdout");
    let json = serde_json::from_slice::<Value>(stdout);
    match format {
        "json" => {
            let values: Vec<Value> = serde_json::Deserializer::from_slice(stdout)
                .into_iter()
                .collect::<Result<_, _>>()
                .unwrap_or_else(|error| panic!("{what}: not JSON ({error}): {text}"));
            assert_eq!(values.len(), 1, "{what}: not one JSON document: {text}");
        }
        "table" => assert!(json.is_err(), "{what}: JSON, not a table: {text}"),
        "csv" => {
            assert!(json.is_err(), "{what}: JSON, not CSV: {text}");
            let header = text.lines().next().unwrap_or_default();
            assert!(header.contains(','), "{what}: no CSV header: {text}");
        }
        other => panic!("{what}: no check for {other}"),
    }
}

/// Run each of `commands` (`TARGET` stands for the lab device) with
/// `--format` set to each format it takes, and check that it printed that.
async fn prints_each_format(commands: &[(&[&str], &[&str])]) {
    let (mut server, target) = lab().await;
    for &(command, formats) in commands {
        for &format in formats {
            let mut args = vec!["--interface", "127.0.0.1", "--port", "0"];
            args.extend(command.iter().map(|&word| match word {
                "TARGET" => target.as_str(),
                word => word,
            }));
            args.extend(["--format", format]);
            let output = run(&args).await;
            let what = format!("{command:?} --format {format}");
            assert!(
                output.status.success(),
                "{what}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_prints(format, &output.stdout, &what);
        }
    }
    server.stop().await.unwrap();
}

const TABLE_JSON: &[&str] = &["table", "json"];

/// The reads, and the inventory's three formats.
#[tokio::test]
async fn each_read_prints_each_format_it_takes() {
    prints_each_format(&[
        (&["read", "TARGET", "ai:1", "pv"], TABLE_JSON),
        (&["read", "TARGET", "ai:1", "all"], TABLE_JSON),
        (&["readm", "TARGET", "ai:1", "pv,object-name"], TABLE_JSON),
        (&["read-range", "TARGET", "trend-log:1"], TABLE_JSON),
        (
            &["inventory", "TARGET", "--types", "ai"],
            &["table", "json", "csv"],
        ),
    ])
    .await;
}

/// Discovery, directed at the lab device so no broadcast is needed.
#[tokio::test]
async fn each_discovery_prints_each_format_it_takes() {
    prints_each_format(&[
        (
            &["discover", "--target", "TARGET", "--wait", "1"],
            TABLE_JSON,
        ),
        (
            &["find", "Zone Temp", "--target", "TARGET", "--wait", "1"],
            TABLE_JSON,
        ),
        (
            &["whois-router", "--router", "TARGET", "--wait", "0"],
            TABLE_JSON,
        ),
    ])
    .await;
}

/// The commands that change the device: a status line, or its JSON.
#[tokio::test]
async fn each_change_prints_each_format_it_takes() {
    prints_each_format(&[
        (
            &["--allow-write", "write", "TARGET", "av:1", "pv", "42"],
            TABLE_JSON,
        ),
        (
            &["--allow-write", "writem", "TARGET", "av:1", "pv=43"],
            TABLE_JSON,
        ),
        (&["--allow-write", "time-sync", "TARGET"], TABLE_JSON),
        // An object each format creates and deletes.
        (
            &["--allow-write", "create-object", "TARGET", "av:100"],
            &["table"],
        ),
        (
            &["--allow-write", "delete-object", "TARGET", "av:100"],
            &["table"],
        ),
        (
            &["--allow-write", "create-object", "TARGET", "av:101"],
            &["json"],
        ),
        (
            &["--allow-write", "delete-object", "TARGET", "av:101"],
            &["json"],
        ),
        (
            &[
                "--allow-write",
                "--yes",
                "control",
                "TARGET",
                "enable",
                "--password",
                "lab-pass",
            ],
            TABLE_JSON,
        ),
        (
            &["--allow-write", "--yes", "reinit", "TARGET", "warmstart"],
            TABLE_JSON,
        ),
    ])
    .await;
}
