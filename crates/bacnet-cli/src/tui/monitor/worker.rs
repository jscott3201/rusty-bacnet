//! The monitor session: open the port to listen only, run `MstpMonitor` on
//! it, and send the screen a sample every [`SAMPLE_EVERY`].
//!
//! Between samples it reads the monitor's statistics as they change, for
//! lost-token events and rotation times, and its live frame stream, for
//! I-Am and routers. Samples are offered, never awaited, and the frame
//! stream is lossy, so neither the monitor nor the screen waits for the
//! other. Nothing here holds a lock across an await.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::NetworkMessageType;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::mstp::codec::FrameType;
use bacnet_transport::mstp::monitor::{
    DurationStats, LineEvent, LostToken, MonitorConfig, MonitorFrames, MonitorRecord, MonitorStats,
    MstpMonitor,
};
use bacnet_types::enums::UnconfirmedServiceChoice;
use tokio::sync::{mpsc, watch};

use super::{
    Lines, MonitorPlan, MonitorReport, MonitorSample, LOST_TOKEN_HISTORY, ROTATION_HISTORY,
    SAMPLE_EVERY,
};
use crate::tui::capture::{self, CaptureCommand};
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::worker::{passive, EventSink, Offer};

/// Serve a monitor session until it is cancelled, disconnected, or the
/// monitor ends. Before returning it stops the monitor and waits for its
/// task, so the port is closed by then.
pub(crate) async fn serve(
    plan: MonitorPlan,
    lines: &Lines,
    sink: &EventSink,
    commands: mpsc::Receiver<Command>,
) {
    let config = MonitorConfig::new(plan.baud).with_timestamp_resolution(plan.resolution);
    let started = lines
        .open_monitor(&plan.port, plan.baud)
        .and_then(|reader| MstpMonitor::start(reader, config));
    let monitor = match started {
        Ok(monitor) => monitor,
        Err(error) => {
            tracing::error!("MS/TP monitor on {}: {error}", plan.port);
            sink.deliver(WorkerEvent::ConnectFailed {
                error: crate::tui::message::Failure::within(&plan.port, &error),
            })
            .await;
            return;
        }
    };
    tracing::info!(
        "MS/TP monitor listening on {} at {} baud; nothing is transmitted",
        plan.port,
        plan.baud
    );
    let report = MonitorReport::Started {
        port: plan.port.clone(),
        baud: plan.baud,
    };
    let mut ended = None;
    if sink.deliver(WorkerEvent::Passive).await && sink.deliver(WorkerEvent::Monitor(report)).await
    {
        let mut watcher = Watcher::new(monitor.statistics(), monitor.frames());
        // The Capture screen's own subscription (#971) runs beside the
        // watcher, which hands it the session's capture commands; it ends
        // when the watcher drops that channel.
        let (capture, capture_rx) = mpsc::channel(4);
        let started = watcher.started;
        let capturing = capture::worker::run_monitor(&monitor, sink, capture_rx, started);
        (ended, ()) = tokio::join!(
            Box::pin(watcher.run(sink, commands, capture)),
            Box::pin(capturing)
        );
    }
    // Wait for the monitor's task: once it returns, the port is closed.
    let stopped = monitor.stop().await;
    tracing::info!("MS/TP monitor on {} closed", plan.port);
    let error = match (ended, stopped) {
        (_, Err(error)) => Some(error.to_string()),
        (Some(()), Ok(_)) => Some("the port closed".into()),
        (None, Ok(_)) => None,
    };
    if let Some(error) = error {
        tracing::warn!("MS/TP monitor on {} ended: {error}", plan.port);
        let report = MonitorReport::Ended(format!("{}: {error}", plan.port));
        sink.deliver(WorkerEvent::Monitor(report)).await;
    }
}

/// What the worker follows between samples.
struct Watcher {
    stats: watch::Receiver<MonitorStats>,
    frames: MonitorFrames,
    started: tokio::time::Instant,
    devices: BTreeMap<u8, u32>,
    routers: BTreeSet<u8>,
    rotations: VecDeque<Duration>,
    /// The station rotations are timed at, its sample count and total.
    rotation_seen: Option<(u8, u64, Duration)>,
    lost: VecDeque<LostToken>,
    lost_seen: u64,
}

impl Watcher {
    fn new(stats: watch::Receiver<MonitorStats>, frames: MonitorFrames) -> Self {
        Self {
            stats,
            frames,
            started: tokio::time::Instant::now(),
            devices: BTreeMap::new(),
            routers: BTreeSet::new(),
            rotations: VecDeque::new(),
            rotation_seen: None,
            lost: VecDeque::new(),
            lost_seen: 0,
        }
    }

    /// Follow the monitor until the session ends (`None`) or the monitor
    /// does (`Some`).
    async fn run(
        &mut self,
        sink: &EventSink,
        mut commands: mpsc::Receiver<Command>,
        capture: mpsc::Sender<CaptureCommand>,
    ) -> Option<()> {
        let mut every = tokio::time::interval(SAMPLE_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = sink.cancelled() => return None,
                command = commands.recv() => match command {
                    None | Some(Command::Disconnect) => return None,
                    // Polled beside this loop, so waiting for room can't
                    // deadlock; it fails only once the capture has ended.
                    Some(Command::Capture(command)) => {
                        if capture.send(command).await.is_err() {
                            tracing::debug!("the monitor capture has ended");
                        }
                    }
                    // Listen only: answer whatever would transmit.
                    Some(command) => passive::refuse(sink, command).await,
                },
                changed = self.stats.changed() => {
                    if changed.is_err() {
                        return Some(());
                    }
                    // Copy out the little it needs: the monitor waits for
                    // a borrow to end before its next update.
                    let seen = Seen::of(&self.stats.borrow_and_update());
                    self.observe(seen);
                }
                record = self.frames.recv() => match record {
                    Some(record) => self.note(&record),
                    None => return Some(()),
                },
                _ = every.tick() => {
                    let sample = self.sample();
                    let report = WorkerEvent::Monitor(MonitorReport::Sample(Box::new(sample)));
                    if let Offer::Closed = sink.offer(report) {
                        return None;
                    }
                }
            }
        }
    }

    fn sample(&self) -> MonitorSample {
        MonitorSample {
            elapsed: self.started.elapsed(),
            stats: self.stats.borrow().clone(),
            devices: self.devices.clone(),
            routers: self.routers.clone(),
            rotations: self.rotations.clone(),
            lost: self.lost.clone(),
            dropped: self.frames.dropped(),
        }
    }

    /// New statistics: log each lost token, and time the ring at its lowest
    /// master.
    fn observe(&mut self, stats: Seen) {
        if stats.lost_tokens > self.lost_seen {
            let missed = stats.lost_tokens - self.lost_seen - 1;
            self.lost_seen = stats.lost_tokens;
            if let Some(lost) = stats.last_lost_token {
                let by = lost
                    .regenerated_by
                    .map_or_else(|| "an unreadable frame".into(), |mac| format!("mstp:{mac}"));
                tracing::warn!(
                    "MS/TP token lost: the line was silent {} ms; {by} started it again{}",
                    lost.silence.as_millis(),
                    if missed > 0 {
                        format!(" ({missed} more missed)")
                    } else {
                        String::new()
                    }
                );
                push(&mut self.lost, lost, LOST_TOKEN_HISTORY);
            }
        }
        let Some((mac, rotation)) = stats.lowest else {
            return;
        };
        match self.rotation_seen {
            Some((seen, count, total)) if seen == mac && rotation.count > count => {
                let more = u32::try_from(rotation.count - count).unwrap_or(u32::MAX);
                let mean = rotation.total.saturating_sub(total) / more;
                push(&mut self.rotations, mean, ROTATION_HISTORY);
            }
            Some((seen, ..)) if seen == mac => return,
            // A new lowest master: start its series afresh.
            _ => self.rotations.clear(),
        }
        self.rotation_seen = Some((mac, rotation.count, rotation.total));
    }

    /// A live record: an I-Am names the device at its source, and a routed
    /// NPDU or a network-layer message marks its source as a router.
    fn note(&mut self, record: &MonitorRecord) {
        let LineEvent::Frame { header, .. } = record.event else {
            return;
        };
        if header.frame_type != FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            return;
        }
        let Ok(npdu) = decode_npdu(record.data.clone()) else {
            return;
        };
        let routed = npdu.source.is_some()
            || npdu.message_type == Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);
        if routed {
            self.routers.insert(header.source);
            return;
        }
        if npdu.is_network_message() {
            return;
        }
        if let Ok(Apdu::UnconfirmedRequest(request)) = decode_apdu(npdu.payload) {
            if request.service_choice == UnconfirmedServiceChoice::I_AM {
                if let Ok(i_am) = IAmRequest::decode(&request.service_request) {
                    let instance = i_am.object_identifier.instance_number();
                    self.devices.insert(header.source, instance);
                }
            }
        }
    }
}

/// What [`Watcher::observe`] reads from new statistics.
struct Seen {
    lost_tokens: u64,
    last_lost_token: Option<LostToken>,
    /// The lowest station that passed the token, and its rotation times.
    lowest: Option<(u8, DurationStats)>,
}

impl Seen {
    fn of(stats: &MonitorStats) -> Self {
        Self {
            lost_tokens: stats.lost_tokens,
            last_lost_token: stats.last_lost_token,
            lowest: stats
                .stations
                .iter()
                .find(|(_, station)| station.tokens_passed > 0)
                .map(|(mac, station)| (*mac, station.token_rotation)),
        }
    }
}

fn push<T>(queue: &mut VecDeque<T>, item: T, cap: usize) {
    if queue.len() == cap {
        queue.pop_front();
    }
    queue.push_back(item);
}
