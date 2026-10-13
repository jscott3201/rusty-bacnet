//! Host timing: when a frame counts as ended on a port that can't drain
//! (#1654).

use bacnet_mstp_codec::FrameType;
use tokio::time::{Duration, Instant};

use super::test_support::{config, Bus, DrainMode, Port, Sent};
use super::{MstpConfig, USB_TRANSMIT_LATENCY_PAD};
use crate::port::TransportPort;

/// Slack for Tokio's millisecond timer wheel.
const TICK: Duration = Duration::from_millis(1);

/// Station 3 in a ring with station 4, holding two queued data frames and
/// the token, on a port that drains as `drain` says, with `pad` as its
/// transmit-latency pad. Returns the station and when the token came.
async fn sending_two_frames(drain: DrainMode, pad: Duration) -> (Bus, Instant) {
    let port = Port {
        drain,
        ..Port::PLAIN
    };
    let config = MstpConfig {
        max_info_frames: 2,
        transmit_latency_pad: pad,
        ..config(3, 4)
    };
    let mut bus = Bus::start_with(config, port).await;
    let token = bus.join(4).await;
    bus.transport.send_unicast(&[1, 0, 1], &[4]).await.unwrap();
    bus.transport.send_unicast(&[1, 0, 2], &[4]).await.unwrap();
    // Not before the station can have finished its Token, padded or not:
    // what arrives sooner is read only after that, and stamped then.
    bus.until(token.end + pad + bus.turnaround).await;
    let heard = bus.send(FrameType::TOKEN, 3, 4, &[]);
    (bus, heard)
}

/// The station's next frame starts T_turnaround after its last one ended
/// (Clause 9.5.5.1), so its start shows when the station took the frame
/// before it to have ended: `after` past that frame's wire end.
fn assert_ended(bus: &Bus, before: &Sent, next: &Sent, after: Duration) {
    let earliest = before.end + after + bus.turnaround;
    assert!(
        next.start >= earliest,
        "{next:?} started {:?} early",
        earliest - next.start
    );
    assert!(next.start <= earliest + TICK, "{next:?}");
}

/// A port without a drain (Windows' `TokioSerialPort`, or a custom backend)
/// can't say when its last stop bit left, so each frame ends no sooner than
/// its write began plus its wire time plus the transmit-latency pad. With
/// no pad that is the wire time alone. A port whose drain works is not
/// padded, whatever the pad: its drain says when the frame ended.
#[tokio::test(start_paused = true)]
async fn a_port_without_drain_is_held_to_its_wire_time_plus_the_pad() {
    let pad = Duration::from_millis(6);
    for (drain, configured, padded) in [
        (DrainMode::Unsupported, Duration::ZERO, Duration::ZERO),
        (DrainMode::Unsupported, pad, pad),
        (DrainMode::Immediate, pad, Duration::ZERO),
    ] {
        let (mut bus, heard) = sending_two_frames(drain, configured).await;
        let first = bus.next().await;
        assert_eq!(first.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
        assert!(first.start >= heard + bus.turnaround, "{first:?}");
        assert!(first.start <= heard + bus.turnaround + TICK, "{first:?}");
        let second = bus.next().await;
        assert_eq!(
            second.frame_type,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
        );
        assert_ended(&bus, &first, &second, padded);
        let pass = bus.next().await;
        assert_eq!((pass.frame_type, pass.destination), (FrameType::TOKEN, 4));
        assert_ended(&bus, &second, &pass, padded);
        bus.until(pass.end + padded + TICK).await;
        let counts = bus.transport.diagnostics().snapshot();
        assert_eq!(counts.serial_write_errors, 0, "{drain:?}");
        // Joining's poll and Token, two data frames and a Token.
        assert_eq!(counts.mac.frames_transmitted, 5, "{drain:?}");
        bus.stop().await;
    }
}

/// The pad is the USB adapter's on Windows, where `TokioSerialPort` has no
/// drain, and nothing elsewhere, where it drains with `tcdrain`.
#[test]
fn the_default_pad_is_the_usb_adapters_on_windows_only() {
    let expected = if cfg!(windows) {
        USB_TRANSMIT_LATENCY_PAD
    } else {
        Duration::ZERO
    };
    assert_eq!(MstpConfig::default().transmit_latency_pad, expected);
    assert_eq!(USB_TRANSMIT_LATENCY_PAD, Duration::from_millis(4));
}

/// Octets heard while the pad runs reach the core only after it, so a pad
/// over T_usage_delay (15 ms) would make a token or poll that came during
/// it be answered late: it is refused, naming the field, and 15 ms is taken.
#[test]
fn a_pad_over_the_usage_delay_is_refused() {
    let at_most = MstpConfig {
        transmit_latency_pad: Duration::from_millis(15),
        ..config(3, 4)
    };
    assert!(at_most.validate().is_ok());
    let too_long = MstpConfig {
        transmit_latency_pad: Duration::from_millis(15) + Duration::from_micros(1),
        ..config(3, 4)
    };
    let error = too_long.validate().unwrap_err().to_string();
    assert!(error.contains("transmit_latency_pad"), "{error}");
    assert!(error.contains("T_usage_delay"), "{error}");
}

/// A padded frame must complete inside the core's transmit watchdog. The
/// watchdog is one second at most baud rates, which leaves the longest
/// standard frame room for any pad allowed; but from about 5,128 to 5,155
/// baud that frame leaves it less than the 4 ms Windows default. There the
/// watchdog is lengthened by the pad, rather than the configuration refused.
#[test]
fn the_watchdog_covers_a_padded_frame() {
    let pad = USB_TRANSMIT_LATENCY_PAD;
    let tight = MstpConfig {
        baud_rate: 5_140,
        transmit_latency_pad: pad,
        ..config(3, 4)
    };
    let unpadded = MstpConfig {
        transmit_latency_pad: Duration::ZERO,
        ..tight.clone()
    };
    let core = unpadded.core_config().unwrap();
    let floor = Duration::from(core.minimum_transmit_watchdog());
    assert_eq!(
        Duration::from(core.transmit_watchdog),
        Duration::from_secs(1)
    );
    assert!(floor + pad > Duration::from_secs(1), "{floor:?}");
    let core = tight.core_config().unwrap();
    assert_eq!(Duration::from(core.transmit_watchdog), floor + pad);
    assert!(core.validate().is_ok());
    // Where the watchdog already covers the padded frame it is kept.
    let roomy = MstpConfig {
        transmit_latency_pad: pad,
        ..config(3, 4)
    };
    let core = roomy.core_config().unwrap();
    assert_eq!(
        Duration::from(core.transmit_watchdog),
        Duration::from_secs(1)
    );
}

/// Only an unsupported drain means "no drain": a drain that fails with any
/// other error leaves the frame's completion unknown, so the frame is
/// aborted and counted as a write error, not taken as sent after the pad.
#[tokio::test(start_paused = true)]
async fn a_failed_drain_aborts_the_frame() {
    let port = Port {
        drain: DrainMode::Fails,
        ..Port::PLAIN
    };
    let config = MstpConfig {
        transmit_latency_pad: Duration::from_millis(6),
        ..config(3, 7)
    };
    let mut bus = Bus::start_with(config, port).await;
    // With no traffic the station generates a token: its first frame is a
    // Poll For Master, written but never confirmed as sent.
    let poll = bus.next().await;
    assert_eq!(poll.frame_type, FrameType::POLL_FOR_MASTER);
    bus.until(poll.end + Duration::from_millis(6) + TICK).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.serial_write_errors, 1);
    assert_eq!(counts.mac.frames_transmitted, 0);
    bus.stop().await;
}
