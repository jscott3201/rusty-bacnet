//! The one-shot `capture --read` (pcap feature): its output for a file is
//! pinned, and `-p` picks the UDP port its BPF filter keeps (#971).
#![cfg(feature = "pcap")]

#[allow(dead_code)]
mod support;
use support::run;

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
    let mut out = Vec::new();
    for word in [0xA1B2_C3D4_u32, 0x0004_0002, 0, 0, 65_535, 1] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for (i, port) in [47808u16, 47809].into_iter().enumerate() {
        let packet = who_is(port);
        for word in [
            45_296 + i as u32,
            789_000,
            packet.len() as u32,
            packet.len() as u32,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&packet);
    }
    let path = dir.join("two-ports.pcap");
    std::fs::write(&path, out).unwrap();
    path
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
            r#"{"timestamp":"12:34:56.789","src":"10.0.0.1:47808","dst":"10.0.0.255:47808","bvlc":"ORIGINAL_BROADCAST_NPDU","service":"WHO_IS"}"#,
            "\n"
        )
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "\nCaptured 1 packets\n"
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
