//! Capture retirement accounting and isolation from scan candidate counters.
use super::mstp_scan::Run;
use super::site::bounded;
use super::*;
use crate::tui::capture::worker::{Source, pump};
use crate::tui::capture::{CaptureCommand, CaptureEvent};
use crate::tui::monitor::scan_fixture::ScanLine;
use crate::tui::worker::EventSink;
use std::sync::{Arc, atomic::AtomicU64};
use tokio::sync::mpsc;

#[tokio::test(start_paused = true)]
async fn capture_retirement_flushes_final_batch_and_reports_ended() {
    let (tx, rx) = mpsc::channel(1);
    let source = Source::Test {
        rx,
        lost: Arc::new(AtomicU64::new(3)),
    };
    let (sink, mut events) = EventSink::channel(8);
    let (commands, mut command_rx) = mpsc::channel::<CaptureCommand>(1);
    let task = tokio::spawn(async move {
        pump(
            source,
            &sink,
            Some(&mut command_rx),
            "retired monitor".into(),
        )
        .await;
    });
    for i in 0..3 {
        tx.send(super::capture::raw_bip(i, false, vec![0x81, 0x0b, 0, 4]))
            .await
            .unwrap();
    }
    // A slot is available only after the final queued frame was consumed.
    drop(tx.reserve().await.unwrap());
    drop(commands);
    bounded(task).await.unwrap();
    let (mut frames, mut lost, mut ended) = (0, 0, false);
    while let Some(event) = events.recv().await {
        match event {
            WorkerEvent::Capture(CaptureEvent::Frames(batch)) => {
                frames += batch.frames.len();
                lost += batch.lost;
            }
            WorkerEvent::Capture(CaptureEvent::Ended { what, why }) => {
                assert_eq!(what, "retired monitor");
                assert!(why.contains("retired"));
                ended = true;
            }
            _ => panic!("unexpected event"),
        }
    }
    assert_eq!((frames, lost, ended), (3, 3, true));
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_preserves_historical_capture_and_requires_explicit_restart() {
    let line = ScanLine::new(&[38400]);
    let mut run = Run::new(line.clone());
    run.ready().await;
    let mut out = Vec::new();
    run.app.show_capture(&mut out);
    run.send(out).await;
    run.pump(|r| !r.app.capture.sources.is_empty()).await;
    line.feed_valid().await;
    tokio::time::advance(Duration::from_millis(250)).await;
    run.pump(|r| r.app.capture.ring.len() > 0).await;
    run.command("mstp scan 1").await;
    run.finish().await;
    let historical = run.app.capture.ring.len();
    assert!(historical > 0);
    assert!(run.app.capture.sources.is_empty());
    assert!(run.app.capture.ended.as_ref().unwrap().contains("retired"));
    // The scan's many decoder frames never entered the historical ring.
    assert_eq!(historical, 7);
    let mut out = Vec::new();
    run.app.show_capture(&mut out);
    assert_eq!(out, vec![Command::Capture(CaptureCommand::Start)]);
    run.send(out).await;
    run.pump(|r| !r.app.capture.sources.is_empty()).await;
    line.feed_valid().await;
    tokio::time::advance(Duration::from_millis(250)).await;
    run.pump(|r| r.app.capture.ring.len() > historical).await;
    run.quit().await;
}

async fn unread_monitor_retirement(capacity: usize) {
    use crate::tui::monitor::scan_fixture::Step;
    use bacnet_transport::mstp::monitor::{MonitorConfig, MstpMonitor};
    let line = ScanLine::new(&[19200]);
    let mut config = MonitorConfig::new(19200);
    config.record_capacity = capacity;
    let mut monitor = MstpMonitor::start(line.monitor(19200).unwrap(), config)
        .await
        .unwrap();
    let source = Source::Monitor {
        frames: monitor.frames(),
        epoch: std::time::SystemTime::now(),
    };
    let mut stats = monitor.statistics();
    bounded(async {
        while stats.borrow().frames.total() < 7 {
            stats.changed().await.unwrap();
        }
    })
    .await;
    assert_eq!(monitor.stop().await.unwrap().frames.total(), 7);
    assert!(line.steps().contains(&Step::Closed(0)));
    // The producer is quiesced before the pump's first poll. Its finite
    // accepted tail includes any broadcast lag not observed by recv yet.
    let (commands, mut command_rx) = mpsc::channel::<CaptureCommand>(1);
    drop(commands);
    let (sink, mut events) = EventSink::channel(8);
    pump(
        source,
        &sink,
        Some(&mut command_rx),
        "retired monitor".into(),
    )
    .await;
    drop(sink);
    let (mut delivered, mut lost, mut ended) = (0, 0, false);
    while let Some(event) = events.recv().await {
        match event {
            WorkerEvent::Capture(CaptureEvent::Frames(batch)) => {
                assert!(!ended, "final accounting precedes Ended");
                delivered += batch.frames.len();
                lost += batch.lost;
            }
            WorkerEvent::Capture(CaptureEvent::Ended { .. }) => {
                assert!(!ended);
                ended = true;
            }
            _ => panic!("unexpected event"),
        }
    }
    assert!(ended);
    assert_eq!(
        delivered as u64 + lost,
        7,
        "accepted records must be retained or counted"
    );
    assert_eq!(
        (delivered, lost),
        if capacity == 2 { (2, 5) } else { (7, 0) }
    );
    for (_, mut peer) in line.take_peers() {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        bounded(peer.read_to_end(&mut bytes)).await.unwrap();
        assert!(bytes.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn capture_retirement_accounts_unread_monitor_records() {
    unread_monitor_retirement(16).await;
}
#[tokio::test(start_paused = true)]
async fn capture_retirement_accounts_unobserved_monitor_lag() {
    unread_monitor_retirement(2).await;
}
