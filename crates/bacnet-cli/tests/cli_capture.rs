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
    let mut out = header();
    for (i, port) in ports.iter().enumerate() {
        out.extend(record(i, *port));
    }
    if cut {
        let whole = record(ports.len(), 47808);
        out.extend_from_slice(&whole[..16 + (whole.len() - 16) / 2]);
    }
    let path = dir.join(name);
    std::fs::write(&path, out).unwrap();
    path
}

/// A pcap file's header: Ethernet frames, up to 65,535 bytes each.
fn header() -> Vec<u8> {
    [0xA1B2_C3D4_u32, 0x0004_0002, 0, 0, 65_535, 1]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect()
}

/// Record `i` of a pcap file: a Who-Is on `port`, `i` seconds after
/// 12:34:56.789.
fn record(i: usize, port: u16) -> Vec<u8> {
    let packet = who_is(port);
    let length = packet.len() as u32;
    let mut out: Vec<u8> = [45_296 + i as u32, 789_000, length, length]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    out.extend(packet);
    out
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

/// `capture --read` from a FIFO the test feeds a packet at a time, so the
/// capture is still running when the test stops it: Unix only.
#[cfg(unix)]
mod running {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::process::{ChildStdout, Stdio};

    use super::support::stream::{KillOnDrop, Lines};
    use super::{header, record, support};

    /// A capture in `format` reading a FIFO in `dir`, with the file header
    /// and its first packet written: the child, its stdout and the FIFO's
    /// writing end.
    async fn capture(dir: &Path, format: &str) -> (KillOnDrop, ChildStdout, File) {
        let path = dir.join("live.pcap");
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let mut child = support::command()
            .args([
                "capture",
                "--read",
                path.to_str().unwrap(),
                "--format",
                format,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let child = KillOnDrop(child);
        // Opening the writing end waits for the capture to open the other.
        let mut fifo = support::bounded(tokio::task::spawn_blocking(move || {
            File::options().write(true).open(&path)
        }))
        .await
        .unwrap()
        .unwrap();
        fifo.write_all(&header()).unwrap();
        fifo.write_all(&record(0, 47808)).unwrap();
        (child, stdout, fifo)
    }

    /// SIGTERM stops a running capture as Ctrl-C does: it reads on until it
    /// sees the signal's flag, then prints its summary and exits 0, every
    /// line JSON. Killed by the signal instead, it would print neither.
    #[tokio::test]
    async fn sigterm_stops_a_capture_with_its_summary() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, stdout, mut fifo) = capture(dir.path(), "json").await;
        let mut lines = Lines::read(stdout);
        assert_eq!(lines.next().await["type"], "packet");
        let killed = std::process::Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        // A packet at a time until the capture stops reading.
        let mut printed = 1;
        for i in 1..=50 {
            if fifo.write_all(&record(i, 47808)).is_err() {
                break;
            }
            match lines.next_or_end().await {
                Some(line) => {
                    assert_eq!(line["type"], "packet", "{line}");
                    printed += 1;
                }
                None => break,
            }
        }
        let status = child.exited().await;
        assert!(status.success(), "{status:?}");
        assert_eq!(child.stderr(), format!("\nCaptured {printed} packets\n"));
    }

    /// A reader that goes away stops the capture quietly in either format:
    /// the next packet finds stdout closed, the summary counts it, and the
    /// status is 0 (#2783).
    #[tokio::test]
    async fn a_closed_stdout_stops_a_capture_quietly() {
        for format in ["json", "table"] {
            let dir = tempfile::tempdir().unwrap();
            let (mut child, stdout, mut fifo) = capture(dir.path(), format).await;
            let line = support::bounded(tokio::task::spawn_blocking(move || {
                let mut line = String::new();
                BufReader::new(stdout).read_line(&mut line).unwrap();
                line
            }))
            .await
            .unwrap();
            assert!(line.contains("WHO_IS"), "{format}: {line}");
            fifo.write_all(&record(1, 47808)).unwrap();
            let status = child.exited().await;
            let stderr = child.stderr();
            assert_eq!(status.code(), Some(0), "{format}: {stderr}");
            assert_eq!(stderr, "\nCaptured 2 packets\n", "{format}");
        }
    }
}
