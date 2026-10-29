//! The watch list's worker: a front end on the watch hub
//! ([`bacnet_ops::watch`], #2115), which holds the managed COV
//! subscriptions with finite lifetimes, polls what a device won't
//! subscribe, and cancels on removal.
//!
//! Each row holds a lease on its point and reads the point's sample stream,
//! so every value reaches the row's history and the CSV log, whatever the
//! hub folds together. [`run`] is polled beside the session loop under
//! `join!`, like the browser, and its loop's arms never await. When the
//! session ends it shuts the hub down, which cancels every subscription
//! (Clauses 13.14.2 and 13.15.2), waiting at most [`CANCEL_WAIT`] for the
//! devices; the lifetimes bound whatever it couldn't cancel.

pub(crate) mod csv;
pub(crate) mod sample;

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use bacnet_client::client::BACnetClient;
use bacnet_ops::engine::Engine;
use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::watch::{
    self as hub, Backoff, Hub, Lease, Lifetime, MissingDevice, Reads, Samples, Snapshot, Via,
    Watched, Why,
};
use bacnet_transport::port::TransportPort;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use self::csv::{CsvLog, field};
use self::sample::{Reading, RowFormat};
use super::EventSink;
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::watched::{Point, RowId, Sample, Source, WatchCommand, WatchEvent, WatchStats};

/// Default `--cov-lifetime`, in seconds.
pub(crate) const DEFAULT_COV_LIFETIME: u32 = 300;
/// Default `--poll-interval`, in seconds.
pub(crate) const DEFAULT_POLL_SECS: u64 = 10;
/// Rows a session may watch: real devices have small COV tables.
pub(crate) const MAX_ROWS: usize = 100;
/// How often samples and changes go to the UI.
const TICK: Duration = Duration::from_millis(250);
/// Samples a row keeps between ticks; older ones are dropped and counted.
pub(crate) const PENDING_PER_ROW: usize = 32;
/// How long the end of a session waits for its cancels.
pub(crate) const CANCEL_WAIT: Duration = Duration::from_secs(5);
/// A confirmed request with every retry at the CLI's defaults: a 6 s APDU
/// timeout, sent four times.
pub(crate) const DEFAULT_REQUEST_BUDGET: Duration = Duration::from_secs(24);

/// The watch list's settings.
#[derive(Debug, Clone)]
pub(crate) struct WatchOptions {
    /// Subscription lifetime in seconds.
    pub(crate) lifetime: u32,
    /// Least time between two polls of a device.
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

/// The hub's settings for the watch list: what a row shows (its value
/// with Status_Flags, named, with its units and state texts), polls down
/// to a second apart, a silent device polled again after 2 s doubling to a
/// minute, a failed subscription tried again after 30 s doubling to five
/// minutes, and a row whose device isn't in the table given up.
pub(crate) fn hub_options(options: &WatchOptions) -> hub::WatchOptions {
    let lifetime = Lifetime::new(options.lifetime).unwrap_or(Lifetime::DEFAULT);
    hub::WatchOptions {
        lifetime,
        renewal_margin: hub::renewal_margin(lifetime.secs(), options.request_budget),
        poll_every: options.poll_every,
        poll_floor: Duration::from_secs(1),
        poll_backoff: Backoff {
            first: Duration::from_secs(2),
            longest: Duration::from_secs(60),
        },
        retry_cov: Backoff {
            first: Duration::from_secs(30),
            longest: Duration::from_secs(300),
        },
        missing: MissingDevice::Fail,
        reads: Reads::value(),
        samples: PENDING_PER_ROW,
        // The list's own MAX_ROWS is the user's limit. The hub's cap also
        // counts points still cancelling, which the list never counted,
        // and no fixed multiple of the rows bounds those: removing rows on
        // a silent device and adding new ones queues cancels that take up
        // to the cancel's give-up each. So the cap is only a backstop, far
        // above what removing and re-adding rows by hand could reach.
        max_points: MAX_ROWS * 100,
        ..hub::WatchOptions::default()
    }
}

/// A row's point as the hub watches it: Present_Value by SubscribeCOV on
/// the object, any other property by SubscribeCOVProperty.
pub(crate) fn hub_point(point: &Point) -> hub::Point {
    let watched = if point.whole_object() {
        Watched::Object
    } else {
        Watched::Property {
            property: point.property,
            index: None,
        }
    };
    hub::Point {
        device: point.device,
        object: point.object,
        watched,
    }
}

/// What a row's snapshot says its source is: `None` until its start
/// ends, and while the hub is between the steps of one change.
fn source_of(snapshot: &Snapshot, lifetime: u32) -> Option<Source> {
    match (&snapshot.why, snapshot.source) {
        (None, Some(hub::Source::Cov)) => Some(Source::Cov { lifetime }),
        (Some(Why::Missing), _) => snapshot.note.clone().map(Source::Failed),
        (Some(why), Some(hub::Source::Poll)) => Some(Source::Polled(match why {
            Why::Refused(error) => format!("COV refused: {error}"),
            Why::NoAnswer(error) => format!("no answer: {error}"),
            Why::Lapsed(Some(error)) => format!("renewal failed: {error}"),
            Why::Lapsed(None) | Why::Missing => "renewal stopped".to_string(),
        })),
        _ => None,
    }
}

struct Row {
    point: Point,
    lease: Lease,
    samples: Samples,
    /// The source last reported.
    source: Source,
    /// Samples the stream had dropped at the last tick.
    dropped: u64,
    /// How its values are written, from its first read.
    format: RowFormat,
}

type Closing = Pin<Box<dyn Future<Output = ()> + Send>>;

struct State<'s, T: TransportPort + 'static> {
    sink: &'s EventSink,
    options: &'s WatchOptions,
    hub: Hub<T>,
    rows: BTreeMap<RowId, Row>,
    /// Notifications the hub's receiver had lost at the last tick.
    lagged: u64,
    csv: Option<CsvLog>,
    closing: Vec<CsvLog>,
    outbox: VecDeque<WatchEvent>,
    stats: WatchStats,
}

impl<'s, T: TransportPort + 'static> State<'s, T> {
    fn source(&mut self, row: RowId, source: Source) {
        self.outbox.push_back(WatchEvent::Source { row, source });
    }

    fn command(&mut self, command: WatchCommand) {
        match command {
            WatchCommand::Add { row, point } => self.add(row, point),
            WatchCommand::Remove { row } => {
                // The hub cancels what the point holds once its last
                // lease goes.
                self.rows.remove(&row);
            }
            WatchCommand::Csv(on) => self.csv(on),
        }
    }

    fn add(&mut self, id: RowId, point: Point) {
        if self.rows.len() >= MAX_ROWS || self.rows.contains_key(&id) {
            self.source(id, Source::Failed(format!("at most {MAX_ROWS} rows")));
            return;
        }
        match self.hub.watch(hub_point(&point)) {
            Ok(lease) => {
                let samples = lease.samples();
                self.rows.insert(
                    id,
                    Row {
                        point,
                        lease,
                        samples,
                        source: Source::Starting,
                        dropped: 0,
                        format: RowFormat::default(),
                    },
                );
            }
            Err(error) => self.source(id, Source::Failed(error.to_string())),
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

    /// Every tick: hand each row's new values to the UI and the CSV, report
    /// a changed source, count what was lost, and report the counters.
    fn tick(&mut self, rate: &RequestLimits) {
        let lifetime = self.options.lifetime;
        let mut samples = Vec::new();
        let mut gone = Vec::new();
        for (&id, row) in &mut self.rows {
            let snapshot = row.lease.snapshot();
            let polled = snapshot.source == Some(hub::Source::Poll);
            while let Some(sample) = row.samples.try_next() {
                let property = row.point.property;
                let reading = match &sample.values {
                    Ok(rows) => match sample.via {
                        Via::Poll => Some(
                            sample::from_rows(property, rows)
                                .unwrap_or_else(|| Reading::failed(hub::poll::MISSING.into())),
                        ),
                        Via::First | Via::Notification => sample::from_rows(property, rows),
                    },
                    // A first read that failed leaves the row without a
                    // value until the next.
                    Err(_) if sample.via == Via::First => None,
                    Err(error) => Some(Reading::failed(error.clone())),
                };
                let Some(mut reading) = reading else {
                    continue;
                };
                reading.at = sample.at.into_std();
                if let (Via::First, Ok(rows)) = (sample.via, &sample.values) {
                    row.format = RowFormat::from_rows(rows);
                    self.outbox.push_back(WatchEvent::Details {
                        row: id,
                        name: bacnet_ops::format::object_name(rows),
                        units: row.format.units_name(),
                    });
                }
                let source = match sample.via {
                    Via::Notification => "cov",
                    Via::Poll => "polled",
                    Via::First if polled => "polled",
                    Via::First => "cov",
                };
                let at = sample
                    .time
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |since| since.as_millis());
                let (shown, logged) = row.format.sample(&row.point, reading);
                if let Some(log) = &self.csv
                    && !log.line(csv_line(&row.point, &shown, &logged, at, source))
                {
                    self.sink.add_dropped(1);
                }
                samples.push((id, shown));
            }
            let dropped = row.samples.dropped();
            self.sink.add_dropped(dropped - row.dropped);
            row.dropped = dropped;
            if let Some(source) = source_of(&snapshot, lifetime)
                && source != row.source
            {
                match &source {
                    Source::Cov { .. } => tracing::info!(
                        "watch row {id}: subscribed to {} on device {} for {lifetime} s",
                        object_label(row.point.object),
                        row.point.device
                    ),
                    Source::Polled(reason) | Source::Failed(reason) => {
                        tracing::info!("watch row {id}: {reason}");
                    }
                    Source::Starting => {}
                }
                if snapshot.state == hub::State::Gone {
                    // Nothing watches it any more.
                    gone.push(id);
                }
                row.source = source.clone();
                self.outbox
                    .push_back(WatchEvent::Source { row: id, source });
            }
        }
        for id in gone {
            self.rows.remove(&id);
        }
        if !samples.is_empty() {
            samples.sort_by_key(|(_, sample)| sample.at);
            self.sink
                .offer(WorkerEvent::Watch(WatchEvent::Samples(samples)));
        }
        let hub_stats = self.hub.stats();
        self.sink
            .add_dropped(hub_stats.lagged.saturating_sub(self.lagged));
        self.lagged = hub_stats.lagged;
        let stats = WatchStats {
            subscriptions: hub_stats.cov + hub_stats.cancelling,
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
/// cancelled, then shut the hub down, cancelling every subscription.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<Command>,
    rate: &Arc<RequestLimits>,
    options: &WatchOptions,
) {
    let engine = Arc::new(Engine::new(Arc::clone(client), Arc::clone(rate)));
    let hub = match Hub::new(engine, hub_options(options)) {
        Ok(hub) => hub,
        Err(error) => {
            tracing::error!("the watch list can't start: {error}");
            return;
        }
    };
    let mut state = State {
        sink,
        options,
        hub,
        rows: BTreeMap::new(),
        lagged: 0,
        csv: None,
        closing: Vec::new(),
        outbox: VecDeque::new(),
        stats: WatchStats::default(),
    };
    let mut closing: FuturesUnordered<Closing> = FuturesUnordered::new();
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
            Some(()) = closing.next(), if !closing.is_empty() => {}
            _ = tick.tick() => state.tick(rate),
        }
        for log in state.closing.drain(..) {
            closing.push(Box::pin(log.close()));
        }
        state.send_outbox();
    }
    // The CSV gets what arrived since the last tick.
    state.tick(rate);
    state.rows.clear();
    state.hub.shutdown(CANCEL_WAIT).await;
    if let Some(log) = state.csv.take() {
        log.close().await;
    }
    for log in state.closing.drain(..) {
        log.close().await;
    }
    while closing.next().await.is_some() {}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
