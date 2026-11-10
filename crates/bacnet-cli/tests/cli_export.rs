//! `bacnet export 223-experimental` against the lab device (#2436): the
//! Turtle it prints or writes, what it costs on the wire, and the options
//! it refuses before sending anything. A relay between the CLI and the
//! device counts the confirmed requests.
#[allow(dead_code)]
mod support;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::Output;
use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bytes::Bytes;
use support::lab::{lab_database, lab_identity};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// The lab device, and its address.
async fn server() -> (BACnetServer<BipTransport>, SocketAddr) {
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(lab_database(1234))
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]])));
    (server, address)
}

/// A B/IP relay on loopback in front of `device` that counts the distinct
/// confirmed requests it sees, and passes on the first `answered` of them
/// (and their retries), dropping the rest, so the device goes silent after
/// them. Returns the CLI's target, the count and the relay's task.
async fn relay(
    device: SocketAddr,
    answered: usize,
) -> (String, Arc<Mutex<HashSet<u8>>>, JoinHandle<()>) {
    let front = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = front.local_addr().unwrap().to_string();
    let seen: Arc<Mutex<HashSet<u8>>> = Arc::default();
    let log = Arc::clone(&seen);
    let task = tokio::spawn(async move {
        let mut cli = None;
        let mut passed = HashSet::new();
        let (mut up, mut down) = ([0u8; 1500], [0u8; 1500]);
        loop {
            tokio::select! {
                received = front.recv_from(&mut up) => {
                    let Ok((length, from)) = received else { return };
                    cli = Some(from);
                    let frame = &up[..length];
                    if let Some(npdu) = frame.get(4..).and_then(|n| decode_npdu(Bytes::copy_from_slice(n)).ok())
                        && let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload)
                    {
                        log.lock().unwrap().insert(request.invoke_id);
                        if !passed.contains(&request.invoke_id) {
                            if passed.len() == answered {
                                continue;
                            }
                            passed.insert(request.invoke_id);
                        }
                    }
                    let _ = back.send_to(frame, device).await;
                }
                received = back.recv_from(&mut down) => {
                    let Ok((length, _)) = received else { return };
                    if let Some(cli) = cli {
                        let _ = front.send_to(&down[..length], cli).await;
                    }
                }
            }
        }
    });
    (target, seen, task)
}

async fn export(args: &[&str]) -> Output {
    export_within("2000", args).await
}

/// [`export`] with an APDU timeout of `timeout` ms.
async fn export_within(timeout: &str, args: &[&str]) -> Output {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--timeout",
        timeout,
        "export",
        "223-experimental",
    ];
    all.extend(args);
    support::run(all).await
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// By address: the device's eight analog, binary and multi-state objects
/// as 223 properties with their references, and nothing else; one request
/// to learn the instance, then the name, the Object_List and one an
/// object.
#[tokio::test]
async fn exports_the_lab_device_by_address() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let output = export(&[&target]).await;
    let text = stdout(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Wrote 8 points of device 1234 to stdout (0 skipped"),
        "the summary goes to stderr: {stderr}"
    );
    assert!(!text.contains("Wrote "), "stdout carries only the Turtle");
    assert!(
        text.starts_with("# EXPERIMENTAL: device 1234 as a proposed"),
        "{text}"
    );
    assert!(
        text.contains("# 8 objects exported, 0 skipped, 0 with an unmapped unit"),
        "{text}"
    );
    assert_eq!(
        text.matches(" a s223:BACnetExternalReference ;").count(),
        8,
        "{text}"
    );
    for (object, class) in [
        ("analog-input-1", "QuantifiableObservableProperty"),
        ("analog-output-1", "QuantifiableActuatableProperty"),
        ("analog-value-1", "QuantifiableActuatableProperty"),
        ("binary-input-1", "EnumeratedObservableProperty"),
        ("binary-value-1", "EnumeratedActuatableProperty"),
        ("multi-state-value-1", "EnumeratedActuatableProperty"),
    ] {
        let line = format!("d:{object}.present-value a s223:{class} ;");
        assert!(text.contains(&line), "{line}\n{text}");
    }
    assert!(
        text.contains("bacnet:device-name \"Lab device 1234\" ;"),
        "{text}"
    );
    assert!(
        text.contains("bacnet:object-name \"Zone Temp\" ;"),
        "{text}"
    );
    assert!(text.contains("qudt:hasUnit unit:DEG_F ;"), "{text}");
    assert!(
        text.contains("d:multi-state-value-1.states.3 a s223:Class"),
        "{text}"
    );
    assert!(!text.contains("trend-log-1") && !text.contains("notification-class"));
    assert!(!text.contains("priority-for-writing"));
    assert_eq!(
        seen.lock().unwrap().len(),
        8 + 3,
        "wildcard read, name, list, 8 objects"
    );
    relay.abort();
    server.stop().await.unwrap();
}

/// `-o` writes the same text to a new file, and refuses one that exists
/// before sending anything; `--types` and `--write-priority` narrow and
/// annotate the export; a bad option is refused before anything is sent.
#[tokio::test]
async fn options_and_the_output_file() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let files = support::Files::new();
    let path = files.0.join("lab.ttl");
    let path_text = path.to_str().unwrap();
    let narrow = [
        "--types",
        "bo,msv",
        "--write-priority",
        "8",
        "-o",
        path_text,
    ];
    let mut args = vec![target.as_str()];
    args.extend(narrow);
    let output = export(&args).await;
    assert!(stdout(&output).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Wrote 2 points of device 1234 to"),
        "{stderr}"
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# 2 objects exported"), "{text}");
    assert_eq!(
        text.matches("bacnet:priority-for-writing \"8\"^^xsd:nonNegativeInteger")
            .count(),
        2
    );
    assert_eq!(seen.lock().unwrap().len(), 2 + 3);

    let again = export(&args).await;
    support::failure(&again, "output file already exists");
    assert_eq!(seen.lock().unwrap().len(), 2 + 3, "refused before sending");

    for (bad, says) in [
        (
            vec!["--base-iri", "example.org/x/"],
            "is not an absolute IRI",
        ),
        (
            vec!["--base-iri", "http://example.org/x"],
            "must end in / or #",
        ),
        (
            vec!["--types", "trend-log"],
            "trend-log objects can't be exported",
        ),
        (vec!["--types", "nope"], "unknown object type"),
        (vec!["--write-priority", "17"], "17 is not in 1..=16"),
    ] {
        let mut args = vec![target.as_str()];
        args.extend(bad);
        support::failure(&export(&args).await, says);
    }
    assert_eq!(seen.lock().unwrap().len(), 2 + 3, "refused before sending");
    relay.abort();
    server.stop().await.unwrap();
}

/// A device that stops answering partway: the Turtle read so far is still
/// written, its header says it is incomplete, and the exit status is 1.
#[tokio::test]
async fn an_incomplete_export_exits_1() {
    let (mut server, device) = server().await;
    // The wildcard read, the name, the list and the first two objects.
    let (target, seen, relay) = relay(device, 5).await;
    let output = export_within("300", &[&target]).await;
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("# Incomplete: the reading stopped early ("),
        "{text}"
    );
    assert!(text.contains("# 2 objects exported"), "{text}");
    assert_eq!(text.matches(" a s223:BACnetExternalReference ;").count(), 2);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Wrote 2 points of device 1234 to stdout"),
        "{stderr}"
    );
    assert!(
        stderr.contains("the export of device 1234 stopped early"),
        "{stderr}"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        6,
        "nothing after the silent object"
    );
    relay.abort();
    server.stop().await.unwrap();
}

/// `--types` that the device has none of: a valid, empty model, exit 0,
/// with the header and stderr saying so.
#[tokio::test]
async fn no_object_of_the_selected_types() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let output = export(&[&target, "--types", "mso"]).await;
    let text = stdout(&output);
    assert!(
        text.contains("# The device has no object of the selected types"),
        "{text}"
    );
    assert!(text.contains("@prefix d: <http://example.org/bacnet/device-1234/> ."));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Device 1234 has no object of the selected types"),
        "{stderr}"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    relay.abort();
    server.stop().await.unwrap();
}
