//! Causal cancellation and release barriers through the coordinator.
use super::mstp_scan::{Run, snapshots};
use super::site::bounded;
use super::*;
use crate::tui::app::{Link, monitor::MonitorTab};
use crate::tui::monitor::scan::{RowStatus, ScanCommand, ScanPhase, ScanReport, Suggestion};
use crate::tui::monitor::scan_fixture::{Fault, ScanLine, Step};
use std::sync::atomic::Ordering;

fn join() -> Command {
    Command::JoinTrunk {
        settings: crate::mstp::MstpArgs {
            port: "scripted-rs485".into(),
            baud: 38400,
            mac: 3,
            max_master: 10,
            max_info_frames: 1,
        },
    }
}
pub(super) fn opened(run: &Run) -> usize {
    run.line
        .steps()
        .iter()
        .filter(|s| matches!(s, Step::Open(..)))
        .count()
}
pub(super) fn inner(event: &WorkerEvent) -> &WorkerEvent {
    if let WorkerEvent::Session { event, .. } = event {
        event
    } else {
        event
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_cancel_each_candidate_waits_for_close_and_restores_only_original() {
    for candidate in 0..6 {
        let index = candidate + 1;
        let line = ScanLine::new(&[38400]);
        line.fault(index, Fault::Hold);
        let mut run = Run::new(line.clone());
        run.ready().await;
        run.command("mstp scan 1").await;
        for previous in 0..candidate {
            run.measure(previous).await;
        }
        run.pump(|r| {
            r.scan().rows[candidate].status == RowStatus::Measuring
                && r.line.steps().contains(&Step::Drained(index))
        })
        .await;
        run.command("mstp scan cancel").await;
        run.pump(|r| r.line.steps().contains(&Step::Closing(index)))
            .await;
        assert_eq!(opened(&run), index + 1);
        assert_eq!(line.held(), Some(index));
        run.send(vec![join()]).await;
        run.pump(|r| r.seen.iter().any(|e| matches!(inner(e), WorkerEvent::Rejected { error } if error.contains("baud scan")))).await;
        assert_eq!(line.participants.load(Ordering::SeqCst), 0);
        assert_eq!(opened(&run), index + 1);
        line.release(index, true);
        run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
            .await;
        assert!(run.scan().cancelled);
        assert_eq!(run.scan().suggestion(), Suggestion::Partial);
        assert_eq!(opened(&run), index + 2);
        assert!(matches!(
            line.steps()
                .iter()
                .rev()
                .find(|s| matches!(s, Step::Open(..))),
            Some(Step::Open(_, 19200))
        ));
        if candidate == 2 {
            snapshots(&mut run, "mstp_scan_cancelled");
        }
        run.quit().await;
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_queued_duplicate_start_join_and_stale_cancel_preserve_reservation() {
    let line = ScanLine::new(&[38400]);
    line.fault(0, Fault::Hold);
    let mut run = Run::new(line.clone());
    run.ready().await;
    run.command("mstp scan 1").await;
    let id = run.scan().id;
    run.send(vec![
        Command::MonitorScan(ScanCommand::Start {
            id,
            window: Duration::from_secs(1),
        }),
        Command::MonitorScan(ScanCommand::Start {
            id: id + 1,
            window: Duration::from_secs(1),
        }),
        join(),
        Command::MonitorScan(ScanCommand::Cancel { id: id + 1 }),
    ])
    .await;
    run.pump(|r| r.line.steps().contains(&Step::Closing(0)) && r.seen.iter().any(|e| matches!(inner(e), WorkerEvent::MonitorScan(ScanReport::Rejected { id: rejected, cancel: true, .. }) if *rejected == id + 1))).await;
    assert_eq!(run.scan().id, id);
    assert!(run.scan().active());
    assert_eq!(opened(&run), 1);
    run.command("mstp scan cancel").await;
    line.release(0, true);
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    assert_eq!(opened(&run), 2);
    assert!(
        run.scan()
            .rows
            .iter()
            .all(|r| r.status == RowStatus::Pending)
    );
    // A fresh command gets a fresh identity and can scan after clean restoration.
    run.command("mstp scan 1").await;
    assert!(run.scan().id > id);
    run.command("mstp scan cancel").await;
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    assert_eq!(line.participants.load(Ordering::SeqCst), 0);
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_failed_release_is_fatal_and_never_opens_a_replacement() {
    for index in [0, 1, 3, 6] {
        let line = ScanLine::new(&[38400]);
        line.fault(index, Fault::Close);
        let mut run = Run::new(line.clone());
        run.ready().await;
        run.command("mstp scan 1").await;
        if index > 0 {
            for candidate in 0..index - 1 {
                run.measure(candidate).await;
            }
            run.pump(|r| r.scan().rows[index - 1].status == RowStatus::Measuring)
                .await;
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        run.pump(|r| r.app.quit).await;
        assert!(
            run.app
                .exit_error
                .as_ref()
                .unwrap()
                .contains("release unproven")
        );
        assert_eq!(opened(&run), index + 1);
        assert_eq!(line.held(), Some(index));
        assert!(
            !run.seen
                .iter()
                .any(|e| matches!(e, WorkerEvent::Disconnected { .. }))
        );
        assert_eq!(line.participants.load(Ordering::SeqCst), 0);
        run.quit().await;
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_read_failure_is_fatal_even_with_successful_close() {
    let line = ScanLine::new(&[38400]);
    line.fault(2, Fault::Read);
    let mut run = Run::new(line.clone());
    run.ready().await;
    run.command("mstp scan 1").await;
    run.measure(0).await;
    run.pump(|r| r.app.quit).await;
    assert!(
        run.app
            .exit_error
            .as_ref()
            .unwrap()
            .contains("injected scan read failure")
    );
    assert!(line.steps().contains(&Step::Closed(2)));
    assert_eq!(opened(&run), 3);
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_restoration_failure_leaves_stopped_evidence_and_refuses_restart() {
    let line = ScanLine::new(&[38400]);
    line.fault(7, Fault::Admission);
    let mut run = Run::new(line);
    run.ready().await;
    run.command("mstp scan 1").await;
    for candidate in 0..6 {
        run.measure(candidate).await;
    }
    run.pump(|r| matches!(r.scan().phase, ScanPhase::Stopped(_)))
        .await;
    assert!(matches!(run.app.link, Link::Failed(_)));
    assert!(!run.app.monitor.as_ref().unwrap().live);
    assert_eq!(run.scan().suggestion(), Suggestion::Rate(38400));
    snapshots(&mut run, "mstp_scan_stopped");
    run.command("mstp scan 1").await;
    assert_eq!(opened(&run), 8);
    assert!(run.app.monitor.as_ref().unwrap().wizard.is_none());
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_disconnect_and_quit_while_close_pending_never_restore() {
    for quit in [false, true] {
        let line = ScanLine::new(&[38400]);
        line.fault(1, Fault::Hold);
        let mut run = Run::new(line.clone());
        run.ready().await;
        run.command("mstp scan 1").await;
        run.pump(|r| r.scan().rows[0].status == RowStatus::Measuring)
            .await;
        if quit {
            run.commands = None;
        } else {
            run.send(vec![Command::Disconnect]).await;
        }
        run.pump(|r| r.line.steps().contains(&Step::Closing(1)))
            .await;
        assert_eq!(opened(&run), 2);
        line.release(1, true);
        if !quit {
            run.pump(|r| r.app.link == Link::Down).await;
        } else {
            bounded(run.task.take().unwrap()).await.unwrap();
        }
        assert_eq!(opened(&run), 2);
        assert!(line.held().is_none());
        if quit {
            for (_, mut peer) in line.take_peers() {
                use tokio::io::AsyncReadExt;
                let mut bytes = Vec::new();
                bounded(peer.read_to_end(&mut bytes)).await.unwrap();
                assert!(bytes.is_empty());
            }
        } else {
            run.quit().await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_stale_session_reports_and_outer_queue_refusal_cannot_change_current_state() {
    let mut run = Run::new(ScanLine::new(&[38400]));
    run.ready().await;
    let out = {
        press(&mut run.app, KeyCode::Char(':'));
        type_text(&mut run.app, "mstp scan 1");
        press(&mut run.app, KeyCode::Enter)
    };
    let request = out[0].clone();
    run.app
        .command_rejected(request, Link::Passive, "outer queue full".into());
    assert!(!run.scan().active());
    assert!(run.app.monitor.as_ref().unwrap().live);
    run.command("mstp scan 1").await;
    let current = run.scan().clone();
    let mut stale = current.clone();
    stale.id -= 1;
    stale.phase = ScanPhase::Restored;
    worker(
        &mut run.app,
        WorkerEvent::MonitorScan(ScanReport::Progress(Box::new(stale))),
    );
    assert_eq!(*run.scan(), current);
    let generation = run.app.generation;
    let mut stale = current.clone();
    stale.phase = ScanPhase::Restored;
    worker(
        &mut run.app,
        WorkerEvent::Session {
            generation: generation - 1,
            event: Box::new(WorkerEvent::MonitorScan(ScanReport::Progress(Box::new(
                stale,
            )))),
        },
    );
    assert_eq!(*run.scan(), current);
    run.app.monitor.as_mut().unwrap().tab = MonitorTab::Scan;
    assert!(press(&mut run.app, KeyCode::Char('J')).is_empty());
    assert!(run.app.monitor.as_ref().unwrap().wizard.is_none());
    run.command("mstp scan cancel").await;
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    run.quit().await;
}

pub(super) async fn line_step(line: &ScanLine, step: Step) {
    bounded(async {
        loop {
            let changed = line.changed.notified();
            if line.steps().contains(&step) {
                return;
            }
            changed.await;
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_sink_loss_while_close_pending_never_restores() {
    let line = ScanLine::new(&[38400]);
    line.fault(1, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    run.command("mstp scan 1").await;
    run.pump(|r| r.scan().rows[0].status == RowStatus::Measuring)
        .await;
    run.events.close();
    line_step(&line, Step::Closing(1)).await;
    assert_eq!(opened(&run), 2);
    line.release(1, true);
    line_step(&line, Step::Closed(1)).await;
    run.commands = None;
    bounded(run.task.take().unwrap()).await.unwrap();
    assert_eq!(opened(&run), 2);
    assert_eq!(line.participants.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_cancel_releases_input_even_when_progress_queue_is_saturated() {
    let line = ScanLine::new(&[38400]);
    line.fault(1, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    run.command("mstp scan 1").await;
    run.pump(|r| r.scan().rows[0].status == RowStatus::Measuring)
        .await;
    line_step(&line, Step::Drained(1)).await;
    // First-observation progress fills the one-event queue; no consumer drains
    // it while cancel must still initiate acknowledged close.
    run.command("mstp scan cancel").await;
    line_step(&line, Step::Closing(1)).await;
    assert_eq!(opened(&run), 2);
    line.release(1, true);
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    assert_eq!(opened(&run), 3);
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_failed_pending_release_refuses_disconnect_and_queued_reconnect_join() {
    let line = ScanLine::new(&[38400]);
    line.fault(1, Fault::Hold);
    let mut run = Run::new(line.clone());
    run.ready().await;
    run.command("mstp scan 1").await;
    run.pump(|r| r.scan().rows[0].status == RowStatus::Measuring)
        .await;
    run.send(vec![Command::Disconnect, Command::Reconnect, join()])
        .await;
    line_step(&line, Step::Closing(1)).await;
    line.release(1, false);
    run.pump(|r| r.app.quit).await;
    assert_eq!(opened(&run), 2);
    assert_eq!(line.participants.load(Ordering::SeqCst), 0);
    assert!(
        !run.seen
            .iter()
            .any(|e| matches!(e, WorkerEvent::Disconnected { .. }))
    );
    assert!(
        run.app
            .exit_error
            .as_ref()
            .unwrap()
            .contains("release unproven")
    );
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_full_progress_queue_does_not_extend_measurement() {
    let line = ScanLine::new(&[38400]);
    line.fault(1, Fault::Hold);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    run.command("mstp scan 1").await;
    run.pump(|r| {
        r.scan().rows[0].status == RowStatus::Opening && r.line.steps().contains(&Step::Drained(1))
    })
    .await;
    // The decoder's Drained notification can arrive before the scan task is
    // scheduled to publish Measuring. Wait for actual bounded-queue occupancy.
    while run.events.is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(run.events.len(), 1, "the Measuring report is still queued");
    tokio::time::advance(Duration::from_secs(1)).await;
    line_step(&line, Step::Closing(1)).await;
    assert_eq!(opened(&run), 2);
    line.release(1, true);
    run.command("mstp scan cancel").await;
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    assert_eq!(run.scan().rows[0].elapsed, Duration::from_secs(1));
    run.quit().await;
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_early_eof_stays_incomplete_behind_full_progress_queue() {
    let line = ScanLine::new(&[38400]);
    let mut run = Run::with_capacity(line.clone(), 1);
    run.ready().await;
    run.command("mstp scan 1").await;
    run.pump(|r| {
        r.scan().rows[0].status == RowStatus::Opening && r.line.steps().contains(&Step::Drained(1))
    })
    .await;
    // Close the peer after actual decoded input, while its first evidence
    // report is blocked behind the queued Measuring report.
    for (index, peer) in line.take_peers() {
        if index != 1 {
            line.return_peer(index, peer);
        }
    }
    line_step(&line, Step::Closed(1)).await;
    while run.events.is_empty() {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    run.pump(|r| {
        matches!(
            r.scan().rows[0].status,
            RowStatus::Complete | RowStatus::Incomplete(_)
        )
    })
    .await;
    assert!(
        matches!(run.scan().rows[0].status, RowStatus::Incomplete(_)),
        "{:?}",
        run.scan().rows[0]
    );
    run.command("mstp scan cancel").await;
    run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
        .await;
    run.quit().await;
}
