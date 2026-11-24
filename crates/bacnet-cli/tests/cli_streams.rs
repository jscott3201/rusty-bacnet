//! `subscribe --json` from real CLI processes (#2783): a long-running
//! command prints JSON Lines, one compact JSON object a line from the
//! opening line on, each flushed as it is written, and nothing else on
//! stdout. A failure once the stream runs ends it with an error line beside
//! the usual stderr message and exit status; a refused subscription prints
//! nothing; a reader that goes away stops the watch quietly, a success.
//! `capture`'s stream is checked in `cli_capture`, the one-document
//! commands' JSON in `cli_format` and the commands' own tests.
#[allow(dead_code)]
mod support;

use std::io::{BufRead, BufReader};
use std::process::Stdio;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::Value;
use support::relay::{relay, server};
use support::stream::{KillOnDrop, json_lines, kinds};

/// The command line for `subscribe` to AI:2 at `target` printing `format`,
/// then `rest`.
fn subscribe_as<'a>(format: &'a str, target: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
        "subscribe",
        target,
        "ai:2",
    ];
    args.extend_from_slice(rest);
    args
}

/// [`subscribe_as`] JSON.
fn subscribe<'a>(target: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    subscribe_as("json", target, rest)
}

/// The stderr line `main` prints for a failure, after `Error: `.
fn stderr_error(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("Error: "))
        .unwrap_or_else(|| panic!("no error on stderr: {stderr}"))
        .to_owned()
}

/// The lab device's AI:2.
fn ai2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
}

/// The lab device's Active_COV_Subscriptions as `read` prints it.
async fn active_subscriptions(target: &str) -> String {
    let output = support::run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--json",
        "read",
        target,
        "device:1234",
        "active-cov-subscriptions",
    ])
    .await;
    assert!(output.status.success(), "{output:?}");
    let read: Value = serde_json::from_slice(&output.stdout).unwrap();
    read["value"].as_str().unwrap().to_owned()
}

/// A device that goes silent mid-stream: the renewal and then the cancel
/// time out, the stream ends with an error line carrying stderr's message,
/// and the status is 1.
#[tokio::test]
async fn a_failure_mid_stream_ends_it_with_an_error_line() {
    let (mut device, address) = server().await;
    // Only the first SubscribeCOV gets through.
    let (target, _seen, relay) = relay(address, 1).await;
    // Renewed after a second; each request given up after 4 x 200 ms.
    let output = support::run(subscribe(&target, &["--lifetime", "2", "--timeout", "200"])).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let values = json_lines(&output.stdout);
    let types = kinds(&values);
    assert_eq!(types.first(), Some(&"subscribed"), "{values:?}");
    assert_eq!(types.last(), Some(&"error"), "{values:?}");
    assert!(
        types[1..types.len() - 1].iter().all(|kind| *kind == "cov"),
        "{values:?}"
    );
    let error = values.last().unwrap();
    assert_eq!(
        error.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["message", "type"]
    );
    let message = error["message"].as_str().unwrap();
    assert_eq!(message, stderr_error(&output.stderr));
    assert!(
        message.starts_with("renewing the subscription failed: ")
            && message.contains("; cancelling the subscription failed: "),
        "{message}"
    );
    relay.abort();
    device.stop().await.unwrap();
}

/// A subscription the device refuses fails before the stream starts:
/// nothing on stdout, the error on stderr, status 1.
#[tokio::test]
async fn a_refused_subscription_prints_nothing() {
    let (mut device, address) = server().await;
    let target = address.to_string();
    let mut args = subscribe(&target, &[]);
    *args.last_mut().unwrap() = "ai:99";
    let output = support::run(&args).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    support::failure(&output, "Error: ");
    device.stop().await.unwrap();
}

/// A reader that goes away (`| head`) stops the watch quietly in either
/// format: the next notification finds stdout closed, the subscription is
/// cancelled, nothing is said on stderr, and the status is 0. A closed pipe
/// is a broken-pipe error on each OS: EPIPE on Unix, and on Windows
/// ERROR_NO_DATA or ERROR_BROKEN_PIPE, both of which std reports as
/// `ErrorKind::BrokenPipe`.
#[tokio::test]
async fn a_closed_stdout_stops_the_watch_quietly_and_cancels() {
    let (mut device, address) = server().await;
    let target = address.to_string();
    let empty = active_subscriptions(&target).await;
    for (format, first, value) in [
        ("json", "\"type\":\"cov\"", 60.5),
        ("table", "[COV] ", 70.5),
    ] {
        let mut child = support::command()
            .args(subscribe_as(format, &target, &[]))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut child = KillOnDrop(child);
        // Up to the initial notification; then the reader goes, closing
        // the pipe.
        support::bounded(tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0, "stdout ended");
                if line.contains(first) {
                    return;
                }
            }
        }))
        .await
        .unwrap();
        assert_ne!(active_subscriptions(&target).await, empty, "{format}");

        device
            .set_present_value_local(&ai2(), PropertyValue::Real(value))
            .await
            .unwrap();
        let status = child.exited().await;
        let stderr = child.stderr();
        assert_eq!(status.code(), Some(0), "{format}: {stderr}");
        let said = stderr.to_lowercase();
        assert!(
            !said.contains("error") && !said.contains("panicked"),
            "{format}: {stderr}"
        );
        // Unix at least says nothing at all.
        if cfg!(unix) {
            assert_eq!(stderr, "", "{format}");
        }
        assert_eq!(
            active_subscriptions(&target).await,
            empty,
            "{format}: not cancelled"
        );
    }
    device.stop().await.unwrap();
}

/// Stopped by SIGTERM: Unix only.
#[cfg(unix)]
mod stopped {
    use std::process::Stdio;

    use bacnet_types::primitives::PropertyValue;
    use serde_json::Value;

    use super::support::relay::server;
    use super::support::stream::{KillOnDrop, Lines};
    use super::{ai2, subscribe, support};

    /// A notification's line for AI:2 with its typed value.
    fn assert_cov(line: &Value, value: f64) {
        assert_eq!(line["type"], "cov", "{line}");
        assert_eq!(line["object"], "ANALOG_INPUT:2", "{line}");
        assert_eq!(line["device"], "device:1234", "{line}");
        assert_eq!(line["decoded"]["PRESENT_VALUE"], value, "{line}");
        assert!(line["values"]["PRESENT_VALUE"].is_string(), "{line}");
    }

    /// Each line is one JSON value, written when it happens: the opening
    /// line while the command runs, then the initial notification and one
    /// for a change, its sender in the link's form. Stopped by SIGTERM, it
    /// ends with no closing line and status 0.
    #[tokio::test]
    async fn subscribe_prints_one_json_value_a_line_as_it_happens() {
        let (mut device, address) = server().await;
        let target = address.to_string();
        let mut child = support::command()
            .args(subscribe(&target, &[]))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = Lines::read(child.stdout.take().unwrap());
        let mut child = KillOnDrop(child);

        let opening = lines.next().await;
        assert!(child.0.try_wait().unwrap().is_none(), "the command ended");
        assert_eq!(opening["type"], "subscribed", "{opening}");
        assert_eq!(opening["status"], "ok", "{opening}");
        let message = opening["message"].as_str().unwrap();
        assert!(
            message.starts_with("Subscribed to ANALOG_INPUT:2"),
            "{message}"
        );
        let initial = lines.next().await;
        assert_cov(&initial, 51.0);
        assert_eq!(
            initial["source_mac"], target,
            "the sender, as B/IP writes it"
        );

        device
            .set_present_value_local(&ai2(), PropertyValue::Real(60.5))
            .await
            .unwrap();
        assert_cov(&lines.next().await, 60.5);

        let killed = std::process::Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        assert!(child.exited().await.success());
        assert_eq!(lines.rest().await, Vec::<Value>::new(), "after the stop");
        device.stop().await.unwrap();
    }
}
