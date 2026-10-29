//! Final capture accounting must not obstruct original input retirement.
use super::mstp_scan::Run;
use super::mstp_scan_lifecycle::{inner, line_step, opened};
use super::site::bounded;
use super::*;
use crate::tui::monitor::scan::{RowStatus, ScanPhase};
use crate::tui::monitor::scan_fixture::{Fault, ScanLine, Step};
use std::sync::atomic::Ordering;

#[tokio::test(start_paused = true)]
async fn mstp_scan_cancel_closes_original_before_final_capture_delivery() {
    use crate::tui::capture::{CaptureCommand, CaptureEvent};
    for success in [true, false] {
        let line = ScanLine::new(&[38400]);
        line.fault(0, Fault::Hold);
        let mut run = Run::with_capacity(line.clone(), 1);
        run.ready().await;
        run.send(vec![Command::Capture(CaptureCommand::Start)])
            .await;
        run.pump(|r| !r.app.capture.sources.is_empty()).await;
        line.feed_valid().await;
        drain_count(&line, 2).await;
        // A real frame batch fills the only event slot, not a guessed sleep.
        tokio::time::advance(Duration::from_millis(250)).await;
        bounded(async {
            while run.events.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert_eq!(run.events.len(), 1);
        line.feed_valid().await;
        drain_count(&line, 3).await;
        run.command("mstp scan 1").await;
        run.command("mstp scan cancel").await;
        line_step(&line, Step::Closing(0)).await;
        assert_eq!(run.events.len(), 1, "close must not consume UI capacity");
        assert_eq!(opened(&run), 1);
        assert_eq!(line.held(), Some(0));
        assert_eq!(line.participants.load(Ordering::SeqCst), 0);
        line.release(0, success);
        if success {
            run.pump(|r| {
                r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live
            })
            .await;
            assert!(run.scan().cancelled);
            assert!(
                run.scan()
                    .rows
                    .iter()
                    .all(|r| r.status == RowStatus::Pending)
            );
            assert_eq!(
                line.steps()
                    .iter()
                    .filter_map(|s| if let Step::Open(_, baud) = s {
                        Some(*baud)
                    } else {
                        None
                    })
                    .collect::<Vec<_>>(),
                [19200, 19200]
            );
        } else {
            run.pump(|r| r.app.quit).await;
            assert!(
                run.app
                    .exit_error
                    .as_ref()
                    .unwrap()
                    .contains("release unproven")
            );
            assert_eq!(opened(&run), 1);
            assert!(
                !run.seen
                    .iter()
                    .any(|e| matches!(e, WorkerEvent::Disconnected { .. }))
            );
        }
        let (mut delivered, mut lost, mut ended) = (0, 0, false);
        for event in &run.seen {
            match inner(event) {
                WorkerEvent::Capture(CaptureEvent::Frames(batch)) => {
                    assert!(!ended);
                    delivered += batch.frames.len();
                    lost += batch.lost;
                }
                WorkerEvent::Capture(CaptureEvent::Ended { .. }) => {
                    assert!(!ended);
                    ended = true;
                }
                _ => {}
            }
        }
        assert!(ended);
        assert_eq!(delivered as u64 + lost, 14);
        run.quit().await;
    }
}

async fn drain_count(line: &ScanLine, count: usize) {
    bounded(async {
        loop {
            let changed = line.changed.notified();
            if line
                .steps()
                .iter()
                .filter(|step| **step == Step::Drained(0))
                .count()
                >= count
            {
                return;
            }
            changed.await;
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_cancel_closes_original_when_capture_started_is_blocked() {
    use crate::tui::capture::{CaptureCommand, CaptureEvent};
    let line = ScanLine::new(&[38400]);
    line.fault(0, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    bounded(async {
        while run.events.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(run.events.len(), 1);
    run.send(vec![Command::Capture(CaptureCommand::Start)])
        .await;
    run.command("mstp scan 1").await;
    run.command("mstp scan cancel").await;
    line_step(&line, Step::Closing(0)).await;
    assert_eq!(run.events.len(), 1);
    assert_eq!(opened(&run), 1);
    line.release(0, true);
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    assert!(run.scan().cancelled);
    assert_eq!(opened(&run), 2);
    let capture_events = run
        .seen
        .iter()
        .filter_map(|e| {
            if let WorkerEvent::Capture(event) = inner(e) {
                Some(event)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        capture_events.first(),
        Some(CaptureEvent::Started { .. })
    ));
    assert!(matches!(
        capture_events.last(),
        Some(CaptureEvent::Ended { .. })
    ));
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_capture_final_delivery_resumes_without_manual_cancel() {
    use crate::tui::capture::CaptureCommand;
    let line = ScanLine::new(&[38400]);
    line.fault(0, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    run.send(vec![Command::Capture(CaptureCommand::Start)])
        .await;
    run.pump(|r| !r.app.capture.sources.is_empty()).await;
    line.feed_valid().await;
    drain_count(&line, 2).await;
    tokio::time::advance(Duration::from_millis(250)).await;
    bounded(async {
        while run.events.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    line.feed_valid().await;
    drain_count(&line, 3).await;
    run.command("mstp scan 1").await;
    line_step(&line, Step::Closing(0)).await;
    assert_eq!(run.events.len(), 1);
    assert_eq!(opened(&run), 1);
    line.release(0, true);
    run.finish().await;
    assert!(!run.scan().cancelled);
    assert_eq!(run.app.capture.ring.len(), 14);
    assert!(run.app.capture.sources.is_empty());
    assert_eq!(opened(&run), 8);
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_capture_boundary_disconnect_and_sink_loss_never_restore() {
    use crate::tui::capture::CaptureCommand;
    for sink_loss in [false, true] {
        let line = ScanLine::new(&[38400]);
        line.fault(0, Fault::Hold);
        let mut run = Run::with_capacity(line.clone(), 1);
        run.ready().await;
        run.send(vec![Command::Capture(CaptureCommand::Start)])
            .await;
        run.pump(|r| !r.app.capture.sources.is_empty()).await;
        line.feed_valid().await;
        drain_count(&line, 2).await;
        tokio::time::advance(Duration::from_millis(250)).await;
        bounded(async {
            while run.events.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        run.command("mstp scan 1").await;
        line_step(&line, Step::Closing(0)).await;
        if sink_loss {
            run.events.close();
        } else {
            run.send(vec![Command::Disconnect]).await;
        }
        assert_eq!(opened(&run), 1);
        assert_eq!(line.held(), Some(0));
        line.release(0, true);
        line_step(&line, Step::Closed(0)).await;
        if !sink_loss {
            run.pump(|r| r.app.link == crate::tui::app::Link::Down)
                .await;
        }
        assert_eq!(opened(&run), 1);
        assert!(line.held().is_none());
        run.quit().await;
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_capture_started_delivery_resumes_without_manual_cancel() {
    use crate::tui::capture::CaptureCommand;
    let line = ScanLine::new(&[38400]);
    line.fault(0, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    bounded(async {
        while run.events.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    run.send(vec![Command::Capture(CaptureCommand::Start)])
        .await;
    run.command("mstp scan 1").await;
    line_step(&line, Step::Closing(0)).await;
    assert_eq!(run.events.len(), 1);
    assert_eq!(opened(&run), 1);
    line.release(0, true);
    run.finish().await;
    assert!(!run.scan().cancelled);
    assert!(run.app.capture.sources.is_empty());
    assert_eq!(opened(&run), 8);
    run.quit().await;
}
