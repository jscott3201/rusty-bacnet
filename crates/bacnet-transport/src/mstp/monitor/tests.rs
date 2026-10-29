//! The monitor's task over an in-memory line, on paused Tokio time.

use std::time::Duration;

use bacnet_mstp_codec::FrameType;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::script::{LineScript, ScriptedRing, analyse};
use super::*;

const BAUD: u32 = 38_400;

/// Play `chunks` into `line`, each at its time from now.
async fn play(line: &mut DuplexStream, chunks: &[(Duration, Vec<u8>)]) {
    let start = tokio::time::Instant::now();
    for (at, octets) in chunks {
        tokio::time::sleep_until(start + *at).await;
        line.write_all(octets).await.unwrap();
    }
}

/// The test's end of the line must hold nothing from the monitor: once the
/// monitor has stopped and dropped its end, reading gives end of file at
/// once, with no octets.
async fn assert_nothing_written(mut line: DuplexStream) {
    let mut written = Vec::new();
    line.read_to_end(&mut written).await.unwrap();
    assert!(written.is_empty(), "the monitor wrote {written:02X?}");
}

fn ring(holds: usize) -> ScriptedRing {
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    ring.rotate(holds, |line, holder| {
        if holder == 1 {
            line.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &[1, 4, 9])
                .frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &[1, 0, 9]);
        }
    });
    ring
}

#[tokio::test(start_paused = true)]
async fn monitor_publishes_what_the_engine_computes_and_writes_nothing() {
    let mut ring = ring(30);
    ring.lose_token(Duration::from_millis(650));
    ring.rotate(10, |_, _| {});
    let expected = analyse(BAUD, ring.line.chunks());

    let (mut line, monitor_end) = tokio::io::duplex(4096);
    let mut monitor = MstpMonitor::start(
        DropOnClose::new(monitor_end),
        MonitorConfig::new(BAUD).with_timestamp_resolution(Duration::ZERO),
    )
    .await
    .unwrap();
    let mut statistics = monitor.statistics();
    play(&mut line, ring.line.chunks()).await;
    let frames = expected.frames.total();
    statistics
        .wait_for(|stats| stats.frames.total() == frames)
        .await
        .unwrap();
    let last = monitor.stop().await.unwrap();

    assert_eq!(last.present_stations().collect::<Vec<_>>(), [1, 2, 5]);
    assert_eq!(last.station(5).unwrap().tokens_received, 0);
    assert_eq!(
        last.station(2).unwrap().max_master,
        Some(MaxMaster::Exact(3))
    );
    assert_eq!(last.lost_tokens, 1);
    // The task's statistics are the engine's. Tokio's timer wakes the
    // player on whole milliseconds, so the times agree to within one.
    assert_eq!(last.frames, expected.frames);
    assert_eq!(last.errors, expected.errors);
    assert_eq!(last.octets, expected.octets);
    assert_eq!(last.timing, expected.timing);
    for (address, station) in &expected.stations {
        let seen = last.station(*address).unwrap();
        assert_eq!(seen.frames_sent, station.frames_sent, "station {address}");
        assert_eq!(seen.frames_received, station.frames_received);
        assert_eq!(seen.tokens_received, station.tokens_received);
        assert_eq!(seen.max_master, station.max_master);
        assert_eq!(seen.token_rotation.count, station.token_rotation.count);
    }
    let silence = last.last_lost_token.unwrap().silence;
    let exact = expected.last_lost_token.unwrap().silence;
    assert!(
        silence.abs_diff(exact) <= Duration::from_millis(1),
        "{silence:?}"
    );
    assert_nothing_written(line).await;
}

#[tokio::test(start_paused = true)]
async fn the_live_stream_carries_frames_and_errors_in_order() {
    let mut script = LineScript::new(BAUD);
    let npdu: Vec<u8> = (0..520u16).map(|value| value as u8).collect();
    script
        .frame(FrameType::TOKEN, 2, 1, &[])
        .damaged(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            255,
            2,
            &[1, 0, 7],
            script::Damage::HeaderCrc,
        )
        .frame(
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            255,
            2,
            &npdu,
        );

    let (mut line, monitor_end) = tokio::io::duplex(4096);
    let mut monitor = MstpMonitor::start(
        super::DropOnClose::new(monitor_end),
        MonitorConfig::new(BAUD),
    )
    .await
    .unwrap();
    let mut frames = monitor.frames();
    play(&mut line, script.chunks()).await;

    let token = frames.recv().await.unwrap();
    assert!(
        matches!(token.event, LineEvent::Frame { header, wire_length: 8 }
        if header.frame_type == FrameType::TOKEN && header.source == 1)
    );
    assert!(token.data.is_empty() && !token.is_decode_only());
    let error = frames.recv().await.unwrap();
    assert!(matches!(
        error.event,
        LineEvent::Error {
            kind: LineErrorKind::HeaderCrc,
            destination: None,
            ..
        }
    ));
    // The rest of the bad header is noise.
    let noise = frames.recv().await.unwrap();
    assert!(matches!(noise.event, LineEvent::Noise { .. }));
    let extended = frames.recv().await.unwrap();
    assert!(extended.is_decode_only());
    assert_eq!(&extended.data[..], &npdu[..]);
    assert!(token.at < error.at && error.at <= noise.at && noise.at < extended.at);
    assert_eq!(frames.dropped(), 0);

    monitor.stop().await.unwrap();
    assert_nothing_written(line).await;
    // The monitor is gone: the stream ends.
    assert!(frames.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_slow_subscriber_loses_the_oldest_records_and_never_holds_up_the_monitor() {
    let mut script = LineScript::new(BAUD);
    for round in 0..20u8 {
        script.frame(FrameType::TOKEN, round % 2 + 1, 2 - round % 2, &[]);
    }
    let config = MonitorConfig {
        record_capacity: 4,
        ..MonitorConfig::new(BAUD)
    };
    let (mut line, monitor_end) = tokio::io::duplex(4096);
    let mut monitor = MstpMonitor::start(super::DropOnClose::new(monitor_end), config)
        .await
        .unwrap();
    let mut frames = monitor.frames();
    let mut statistics = monitor.statistics();
    play(&mut line, script.chunks()).await;
    // Nobody reads the stream, and the statistics still see every frame.
    statistics
        .wait_for(|stats| stats.token_passes == 20)
        .await
        .unwrap();
    let mut kept = 0;
    while frames.try_recv().is_some() {
        kept += 1;
    }
    assert_eq!(kept, 4);
    assert_eq!(frames.dropped(), 16);
    monitor.stop().await.unwrap();
    assert_nothing_written(line).await;
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_stream_ends_the_monitor() {
    let (mut line, monitor_end) = tokio::io::duplex(64);
    let mut monitor = MstpMonitor::start(
        super::DropOnClose::new(monitor_end),
        MonitorConfig::new(BAUD),
    )
    .await
    .unwrap();
    let mut script = LineScript::new(BAUD);
    script.frame(FrameType::TOKEN, 2, 1, &[]);
    play(&mut line, script.chunks()).await;
    line.shutdown().await.unwrap();
    let mut statistics = monitor.statistics();
    statistics
        .wait_for(|stats| stats.token_passes == 1)
        .await
        .unwrap();
    while !monitor.is_finished() {
        tokio::task::yield_now().await;
    }
    let last = monitor.stop().await.unwrap();
    assert_eq!(last.token_passes, 1);
    assert_eq!(last.timing.reads, 1);
    assert_eq!(last.octets, 8);
}

/// A reader that fails.
struct Broken;

impl AsyncRead for Broken {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("unplugged")))
    }
}

#[tokio::test]
async fn a_read_error_ends_the_monitor_with_that_error() {
    let mut monitor = MstpMonitor::start(super::DropOnClose::new(Broken), MonitorConfig::new(BAUD))
        .await
        .unwrap();
    while !monitor.is_finished() {
        tokio::task::yield_now().await;
    }
    let error = monitor.stop().await.unwrap_err();
    assert!(matches!(error, Error::Transport(ref io) if io.to_string() == "unplugged"));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_monitor_stops_its_task() {
    let (line, monitor_end) = tokio::io::duplex(64);
    let monitor = MstpMonitor::start(
        super::DropOnClose::new(monitor_end),
        MonitorConfig::new(BAUD),
    )
    .await
    .unwrap();
    let mut frames = monitor.frames();
    drop(monitor);
    // The task ends, dropping its end of the line and the stream's sender.
    assert!(frames.recv().await.is_none());
    assert_nothing_written(line).await;
}

#[test]
fn settings_are_checked() {
    assert!(MonitorConfig::new(BAUD).validate().is_ok());
    assert!(MonitorConfig::new(0).validate().is_err());
    let no_abort = MonitorConfig {
        frame_abort: Duration::ZERO,
        ..MonitorConfig::new(BAUD)
    };
    assert!(no_abort.validate().is_err());
    let no_records = MonitorConfig {
        record_capacity: 0,
        ..MonitorConfig::new(BAUD)
    };
    assert!(no_records.validate().is_err());
    assert_eq!(
        MonitorConfig::new(BAUD).timestamp_resolution,
        DEFAULT_TIMESTAMP_RESOLUTION
    );
}
