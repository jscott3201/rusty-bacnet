//! One device's scheduler: a task per watched device that starts, polls
//! and cancels its points, one job at a time.
//!
//! The loop's arms never await: a job (a start, a cancel or a poll of
//! every point due) is one owned future the loop polls beside its wake-up
//! and a tick, so nothing it waits for can sit unpolled while the loop is
//! busy, and it holds no lock across an await. Points arrive through the
//! device's inbox and leave by their stop flag; a point removed while its
//! start runs is cancelled once the start ends. When the device has no
//! points left the task ends, deciding so under the table's lock so no
//! arrival is stranded.
//!
//! The device's polled points are read together, `poll_every` after the
//! last poll ended, and a device that stops answering is polled after a
//! backoff instead. A first read stands in for a poll.
//!
//! When a request goes unanswered (a poll, a first read, a subscription;
//! a point whose renewal failed goes to the next poll), a job asks where
//! the device's instance went; a move found there, or one a following
//! handle made ([`DeviceSlot::moved`]), gives every point up with
//! [`Why::Moved`]. Their renewals stop and nothing more is sent to the old
//! address, cancels included: a subscription still held there lapses with
//! its lifetime.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_client::client::{CovTarget, ManagedCOVSubscription, ManagedCOVSubscriptionEvent};
use bacnet_transport::port::TransportPort;
use tokio::sync::broadcast::{self, error::TryRecvError};
use tokio::time::Instant;

use super::jobs::{self, Outcome, Polled, Started};
use super::options::MissingDevice;
use super::sample::Via;
use super::shared::{Arrival, DeviceSlot, FirstSample, Running, Shared};
use super::{Inner, MISSING, Source, State, Why};
use crate::client::batch::Missing;
use crate::client::unreachable;
use crate::device::DeviceAddress;
use crate::format::PropertyError;
use crate::limits::charged;

/// How often the scheduler looks at renewals and what is due.
const TICK: Duration = Duration::from_millis(250);

enum Mode {
    /// Waiting for the device.
    Queued,
    /// Its start job is running.
    Starting,
    /// Subscribed; the client renews it.
    Cov(
        Box<ManagedCOVSubscription>,
        broadcast::Receiver<ManagedCOVSubscriptionEvent>,
    ),
    /// Polled.
    Polled,
    /// No route to its device is known: looked for again at
    /// `resubscribe`.
    Waiting,
    /// Given up ([`State::Gone`]).
    Gone,
}

struct Row {
    shared: Arc<Shared>,
    running: Running,
    mode: Mode,
    /// Where its device is, once a start has looked it up.
    target: Option<CovTarget>,
    /// Removed while starting: cancel once the start ends.
    removing: bool,
    /// A subscription may exist although the point isn't subscribed: a
    /// request that went unanswered, or a renewal that failed.
    maybe_subscribed: bool,
    /// When to start again, and the wait that led there.
    resubscribe: Option<(Instant, Duration)>,
}

enum Op {
    Start(u32),
    /// Ask where the device went, once its instance is known.
    Locate(u32),
    Cancel {
        shared: Arc<Shared>,
        /// What the cancel keeps counted until it ends.
        holder: Holder,
        target: CovTarget,
        managed: Option<ManagedCOVSubscription>,
    },
}

/// What a cancel holds until it ends.
enum Holder {
    /// The point's row, done once cancelled.
    Row(Box<Row>),
    /// A count of its own: the cancel of an earlier attempt's subscription
    /// for a point that stays (given up, [`State::Gone`]).
    Alone(Running),
}

enum Done {
    Started { process: u32, started: Box<Started> },
    Cancelled(Holder),
    Polled(Polled),
    Located(Option<DeviceAddress>),
}

type Job = Pin<Box<dyn Future<Output = Done> + Send>>;

struct Device<T: TransportPort + 'static> {
    inner: Arc<Inner<T>>,
    /// Where the device answers: the scheduler's key.
    address: DeviceAddress,
    /// The instance it claims, as a start last found it.
    instance: Option<u32>,
    /// A locate is queued or running.
    locating: bool,
    /// Where the device answers now, once a move was found: every point
    /// here is given up, those arriving or starting since included.
    moved_to: Option<DeviceAddress>,
    rows: BTreeMap<u32, Row>,
    ops: VecDeque<Op>,
    /// The device stopped answering polls: the current backoff, and when
    /// to poll it again.
    backoff: Duration,
    retry_at: Option<Instant>,
    /// When its last poll, or a polled point's first read, ended.
    last_polled: Option<Instant>,
}

/// Run `device`'s points until it has none left.
pub(super) async fn run<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    slot: Arc<DeviceSlot>,
) {
    let mut device = Device {
        inner,
        address,
        instance: None,
        locating: false,
        moved_to: None,
        rows: BTreeMap::new(),
        ops: VecDeque::new(),
        backoff: Duration::ZERO,
        retry_at: None,
        last_polled: None,
    };
    let mut job: Option<Job> = None;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let arrivals: Vec<Arrival> = slot.inbox().drain(..).collect();
        device.arrive(arrivals);
        if let Some((instance, to)) = slot.take_moved() {
            // A handle following another device's instance here speaks
            // for this one only when it claims that instance.
            if device.instance.is_none_or(|known| known == instance) {
                device.moved(to);
            }
        }
        device.reap();
        if job.is_none() {
            job = device.launch();
        }
        if job.is_none() && device.rows.is_empty() && device.ops.is_empty() {
            #[cfg(test)]
            {
                let hook = device.inner.registry.before_end.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            // An arrival may have come since the inbox was drained; under
            // the table's lock none can come, so an empty inbox ends it.
            let mut table = device.inner.registry.table();
            if slot.inbox().is_empty() {
                if table
                    .devices
                    .get(&device.address)
                    .is_some_and(|known| Arc::ptr_eq(known, &slot))
                {
                    table.devices.remove(&device.address);
                }
                return;
            }
            continue;
        }
        tokio::select! {
            biased;
            () = slot.wake.notified() => {}
            done = async { job.as_mut().expect("guarded by the arm's condition").await },
                if job.is_some() =>
            {
                job = None;
                device.done(done);
            }
            _ = tick.tick() => device.tick(),
        }
    }
}

impl<T: TransportPort + 'static> Device<T> {
    fn seq(&self) -> &std::sync::atomic::AtomicU64 {
        &self.inner.registry.seq
    }

    /// Take in points handed to the device; once it has moved, given up
    /// at once.
    fn arrive(&mut self, arrivals: Vec<Arrival>) {
        for Arrival { shared, running } in arrivals {
            let process = shared.process;
            let mut row = Row {
                shared,
                running,
                mode: Mode::Queued,
                target: None,
                removing: false,
                maybe_subscribed: false,
                resubscribe: None,
            };
            match &self.moved_to {
                Some(to) => give_up(&mut row, to, self.seq()),
                None => self.ops.push_back(Op::Start(process)),
            }
            self.rows.insert(process, row);
        }
    }

    /// Remove the points whose last lease went.
    fn reap(&mut self) {
        let stopped: Vec<u32> = self
            .rows
            .iter()
            .filter(|(_, row)| !row.removing && row.shared.is_stopped())
            .map(|(process, _)| *process)
            .collect();
        for process in stopped {
            self.remove(process);
        }
    }

    /// Stop watching `process`, cancelling what it holds on the device.
    fn remove(&mut self, process: u32) {
        let Some(row) = self.rows.get_mut(&process) else {
            return;
        };
        match row.mode {
            Mode::Starting => {
                // The start's end decides.
                row.removing = true;
                return;
            }
            Mode::Queued => self
                .ops
                .retain(|op| !matches!(op, Op::Start(start) if *start == process)),
            _ => {}
        }
        let Some(mut row) = self.rows.remove(&process) else {
            return;
        };
        let managed = match std::mem::replace(&mut row.mode, Mode::Gone) {
            Mode::Cov(managed, _) => Some(*managed),
            _ => None,
        };
        self.cancel(row, managed);
    }

    /// Queue `row`'s cancel if it holds or may hold a subscription;
    /// otherwise it is done.
    fn cancel(&mut self, row: Row, managed: Option<ManagedCOVSubscription>) {
        let held = managed.is_some() || row.maybe_subscribed;
        match (held, row.target.clone()) {
            (true, Some(target)) => {
                let shared = Arc::clone(&row.shared);
                self.queue_cancel(&shared, Holder::Row(Box::new(row)), target, managed);
            }
            _ => self.finish(row),
        }
    }

    fn queue_cancel(
        &mut self,
        shared: &Arc<Shared>,
        holder: Holder,
        target: CovTarget,
        managed: Option<ManagedCOVSubscription>,
    ) {
        self.inner
            .registry
            .cancelling
            .fetch_add(1, Ordering::Relaxed);
        self.ops.push_back(Op::Cancel {
            shared: Arc::clone(shared),
            holder,
            target,
            managed,
        });
    }

    /// `row` is done: say so, and count it out.
    fn finish(&self, row: Row) {
        let note = row.shared.lock().snapshot.note.clone();
        row.shared.set_state(self.seq(), None, State::Stopped, note);
        drop(row.running);
    }

    /// Start the next job, if the device is idle and one is due: a queued
    /// start or cancel first, then a poll of its polled points.
    fn launch(&mut self) -> Option<Job> {
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        match self.ops.pop_front() {
            Some(Op::Start(process)) => {
                let row = self.rows.get_mut(&process)?;
                row.mode = Mode::Starting;
                let shared = Arc::clone(&row.shared);
                Some(Box::pin(async move {
                    let started = charged(account, jobs::start(inner, shared)).await;
                    Done::Started {
                        process,
                        started: Box::new(started),
                    }
                }))
            }
            Some(Op::Locate(instance)) => {
                let address = self.address.clone();
                Some(Box::pin(async move {
                    Done::Located(jobs::locate(inner, address, instance).await)
                }))
            }
            Some(Op::Cancel {
                shared,
                holder,
                target,
                managed,
            }) => {
                let process = shared.process;
                let point = shared.point.clone();
                Some(Box::pin(async move {
                    jobs::cancel(inner, process, point, target, managed).await;
                    Done::Cancelled(holder)
                }))
            }
            None => {
                let now = Instant::now();
                if self.retry_at.is_some_and(|at| at > now)
                    || self.last_polled.is_some_and(|at| {
                        now.saturating_duration_since(at) < inner.options.poll_every
                    })
                {
                    return None;
                }
                let reads = &inner.options.reads;
                let due: Vec<_> = self
                    .rows
                    .iter()
                    .filter(|(_, row)| matches!(row.mode, Mode::Polled))
                    .map(|(process, row)| jobs::poll_row(reads, *process, &row.shared.point))
                    .collect();
                if due.is_empty() {
                    return None;
                }
                let address = self.address.clone();
                Some(Box::pin(async move {
                    Done::Polled(charged(account, jobs::poll(inner, address, due)).await)
                }))
            }
        }
    }

    fn done(&mut self, done: Done) {
        match done {
            Done::Started { process, started } => self.started(process, *started),
            Done::Cancelled(holder) => {
                self.inner
                    .registry
                    .cancelling
                    .fetch_sub(1, Ordering::Relaxed);
                match holder {
                    Holder::Row(row) => self.finish(*row),
                    Holder::Alone(running) => drop(running),
                }
            }
            Done::Polled(polled) => self.polled(polled),
            Done::Located(to) => {
                self.locating = false;
                if let Some(to) = to {
                    self.moved(to);
                }
            }
        }
    }

    /// Ask where the device went, unless that is under way or its instance
    /// isn't known.
    fn locate(&mut self) {
        if let (false, Some(instance)) = (self.locating, self.instance) {
            self.locating = true;
            self.ops.push_back(Op::Locate(instance));
        }
    }

    /// The device answers at `to` now: give every point up, stopping its
    /// renewals and sending nothing more to this address. A point whose
    /// start is running is given up when it ends.
    fn moved(&mut self, to: DeviceAddress) {
        if self.moved_to.is_some() {
            return;
        }
        tracing::info!(
            "device at {} answers at {to} now; its watches end",
            self.address
        );
        self.ops.retain(|op| !matches!(op, Op::Start(_)));
        let seq = &self.inner.registry.seq;
        for row in self.rows.values_mut() {
            if !matches!(row.mode, Mode::Gone | Mode::Starting) && !row.removing {
                give_up(row, &to, seq);
            }
        }
        self.moved_to = Some(to);
    }

    fn started(&mut self, process: u32, started: Started) {
        let missing = self.inner.options.missing;
        let poll_backoff = self.inner.options.poll_backoff;
        let retry_cov = self.inner.options.retry_cov;
        let seq = &self.inner.registry.seq;
        let Some(row) = self.rows.get_mut(&process) else {
            return;
        };
        let shared = Arc::clone(&row.shared);
        let leaving = row.removing || shared.is_stopped();
        // The job noted the address for the receiver. A retry can fail
        // before finding the device; keep the address a cancel of an
        // earlier attempt's subscription needs.
        if let Some(target) = &started.target {
            row.target = Some(target.clone());
        }
        if started.instance.is_some() {
            self.instance = started.instance;
        }
        shared.lock().snapshot.cov = started.cov;
        if let Some(to) = &self.moved_to {
            // Moved while this start ran: nothing more goes to this
            // address, so what the start made lapses with its lifetime, as
            // the device's other points' subscriptions do.
            if leaving {
                let row = self.rows.remove(&process).expect("looked up above");
                self.finish(row);
            } else {
                give_up(row, to, seq);
            }
            return;
        }
        let Some(first) = started.first else {
            // No route to the device is known.
            let address = &shared.point.device;
            let note = format!(
                "no route to {address}: the discovery table knows no router to network {}",
                address.network()
            );
            if leaving {
                let row = self.rows.remove(&process).expect("looked up above");
                self.cancel(row, None);
                return;
            }
            match missing {
                MissingDevice::Fail => {
                    row.mode = Mode::Gone;
                    row.resubscribe = None;
                    let prior = std::mem::take(&mut row.maybe_subscribed);
                    shared.lock().snapshot.why = Some(Why::Missing);
                    shared.set_state(seq, None, State::Gone, Some(note));
                    if let (true, Some(target)) = (prior, row.target.clone()) {
                        // An earlier attempt may have subscribed. The
                        // cancel counts as running on its own, so a
                        // shutdown waits for it as for any other.
                        let running = Running::new(&self.inner.registry);
                        self.queue_cancel(&shared, Holder::Alone(running), target, None);
                    }
                }
                MissingDevice::Poll => {
                    let previous = match row.mode {
                        Mode::Starting => row.resubscribe.map_or(Duration::ZERO, |(_, w)| w),
                        _ => Duration::ZERO,
                    };
                    let wait = poll_backoff.next(previous);
                    row.mode = Mode::Waiting;
                    row.resubscribe = Some((Instant::now() + wait, wait));
                    shared.lock().snapshot.why = Some(Why::Missing);
                    shared.set_state(seq, Some(Source::Poll), State::Unreachable, Some(note));
                }
            }
            return;
        };
        let read_first = first.is_ok();
        let read_at = started
            .read_at
            .unwrap_or_else(|| (Instant::now(), std::time::SystemTime::now()));
        let first_at = read_at.0;
        // Handed on as this returns, whichever way.
        let _first_sample = FirstSample {
            shared: &shared,
            values: Some(
                first
                    .as_ref()
                    .map(Clone::clone)
                    .map_err(ToString::to_string),
            ),
            read_at,
        };
        let mut lost = false;
        match first {
            Ok(rows) => {
                shared.lock().snapshot.note = None;
                shared.read(seq, rows, false, Some(first_at));
            }
            Err(error) => {
                let note = error.to_string();
                let (source, state) = if unreachable(&error) {
                    lost = true;
                    shared.lock().snapshot.why = Some(Why::NoAnswer(note.clone()));
                    (Some(Source::Poll), State::Unreachable)
                } else {
                    shared.lock().failure = Some(PropertyError::from(error));
                    (None, State::Failed)
                };
                shared.set_state(seq, source, state, Some(note));
            }
        }
        // A failure for want of an answer retries; a refusal doesn't.
        let transient = match &started.outcome {
            Outcome::Asked(Err(error)) => unreachable(error),
            Outcome::Refused => false,
            Outcome::Asked(Ok(_)) | Outcome::NotAsked => true,
        };
        // `None` when nothing was asked: the first read had no answer.
        let refusal = match started.outcome {
            Outcome::Asked(Ok(managed)) if leaving => {
                let row = self.rows.remove(&process).expect("looked up above");
                self.cancel(row, Some(managed));
                return;
            }
            Outcome::Asked(Ok(managed)) => {
                let events = managed.events();
                row.mode = Mode::Cov(Box::new(managed), events);
                row.resubscribe = None;
                row.maybe_subscribed = false;
                shared.lock().snapshot.why = None;
                shared.set_state(seq, Some(Source::Cov), State::Live, None);
                return;
            }
            Outcome::Asked(Err(error)) => Some(error.to_string()),
            Outcome::Refused => Some(REFUSED_BEFORE.to_string()),
            Outcome::NotAsked => None,
        };
        lost |= refusal.is_some() && transient;
        // An unanswered subscription may have reached the device.
        row.maybe_subscribed |= refusal.is_some() && transient;
        if leaving {
            let row = self.rows.remove(&process).expect("looked up above");
            self.cancel(row, None);
            return;
        }
        row.mode = Mode::Polled;
        if read_first {
            // The first read stands in for a poll.
            self.last_polled.get_or_insert_with(Instant::now);
        }
        // Transient failures retry; a refusal stops retries without
        // forgetting a subscription an earlier attempt may have made.
        row.resubscribe = transient.then(|| {
            let previous = row.resubscribe.map_or(Duration::ZERO, |(_, w)| w);
            let wait = retry_cov.next(previous);
            (Instant::now() + wait, wait)
        });
        if let Some(error) = refusal {
            let state = if transient {
                State::Unreachable
            } else {
                shared.lock().snapshot.state
            };
            shared.lock().snapshot.why = Some(Why::Refused(error.clone()));
            shared.set_state(
                seq,
                Some(Source::Poll),
                state,
                Some(format!("COV refused: {error}")),
            );
        }
        if lost {
            self.locate();
        }
    }

    fn polled(&mut self, polled: Polled) {
        let now = Instant::now();
        self.last_polled = Some(now);
        let unreachable = polled.unreachable();
        if unreachable {
            self.backoff = self.inner.options.poll_backoff.next(self.backoff);
            self.retry_at = Some(now + self.backoff);
            tracing::info!(
                "device at {} isn't answering polls; next try in {} s",
                self.address,
                self.backoff.as_secs()
            );
            self.locate();
        } else {
            self.backoff = Duration::ZERO;
            self.retry_at = None;
        }
        let state = if unreachable {
            State::Unreachable
        } else {
            State::Failed
        };
        let seq = &self.inner.registry.seq;
        let Polled {
            rows,
            stopped,
            unread,
        } = polled;
        let stopped = stopped.map(|error| error.to_string());
        let failed = |row: &Row, error: String, failure: PropertyError| {
            let shared = &row.shared;
            if !unreachable {
                shared.lock().failure = Some(failure);
            }
            shared.set_state(seq, Some(Source::Poll), state, Some(error.clone()));
            shared.sample(Err(error), Via::Poll);
        };
        for (process, rows) in rows {
            let Some(row) = self.rows.get(&process) else {
                continue;
            };
            match rows {
                Ok(rows) => row.shared.read(seq, rows, true, None),
                Err(Missing) => failed(row, MISSING.to_string(), PropertyError::NoValue),
            }
        }
        if let Some(error) = stopped {
            for process in unread {
                if let Some(row) = self.rows.get(&process) {
                    failed(row, error.clone(), PropertyError::Other(error.clone()));
                }
            }
        }
    }

    /// Every tick: count renewals, notice ended ones, and queue the starts
    /// due again.
    fn tick(&mut self) {
        let now = Instant::now();
        let retry_cov = self.inner.options.retry_cov;
        let seq = &self.inner.registry.seq;
        for (process, row) in &mut self.rows {
            if matches!(row.mode, Mode::Polled | Mode::Waiting)
                && !row.shared.is_stopped()
                && row.resubscribe.is_some_and(|(at, _)| at <= now)
            {
                row.mode = Mode::Queued;
                self.ops.push_back(Op::Start(*process));
                continue;
            }
            let Mode::Cov(managed, events) = &mut row.mode else {
                continue;
            };
            loop {
                match events.try_recv() {
                    Ok(ManagedCOVSubscriptionEvent::Renewed { .. }) => {
                        row.shared.lock().snapshot.renewals += 1;
                    }
                    Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            }
            if !managed.is_finished() {
                continue;
            }
            let failed = match managed.last_event() {
                Some(ManagedCOVSubscriptionEvent::RenewalFailed { error }) => {
                    Some(error.to_string())
                }
                _ => None,
            };
            let why = match &failed {
                Some(error) => format!("COV renewal failed: {error}"),
                None => "COV renewal stopped".to_string(),
            };
            let wait = retry_cov.first;
            tracing::info!(
                "watch of {}: {why}; polling, and subscribing again in {} s",
                row.shared.point,
                wait.as_secs()
            );
            row.mode = Mode::Polled;
            // The failed renewal may have reached the device.
            row.maybe_subscribed = true;
            row.resubscribe = Some((now + wait, wait));
            row.shared.lock().snapshot.why = Some(Why::Lapsed(failed));
            row.shared
                .set_state(seq, Some(Source::Poll), State::Unreachable, Some(why));
        }
    }
}

/// Give `row` up: its device answers at `to` now. Its managed
/// subscription, dropped, stops renewing, and nothing is sent for it: what
/// the old address may hold lapses with its lifetime.
fn give_up(row: &mut Row, to: &DeviceAddress, seq: &std::sync::atomic::AtomicU64) {
    row.mode = Mode::Gone;
    row.maybe_subscribed = false;
    row.resubscribe = None;
    row.shared.lock().snapshot.why = Some(Why::Moved { to: to.clone() });
    let note = format!("the device answers at {to} now; watch it there");
    row.shared.set_state(seq, None, State::Gone, Some(note));
}

/// Why a point is polled whose device refused the COV service itself for
/// an earlier point.
const REFUSED_BEFORE: &str = "the device refused this COV service before; not asked again";
