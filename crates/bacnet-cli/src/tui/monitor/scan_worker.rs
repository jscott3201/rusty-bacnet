//! Serial ownership transitions for one explicit receive-only scan.

use super::scan::{Evidence, RowStatus, Scan, ScanCommand, ScanGate, ScanPhase, ScanReport};
use super::{Lines, MonitorPlan};
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::worker::{EventSink, passive};
use bacnet_transport::mstp::monitor::{
    MonitorAdmissionError, MonitorConfig, MonitorStats, MstpMonitor,
};
use std::time::Duration;
use tokio::sync::mpsc;

/// Acquire directly: no detached task or unread result can hide an owner.
pub(super) async fn open(
    plan: &MonitorPlan,
    lines: &Lines,
) -> Result<MstpMonitor, MonitorAdmissionError> {
    let reader = lines.open_monitor(&plan.port, plan.baud)?;
    MstpMonitor::start(
        reader,
        MonitorConfig::new(plan.baud).with_timestamp_resolution(plan.resolution),
    )
    .await
}

/// Cancellation intent survives waits for confirmed input release.
struct Control {
    id: u64,
    manual: bool,
    exit: bool,
}
impl Control {
    async fn command(&mut self, command: Option<Command>, sink: &EventSink) {
        match command {
            None | Some(Command::Disconnect) => self.exit = true,
            Some(Command::MonitorScan(ScanCommand::Cancel { id })) if id == self.id => {
                self.manual = true
            }
            Some(Command::MonitorScan(request)) => {
                // The coordinator normally rejects these before forwarding.
                // This defensive path retains their identity too.
                sink.offer(WorkerEvent::MonitorScan(ScanReport::Rejected {
                    id: request.id(),
                    cancel: matches!(
                        request,
                        crate::tui::monitor::scan::ScanCommand::Cancel { .. }
                    ),
                    reason: "another scan owns the monitor".into(),
                }));
            }
            Some(command) => {
                if let Some(event) = passive::refusal(command) {
                    sink.offer(event);
                }
            }
        }
    }
    async fn pending(&mut self, sink: &EventSink, commands: &mut mpsc::Receiver<Command>) {
        if sink.is_cancelled() {
            self.exit = true;
        }
        loop {
            match commands.try_recv() {
                Ok(command) => self.command(Some(command), sink).await,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.exit = true;
                    break;
                }
            }
        }
        // Poll sink closure without inventing a deadline.
        tokio::select! { biased; () = sink.closed() => self.exit = true, () = std::future::ready(()) => {} }
    }
    async fn report(
        &mut self,
        scan: &Scan,
        sink: &EventSink,
        commands: &mut mpsc::Receiver<Command>,
    ) {
        let superseded = !matches!(
            scan.phase,
            ScanPhase::Restoring | ScanPhase::Restored | ScanPhase::Stopped(_)
        );
        if self.exit || (self.manual && superseded) {
            return;
        }
        let deliver = sink.deliver(WorkerEvent::MonitorScan(ScanReport::Progress(Box::new(
            scan.clone(),
        ))));
        tokio::pin!(deliver);
        loop {
            tokio::select! {
                biased;
                () = sink.cancelled() => { self.exit = true; return; }
                () = sink.closed() => { self.exit = true; return; }
                command = commands.recv() => { self.command(command, sink).await; if self.exit || (self.manual && superseded) { return; } }
                delivered = &mut deliver => { if !delivered { self.exit = true; } return; }
            }
        }
    }
    /// Progress is superseded at the measurement deadline, even under backpressure.
    async fn report_until(
        &mut self,
        scan: &Scan,
        sink: &EventSink,
        commands: &mut mpsc::Receiver<Command>,
        deadline: tokio::time::Instant,
    ) -> bool {
        tokio::select! {
            biased;
            () = tokio::time::sleep_until(deadline) => true,
            () = self.report(scan, sink, commands) => false,
        }
    }
    async fn close_row(
        &mut self,
        monitor: &mut MstpMonitor,
        scan: &Scan,
        sink: &EventSink,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<MonitorStats, String> {
        // Initiate stop before publishing Closing: a full event queue must not
        // extend the measurement while we are claiming to wait for release.
        let first = {
            let stopped = monitor.stop();
            tokio::pin!(stopped);
            std::future::poll_fn(|cx| std::task::Poll::Ready(stopped.as_mut().poll(cx))).await
        };
        self.report(scan, sink, commands).await;
        match first {
            std::task::Poll::Ready(result) => {
                result.map_err(|error| format!("MS/TP input release unproven: {error}"))
            }
            std::task::Poll::Pending => self.close(monitor, sink, commands).await,
        }
    }
    /// Keep matching cancellation live while both input close and finite
    /// capture accounting are retained by the ordinary session's future.
    async fn retire(
        &mut self,
        retiring: impl Future<Output = Result<(), String>>,
        sink: &EventSink,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<(), String> {
        tokio::pin!(retiring);
        loop {
            tokio::select! {
                biased;
                () = sink.cancelled(), if !self.exit => self.exit = true,
                () = sink.closed(), if !self.exit => self.exit = true,
                command = commands.recv(), if !self.exit => self.command(command, sink).await,
                result = &mut retiring => return result,
            }
        }
    }
    async fn close(
        &mut self,
        monitor: &mut MstpMonitor,
        sink: &EventSink,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<MonitorStats, String> {
        loop {
            tokio::select! {
                biased;
                () = sink.cancelled(), if !self.exit => self.exit = true,
                () = sink.closed(), if !self.exit => self.exit = true,
                command = commands.recv(), if !self.exit => self.command(command, sink).await,
                stopped = monitor.stop() => return stopped.map_err(|error| format!("MS/TP input release unproven: {error}")),
            }
        }
    }
}

/// Retire the ordinary owner, measure candidates, and restore only the original.
/// An error prohibits any replacement; the coordinator owns the fatal outcome.
pub(super) async fn run(
    retiring: impl Future<Output = Result<(), String>>,
    plan: &MonitorPlan,
    lines: &Lines,
    sink: &EventSink,
    commands: &mut mpsc::Receiver<Command>,
    gate: &ScanGate,
    request: ScanCommand,
) -> Result<Option<MstpMonitor>, String> {
    let ScanCommand::Start { id, window } = request else {
        unreachable!("watcher admits only start")
    };
    let mut scan = Scan::new(id, plan.baud, window);
    let mut control = Control {
        id,
        manual: false,
        exit: false,
    };
    // Poll close and final capture delivery together while retaining command
    // handling. Do not park one queued sink sender behind another progress
    // send: the App already shows Retiring from its accepted start request.
    let retired = control.retire(retiring, sink, commands).await;
    if let Err(error) = retired {
        scan.phase = ScanPhase::Stopped(error.clone());
        control.report(&scan, sink, commands).await;
        return Err(error);
    }
    scan.phase = ScanPhase::Candidates;
    for index in 0..scan.rows.len() {
        control.pending(sink, commands).await;
        if control.exit || control.manual {
            break;
        }
        scan.rows[index].status = RowStatus::Opening;
        control.report(&scan, sink, commands).await;
        control.pending(sink, commands).await;
        if control.exit || control.manual {
            break;
        }
        let mut candidate = plan.clone();
        candidate.baud = scan.rows[index].baud;
        let mut monitor = match open(&candidate, lines).await {
            Ok(monitor) => monitor,
            Err(error) => {
                scan.rows[index].status = RowStatus::Failed(error.to_string());
                if !error.release_confirmed() {
                    let error = format!("MS/TP input release unproven: {error}");
                    scan.phase = ScanPhase::Stopped(error.clone());
                    control.report(&scan, sink, commands).await;
                    return Err(error);
                }
                control.report(&scan, sink, commands).await;
                continue;
            }
        };
        // Even cancellation during synchronous open or awaited admission keeps
        // this owner until close acknowledges release.
        control.pending(sink, commands).await;
        let started = tokio::time::Instant::now();
        let mut stats = monitor.statistics();
        scan.rows[index].status = RowStatus::Measuring;
        let mut full = control
            .report_until(&scan, sink, commands, started + window)
            .await;
        let mut observed = false;
        let mut reason = "cancelled";
        if !full && !control.exit && !control.manual {
            loop {
                tokio::select! {
                    biased;
                    () = sink.cancelled() => { control.exit = true; break; }
                    () = sink.closed() => { control.exit = true; break; }
                    command = commands.recv() => {
                        control.command(command, sink).await;
                        if control.exit || control.manual { break; }
                    }
                    () = tokio::time::sleep_until(started + window) => { full = true; break; }
                    changed = stats.changed() => {
                        if changed.is_err() { reason = "input ended before the full window"; break; }
                        // One bounded first-observation report per candidate:
                        // useful live evidence and a causal decoder receipt.
                        let evidence = Evidence::of(&stats.borrow_and_update());
                        if !observed && (evidence.valid > 0 || evidence.errors() > 0 || evidence.noise > 0) {
                            observed = true;
                            scan.rows[index].evidence = evidence;
                            full = control.report_until(&scan, sink, commands, started + window).await;
                            if full || control.exit || control.manual { break; }
                        }
                    }
                }
            }
        }
        // A blocked progress report may outlive the input. Reaching the
        // deadline cannot turn an already-ended stream into a full window.
        if stats.has_changed().is_err() {
            full = false;
            reason = "input ended before the full window";
        }
        scan.rows[index].elapsed = started.elapsed();
        scan.rows[index].evidence = Evidence::of(&stats.borrow());
        scan.rows[index].status = RowStatus::Closing;
        match control.close_row(&mut monitor, &scan, sink, commands).await {
            Ok(stats) => scan.rows[index].evidence = Evidence::of(&stats),
            Err(error) => {
                scan.rows[index].status = RowStatus::Failed(error.clone());
                scan.phase = ScanPhase::Stopped(error.clone());
                control.report(&scan, sink, commands).await;
                return Err(error);
            }
        }
        scan.rows[index].status = if full {
            RowStatus::Complete
        } else {
            RowStatus::Incomplete(reason.into())
        };
        control.report(&scan, sink, commands).await;
    }
    control.pending(sink, commands).await;
    scan.cancelled = control.manual;
    if control.exit {
        gate.release(id);
        return Ok(None);
    }
    scan.phase = ScanPhase::Restoring;
    control.report(&scan, sink, commands).await;
    control.pending(sink, commands).await;
    if control.exit {
        gate.release(id);
        return Ok(None);
    }
    let mut restored = match open(plan, lines).await {
        Ok(monitor) => monitor,
        Err(error) => {
            scan.phase = ScanPhase::Stopped(format!(
                "original {} baud was not restored: {error}",
                plan.baud
            ));
            control.report(&scan, sink, commands).await;
            if !error.release_confirmed() {
                return Err(format!("MS/TP input release unproven: {error}"));
            }
            gate.release(id);
            return Ok(None);
        }
    };
    control.pending(sink, commands).await;
    if control.exit {
        control.close(&mut restored, sink, commands).await?;
        gate.release(id);
        return Ok(None);
    }
    scan.cancelled = control.manual;
    scan.phase = ScanPhase::Restored;
    control.report(&scan, sink, commands).await;
    if control.exit {
        control.close(&mut restored, sink, commands).await?;
        gate.release(id);
        return Ok(None);
    }
    gate.release(id);
    Ok(Some(restored))
}

/// Validate at both UI-independent admission boundaries.
pub(crate) fn valid_window(window: Duration) -> bool {
    (Duration::from_secs(1)..=Duration::from_secs(3600)).contains(&window)
        && window.subsec_nanos() == 0
}
