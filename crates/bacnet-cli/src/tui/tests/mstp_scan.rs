//! Operator-driven scanner acceptance with actual decoder input and paused time.
use super::site::bounded;
use super::*;
use crate::tui::app::monitor::MonitorTab;
use crate::tui::monitor::scan::{RowStatus, Scan, ScanPhase, Suggestion};
use crate::tui::monitor::scan_fixture::{Fault, ScanLine, Step};
use crate::tui::monitor::{Lines, MonitorPlan, MstpOptions};
use crate::tui::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{EventSink, coordinator};
use std::sync::{Arc, atomic::Ordering};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(super) struct Run {
    pub(super) app: App,
    pub(super) seen: Vec<WorkerEvent>,
    pub(super) line: Arc<ScanLine>,
    pub(super) commands: Option<mpsc::Sender<Command>>,
    pub(super) events: mpsc::Receiver<WorkerEvent>,
    pub(super) task: Option<JoinHandle<()>>,
}
impl Run {
    pub(super) fn new(line: Arc<ScanLine>) -> Self {
        Self::with_capacity(line, 64)
    }
    pub(super) fn with_capacity(line: Arc<ScanLine>, capacity: usize) -> Self {
        let monitor = MonitorPlan::new("scripted-rs485".into(), 19200, 1);
        let options = SessionOptions {
            mstp: MstpOptions {
                monitor: Some(monitor.clone()),
                lines: Lines::Scan(line.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        let plan = ConnectPlan {
            kind: TransportKind::MstpMonitor,
            args: crate::tui::profile::Profile::default()
                .transport_args()
                .unwrap(),
            await_interface: false,
        };
        let (sink, events) = EventSink::channel(capacity);
        let (commands, rx) = mpsc::channel(16);
        let task = coordinator::spawn(Some(plan), None, sink, rx, options);
        let mut app = App::new(
            AppConfig {
                transport: "MSTP",
                style: AddressStyle::Mstp,
                color: false,
                picker: None,
                join: None,
                passive: false,
                now: t0(),
            },
            LogRing::new(100),
        );
        app.start_monitor(&monitor, true);
        Self {
            app,
            seen: Vec::new(),
            line,
            commands: Some(commands),
            events,
            task: Some(task),
        }
    }
    pub(super) fn scan(&self) -> &Scan {
        self.app.monitor.as_ref().unwrap().scan.as_deref().unwrap()
    }
    pub(super) async fn pump(&mut self, done: impl Fn(&Self) -> bool) {
        bounded(async {
            loop {
                if done(self) {
                    return;
                }
                let changed = self.line.changed.clone();
                tokio::select! {
                    event = self.events.recv() => {
                        let event = event.expect("coordinator event stream ended");
                        self.seen.push(event.clone());
                        let out = worker(&mut self.app, event);
                        self.send(out).await;
                    }
                    () = changed.notified() => {}
                }
            }
        })
        .await;
    }
    pub(super) async fn send(&self, out: Vec<Command>) {
        for command in out {
            self.commands.as_ref().unwrap().send(command).await.unwrap();
        }
    }
    pub(super) async fn command(&mut self, text: &str) {
        let out = press(&mut self.app, KeyCode::Char(':'));
        self.send(out).await;
        type_text(&mut self.app, text);
        let out = press(&mut self.app, KeyCode::Enter);
        self.send(out).await;
    }
    pub(super) async fn ready(&mut self) {
        self.pump(|r| r.app.monitor.as_ref().unwrap().live).await;
    }
    pub(super) async fn measure(&mut self, candidate: usize) {
        self.pump(|r| {
            r.scan().rows[candidate].status == RowStatus::Measuring
                && r.line.steps().contains(&Step::Drained(candidate + 1))
        })
        .await;
        // Drained is recorded on the read AFTER the encoded bytes were parsed.
        tokio::time::advance(Duration::from_secs(1)).await;
        self.pump(|r| r.scan().rows[candidate].status == RowStatus::Complete)
            .await;
    }
    pub(super) async fn finish(&mut self) {
        for candidate in 0..6 {
            self.measure(candidate).await;
        }
        self.pump(|r| {
            r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live
        })
        .await;
    }
    pub(super) async fn quiet(&self) {
        // Keeping each peer also detects delivered writes after shutdown.
        for (index, mut peer) in self.line.take_peers() {
            let mut byte = [0];
            tokio::select! { biased;
                read = peer.read(&mut byte) => assert_eq!(read.unwrap(), 0, "a byte reached a peer"),
                () = std::future::ready(()) => {}
            }
            self.line.return_peer(index, peer);
        }
    }
    pub(super) async fn quit(mut self) {
        self.quiet().await;
        self.commands = None;
        bounded(self.task.take().unwrap()).await.unwrap();
        for (_, mut peer) in self.line.take_peers() {
            let mut bytes = Vec::new();
            bounded(peer.read_to_end(&mut bytes)).await.unwrap();
            assert!(bytes.is_empty(), "delivered output: {bytes:?}");
        }
        assert_eq!(self.line.participants.load(Ordering::SeqCst), 0);
    }
}

pub(super) fn snapshots(run: &mut Run, name: &str) {
    run.app.monitor.as_mut().unwrap().tab = MonitorTab::Scan;
    for (width, height) in [(80, 24), (120, 40)] {
        insta::assert_snapshot!(
            format!("{name}_{width}x{height}"),
            screen(&mut run.app, width, height)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_operator_selects_actual_rate_and_restores_original() {
    for rate in [38400, 76800] {
        let line = ScanLine::new(&[rate]);
        let mut run = Run::new(line.clone());
        run.ready().await;
        run.command("mstp scan 1").await;
        if rate == 38400 {
            run.pump(|r| r.scan().rows[0].status == RowStatus::Measuring)
                .await;
            snapshots(&mut run, "mstp_scan_progress");
        }
        run.finish().await;
        assert_eq!(run.scan().suggestion(), Suggestion::Rate(rate));
        assert_eq!(run.scan().original, 19200);
        assert!(
            run.scan()
                .rows
                .iter()
                .all(|row| row.elapsed == Duration::from_secs(1))
        );
        for row in &run.scan().rows {
            assert_eq!(row.evidence.valid > 0, row.baud == rate);
            if row.baud != rate {
                assert!(row.evidence.errors[0] > 0);
                assert!(row.evidence.noise > 0);
            }
        }
        let opens: Vec<_> = line
            .steps()
            .into_iter()
            .filter_map(|step| match step {
                Step::Open(_, baud) => Some(baud),
                _ => None,
            })
            .collect();
        assert_eq!(
            opens,
            [19200, 9600, 19200, 38400, 57600, 76800, 115200, 19200]
        );
        for index in 0..7 {
            let steps = line.steps();
            let closed = steps
                .iter()
                .position(|step| *step == Step::Closed(index))
                .unwrap();
            let opened = steps
                .iter()
                .position(|step| matches!(step, Step::Open(i, _) if *i == index + 1))
                .unwrap();
            assert!(closed < opened);
        }
        assert!(run.app.monitor.as_ref().unwrap().wizard.is_none());
        if rate == 38400 {
            snapshots(&mut run, "mstp_scan_result");
        }
        run.quit().await;
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_operator_reports_ambiguity_and_insufficient_traffic() {
    for (rates, expected, name) in [
        (
            vec![38400, 76800],
            Suggestion::Ambiguous(vec![38400, 76800]),
            "mstp_scan_ambiguous",
        ),
        (vec![], Suggestion::Insufficient, "mstp_scan_insufficient"),
    ] {
        let mut run = Run::new(ScanLine::new(&rates));
        run.ready().await;
        run.command("mstp scan 1").await;
        run.finish().await;
        assert_eq!(run.scan().suggestion(), expected);
        snapshots(&mut run, name);
        run.quit().await;
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_safe_admission_failure_and_eof_remain_partial() {
    for fault in [Fault::Admission, Fault::Eof] {
        let line = ScanLine::new(&[38400]);
        line.fault(1, fault);
        let mut run = Run::new(line);
        run.ready().await;
        run.command("mstp scan 1").await;
        for candidate in 1..6 {
            run.measure(candidate).await;
        }
        run.pump(|r| r.scan().phase == ScanPhase::Restored && r.app.monitor.as_ref().unwrap().live)
            .await;
        assert_eq!(run.scan().suggestion(), Suggestion::Partial);
        assert!(matches!(
            run.scan().rows[0].status,
            RowStatus::Failed(_) | RowStatus::Incomplete(_)
        ));
        if fault == Fault::Admission {
            snapshots(&mut run, "mstp_scan_partial");
        }
        run.quit().await;
    }
}
