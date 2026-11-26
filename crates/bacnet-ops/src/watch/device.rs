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
//! the device's instance went. The address it finds, or one a following
//! handle moved to ([`DeviceSlot::moved`]), is a cue ([`moves`]): a job
//! reads which device answers there, and only one naming the instance
//! gives every pinned point up with [`Why::Moved`] and hands every
//! following one to the scheduler there ([`follow`]): their renewals here
//! stop and nothing more is sent for them here, cancels included, so a
//! subscription still held here lapses with its lifetime. The move doesn't
//! stick for pinned points: a point arriving later starts as any other, and
//! the given-up points start again once the address answers or announces
//! itself for the instance again.
//!
//! Cancels go while the device answers. One it leaves unanswered puts the
//! device in its poll backoff, and while it is there the cancels left are
//! not sent, so dropping every point of a silent device costs at most one
//! cancel's wait.
//!
//! Each point keeps the instance its address named when its watch started,
//! a following point the instance it follows ([`identity`]): another device
//! answering there is never taken for it, and the device announcing itself
//! again is subscribed again early.

mod follow;
mod identity;
mod moves;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_client::client::{ManagedCOVSubscription, ManagedCOVSubscriptionEvent, Station};
use bacnet_transport::port::TransportPort;
use tokio::sync::broadcast::{self, error::TryRecvError};
use tokio::time::Instant;

use super::Why;
use super::hub::{Inner, MISSING, Phase, Source};
use super::jobs::{self, Answer, Outcome, Polled, Skipped, Started};
use super::options::MissingDevice;
use super::sample::Via;
use super::shared::{Arrival, DeviceSlot, FirstSample, Running, Shared};
use super::support::fills_table;
use crate::client::batch::Missing;
use crate::client::{failed_locally, unreachable};
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
    /// Its start was answered while its address was said to hold another
    /// device: it waits for the device there to say which it is
    /// ([`identity`]).
    Held,
    /// Given up ([`Phase::Gone`]).
    Gone,
}

struct Row {
    shared: Arc<Shared>,
    running: Running,
    mode: Mode,
    /// Where its device is, once a start has looked it up.
    target: Option<Station>,
    /// Removed while starting: cancel once the start ends.
    removing: bool,
    /// Given up because the device of this instance answers elsewhere
    /// now, or another answers at its address: started again if its
    /// address answers for that instance again.
    moved: Option<u32>,
    /// A subscription may exist although the point isn't subscribed: a
    /// request that went unanswered, or a renewal that failed.
    maybe_subscribed: bool,
    /// When to start again, and the wait that led there.
    resubscribe: Option<(Instant, Duration)>,
}

enum Op {
    Start(u32),
    /// Ask where the device of an instance went.
    Locate(u32),
    /// Ask which device answers at an address the device of an instance
    /// is said to have moved to ([`moves`]).
    Arrived(u32, DeviceAddress),
    /// Ask whether the device of an instance still answers at the address,
    /// another having named itself so ([`moves`]).
    Left(u32),
    /// Which device answers at the address, or when it restarted.
    Check(identity::Check),
    Cancel {
        shared: Arc<Shared>,
        /// What the cancel keeps counted until it ends.
        holder: Holder,
        target: Station,
        managed: Option<ManagedCOVSubscription>,
    },
}

/// What a cancel holds until it ends.
enum Holder {
    /// The point's row, done once cancelled.
    Row(Box<Row>),
    /// A count of its own: the cancel of an earlier attempt's subscription
    /// for a point that stays (given up, [`Phase::Gone`]).
    Alone(Running),
}

enum Done {
    Started {
        process: u32,
        started: Box<Started>,
    },
    /// A cancel ended; whether the device answered it.
    Cancelled(Holder, bool),
    Polled(Polled),
    /// Where the instance looked for is said to answer now, if anywhere
    /// else.
    Located(u32, Option<DeviceAddress>),
    /// The instance the device at an address named itself, asked for a
    /// move of an instance there.
    Arrived(u32, DeviceAddress, Option<u32>),
    /// What the device at the address answered, asked whether the device
    /// of an instance is still there.
    Left(u32, Answer),
    Checked(identity::Checked),
}

type Job = Pin<Box<dyn Future<Output = Done> + Send>>;

struct Device<T: TransportPort + 'static> {
    inner: Arc<Inner<T>>,
    /// Where the device answers: the scheduler's key.
    address: DeviceAddress,
    /// What the table knows of the scheduler: its inbox, and what was
    /// announced at the address.
    slot: Arc<DeviceSlot>,
    /// The instances whose locate is queued or running.
    locating: BTreeSet<u32>,
    /// Which device answers at the address, and when it restarted.
    ids: identity::Ids,
    /// The reads where the device is said to have moved.
    moves: moves::Moves,
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
        slot: Arc::clone(&slot),
        locating: BTreeSet::new(),
        ids: identity::Ids::default(),
        moves: moves::Moves::default(),
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
            device.cue_move(instance, to, false);
        }
        device.heard();
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
                table.replaced.retain(|_, addresses| {
                    addresses.remove(&device.address);
                    !addresses.is_empty()
                });
                return;
            }
            continue;
        }
        // Unbiased: a flood of wake-ups can't keep the job or the tick
        // unpolled. Any order is safe, as each turn takes in everything
        // noted before it waits, and a wake-up not taken stays stored.
        tokio::select! {
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

    /// Take in points handed to the device.
    fn arrive(&mut self, arrivals: Vec<Arrival>) {
        for Arrival { shared, running } in arrivals {
            let process = shared.process;
            let row = Row {
                shared,
                running,
                mode: Mode::Queued,
                target: None,
                removing: false,
                moved: None,
                maybe_subscribed: false,
                resubscribe: None,
            };
            self.ops.push_back(Op::Start(process));
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
        target: Station,
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
        row.shared.set_state(self.seq(), None, Phase::Stopped, note);
        drop(row.running);
    }

    /// Whether the device is in its backoff: it left a poll or a cancel
    /// unanswered, and isn't due to be polled again yet.
    fn backing_off(&self) -> bool {
        self.retry_at.is_some_and(|at| at > Instant::now())
    }

    /// Start the next job, if the device is idle and one is due: a queued
    /// start or cancel first, then a poll of its polled points. A cancel
    /// due while the device backs off ends without a request.
    fn launch(&mut self) -> Option<Job> {
        let inner = Arc::clone(&self.inner);
        let account = Arc::clone(&inner.account);
        while matches!(self.ops.front(), Some(Op::Cancel { .. })) && self.backing_off() {
            let Some(Op::Cancel { holder, .. }) = self.ops.pop_front() else {
                break;
            };
            // Its managed subscription, dropped with the op, stops renewing;
            // the device's subscription lapses with its lifetime.
            tracing::debug!(
                "device at {} is backing off: a cancel is left to lapse",
                self.address
            );
            self.done(Done::Cancelled(holder, false));
        }
        match self.ops.pop_front() {
            Some(Op::Start(process)) => {
                // The device an answer named for the address's other points
                // is this one's too, unless an answer named one for it.
                let known = self.known();
                let row = self.rows.get_mut(&process)?;
                if known.is_some() {
                    row.shared.learn(known, true);
                }
                row.mode = Mode::Starting;
                let shared = Arc::clone(&row.shared);
                let address = self.address.clone();
                Some(Box::pin(async move {
                    let started = charged(account, jobs::start(inner, shared, address)).await;
                    Done::Started {
                        process,
                        started: Box::new(started),
                    }
                }))
            }
            Some(Op::Locate(instance)) => {
                let address = self.address.clone();
                Some(Box::pin(async move {
                    Done::Located(instance, jobs::locate(inner, address, instance).await)
                }))
            }
            Some(Op::Check(check)) => Some(self.check_job(check)),
            Some(Op::Arrived(instance, to)) => {
                self.arrival_job(instance, to).or_else(|| self.launch())
            }
            Some(Op::Left(instance)) => self.left_job(instance).or_else(|| self.launch()),
            Some(Op::Cancel {
                shared,
                holder,
                target,
                managed,
            }) => {
                let process = shared.process;
                let point = (shared.point.clone(), self.address.clone());
                Some(Box::pin(async move {
                    let answered = jobs::cancel(inner, process, point, target, managed).await;
                    Done::Cancelled(holder, answered)
                }))
            }
            None => {
                let now = Instant::now();
                if self.backing_off()
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
            Done::Cancelled(holder, answered) => {
                self.inner
                    .registry
                    .cancelling
                    .fetch_sub(1, Ordering::Relaxed);
                match holder {
                    Holder::Row(row) => self.finish(*row),
                    Holder::Alone(running) => drop(running),
                }
                if !answered && !self.backing_off() {
                    self.back_off();
                }
            }
            Done::Polled(polled) => self.polled(polled),
            Done::Located(instance, to) => {
                self.locating.remove(&instance);
                if let Some(to) = to {
                    self.cue_move(instance, to, true);
                }
            }
            Done::Arrived(instance, to, found) => self.arrived(instance, to, found),
            Done::Left(instance, answer) => self.left(instance, answer),
            Done::Checked(checked) => self.checked(checked),
        }
    }

    /// Ask where the device of `instance` went, unless that is under way.
    fn locate(&mut self, instance: u32) {
        if self.locating.insert(instance) {
            self.ops.push_back(Op::Locate(instance));
        }
    }

    /// Ask where the devices of the points went: each instance once.
    fn locate_all(&mut self) {
        let instances: BTreeSet<u32> = self
            .rows
            .values()
            .filter(|row| !matches!(row.mode, Mode::Gone))
            .filter_map(|row| row.shared.identity())
            .collect();
        for instance in instances {
            self.locate(instance);
        }
    }

    /// Wait before the next poll, longer each time in a row: the device
    /// left a request unanswered.
    fn back_off(&mut self) {
        self.backoff = self.inner.options.poll_backoff.next(self.backoff);
        self.retry_at = Some(Instant::now() + self.backoff);
    }

    fn started(&mut self, process: u32, started: Started) {
        if self.started_elsewhere(process, &started) {
            return;
        }
        if started.skipped == Some(Skipped::Unconfirmed) {
            self.unconfirmed(process);
            return;
        }
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
        shared.lock().snapshot.cov = started.cov;
        let Some(first) = started.first else {
            // No route to the device is known.
            let address = &self.address;
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
                    shared.set_state(seq, None, Phase::Gone, Some(note));
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
                    shared.set_state(seq, Some(Source::Poll), Phase::Unreachable, Some(note));
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
                    (Some(Source::Poll), Phase::Unreachable)
                } else {
                    shared.lock().failure = Some(PropertyError::from(error));
                    (None, Phase::Failed)
                };
                shared.set_state(seq, source, state, Some(note));
            }
        }
        // A subscription with no answer, one the client itself failed to
        // ask (its link reconnecting, say: no word of the device's), or one
        // refused for a full table, is asked for again; any other refusal
        // isn't.
        let unanswered = matches!(
            &started.outcome,
            Outcome::Asked(Err(e)) if unreachable(e) || failed_locally(e)
        );
        let full = matches!(&started.outcome, Outcome::Asked(Err(e)) if fills_table(e));
        let poll_only = matches!(started.outcome, Outcome::PollOnly);
        let transient = match &started.outcome {
            Outcome::Asked(Err(_)) => unanswered || full,
            Outcome::Refused | Outcome::PollOnly => false,
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
                {
                    let mut entry = shared.lock();
                    entry.snapshot.why = None;
                    // Subscribed: a first read's error no longer stands.
                    entry.failure = None;
                }
                shared.set_state(seq, Some(Source::Cov), Phase::Live, None);
                self.subscribed();
                if read_first {
                    self.answered_at(started.instance);
                }
                return;
            }
            Outcome::Asked(Err(error)) => Some(error.to_string()),
            Outcome::Refused => Some(REFUSED_BEFORE.to_string()),
            Outcome::PollOnly => Some(POLL_ONLY.to_string()),
            Outcome::NotAsked => None,
        };
        lost |= unanswered;
        // An unanswered subscription may have reached the device.
        row.maybe_subscribed |= unanswered;
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
            let state = if unanswered {
                Phase::Unreachable
            } else {
                shared.lock().snapshot.state
            };
            // `Refused` only for the device's own no.
            let (why, note) = if poll_only {
                (Why::PollOnly, error)
            } else if full {
                let note = format!("COV table full: {error}");
                (Why::Full(error), note)
            } else if unanswered {
                let note = format!("no answer to the subscription: {error}");
                (Why::NoAnswer(error), note)
            } else {
                let note = format!("COV refused: {error}");
                (Why::Refused(error), note)
            };
            shared.lock().snapshot.why = Some(why);
            shared.set_state(seq, Some(Source::Poll), state, Some(note));
        }
        if lost {
            if let Some(instance) = shared.identity() {
                self.locate(instance);
            }
        } else if read_first {
            self.answered_at(started.instance);
        }
    }

    fn polled(&mut self, polled: Polled) {
        // Rows answered by another device than their points' are none of
        // theirs: given up, or left for the device there to say.
        let foreign = self.poll_identity(&polled);
        let now = Instant::now();
        self.last_polled = Some(now);
        let unreachable = polled.unreachable();
        if unreachable {
            self.back_off();
            tracing::info!(
                "device at {} isn't answering polls; next try in {} s",
                self.address,
                self.backoff.as_secs()
            );
            self.locate_all();
        } else {
            self.backoff = Duration::ZERO;
            self.retry_at = None;
        }
        let state = if unreachable {
            Phase::Unreachable
        } else {
            Phase::Failed
        };
        let seq = &self.inner.registry.seq;
        let Polled {
            rows,
            stopped,
            unread,
            ..
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
        // A point given up while the poll was out stays so.
        let live =
            |row: &&Row| !matches!(row.mode, Mode::Gone) && !foreign.contains(&row.shared.process);
        for (process, rows) in rows {
            let Some(row) = self.rows.get(&process).filter(live) else {
                continue;
            };
            match rows {
                Ok(rows) => row.shared.read(seq, rows, true, None),
                Err(Missing) => failed(row, MISSING.to_string(), PropertyError::NoValue),
            }
        }
        if let Some(error) = stopped {
            for process in unread {
                if let Some(row) = self.rows.get(&process).filter(live) {
                    failed(row, error.clone(), PropertyError::Other(error.clone()));
                }
            }
        }
    }

    /// Every tick: count renewals, notice ended ones, and queue the starts
    /// due again.
    fn tick(&mut self) {
        let now = Instant::now();
        self.identity_due(now);
        self.moves_due(now);
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
                .set_state(seq, Some(Source::Poll), Phase::Unreachable, Some(why));
        }
    }
}

/// Why a point is polled whose device refused the COV service itself for
/// an earlier point.
const REFUSED_BEFORE: &str = "the device refused this COV service before; not asked again";

/// Why a point of a hub that polls every point is polled.
const POLL_ONLY: &str = "polled: the watch's settings poll every point";
