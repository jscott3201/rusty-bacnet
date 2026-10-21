//! The watch list's worker: managed COV subscriptions with finite lifetimes,
//! polling when a device won't take one, and an aggregator that folds COV
//! notifications into a bounded ring per row.
//!
//! [`run`] is polled beside the session loop under `join!`, like the
//! browser. Its loop's arms never await: requests run as jobs in a
//! `FuturesUnordered` the loop keeps polling, one job per device at a time.
//! When the session ends, it cancels every subscription it holds (Clauses
//! 13.14.2 and 13.15.2), waiting at most [`CANCEL_WAIT`] for the devices;
//! the lifetimes bound whatever it couldn't cancel.

pub(crate) mod csv;
pub(crate) mod jobs;
pub(crate) mod sample;

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{
    BACnetClient, ManagedCOVSubscription, ManagedCOVSubscriptionEvent, ReceivedCOVNotification,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::time::Instant;

use self::csv::{field, unix_ms, CsvLog};
use self::jobs::{Lifetime, Polled, Refusal, Started};
use self::sample::{Reading, RowFormat};
use super::browse::limits::RateCap;
use super::EventSink;
use crate::tui::app::objects::{kebab, object_label};
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::watched::{Point, RowId, Sample, Source, WatchCommand, WatchEvent, WatchStats};

/// Default `--cov-lifetime`, in seconds.
pub(crate) const DEFAULT_COV_LIFETIME: u32 = 300;
/// Default `--poll-interval`, in seconds.
pub(crate) const DEFAULT_POLL_SECS: u64 = 10;
/// Rows a session may watch: real devices have small COV tables.
pub(crate) const MAX_ROWS: usize = 100;
/// How often samples go to the UI and polls are scheduled.
const TICK: Duration = Duration::from_millis(250);
/// Samples a row keeps between ticks; older ones are dropped and counted.
pub(crate) const PENDING_PER_ROW: usize = 32;
/// How long the end of a session waits for its cancels.
pub(crate) const CANCEL_WAIT: Duration = Duration::from_secs(5);
/// A confirmed request with every retry at the CLI's defaults: a 6 s APDU
/// timeout, sent four times.
pub(crate) const DEFAULT_REQUEST_BUDGET: Duration = Duration::from_secs(24);
/// The first and the longest wait before polling a device that stopped
/// answering again.
const POLL_BACKOFF: (Duration, Duration) = (Duration::from_secs(2), Duration::from_secs(60));
/// The first and the longest wait before subscribing again after a
/// renewal failed.
const RESUBSCRIBE_BACKOFF: (Duration, Duration) =
    (Duration::from_secs(30), Duration::from_secs(300));

/// The watch list's settings.
#[derive(Debug, Clone)]
pub(crate) struct WatchOptions {
    /// Subscription lifetime in seconds.
    pub(crate) lifetime: u32,
    /// Least time between two polls of a point.
    pub(crate) poll_every: Duration,
    /// The profile workspace, where CSV logs go.
    pub(crate) workspace: Option<PathBuf>,
    /// How long one confirmed request can take with its retries; renewals
    /// go at least this long before a lifetime ends.
    pub(crate) request_budget: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            lifetime: DEFAULT_COV_LIFETIME,
            poll_every: Duration::from_secs(DEFAULT_POLL_SECS),
            workspace: None,
            request_budget: DEFAULT_REQUEST_BUDGET,
        }
    }
}

/// The next wait in an exponential backoff of `(first, longest)`.
fn backoff(previous: Duration, (first, longest): (Duration, Duration)) -> Duration {
    if previous.is_zero() {
        first
    } else {
        (previous * 2).min(longest)
    }
}

enum Mode {
    /// Waiting for its device.
    Queued,
    /// Its start job is running.
    Starting,
    /// Subscribed; the client renews it.
    Cov(ManagedCOVSubscription),
    /// Polled.
    Polled,
}

struct Row {
    point: Point,
    mode: Mode,
    mac: Option<MacAddr>,
    /// Removed while starting: cancel once the start job ends.
    removing: bool,
    /// A subscription may exist although the row isn't subscribed.
    maybe_subscribed: bool,
    /// After a renewal failed: when to subscribe again, and the wait that
    /// led there.
    resubscribe: Option<(Instant, Duration)>,
    /// How its values are written, from its first read.
    format: RowFormat,
    /// Samples since the last tick, each with its value in full and its
    /// wall-clock time for the CSV.
    pending: VecDeque<(Sample, Result<String, String>, u128)>,
}

enum Op {
    Start(RowId),
    Cancel {
        row: RowId,
        point: Point,
        mac: MacAddr,
        managed: Option<ManagedCOVSubscription>,
    },
}

#[derive(Default)]
struct Device {
    busy: bool,
    ops: VecDeque<Op>,
    no_rpm: bool,
    /// The device stopped answering polls: the current backoff, and when
    /// to poll it again.
    backoff: Duration,
    retry_at: Option<Instant>,
    /// When its last poll, or a polled row's first read, ended. A device
    /// is polled, all its polled rows in one go, `poll_every` after that:
    /// rows added at different times share its requests, no row is polled
    /// more often than that, and a slow poll is never followed at once by
    /// the next.
    last_polled: Option<Instant>,
}

enum Done {
    Started {
        row: RowId,
        device: u32,
        started: Box<Started>,
    },
    Cancelled {
        row: RowId,
        device: u32,
        result: Result<(), Error>,
    },
    Polled {
        device: u32,
        polled: Polled,
    },
    CsvClosed,
}

type Job<'a> = Pin<Box<dyn Future<Output = Done> + Send + 'a>>;

struct State<'s> {
    sink: &'s EventSink,
    options: &'s WatchOptions,
    rows: BTreeMap<RowId, Row>,
    devices: BTreeMap<u32, Device>,
    /// Cancels queued or running.
    cancels: usize,
    shutting_down: bool,
    csv: Option<CsvLog>,
    closing: Vec<CsvLog>,
    outbox: VecDeque<WatchEvent>,
    stats: WatchStats,
}

impl<'s> State<'s> {
    fn new(sink: &'s EventSink, options: &'s WatchOptions) -> Self {
        Self {
            sink,
            options,
            rows: BTreeMap::new(),
            devices: BTreeMap::new(),
            cancels: 0,
            shutting_down: false,
            csv: None,
            closing: Vec::new(),
            outbox: VecDeque::new(),
            stats: WatchStats::default(),
        }
    }

    fn source(&mut self, row: RowId, source: Source) {
        self.outbox.push_back(WatchEvent::Source { row, source });
    }

    fn command(&mut self, command: WatchCommand) {
        match command {
            WatchCommand::Add { row, point } => {
                let watched = self.rows.values().filter(|row| !row.removing).count();
                if watched >= MAX_ROWS || self.rows.contains_key(&row) {
                    self.source(row, Source::Failed(format!("at most {MAX_ROWS} rows")));
                    return;
                }
                self.rows.insert(
                    row,
                    Row {
                        point,
                        mode: Mode::Queued,
                        mac: None,
                        removing: false,
                        maybe_subscribed: false,
                        resubscribe: None,
                        format: RowFormat::default(),
                        pending: VecDeque::new(),
                    },
                );
                let device = self.devices.entry(point.device).or_default();
                device.ops.push_back(Op::Start(row));
            }
            WatchCommand::Remove { row } => self.remove(row),
            WatchCommand::Csv(on) => self.csv(on),
        }
    }

    fn cancel(
        &mut self,
        row: RowId,
        point: Point,
        mac: MacAddr,
        managed: Option<ManagedCOVSubscription>,
    ) {
        self.cancels += 1;
        let device = self.devices.entry(point.device).or_default();
        device.ops.push_back(Op::Cancel {
            row,
            point,
            mac,
            managed,
        });
    }

    /// Stop watching `id`, cancelling what it holds on the device.
    fn remove(&mut self, id: RowId) {
        let Some(row) = self.rows.get_mut(&id) else {
            return;
        };
        match row.mode {
            Mode::Starting => {
                // The start job's end decides.
                row.removing = true;
                return;
            }
            Mode::Queued => {
                if let Some(device) = self.devices.get_mut(&row.point.device) {
                    device
                        .ops
                        .retain(|op| !matches!(op, Op::Start(start) if *start == id));
                }
            }
            Mode::Cov(_) | Mode::Polled => {}
        }
        let Some(row) = self.rows.remove(&id) else {
            return;
        };
        let held = match row.mode {
            Mode::Cov(managed) => Some(Some(managed)),
            Mode::Polled if row.maybe_subscribed => Some(None),
            _ => None,
        };
        if let (Some(managed), Some(mac)) = (held, row.mac) {
            self.cancel(id, row.point, mac, managed);
        }
    }

    fn csv(&mut self, on: bool) {
        let notice = match (on, self.csv.take(), &self.options.workspace) {
            (true, Some(log), _) => {
                let text = format!("Already logging to {}.", log.path.display());
                self.csv = Some(log);
                text
            }
            (true, None, None) => {
                "CSV logs go to a profile workspace; connect with a profile to log.".into()
            }
            (true, None, Some(dir)) => match CsvLog::open(dir) {
                Ok(log) => {
                    let text = format!("Logging watched values to {}.", log.path.display());
                    self.csv = Some(log);
                    text
                }
                Err(error) => error,
            },
            (false, Some(log), _) => {
                let text = format!("Stopped logging to {}.", log.path.display());
                self.closing.push(log);
                text
            }
            (false, None, _) => "Not logging.".into(),
        };
        tracing::info!("{notice}");
        self.outbox.push_back(WatchEvent::Notice(notice));
    }

    fn done(&mut self, done: Done) {
        let device = match &done {
            Done::Started { device, .. }
            | Done::Cancelled { device, .. }
            | Done::Polled { device, .. } => Some(*device),
            Done::CsvClosed => None,
        };
        if let Some(device) = device.and_then(|d| self.devices.get_mut(&d)) {
            device.busy = false;
        }
        match done {
            Done::Started { row, started, .. } => self.started(row, *started),
            Done::Cancelled { row, result, .. } => {
                self.cancels = self.cancels.saturating_sub(1);
                match result {
                    Ok(()) => tracing::info!("watch row {row}: COV subscription cancelled"),
                    Err(error) => tracing::warn!(
                        "watch row {row}: cancelling its COV subscription failed ({error}); it ends with its lifetime"
                    ),
                }
            }
            Done::Polled { device, polled } => {
                let now = Instant::now();
                let entry = self.devices.entry(device).or_default();
                entry.no_rpm |= polled.refused_rpm;
                entry.last_polled = Some(now);
                if polled.unreachable {
                    entry.backoff = backoff(entry.backoff, POLL_BACKOFF);
                    entry.retry_at = Some(now + entry.backoff);
                    tracing::info!(
                        "device {device} isn't answering polls; next try in {} s",
                        entry.backoff.as_secs()
                    );
                } else {
                    entry.backoff = Duration::ZERO;
                    entry.retry_at = None;
                }
                for (id, reading) in polled.readings {
                    if let Some(row) = self.rows.get_mut(&id) {
                        push(self.sink, row, reading);
                    }
                }
            }
            Done::CsvClosed => {}
        }
    }

    fn started(&mut self, id: RowId, started: Started) {
        let device = self.rows.get(&id).map(|row| row.point.device);
        if started.refused_rpm {
            if let Some(device) = device {
                self.devices.entry(device).or_default().no_rpm = true;
            }
        }
        let Some(row) = self.rows.get_mut(&id) else {
            return;
        };
        row.mac = started.mac.clone();
        let read_first = started.first.is_some();
        if let Some((reading, format)) = started.first {
            row.format = format;
            push(self.sink, row, reading);
            self.outbox.push_back(WatchEvent::Details {
                row: id,
                name: started.name,
                units: row.format.units_name(),
            });
        }
        let leaving = row.removing || self.shutting_down;
        let point = row.point;
        match started.outcome {
            Ok(managed) if leaving => {
                self.rows.remove(&id);
                if let Some(mac) = started.mac {
                    self.cancel(id, point, mac, Some(managed));
                }
            }
            Ok(managed) => {
                row.mode = Mode::Cov(managed);
                row.resubscribe = None;
                row.maybe_subscribed = false;
                let lifetime = self.options.lifetime;
                tracing::info!(
                    "watch row {id}: subscribed to {} on device {} for {lifetime} s",
                    object_label(point.object),
                    point.device
                );
                self.source(id, Source::Cov { lifetime });
            }
            Err(refusal) if leaving => {
                self.rows.remove(&id);
                if let (true, Some(mac)) = (refusal.maybe_subscribed, started.mac) {
                    self.cancel(id, point, mac, None);
                }
            }
            Err(Refusal {
                reason,
                maybe_subscribed,
                fatal,
            }) => {
                tracing::info!("watch row {id}: {reason}");
                if fatal {
                    self.rows.remove(&id);
                    self.source(id, Source::Failed(reason));
                    return;
                }
                row.mode = Mode::Polled;
                row.maybe_subscribed |= maybe_subscribed;
                if read_first {
                    // The first read stands in for a poll.
                    let device = self.devices.entry(point.device).or_default();
                    device.last_polled.get_or_insert_with(Instant::now);
                }
                // A row that was subscribed once keeps trying, ever more
                // slowly; a device that refuses COV outright is only polled.
                if let Some((_, wait)) = row.resubscribe {
                    let wait = backoff(wait, RESUBSCRIBE_BACKOFF);
                    row.resubscribe = Some((Instant::now() + wait, wait));
                }
                self.source(id, Source::Polled(reason));
            }
        }
    }

    /// Fold one receive from `cov_notifications()`. Returns false once the
    /// channel has closed.
    fn notification(&mut self, received: Result<ReceivedCOVNotification, RecvError>) -> bool {
        match received {
            Ok(received) => {
                let id = received.notification.subscriber_process_identifier;
                if let Some(row) = self.rows.get_mut(&id) {
                    if matches!(row.mode, Mode::Starting | Mode::Cov(_)) {
                        if let Some(reading) = sample::from_notification(&row.point, &received) {
                            push(self.sink, row, reading);
                        }
                    }
                }
                true
            }
            Err(RecvError::Lagged(missed)) => {
                self.sink.add_dropped(missed);
                true
            }
            Err(RecvError::Closed) => false,
        }
    }

    /// Hand pending samples to the UI and the CSV.
    fn flush(&mut self) {
        let mut samples = Vec::new();
        for (id, row) in &mut self.rows {
            let source = match row.mode {
                Mode::Cov(_) | Mode::Starting => "cov",
                _ => "polled",
            };
            for (sample, logged, at) in row.pending.drain(..) {
                if let Some(log) = &self.csv {
                    if !log.line(csv_line(&row.point, &sample, &logged, at, source)) {
                        self.sink.add_dropped(1);
                    }
                }
                samples.push((*id, sample));
            }
        }
        if !samples.is_empty() {
            samples.sort_by_key(|(_, sample)| sample.at);
            self.sink
                .offer(WorkerEvent::Watch(WatchEvent::Samples(samples)));
        }
    }

    /// Every tick: hand samples on, notice ended renewals, queue the
    /// subscriptions due again, and report the counters.
    fn tick(&mut self, rate: &RateCap) {
        self.flush();
        let now = Instant::now();
        for (id, row) in &mut self.rows {
            if let (Mode::Polled, Some((at, _)), false) =
                (&row.mode, row.resubscribe, self.shutting_down)
            {
                if at <= now {
                    row.mode = Mode::Queued;
                    let device = self.devices.entry(row.point.device).or_default();
                    device.ops.push_back(Op::Start(*id));
                }
            }
            if let Mode::Cov(managed) = &row.mode {
                if managed.is_finished() {
                    let reason = match managed.last_event() {
                        Some(ManagedCOVSubscriptionEvent::RenewalFailed { error }) => {
                            format!("renewal failed: {error}")
                        }
                        _ => "renewal stopped".to_string(),
                    };
                    let wait = RESUBSCRIBE_BACKOFF.0;
                    tracing::warn!(
                        "watch row {id}: {reason}; polling, and subscribing again in {} s",
                        wait.as_secs()
                    );
                    row.mode = Mode::Polled;
                    row.maybe_subscribed = true;
                    row.resubscribe = Some((now + wait, wait));
                    self.outbox.push_back(WatchEvent::Source {
                        row: *id,
                        source: Source::Polled(reason),
                    });
                }
            }
        }
        let stats = WatchStats {
            subscriptions: self
                .rows
                .values()
                .filter(|row| matches!(row.mode, Mode::Cov(_)))
                .count()
                + self.cancels,
            recent_requests: rate.recent(),
            csv: self.csv.as_ref().map(|log| log.path.display().to_string()),
        };
        if stats != self.stats {
            self.stats = stats.clone();
            // Only the newest counters matter: the outbox holds one.
            self.outbox
                .retain(|event| !matches!(event, WatchEvent::Stats(_)));
            self.outbox.push_back(WatchEvent::Stats(stats));
        }
    }

    /// Queue what waits in the outbox, keeping it when the UI's channel is
    /// full.
    fn send_outbox(&mut self) {
        while let Some(event) = self.outbox.pop_front() {
            if let Some(event) = self.sink.offer_or_keep(WorkerEvent::Watch(event)) {
                if let WorkerEvent::Watch(event) = event {
                    self.outbox.push_front(event);
                }
                break;
            }
        }
    }

    /// Start a job on every idle device that has one: a queued start or
    /// cancel first, then a poll of its due rows.
    fn launch<'a, T: TransportPort + 'static>(
        &mut self,
        client: &'a Arc<BACnetClient<T>>,
        rate: &'a RateCap,
        jobs: &mut FuturesUnordered<Job<'a>>,
    ) {
        for log in self.closing.drain(..) {
            jobs.push(Box::pin(async move {
                log.close().await;
                Done::CsvClosed
            }));
        }
        let now = Instant::now();
        let lifetime = Lifetime::new(self.options.lifetime, self.options.request_budget);
        for (&id, device) in &mut self.devices {
            if device.busy {
                continue;
            }
            let rpm = !device.no_rpm;
            let job: Job<'a> = match device.ops.pop_front() {
                Some(Op::Start(row)) => {
                    let Some(entry) = self.rows.get_mut(&row) else {
                        continue;
                    };
                    entry.mode = Mode::Starting;
                    let point = entry.point;
                    Box::pin(async move {
                        let started = jobs::start(client, rate, row, point, lifetime, rpm).await;
                        Done::Started {
                            row,
                            device: id,
                            started: Box::new(started),
                        }
                    })
                }
                Some(Op::Cancel {
                    row,
                    point,
                    mac,
                    managed,
                }) => Box::pin(async move {
                    let result = jobs::cancel(client, rate, row, point, mac, managed).await;
                    Done::Cancelled {
                        row,
                        device: id,
                        result,
                    }
                }),
                None if self.shutting_down || device.retry_at.is_some_and(|at| at > now) => {
                    continue
                }
                None => {
                    let every = self.options.poll_every;
                    if device
                        .last_polled
                        .is_some_and(|at| now.saturating_duration_since(at) < every)
                    {
                        continue;
                    }
                    let due: Vec<(RowId, Point)> = self
                        .rows
                        .iter()
                        .filter(|(_, row)| {
                            row.point.device == id && matches!(row.mode, Mode::Polled)
                        })
                        .map(|(row_id, row)| (*row_id, row.point))
                        .collect();
                    if due.is_empty() {
                        continue;
                    }
                    Box::pin(async move {
                        let polled = jobs::poll(client, rate, id, due, rpm).await;
                        Done::Polled { device: id, polled }
                    })
                }
            };
            device.busy = true;
            jobs.push(job);
        }
    }

    /// The session is ending: remove every row, which queues the cancels.
    fn shutdown(&mut self) {
        self.shutting_down = true;
        // The CSV gets what arrived since the last tick.
        self.flush();
        let ids: Vec<RowId> = self.rows.keys().copied().collect();
        for id in ids {
            self.remove(id);
        }
        if let Some(log) = self.csv.take() {
            self.closing.push(log);
        }
    }

    /// After the cancel deadline: stop the renewals of cancels that never
    /// ran, so no task keeps the client.
    async fn stop_remaining(&mut self) {
        for device in self.devices.values_mut() {
            for op in device.ops.drain(..) {
                if let Op::Cancel {
                    managed: Some(managed),
                    ..
                } = op
                {
                    managed.stop().await;
                }
            }
        }
        for log in self.closing.drain(..) {
            log.close().await;
        }
    }
}

/// Write a reading out and add it to a row's ring, dropping and counting
/// the oldest when full.
fn push(sink: &EventSink, row: &mut Row, reading: Reading) {
    if row.pending.len() >= PENDING_PER_ROW {
        row.pending.pop_front();
        sink.add_dropped(1);
    }
    let (sample, logged) = row.format.sample(&row.point, reading);
    row.pending.push_back((sample, logged, unix_ms()));
}

/// One CSV line: the value in full (`logged`), not the screen's rounding.
fn csv_line(
    point: &Point,
    sample: &Sample,
    logged: &Result<String, String>,
    at: u128,
    source: &str,
) -> String {
    let value = match logged {
        Ok(value) => value.clone(),
        Err(error) => format!("error: {error}"),
    };
    let flags = sample
        .flags
        .map(|flags| flags.badges().concat())
        .unwrap_or_default();
    format!(
        "{at},{},{},{},{},{},{source},{}",
        point.device,
        field(&object_label(point.object)),
        field(&kebab(point.property)),
        field(&value),
        flags,
        point.network.map(|n| n.to_string()).unwrap_or_default(),
    )
}

/// Serve watch commands until `commands` closes or the session is
/// cancelled, then cancel every subscription.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<Command>,
    rate: &RateCap,
    options: &WatchOptions,
) {
    let mut state = State::new(sink, options);
    let mut notifications = client.cov_notifications();
    let mut notifications_open = true;
    let mut jobs: FuturesUnordered<Job<'_>> = FuturesUnordered::new();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            command = commands.recv() => match command {
                Some(Command::Watch(command)) => state.command(command),
                Some(_) => {}
                None => break,
            },
            Some(done) = jobs.next(), if !jobs.is_empty() => state.done(done),
            received = notifications.recv(), if notifications_open => {
                notifications_open = state.notification(received);
            }
            _ = tick.tick() => state.tick(rate),
        }
        state.launch(client, rate, &mut jobs);
        state.send_outbox();
    }
    state.shutdown();
    state.launch(client, rate, &mut jobs);
    let deadline = Instant::now() + CANCEL_WAIT;
    while !jobs.is_empty() {
        tokio::select! {
            Some(done) = jobs.next() => state.done(done),
            () = tokio::time::sleep_until(deadline) => {
                tracing::warn!(
                    "gave up waiting for COV cancels; those subscriptions end with their lifetime"
                );
                break;
            }
        }
        state.launch(client, rate, &mut jobs);
    }
    drop(jobs);
    state.stop_remaining().await;
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
