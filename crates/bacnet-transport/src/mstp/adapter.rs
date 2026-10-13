//! The Tokio adapter that drives the portable master core.
//!
//! The transport's task owns the [`MasterCore`], its receive storage and a
//! boxed [`ExtendedActionBuffers`]; the core borrows the two buffers for as
//! long as the task's future lives, so they go away with the task, whether
//! it ends or is aborted. The core's queues are shared with the send calls
//! (see the `queues` module).
//!
//! # Extended frames
//!
//! The action buffers, and so the core's `NPDU` parameter, are always sized
//! for extended frames, whatever the configuration: on a host they cost only
//! memory, and one core type serves both settings. What changes behaviour
//! follows [`MstpConfig::extended_frames`](super::MstpConfig::extended_frames):
//! the core's flag, the receive storage (a standard frame's, or an extended
//! one's), and the queues' slot size, which is what refuses an NPDU too long
//! to send at the send call.
//!
//! # Input: the octet path
//!
//! Every octet a serial read returns goes to [`MasterCore::on_octet`],
//! stamped with the time that read returned, and the core's own stream
//! decoder assembles the frames: the same receive path the simulator and the
//! fuzzers exercise. A host learns of octets in chunks, after they arrived,
//! so a stamp can only be late. A late stamp moves the line's last activity
//! later, which can only defer this station's own sends and lengthen its
//! silence timers: the safe direction. The reply window the core measures
//! from a late stamp is covered by its reply transmit margin. Chunk gaps
//! inside a frame are covered by a longer T_frame_abort
//! ([`HOST_FRAME_ABORT_MS`](super::HOST_FRAME_ABORT_MS)).
//!
//! # Deadlines
//!
//! The task sleeps until the earliest of [`MasterCore::next_deadline`]
//! (mapped onto Tokio time from the moment the transport started), the echo
//! filter's next deadline and the first expiry in the deferred reply set
//! (see below). It stamps [`MasterCore::on_deadline`] with the time it
//! actually woke. A deadline serviced a millisecond or more late counts in
//! the core's `deadline_misses`, published in
//! [`MstpDiagnosticsSnapshot::mac`](super::MstpDiagnosticsSnapshot::mac).
//! Reads are served first when both are ready, and a deadline that is
//! already due is serviced right after each read, so a port that is always
//! readable cannot hold a deadline back.
//!
//! # Sending a frame
//!
//! After every call into the core the task polls its actions until none is
//! left. `SetDirection` needs nothing here: every [`SerialPort`] switches
//! its own transceiver as it writes. The `transmit` module sends each frame
//! the core hands over, following the core's transmit contract, and strips
//! its echo on a port that has one.
//!
//! # Answering a request
//!
//! A data-expecting-reply frame for this station goes up with a one-use
//! reply channel. An answer through it goes to
//! [`MasterCore::complete_request`]; a dropped channel asks the core for
//! Reply Postponed. Once the core has finished the request without the
//! answer (Reply Postponed at its decision deadline, or the requester may
//! have stopped waiting), the channel is kept in a small deferred set, and
//! an answer that comes later is queued as an ordinary NPDU to the
//! requester, to go out when this station holds the token, as Clause
//! 9.5.6.9 has a deferred reply do. It keeps its request's expiry (arrival
//! plus the queue's maximum age) in the queue. The `replies` module bounds
//! the set; the task wakes at each channel's expiry to drop it, so an idle
//! station holds no channel past its age.
//!
//! # End of file
//!
//! A read that returns no octets is the port's end of file (see
//! [`SerialPort::read`]). The task lets other work run after one, and ends
//! at the [`END_OF_FILE_READS`]th in a row, as it does at a read error: it
//! counts it in `serial_read_errors` and logs it, and its end closes the
//! NPDU channel and refuses later sends. Reads that return octets in
//! between start the count again.

use std::future::Future;
use std::ops::Range;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use bacnet_mstp_codec::{
    FrameType, BROADCAST_MAC, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA,
    MAX_STANDARD_FRAME_LENGTH,
};
use bacnet_mstp_core::{
    Action, CoreError, Direction, ExtendedActionBuffers, MasterCore, MasterState, MstpCoreConfig,
    QueueRefusal, ReplyDecision, RequestId, TransmitId,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::diagnostics::{increment, Counters};
use super::echo::{EchoFilter, Ended, Pushed};
use super::queues::{CoreClock, CoreQueues, EnqueueError, SharedQueues};
use super::replies::{Finished, Late, Replies, Resolved};
use super::{Echo, SerialPort};
use crate::port::{ReceivedNpdu, TransportProvenance};

mod transmit;

/// Octets asked for in one serial read.
const READ_CHUNK: usize = 512;

/// Reads in a row that return no octets after which the task takes the
/// port to be at its end of file. More than one, so that a single empty read
/// from a port that breaks the contract does not end it.
pub(super) const END_OF_FILE_READS: u32 = 3;

type Core<'a> = MasterCore<'a, CoreQueues, MAX_EXTENDED_MPDU_DATA>;

/// The transport's task: everything it needs, moved in at start.
pub(super) struct Task<S> {
    pub(super) serial: S,
    pub(super) config: MstpCoreConfig,
    pub(super) queues: SharedQueues,
    pub(super) clock: CoreClock,
    pub(super) npdus: mpsc::Sender<ReceivedNpdu>,
    pub(super) counts: Arc<Counters>,
    /// [`MstpConfig::transmit_latency_pad`](super::MstpConfig::transmit_latency_pad).
    pub(super) transmit_pad: Duration,
}

impl<S: SerialPort> Task<S> {
    /// Run the core until the serial port fails or the task is aborted.
    pub(super) async fn run(self) {
        let Task {
            serial,
            config,
            queues,
            clock,
            npdus,
            counts,
            transmit_pad,
        } = self;
        let _closed = CloseOnExit(queues.clone());
        let mut rx_storage = vec![
            0u8;
            if config.extended_frames {
                MAX_EXTENDED_FRAME_LENGTH
            } else {
                MAX_STANDARD_FRAME_LENGTH
            }
        ];
        let mut buffers: Box<ExtendedActionBuffers> = Box::default();
        let core_queues = queues.for_core();
        let mut core = match MasterCore::new(
            config,
            core_queues,
            &mut rx_storage,
            &mut buffers,
            clock.now(),
        ) {
            Ok(core) => core,
            Err(error) => {
                warn!(?error, "MS/TP core refused its configuration");
                return;
            }
        };
        let max_age = Duration::from(config.queue_max_age);
        let mut mac = Mac {
            echo: serial.echo(),
            serial,
            clock,
            npdus,
            counts,
            queues,
            octet_time: Duration::from(config.octet_time()),
            transmit_pad,
            read_buf: vec![0; READ_CHUNK],
            tx_frame: Vec::with_capacity(MAX_EXTENDED_FRAME_LENGTH),
            outgoing: None,
            replies: Replies::new(max_age),
            filter: EchoFilter::new(Duration::from_millis(super::HOST_FRAME_ABORT_MS)),
            empty_reads: 0,
        };
        mac.run(&mut core).await;
    }
}

/// Refuses sends once the task is gone, however it ends.
struct CloseOnExit(SharedQueues);

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// The serial port failed a read, or reached its end of file; the task
/// ends.
struct ReadFailed;

/// What a frame being sent counts as in the diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SentKind {
    DataExpectingReply,
    DirectAnswer,
    QueuedData,
    ReplyPostponed,
    Other,
}

impl SentKind {
    /// Classify an encoded frame; `answering` says the core is answering a
    /// request, so a data frame is that answer.
    fn of(frame: &[u8], answering: bool) -> Self {
        match frame.get(2).copied().map(FrameType::from_raw) {
            Some(
                FrameType::BACNET_DATA_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            ) => Self::DataExpectingReply,
            Some(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            ) if answering => Self::DirectAnswer,
            Some(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            ) => Self::QueuedData,
            Some(FrameType::REPLY_POSTPONED) => Self::ReplyPostponed,
            _ => Self::Other,
        }
    }
}

/// A frame the core handed over that has not started.
struct Outgoing {
    id: TransmitId,
    kind: SentKind,
}

/// An action copied out of the core, so its borrow ends.
enum Polled {
    Idle,
    Receive,
    Transmit(TransmitId, SentKind),
    Deliver {
        source: u8,
        destination: u8,
        npdu: Bytes,
        request: Option<RequestId>,
    },
    Frame(FrameType, u8),
}

/// The adapter's state besides the core.
struct Mac<S> {
    /// First, so that it drops first: fields drop in declaration order, and
    /// when the task ends or is aborted, an application holding a reply
    /// sender must find its channel closed by the time it can see the NPDU
    /// channel close or the serial port go.
    replies: Replies,
    serial: S,
    echo: Echo,
    clock: CoreClock,
    npdus: mpsc::Sender<ReceivedNpdu>,
    counts: Arc<Counters>,
    /// The core's queues, for late answers (queued as a send call queues).
    queues: SharedQueues,
    octet_time: Duration,
    /// Added to a frame's wire time when the port has no drain.
    transmit_pad: Duration,
    read_buf: Vec<u8>,
    /// The octets of the frame handed over last.
    tx_frame: Vec<u8>,
    outgoing: Option<Outgoing>,
    filter: EchoFilter,
    /// Reads in a row that returned no octets.
    empty_reads: u32,
}

impl<S: SerialPort> Mac<S> {
    async fn run(&mut self, core: &mut Core<'_>) {
        loop {
            self.settle(core);
            if self.outgoing.is_some() {
                if self.transmit(core).await.is_err() {
                    return;
                }
                continue;
            }
            let wake = [
                core.next_deadline().map(|at| self.clock.instant(at)),
                self.filter.deadline(),
                self.replies.next_expiry(),
            ]
            .into_iter()
            .flatten()
            .min();
            let sleep =
                wake.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3_600));
            // Input before a deadline at the same moment: the core judges
            // the deadline against the activity it has been given.
            tokio::select! {
                biased;
                resolved = self.replies.resolved(), if self.replies.waiting() => match resolved {
                    Resolved::Current(answer) => self.answer(core, answer),
                    Resolved::Late(late) => self.late(late),
                },
                read = self.serial.read(&mut self.read_buf) => {
                    match self.outcome(read) {
                        // Nothing read, short of the end of file: let other
                        // work, an abort included, run before the next read.
                        Ok(0) => tokio::task::yield_now().await,
                        Ok(count) => {
                            let now = self.clock.now();
                            self.receive(core, 0..count, now);
                        }
                        Err(ReadFailed) => return,
                    }
                    self.timers(core);
                }
                () = tokio::time::sleep_until(sleep), if wake.is_some() => self.timers(core),
            }
        }
    }

    /// Carry out the core's actions, defer a finished request's reply
    /// channel, and add the core's counters to the diagnostics' totals.
    fn settle(&mut self, core: &mut Core<'_>) {
        self.drain_actions(core);
        let finished = self
            .replies
            .current_id()
            .is_some_and(|id| core.snapshot().request_id != Some(id));
        if finished {
            self.finish_request();
        }
        let expired = self.replies.prune(tokio::time::Instant::now());
        for _ in 0..expired {
            increment(&self.counts.late_answers_expired);
        }
        self.counts.publish(|| core.take_counters());
    }

    /// Poll the core until it has nothing left.
    fn drain_actions(&mut self, core: &mut Core<'_>) {
        loop {
            match self.poll(core) {
                Polled::Idle => return,
                Polled::Receive => {
                    // Back to receive before the start: the core took the
                    // frame back, so it is not sent.
                    if self.outgoing.take().is_some() {
                        debug!("MS/TP core took back a frame before it started");
                    }
                }
                Polled::Transmit(id, kind) => self.outgoing = Some(Outgoing { id, kind }),
                Polled::Deliver {
                    source,
                    destination,
                    npdu,
                    request,
                } => self.deliver(source, destination, npdu, request),
                Polled::Frame(frame_type, source) => {
                    debug!(
                        ?frame_type,
                        source, "MS/TP frame without an NPDU not passed up"
                    );
                }
            }
        }
    }

    fn poll(&mut self, core: &mut Core<'_>) -> Polled {
        loop {
            let answering = core.state() == MasterState::AnswerDataRequest;
            return match core.poll_action() {
                Action::Idle => Polled::Idle,
                Action::SetDirection(Direction::Receive) => Polled::Receive,
                // Every SerialPort switches its transceiver as it writes,
                // and writes only once the start is confirmed.
                Action::SetDirection(Direction::Transmit) => continue,
                Action::Transmit { id, frame } => {
                    self.tx_frame.clear();
                    self.tx_frame.extend_from_slice(frame);
                    Polled::Transmit(id, SentKind::of(frame, answering))
                }
                Action::DeliverNpdu {
                    source,
                    destination,
                    npdu,
                    request_id,
                } => Polled::Deliver {
                    source,
                    destination,
                    npdu: Bytes::copy_from_slice(npdu),
                    request: request_id,
                },
                Action::DeliverFrame {
                    frame_type, source, ..
                } => Polled::Frame(frame_type, source),
            };
        }
    }

    /// Hand an NPDU up; a request gets a one-use reply channel.
    fn deliver(&mut self, source: u8, destination: u8, npdu: Bytes, request: Option<RequestId>) {
        let reply_tx = request.map(|id| {
            // The core holds one request at a time, so an older one is over.
            self.finish_request();
            self.replies.open(id, source, tokio::time::Instant::now())
        });
        let result = self.npdus.try_send(ReceivedNpdu {
            direct_response: None,
            npdu,
            source_mac: MacAddr::from_slice(&[source]),
            link_layer_group: destination == BROADCAST_MAC,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx,
        });
        // A request that could not go up drops its reply sender, which the
        // next turn of the loop takes for Reply Postponed.
        self.counts.delivery(&result);
    }

    /// The core finished the current request without its answer: keep the
    /// channel for a late answer, or queue one already there.
    fn finish_request(&mut self) {
        match self.replies.finish_current() {
            Some(Finished::Answer {
                requester,
                answer,
                expires,
            }) => self.enqueue_late_answer(requester, &answer, expires),
            Some(Finished::Deferred { dropped_oldest }) => {
                if dropped_oldest {
                    increment(&self.counts.late_answers_dropped);
                }
            }
            Some(Finished::Closed) | None => {}
        }
    }

    /// A deferred channel resolved.
    fn late(&mut self, late: Late) {
        match late {
            Late::Answer {
                requester,
                answer,
                expires,
            } => self.enqueue_late_answer(requester, &answer, expires),
            Late::Expired => increment(&self.counts.late_answers_expired),
            Late::Closed => {}
        }
    }

    /// The application answered the current request, or dropped its channel.
    fn answer(&mut self, core: &mut Core<'_>, answer: Option<Bytes>) {
        let Some(current) = self.replies.take_current() else {
            return;
        };
        let now = self.clock.now();
        let Some(answer) = answer else {
            if let Err(error) = core.complete_request(current.id, ReplyDecision::Postponed, now) {
                debug!(?error, "MS/TP request already finished");
            }
            return;
        };
        match core.complete_request(current.id, ReplyDecision::Data(&answer), now) {
            Ok(()) => {}
            Err(CoreError::ReplyOversize { length, maximum }) => {
                increment(&self.counts.outbound_oversize);
                warn!("MS/TP application reply length {length} exceeds the port's maximum NPDU {maximum}");
                // No answer can be sent, so the request is postponed.
                let _ = core.complete_request(current.id, ReplyDecision::Postponed, now);
            }
            // Past the decision deadline, or the request is gone: the core
            // sends or has sent Reply Postponed, and the answer follows with
            // the token.
            Err(_) => self.enqueue_late_answer(current.requester, &answer, current.expires),
        }
    }

    /// Queue an answer the core no longer takes, as a send call would queue
    /// it, so it goes out when this station holds the token. It expires at
    /// its request's arrival plus the queue's maximum age, not that age
    /// after now.
    fn enqueue_late_answer(&mut self, requester: u8, answer: &[u8], expires: tokio::time::Instant) {
        let expires = self.clock.at(expires);
        let result = self
            .queues
            .enqueue_until(requester, answer, self.clock.now(), expires);
        match result {
            Ok(()) => increment(&self.counts.late_answers_queued),
            Err(EnqueueError::Rejected(QueueRefusal::Oversize)) => {
                increment(&self.counts.outbound_oversize);
            }
            Err(EnqueueError::Rejected(QueueRefusal::Full)) => {
                increment(&self.counts.outbound_queue_full);
            }
            Err(error) => debug!(?error, "MS/TP late answer not queued"),
        }
    }

    /// Feed the octets `range` of the read buffer to the core through the
    /// echo filter, all stamped `now`, carrying out the actions after each.
    fn receive(
        &mut self,
        core: &mut Core<'_>,
        range: Range<usize>,
        now: bacnet_mstp_core::Instant,
    ) {
        let read_at = tokio::time::Instant::now();
        let ended = self.filter.timer(read_at);
        self.hunt_ended(core, ended, now);
        for index in range {
            let octet = self.read_buf[index];
            let pushed = self.filter.push(octet, read_at);
            self.pass_on(core, pushed, octet, now);
        }
    }

    /// Carry out what the echo filter made of one octet: count, and feed
    /// the core what passes on.
    fn pass_on(
        &mut self,
        core: &mut Core<'_>,
        pushed: Pushed,
        octet: u8,
        now: bacnet_mstp_core::Instant,
    ) {
        if pushed.stale > 0 {
            increment(&self.counts.echo_fragments_dropped);
        }
        if pushed.first_mismatch {
            increment(&self.counts.echo_mismatches);
        }
        self.release(core, pushed.release, now);
        if pushed.pass {
            self.feed(core, octet, now);
        }
    }

    /// Carry out the end of a grace, a probe or a hunt.
    fn hunt_ended(&mut self, core: &mut Core<'_>, ended: Ended, now: bacnet_mstp_core::Instant) {
        if ended.missing {
            increment(&self.counts.echo_missing);
        }
        if ended.stale > 0 {
            increment(&self.counts.echo_fragments_dropped);
        }
        self.release(core, ended.release, now);
    }

    /// Feed the first `held` octets the echo filter held back.
    fn release(&mut self, core: &mut Core<'_>, held: usize, now: bacnet_mstp_core::Instant) {
        for index in 0..held {
            let octet = self.filter.held(held)[index];
            self.feed(core, octet, now);
        }
    }

    /// Feed one octet to the core and carry out the actions.
    fn feed(&mut self, core: &mut Core<'_>, octet: u8, now: bacnet_mstp_core::Instant) {
        if let Err(error) = core.on_octet(octet, now) {
            debug!(?error, "MS/TP core refused an octet");
        }
        self.drain_actions(core);
    }

    /// Service whatever timer is due: the echo filter's (the end of a grace
    /// or a probe) and the core's deadline.
    fn timers(&mut self, core: &mut Core<'_>) {
        let ended = self.filter.timer(tokio::time::Instant::now());
        let now = self.clock.now();
        self.hunt_ended(core, ended, now);
        self.overdue(core);
    }

    /// Service the core's deadline if it is already due.
    fn overdue(&mut self, core: &mut Core<'_>) {
        if core
            .next_deadline()
            .is_some_and(|deadline| deadline <= self.clock.now())
        {
            self.deadline(core);
        }
    }

    fn deadline(&mut self, core: &mut Core<'_>) {
        let before = core.state();
        let invalid = (before == MasterState::WaitForReply).then(|| core.counters().parser_errors);
        if let Err(error) = core.on_deadline(self.clock.now()) {
            debug!(?error, "MS/TP core refused a deadline");
        }
        // Leaving WAIT_FOR_REPLY at a deadline without an invalid frame (a
        // timed-out partial one) is the reply timeout.
        if let Some(invalid) = invalid {
            if core.state() != MasterState::WaitForReply && core.counters().parser_errors == invalid
            {
                increment(&self.counts.wait_for_reply_timeouts);
            }
        }
    }

    /// Take what a read returned: the count of octets read, zero for an
    /// empty read short of the end of file, or [`ReadFailed`] for an error
    /// or the end of file, counted and logged.
    fn outcome(&mut self, read: Result<usize, Error>) -> Result<usize, ReadFailed> {
        match read {
            Ok(0) => {
                self.empty_reads = self.empty_reads.saturating_add(1);
                if self.empty_reads < END_OF_FILE_READS {
                    return Ok(0);
                }
                increment(&self.counts.serial_read_errors);
                warn!(
                    "MS/TP serial port at end of file: {END_OF_FILE_READS} reads in a row returned nothing"
                );
                Err(ReadFailed)
            }
            Ok(count) => {
                self.empty_reads = 0;
                Ok(count)
            }
            Err(error) => {
                increment(&self.counts.serial_read_errors);
                warn!("MS/TP serial read error: {error}");
                Err(ReadFailed)
            }
        }
    }
}

/// Poll `future` once: its output if it is ready now, otherwise `None`, and
/// the future is dropped. It runs outside Tokio's cooperative budget, so a
/// spent budget cannot make a ready read look pending.
async fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(tokio::task::unconstrained(future));
    std::future::poll_fn(|cx| {
        Poll::Ready(match future.as_mut().poll(cx) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_frames_are_classified_by_type_and_answering() {
        let frame = |kind: u8| [0x55, 0xFF, kind, 1, 2, 0, 0, 0];
        assert_eq!(SentKind::of(&frame(5), false), SentKind::DataExpectingReply);
        assert_eq!(SentKind::of(&frame(6), true), SentKind::DirectAnswer);
        assert_eq!(SentKind::of(&frame(6), false), SentKind::QueuedData);
        assert_eq!(SentKind::of(&frame(7), true), SentKind::ReplyPostponed);
        assert_eq!(
            SentKind::of(&frame(0x20), false),
            SentKind::DataExpectingReply
        );
        assert_eq!(SentKind::of(&frame(0x21), true), SentKind::DirectAnswer);
        assert_eq!(SentKind::of(&frame(0x21), false), SentKind::QueuedData);
        assert_eq!(SentKind::of(&frame(0), false), SentKind::Other);
        assert_eq!(SentKind::of(&[0x55], false), SentKind::Other);
    }

    #[tokio::test]
    async fn poll_once_returns_only_what_is_ready() {
        assert_eq!(poll_once(async { 7 }).await, Some(7));
        assert_eq!(poll_once(std::future::pending::<u8>()).await, None);
    }
}
