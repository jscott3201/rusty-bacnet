//! The monitor on a pseudo-terminal pair: the native serial backend, end to
//! end, with nothing written back to the line.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio_serial::{SerialPort as _, SerialStream};

use super::{ReceiveOnlyPort, SerialConfig};
use crate::mstp::codec::FrameType;
use crate::mstp::monitor::script::{ScriptedRing, analyse};
use crate::mstp::monitor::{MaxMaster, MonitorConfig, MstpMonitor};

const BAUD: u32 = 38_400;
/// Generous: a loaded CI host can be slow to pass octets through a PTY.
const DEADLINE: Duration = Duration::from_secs(30);

#[tokio::test]
async fn monitor_over_a_pseudo_terminal_counts_the_ring_and_writes_nothing() {
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    ring.rotate(30, |line, holder| {
        if holder == 1 {
            line.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &[1, 4, 9])
                .frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &[1, 0, 9]);
        }
    });
    let expected = analyse(BAUD, ring.line.chunks());

    let (mut line, slave) = SerialStream::pair().unwrap();
    let serial_config = SerialConfig {
        port_name: slave.name().unwrap(),
        baud_rate: BAUD,
    };
    drop(slave);
    // A pseudo-terminal isn't a timed line, and a loaded host can stall
    // between two parts of a frame: never give one up.
    let config = MonitorConfig {
        frame_abort: Duration::from_secs(3600),
        ..MonitorConfig::new(BAUD)
    };
    let mut monitor =
        MstpMonitor::start(ReceiveOnlyPort::open(&serial_config, None).unwrap(), config)
            .await
            .unwrap();
    let mut statistics = monitor.statistics();
    // The octets go in as fast as the pseudo-terminal takes them: no count
    // depends on timing.
    let mut sent = 0u64;
    for (_, octets) in ring.line.chunks() {
        line.write_all(octets).await.unwrap();
        sent += octets.len() as u64;
    }
    let frames = expected.frames.total();
    tokio::time::timeout(
        DEADLINE,
        statistics.wait_for(|stats| stats.frames.total() == frames),
    )
    .await
    .expect("the monitor saw every frame")
    .unwrap();
    // Not one octet came back from the monitor's end of the pair. Asked
    // while the monitor still holds its end: once that end closes, Linux
    // hangs the pair up and every ioctl on this end fails with EIO, so the
    // count can't be read after stop there.
    assert_eq!(line.bytes_to_read().unwrap(), 0);
    let last = monitor.stop().await.unwrap();

    assert_eq!(last.present_stations().collect::<Vec<_>>(), [1, 2, 5]);
    assert_eq!(last.station(5).unwrap().tokens_received, 0);
    assert_eq!(
        last.station(2).unwrap().max_master,
        Some(MaxMaster::Exact(3))
    );
    assert_eq!(last.frames, expected.frames);
    assert_eq!(last.errors, expected.errors);
    assert_eq!(last.lost_tokens, 0);
    assert_eq!(last.octets, sent);
    // Nor while it stopped, where the platform still answers.
    #[cfg(not(target_os = "linux"))]
    assert_eq!(line.bytes_to_read().unwrap(), 0);
}
