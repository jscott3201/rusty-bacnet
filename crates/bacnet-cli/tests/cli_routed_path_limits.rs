//! `--routed-path-limit` and the routed-path summary (#2667), against a raw
//! UDP peer playing a router to network 5, where station 0x0A answers.
//!
//! The CLI runs as plain B/IP, whose log lines went to stdout before #2667:
//! the client's warning about a too-long reject it did not believe must
//! now leave a `--json` result parseable. The peer learns nothing from the
//! CLI's Who-Is (a local broadcast on loopback reaches only the CLI's own
//! port), so the CLI binds a port the test picks, and the peer sends the
//! station's I-Am, relayed, to that port until the CLI's request arrives.
#[allow(dead_code)]
mod support;

use std::net::{Ipv4Addr, SocketAddr};
use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use support::run;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

const NETWORK: u16 = 5;
const STATION: u8 = 0x0A;
const INSTANCE: u32 = 1234;

/// How the peer answers each confirmed request.
#[derive(Clone, Copy)]
enum Script {
    /// The station's acknowledgement.
    Answer,
    /// A too-long reject for the network, then the acknowledgement: the
    /// reject was for other traffic. The answer waits 100 ms, so the client
    /// takes the reject while the request is still active, as a reject for
    /// other traffic would land.
    RejectThenAnswer,
    /// A too-long reject and nothing else: the request times out.
    RejectOnly,
}

fn bvll(npdu: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + npdu.len()).unwrap().to_be_bytes();
    [&[0x81, 0x0A, length[0], length[1]], npdu].concat()
}

/// `apdu` from the station, as the router relays it (SNET 5, SADR 0x0A).
fn relayed(apdu: &[u8]) -> Vec<u8> {
    let [hi, lo] = NETWORK.to_be_bytes();
    bvll(&[&[0x01, 0x08, hi, lo, 1, STATION], apdu].concat())
}

/// The station's I-Am: Max APDU 1476, no segmentation, vendor 555.
fn i_am() -> Vec<u8> {
    let id = ((8u32 << 22) | INSTANCE).to_be_bytes();
    relayed(
        &[
            &[0x10, 0x00, 0xC4][..],
            &id,
            &[0x22, 0x05, 0xC4, 0x91, 0x03, 0x22, 0x02, 0x2B],
        ]
        .concat(),
    )
}

/// A Reject-Message-To-Network, reason 4 (message too long), for network 5.
fn too_long() -> Vec<u8> {
    let [hi, lo] = NETWORK.to_be_bytes();
    bvll(&[0x01, 0x80, 0x03, 0x04, hi, lo])
}

/// The APDU of a routed Original-Unicast-NPDU to the station, if it is a
/// confirmed request.
fn routed_request(datagram: &[u8]) -> Option<&[u8]> {
    let npdu = datagram.strip_prefix(&[0x81, 0x0A])?.get(2..)?;
    if npdu.get(1)? & 0x20 == 0 {
        return None;
    }
    let dlen = usize::from(*npdu.get(4)?);
    let apdu = npdu.get(5 + dlen + 1..)?;
    (apdu.first()? >> 4 == 0).then_some(apdu)
}

/// The peer: sends the I-Am to the CLI at `cli` until the first request
/// comes, then answers every request by `script`.
async fn router(script: Script, cli: SocketAddr) -> (JoinHandle<()>, u16) {
    let socket = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
    let port = socket.local_addr().unwrap().port();
    let asked = Arc::new(AtomicBool::new(false));
    let announce = {
        let (socket, asked) = (Arc::clone(&socket), Arc::clone(&asked));
        tokio::spawn(async move {
            while !asked.load(Ordering::Relaxed) {
                let _ = socket.send_to(&i_am(), cli).await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let task = tokio::spawn(async move {
        let _announce = announce;
        let mut buffer = [0u8; 1500];
        loop {
            // Windows reports an I-Am sent before the CLI bound its port
            // as a reset on the next receive: not the end of the peer.
            let Ok((length, from)) = socket.recv_from(&mut buffer).await else {
                tokio::task::yield_now().await;
                continue;
            };
            let Some(apdu) = routed_request(&buffer[..length]) else {
                continue;
            };
            asked.store(true, Ordering::Relaxed);
            // A ReadProperty-ACK echoing the reference, Present_Value 72.5.
            let (invoke, body) = (apdu[2], &apdu[4..]);
            let ack = relayed(
                &[
                    &[0x30, invoke, 0x0C],
                    body,
                    &[0x3E, 0x44],
                    &72.5f32.to_be_bytes(),
                    &[0x3F],
                ]
                .concat(),
            );
            if matches!(script, Script::RejectThenAnswer | Script::RejectOnly) {
                let _ = socket.send_to(&too_long(), from).await;
            }
            if matches!(script, Script::RejectThenAnswer) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if matches!(script, Script::Answer | Script::RejectThenAnswer) {
                let _ = socket.send_to(&ack, from).await;
            }
        }
    });
    (task, port)
}

/// Run the CLI on a port picked here, with `args` after the transport
/// flags, against a peer answering by `script`; the peer's port is given
/// to `args`. A port taken between picking and binding is tried again.
async fn routed_run(script: Script, args: impl Fn(u16) -> Vec<String>) -> Output {
    for _ in 0..3 {
        let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (peer, router_port) = router(script, (Ipv4Addr::LOCALHOST, port).into()).await;
        let mut command = vec![
            "-i".to_owned(),
            "127.0.0.1".to_owned(),
            "-p".to_owned(),
            port.to_string(),
            "-b".to_owned(),
            "127.0.0.1".to_owned(),
            "-t".to_owned(),
            "300".to_owned(),
        ];
        command.extend(args(router_port));
        let output = run(command).await;
        peer.abort();
        if !String::from_utf8_lossy(&output.stderr).contains("in use") {
            return output;
        }
    }
    panic!("no free port for the CLI in three tries");
}

fn read(extra: &[&str]) -> Vec<String> {
    words(extra, &["read", "5:0x0a", "ai:1", "pv"])
}

fn words(extra: &[&str], command: &[&str]) -> Vec<String> {
    extra
        .iter()
        .chain(command)
        .map(|text| (*text).to_owned())
        .collect()
}

/// The JSON line on stderr that holds the routed-path summary.
fn summary(output: &Output) -> Value {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr
        .lines()
        .find(|line| line.starts_with("{\"routed_paths\""))
        .unwrap_or_else(|| panic!("no summary line: {stderr}"));
    serde_json::from_str(line).unwrap()
}

/// A too-long reject for a 20-octet read is not believed: the read
/// completes, its JSON alone is on stdout, and the client's warning and the
/// summary go to stderr.
#[tokio::test]
async fn a_warning_during_a_json_command_leaves_stdout_parseable() {
    let output = routed_run(Script::RejectThenAnswer, |_| read(&["--json"])).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("stdout is not JSON ({error}): {:?}", output.stdout));
    assert!(
        result["value"]
            .as_str()
            .is_some_and(|value| value.contains("72.5")),
        "{result}"
    );
    assert!(
        stderr.contains("Ignored a reason-4 (too long) network reject"),
        "{stderr}"
    );
    assert!(
        stderr.contains("the path's limit is set by hand"),
        "{stderr}"
    );
    let summary = summary(&output);
    assert_eq!(summary["below_minimum_reject_drops"], 1, "{summary}");
    assert_eq!(summary["too_long_reject_drops"], 0, "{summary}");
    assert_eq!(summary["hint"], Value::Null, "{summary}");
    let [path] = summary["routed_paths"].as_array().unwrap().as_slice() else {
        panic!("one path: {summary}");
    };
    assert_eq!(path["network"], 5);
    assert_eq!(path["source"], "default");
    assert_eq!(path["max_npdu"], 228);
    assert_eq!(path["below_minimum_reject_seen"], true);
    assert!(path["router"].as_str().unwrap().starts_with("127.0.0.1:"));
}

/// A command whose every try draws only a too-long reject times out, and
/// the summary's hint names the flag for that path: `read`, whose error is
/// the client's, and `alarms`, whose error is an `OpsError`.
#[tokio::test]
async fn a_timeout_after_a_small_request_was_refused_names_the_flag() {
    for command in [&["read", "5:0x0a", "ai:1", "pv"][..], &["alarms", "5:0x0a"]] {
        timeout_names_the_flag(command).await;
    }
}

async fn timeout_names_the_flag(command: &[&str]) {
    let output = routed_run(Script::RejectOnly, |_| words(&["--json"], command)).await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty(), "{stderr}");
    let summary = summary(&output);
    assert!(summary["below_minimum_reject_drops"].as_u64().unwrap() >= 1);
    let router = summary["routed_paths"][0]["router"]
        .as_str()
        .unwrap()
        .to_owned();
    let hint = summary["hint"]
        .as_str()
        .unwrap_or_else(|| panic!("no hint: {stderr}"));
    assert!(
        hint.contains(&format!("--routed-path-limit {router}/5=SIZE")),
        "{command:?}: {hint}"
    );
    if command[0] == "read" {
        assert!(
            stderr
                .lines()
                .any(|line| line.starts_with("Error:") && line.contains("tsm-timeout")),
            "{stderr}"
        );
    }
}

/// A limit set by flag is used and listed: with `-v` the summary shows it
/// as configured; without, nothing unasked happened and no summary prints.
#[tokio::test]
async fn a_flag_limit_is_listed_with_v_only() {
    let flag = |router_port: u16| format!("127.0.0.1:{router_port}/5=480");
    let output = routed_run(Script::Answer, |port| {
        let flag = flag(port);
        let mut args = vec!["-v".to_owned(), "--format".to_owned(), "table".to_owned()];
        args.extend(["--routed-path-limit".to_owned(), flag]);
        args.extend(read(&[]));
        args
    })
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("Routed paths:"), "{stderr}");
    assert!(stderr.contains("configured"), "{stderr}");
    assert!(stderr.contains("480"), "{stderr}");

    let output = routed_run(Script::Answer, |port| {
        let mut args = vec![
            "--json".to_owned(),
            "--routed-path-limit".to_owned(),
            flag(port),
        ];
        args.extend(read(&[]));
        args
    })
    .await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("routed_paths"), "{stderr}");
    serde_json::from_slice::<Value>(&output.stdout).unwrap();
}

/// A flag the client refuses stops the command before it sends anything:
/// a limit too small for any request, a router named as a device or a
/// routed station, and one path named twice, however its router is written.
#[tokio::test]
async fn a_flag_the_client_refuses_stops_the_command() {
    let station = "names a device or a station behind a router";
    for (flags, why) in [
        (&["127.0.0.1:47808/5=65"][..], "give at least 66"),
        (&["1234/5=480"], station),
        (&["5:0x0a/5=480"], station),
        (
            &["127.0.0.1:47808/5=480", "127.0.0.1/5=300"],
            "names network 5 through router 127.0.0.1:47808 twice \
             (127.0.0.1:47808/5 and 127.0.0.1/5)",
        ),
    ] {
        let mut args = words(
            &["-i", "127.0.0.1", "-p", "0", "-b", "127.0.0.1", "--json"],
            &[],
        );
        for flag in flags {
            args.extend(["--routed-path-limit".to_owned(), (*flag).to_owned()]);
        }
        args.extend(read(&[]));
        let output = run(args).await;
        support::failure(&output, why);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--routed-path-limit "),
            "{flags:?}"
        );
    }
}
