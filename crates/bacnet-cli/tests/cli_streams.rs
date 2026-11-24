//! `subscribe --json` from real CLI processes (#2783): a long-running
//! command prints JSON Lines, one compact JSON object a line from the
//! opening line on, each flushed as it is written, and nothing else on
//! stdout. A failure once the stream runs ends it with an error line beside
//! the usual stderr message and exit status; a refused subscription prints
//! nothing. `capture`'s stream is checked in `cli_capture`, the one-document
//! commands' JSON in `cli_format` and the commands' own tests.
#[allow(dead_code)]
mod support;

use support::relay::{relay, server};
use support::stream::{json_lines, kinds};

/// The command line for `subscribe --json` to AI:2 at `target`, then `rest`.
fn subscribe<'a>(target: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--json",
        "subscribe",
        target,
        "ai:2",
    ];
    args.extend_from_slice(rest);
    args
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

/// The streams a test stops, by SIGTERM or by closing its end of stdout:
/// Unix only.
#[cfg(unix)]
mod stopped {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    use serde_json::Value;

    use super::support::relay::server;
    use super::support::stream::{Lines, json_lines, kinds};
    use super::{stderr_error, subscribe, support};

    /// The lab device's AI:2.
    fn ai2() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
    }

    /// A notification's line for AI:2 with its typed value.
    fn assert_cov(line: &Value, value: f64) {
        assert_eq!(line["type"], "cov", "{line}");
        assert_eq!(line["object"], "ANALOG_INPUT:2", "{line}");
        assert_eq!(line["device"], "device:1234", "{line}");
        assert_eq!(line["decoded"]["PRESENT_VALUE"], value, "{line}");
        assert!(line["values"]["PRESENT_VALUE"].is_string(), "{line}");
    }

    /// A child process killed and reaped when dropped, unless it has exited.
    struct KillOnDrop(std::process::Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    impl KillOnDrop {
        /// The exit status, within the suite's bound.
        async fn exited(&mut self) -> std::process::ExitStatus {
            support::bounded(async {
                loop {
                    if let Some(status) = self.0.try_wait().unwrap() {
                        return status;
                    }
                    // Process-exit polling only.
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
        }
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

    /// Each line is one JSON value, written when it happens: the opening
    /// line while the command runs, then the initial notification and one
    /// for a change. Stopped by SIGTERM, it ends with no closing line and
    /// status 0.
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
        assert_cov(&lines.next().await, 51.0);

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

    /// A reader that goes away ends the watch: the next notification's
    /// write fails, the subscription is cancelled, and stderr says why
    /// (status 1). On Unix a write to a pipe with no reader fails at once
    /// (EPIPE: Rust ignores SIGPIPE).
    #[tokio::test]
    async fn a_closed_stdout_ends_the_watch_and_cancels_the_subscription() {
        let (mut device, address) = server().await;
        let target = address.to_string();
        let empty = active_subscriptions(&target).await;
        let mut child = support::command()
            .args(subscribe(&target, &[]))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut child = KillOnDrop(child);
        // The opening line and the initial notification; then the reader
        // goes, closing the pipe.
        let read = support::bounded(tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            let mut text = String::new();
            for _ in 0..2 {
                assert_ne!(reader.read_line(&mut text).unwrap(), 0, "stdout ended");
            }
            text
        }))
        .await
        .unwrap();
        assert_eq!(kinds(&json_lines(read.as_bytes())), ["subscribed", "cov"]);
        assert_ne!(active_subscriptions(&target).await, empty);

        device
            .set_present_value_local(&ai2(), PropertyValue::Real(60.5))
            .await
            .unwrap();
        assert_eq!(child.exited().await.code(), Some(1));
        let mut stderr = Vec::new();
        let mut pipe = child.0.stderr.take().unwrap();
        std::io::Read::read_to_end(&mut pipe, &mut stderr).unwrap();
        assert!(
            stderr_error(&stderr).starts_with("writing to stdout failed: "),
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(active_subscriptions(&target).await, empty, "not cancelled");
        device.stop().await.unwrap();
    }
}
