//! The one-shot `capture --read` (pcap feature): its output for a file is
//! pinned, `-p` picks the UDP port its BPF filter keeps (#971), and with
//! JSON it prints JSON Lines, a failed read ending with an error line
//! (#2783).
#![cfg(feature = "pcap")]

#[allow(dead_code)]
mod support;
use support::run;
use support::stream::{json_lines, kinds};

/// A Who-Is broadcast from 10.0.0.1:`port` to 10.0.0.255:`port`, in an
/// Ethernet frame.
fn who_is(port: u16) -> Vec<u8> {
    let payload = [0x81, 0x0B, 0x00, 0x08, 0x01, 0x00, 0x10, 0x08];
    let mut frame = vec![0xFF; 6];
    frame.extend_from_slice(&[0x02, 0x00, 10, 0, 0, 1, 0x08, 0x00]);
    let total = (20 + 8 + payload.len()) as u16;
    let mut ip = vec![0x45, 0];
    ip.extend_from_slice(&total.to_be_bytes());
    ip.extend_from_slice(&[0, 0, 0x40, 0, 64, 17, 0, 0, 10, 0, 0, 1, 10, 0, 0, 255]);
    let sum = ip
        .chunks(2)
        .map(|p| u32::from(u16::from_be_bytes([p[0], p[1]])))
        .sum::<u32>();
    let sum = !((sum & 0xFFFF) + (sum >> 16)) as u16;
    ip[10..12].copy_from_slice(&sum.to_be_bytes());
    frame.extend_from_slice(&ip);
    frame.extend_from_slice(&port.to_be_bytes());
    frame.extend_from_slice(&port.to_be_bytes());
    frame.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&payload);
    frame
}

/// A pcap file of Who-Is broadcasts on 47808 and 47809, one second apart.
fn file(dir: &std::path::Path) -> std::path::PathBuf {
    pcap(dir, "two-ports.pcap", &[47808, 47809], false)
}

/// A pcap file `name` of a Who-Is broadcast on each of `ports`, one second
/// apart from 12:34:56.789, and when `cut` a last record cut short: its
/// header claims more bytes than the file holds.
fn pcap(dir: &std::path::Path, name: &str, ports: &[u16], cut: bool) -> std::path::PathBuf {
    let mut out = Vec::new();
    for word in [0xA1B2_C3D4_u32, 0x0004_0002, 0, 0, 65_535, 1] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let last = cut.then_some(ports.len());
    for (i, port) in ports.iter().copied().chain(last.map(|_| 47808)).enumerate() {
        let packet = who_is(port);
        for word in [
            45_296 + i as u32,
            789_000,
            packet.len() as u32,
            packet.len() as u32,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        let kept = if Some(i) == last {
            packet.len() / 2
        } else {
            packet.len()
        };
        out.extend_from_slice(&packet[..kept]);
    }
    let path = dir.join(name);
    std::fs::write(&path, out).unwrap();
    path
}

/// With JSON, a line a packet, each one compact object of type `packet`,
/// and nothing else on stdout (#2783); `--count` stops after that many.
#[tokio::test]
async fn capture_json_is_one_value_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = pcap(dir.path(), "three.pcap", &[47808; 3], false);
    let path = path.to_str().unwrap();
    for (count, expected) in [(None, 3), (Some("2"), 2)] {
        let mut args = vec!["capture", "--read", path, "--json"];
        args.extend(count.map(|count| ["--count", count]).into_iter().flatten());
        let output = run(&args).await;
        assert!(output.status.success(), "{output:?}");
        let values = json_lines(&output.stdout);
        assert_eq!(kinds(&values), vec!["packet"; expected], "{args:?}");
        assert_eq!(values[1]["timestamp"], "12:34:57.789");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("\nCaptured {expected} packets\n")
        );
    }
}

/// A file cut short mid-record fails after the packets before it: with
/// JSON the stream ends with an error line carrying stderr's message; with
/// a table stdout has only the packets. Status 1 either way (#2783).
#[tokio::test]
async fn a_capture_that_fails_mid_stream_ends_with_an_error_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = pcap(dir.path(), "cut.pcap", &[47808, 47808], true);
    let path = path.to_str().unwrap();
    let output = run(["capture", "--read", path, "--json"]).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let values = json_lines(&output.stdout);
    assert_eq!(kinds(&values), ["packet", "packet", "error"]);
    let message = values[2]["message"].as_str().unwrap();
    assert!(message.contains("truncated"), "{message}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, format!("Error: {message}\n"));

    let output = run(["capture", "--read", path, "--format", "table"]).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.lines().all(|line| line.contains("WHO_IS")), "{text}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, format!("Error: {message}\n"));
}

#[tokio::test]
async fn capture_read_output_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    let output = run(["capture", "--read", path.to_str().unwrap()]).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            r#"{"type":"packet","timestamp":"12:34:56.789","src":"10.0.0.1:47808","dst":"10.0.0.255:47808","bvlc":"ORIGINAL_BROADCAST_NPDU","service":"WHO_IS"}"#,
            "\n"
        )
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "\nCaptured 1 packets\n"
    );
    // `--format table`: the same packet as a line, not JSON (#2778).
    let output = run([
        "capture",
        "--read",
        path.to_str().unwrap(),
        "--format",
        "table",
    ])
    .await;
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success(), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(
        text.starts_with("12:34:56.789") && text.contains("WHO_IS"),
        "{text}"
    );
}

#[tokio::test]
async fn capture_with_p_filters_on_that_port() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path());
    for args in [
        vec!["-p", "47809", "capture", "--read", path.to_str().unwrap()],
        vec!["capture", "--read", path.to_str().unwrap(), "-p", "47809"],
    ] {
        let output = run(&args).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains(r#""src":"10.0.0.1:47809""#), "{text}");
        assert!(text.contains(r#""timestamp":"12:34:57.789""#), "{text}");
    }
}
