//! Clause 9.5.6 master state machine.

use core::fmt;

use bacnet_mstp_codec::{
    BROADCAST_MAC, DecodeEvent, Frame, FrameError, FrameType, HEADER_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA, PREAMBLE,
    StreamDecoder, StreamPhase,
};
use bacnet_time::{Duration, Instant, earliest};

use crate::actions::{
    Action, ActionBuffers, ActionQueue, RequestId, TransmitFailure, TransmitId, carries_npdu,
};
use crate::config::{
    ConfigError, CoreBuffer, MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT, MstpCoreConfig, N_MIN_OCTETS,
    N_POLL, N_RETRY_TOKEN, T_NO_TOKEN, T_SLOT,
};
use crate::counters::{MstpCounters, QueueCounters};
use crate::errors::{RequestError, TransmitError};
use crate::queue::{QueueKind, QueueRefusal, QueueStorage};
use crate::storage::{Storage, receive_window};
use bacnet_npdu::NetworkPriority;

mod input;
mod pending;
mod receive;
mod reply;
mod token;
mod transmission;

use pending::{Origin, PendingSend};

/// The smallest receive storage [`MasterCore::new`] accepts: a preamble and
/// a header, the size of the shortest frame.
pub const MIN_RX_STORAGE: usize = PREAMBLE.len() + HEADER_LENGTH;

/// Token-passing master state names from Clause 9.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MasterState {
    /// Waiting for a frame or token-loss timeout.
    Idle,
    /// Waiting for this station's token-generation slot.
    NoToken,
    /// Holding the token and selecting information frames.
    UseToken,
    /// Selecting token-pass or maintenance work.
    DoneWithToken,
    /// The token went to the next station; watching the line for traffic that
    /// shows it was taken up.
    PassToken,
    /// Waiting for a reply to a data-expecting-reply frame.
    WaitForReply,
    /// Waiting for a ReplyToPollForMaster frame.
    PollForMaster,
    /// Waiting for the application to answer a received request.
    AnswerDataRequest,
}

/// UART receive failures that count as bus activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReceiveError {
    /// Character framing failure.
    Framing,
    /// UART overrun.
    Overrun,
    /// Character parity failure.
    Parity,
    /// Break condition.
    Break,
}

/// Application decision for a request-bearing delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyDecision<'a> {
    /// Send a BACnetDataNotExpectingReply response immediately.
    Data(&'a [u8]),
    /// Tell the requester that the response will be sent later with a token.
    Postponed,
    /// Abandon the bound request without emitting a wire response.
    Abandoned,
}

/// Copyable state for diagnostics without exposing mutable internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MasterSnapshot {
    /// Current Clause 9.5.6 state.
    pub state: MasterState,
    /// Known successor station.
    pub next_station: u8,
    /// Station currently being polled.
    pub poll_station: u8,
    /// Information frames sent during the current token use.
    pub frame_count: u8,
    /// Maintenance token count.
    pub token_count: u8,
    /// Whether this station currently believes it is the sole master.
    pub sole_master: bool,
    /// Active transmit generation.
    pub transmit_id: Option<TransmitId>,
    /// Active application request generation.
    pub request_id: Option<RequestId>,
}

#[derive(Debug, Clone, Copy)]
struct PendingRequest {
    id: RequestId,
    source: u8,
    /// When the core stops waiting for the application and sends Reply
    /// Postponed: T_reply_delay less the transmit margin.
    decide_by: Instant,
    window: ReplyWindow,
}

/// When an answer to a received frame may still go out.
#[derive(Debug, Clone, Copy)]
struct ReplyWindow {
    /// The end of T_reply_delay, or of T_usage_delay for Reply To Poll For
    /// Master. An answer that starts later is counted late.
    send_by: Instant,
    /// When the asker may stop waiting, at its shortest timeout. From here
    /// on it may be transmitting again, so nothing is sent.
    give_up: Instant,
}

#[derive(Debug, Clone, Copy)]
enum PostTransmit {
    Idle,
    ContinueToken,
    WaitForReply,
    PassToken,
    PollForMaster,
}

/// A frame handed to the adapter, from the hand-over until it completes or
/// is aborted or taken back.
#[derive(Debug, Clone, Copy)]
struct ActiveTransmit {
    id: TransmitId,
    asserted_at: Instant,
    /// The adapter confirmed the start; until then the frame is also still
    /// the waiting [`PendingSend`].
    started: bool,
    post: PostTransmit,
}

/// Synchronous, allocation-free MS/TP master core.
///
/// # Driving it
///
/// Feed it what the line does, timestamped by the caller's monotonic clock:
/// [`Self::on_octet`] or [`Self::on_receive_error`] for each received
/// character, or from a host-side assembler [`Self::on_host_activity`] for
/// raw octets and [`Self::on_validated_frame`] or [`Self::on_invalid_frame`]
/// for each frame it completes; and [`Self::on_deadline`] at
/// [`Self::next_deadline`].
///
/// After each call, carry out the actions from [`Self::poll_action`] until it
/// returns [`Action::Idle`], and before the next transmit event at the
/// latest. Control actions never run out of room: at most a switch to
/// receive and one transmit wait at a time. Received data waits in the
/// delivery slots of [`ActionBuffers`], and data that finds them all full is
/// dropped and counted in [`MstpCounters::delivery_drops`].
///
/// A transmit goes through three calls. Poll [`Action::Transmit`]; when the
/// driver is about to go on, call [`Self::on_transmit_started`], which hands
/// over the frame's octets only if it confirms the start; at the final stop
/// bit, call
/// [`Self::on_transmit_complete`]. Until the start is confirmed, keep
/// receiving and feed the core what arrives, draining the receive FIFO or
/// DMA buffer right before the start call (octets stamped with the drain
/// time are safe: a later stamp can only defer the start). Line activity in
/// that time pushes the frame back (Clause 9.5.5.1), and the core may take
/// the frame back on its own, queueing
/// [`Direction::Receive`](crate::Direction::Receive) before the start; forget
/// the id then. Input after the confirmed start is taken for the
/// station's own echo and ignored, whichever call reports it.
///
/// Line activity only delays a frame the core waits to send. A frame is
/// dropped, and counted, only on the evidence its sending state watches for
/// (another station using the token after a retry, or active while this one
/// generates a token) or at a time limit taken from that state. The crate's
/// `src/state/pending.rs` tables every frame that can wait, with its rules
/// and clauses.
///
/// # Storage
///
/// The core borrows its two large buffers, the receive storage and the
/// [`ActionBuffers`], for its lifetime `'rx`, and owns its queues `Q`, which
/// may be a `&mut` borrow of queues kept elsewhere. Everything it holds by
/// value is small, so [`Self::new`] can return it on a microcontroller's
/// stack; the crate docs show the pattern.
///
/// `NPDU` is the size of the [`ActionBuffers`] delivery slots, which the
/// core takes from them, and the longest queued NPDU it sends; the queue
/// drops a longer one. A queued NPDU is never copied onto the stack: the
/// queue lends it, and the core encodes its frame from there. `NPDU`
/// defaults to a standard frame's 501 octets, so a core without extended
/// frames costs what it always did; one with
/// [`MstpCoreConfig::extended_frames`] on needs
/// [`MAX_EXTENDED_MPDU_DATA`]
/// (see [`ExtendedActionBuffers`](crate::ExtendedActionBuffers)).
///
/// # Late token generation (a local policy)
///
/// Clause 9.5.6.7 lets a station generate a token only while the line's
/// silence lies inside the station's own slot, and gives it no way on once
/// that slot has gone by in silence: read literally, a sole master whose
/// NO_TOKEN deadline was serviced late would wait there for ever. The core
/// opens the station's window again every T_slot × (Max_Master + 1) of
/// continued silence instead, which keeps every station's turn in the same
/// order, and counts each token generated that way in
/// [`MstpCounters::missed_generation_slots`]. It departs from the clause only
/// when the deadline is serviced after the slot has closed; on time, the
/// first slot is used.
pub struct MasterCore<'rx, Q, const NPDU: usize = MAX_STANDARD_MPDU_DATA> {
    config: MstpCoreConfig,
    queues: Q,
    decoder: StreamDecoder<'rx>,
    actions: ActionQueue<'rx, NPDU>,
    state: MasterState,
    next_station: u8,
    poll_station: u8,
    token_count: u8,
    frame_count: u8,
    retry_token_count: u8,
    event_count: u32,
    sole_master: bool,
    /// A Token for this station arrived while it was answering a request;
    /// it is taken up on the way back to IDLE.
    pending_token: bool,
    pending_request: Option<PendingRequest>,
    next_request_generation: u32,
    active_transmit: Option<ActiveTransmit>,
    pending_send: Option<PendingSend>,
    next_transmit_generation: u32,
    protocol_deadline: Option<Instant>,
    last_activity: Instant,
    last_octet: Option<Instant>,
    parser_partial: bool,
    last_now: Instant,
    consecutive_broadcast: u8,
    counters: MstpCounters,
}

/// The core's state, not its buffers: its configuration, the Clause 9.5.6
/// state and variables, what it waits for and until when, the actions and
/// queued NPDUs waiting, and the counters since they were last taken.
impl<Q: QueueStorage, const NPDU: usize> fmt::Debug for MasterCore<'_, Q, NPDU> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterCore")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("next_station", &self.next_station)
            .field("poll_station", &self.poll_station)
            .field("token_count", &self.token_count)
            .field("frame_count", &self.frame_count)
            .field("retry_token_count", &self.retry_token_count)
            .field("event_count", &self.event_count)
            .field("sole_master", &self.sole_master)
            .field("pending_token", &self.pending_token)
            .field("pending_request", &self.pending_request)
            .field("active_transmit", &self.active_transmit)
            .field("pending_send", &self.pending_send)
            .field("next_deadline", &self.next_deadline())
            .field("last_activity", &self.last_activity)
            .field("last_now", &self.last_now)
            .field("decoder", &self.decoder)
            .field("actions", &self.actions)
            .field("unicast_queued", &self.queues.depth(QueueKind::Unicast))
            .field("broadcast_queued", &self.queues.depth(QueueKind::Broadcast))
            .field("counters", &self.counters())
            .finish_non_exhaustive()
    }
}

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, &'rx mut Q, NPDU> {
    /// Construct a core over a [`Storage`] bundle, which it borrows for its
    /// lifetime: its receive storage, its queues and its [`ActionBuffers`].
    ///
    /// The core uses as much of the receive storage as its configuration
    /// takes: the longest extended frame with
    /// [`MstpCoreConfig::extended_frames`] on, the longest standard frame
    /// without (see [`Storage`]). With the flag on, every buffer must be
    /// sized for extended frames, as
    /// [`ExtendedStorage`](crate::ExtendedStorage)'s are; a smaller one is
    /// refused with [`ConfigError::ExtendedStorageTooSmall`], naming it. An
    /// invalid configuration is refused with the [`ConfigError`] that
    /// [`MstpCoreConfig::validate`] returns.
    pub fn new<const DELIVERIES: usize, const FRAME: usize>(
        config: MstpCoreConfig,
        storage: &'rx mut Storage<Q, DELIVERIES, NPDU, FRAME>,
        now: Instant,
    ) -> Result<Self, ConfigError> {
        let Storage {
            rx,
            queues,
            actions,
        } = storage;
        let window = receive_window(config.extended_frames, FRAME);
        MasterCore::from_parts(config, queues, &mut rx[..window], actions, now)
    }
}

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Construct a core over its parts, for a caller that keeps them apart
    /// rather than in a [`Storage`]: a receive storage of any length (a
    /// test of a short one, say), or queues that aren't borrowed. Most
    /// callers want [`MasterCore::new`].
    ///
    /// `rx_storage` is the stream decoder's buffer for [`Self::on_octet`].
    /// Size it for the largest frame this station accepts:
    /// [`MAX_STANDARD_FRAME_LENGTH`](bacnet_mstp_codec::MAX_STANDARD_FRAME_LENGTH)
    /// takes any standard frame, and
    /// [`MAX_EXTENDED_FRAME_LENGTH`] any extended frame
    /// that carries a BACnet NPDU. A longer frame addressed to this station
    /// or broadcast counts as an invalid frame (Clause 9.5.4.4), at its
    /// header; one for another station is skipped. Without
    /// [`MstpCoreConfig::extended_frames`] an extended frame the storage
    /// holds counts the same way at its last octet. Storage shorter than
    /// [`MIN_RX_STORAGE`] is refused.
    ///
    /// `action_buffers` holds the frame this station sends and the received
    /// data waiting to be polled; its `DELIVERIES` sets how much may wait,
    /// and its `NPDU` how long each may be (see [`ActionBuffers`]). Whatever
    /// either buffer held is overwritten before it is read.
    ///
    /// With [`MstpCoreConfig::extended_frames`] on, every buffer must hold
    /// what an extended frame needs: the receive storage and the action
    /// buffers' frame [`MAX_EXTENDED_FRAME_LENGTH`] octets, and a delivery
    /// slot and a queue slot ([`QueueStorage::max_npdu`])
    /// [`MAX_EXTENDED_MPDU_DATA`]. A smaller one is refused with
    /// [`ConfigError::ExtendedStorageTooSmall`], naming it.
    pub fn from_parts<const DELIVERIES: usize, const FRAME: usize>(
        config: MstpCoreConfig,
        mut queues: Q,
        rx_storage: &'rx mut [u8],
        action_buffers: &'rx mut ActionBuffers<DELIVERIES, NPDU, FRAME>,
        now: Instant,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        if rx_storage.len() < MIN_RX_STORAGE {
            return Err(ConfigError::ReceiveStorageTooSmall {
                length: rx_storage.len(),
                minimum: MIN_RX_STORAGE,
            });
        }
        if config.extended_frames {
            for (buffer, length, minimum) in [
                (
                    CoreBuffer::ReceiveStorage,
                    rx_storage.len(),
                    MAX_EXTENDED_FRAME_LENGTH,
                ),
                (CoreBuffer::TransmitFrame, FRAME, MAX_EXTENDED_FRAME_LENGTH),
                (CoreBuffer::DeliverySlot, NPDU, MAX_EXTENDED_MPDU_DATA),
                (
                    CoreBuffer::QueueSlot,
                    queues.max_npdu(),
                    MAX_EXTENDED_MPDU_DATA,
                ),
            ] {
                if length < minimum {
                    return Err(ConfigError::ExtendedStorageTooSmall {
                        buffer,
                        length,
                        minimum,
                    });
                }
            }
        }
        queues.set_starvation_limit(config.starvation_limit);
        let decoder = StreamDecoder::new(rx_storage);
        Ok(Self {
            config,
            queues,
            // A node with COBS support checks every COBS type's encoded
            // fields, the reserved ones included (Clause 9.5.4.9).
            decoder: if config.extended_frames {
                decoder.with_reserved_cobs()
            } else {
                decoder
            },
            actions: ActionQueue::new(action_buffers),
            state: MasterState::Idle,
            next_station: config.station,
            poll_station: config.station,
            token_count: N_POLL,
            frame_count: 0,
            retry_token_count: 0,
            event_count: 0,
            sole_master: false,
            pending_token: false,
            pending_request: None,
            next_request_generation: 0,
            active_transmit: None,
            pending_send: None,
            next_transmit_generation: 0,
            protocol_deadline: Some(now + T_NO_TOKEN),
            last_activity: now,
            last_octet: None,
            parser_partial: false,
            last_now: now,
            consecutive_broadcast: 0,
            counters: MstpCounters::default(),
        })
    }

    /// Return the current state.
    #[must_use]
    pub const fn state(&self) -> MasterState {
        self.state
    }

    /// Return the validated configuration.
    #[must_use]
    pub const fn config(&self) -> &MstpCoreConfig {
        &self.config
    }

    /// Return a copyable diagnostic snapshot.
    #[must_use]
    pub fn snapshot(&self) -> MasterSnapshot {
        MasterSnapshot {
            state: self.state,
            next_station: self.next_station,
            poll_station: self.poll_station,
            frame_count: self.frame_count,
            token_count: self.token_count,
            sole_master: self.sole_master,
            transmit_id: self.active_transmit.map(|tx| tx.id),
            request_id: self.pending_request.map(|request| request.id),
        }
    }

    /// The counts since the counters were last taken, which then start
    /// again from zero ([`MstpCounters`]): add them to your own totals, as
    /// wide as you need. Each count is a `u32` that saturates between takes.
    ///
    /// The queue counters are the queue storage's, taken with these
    /// ([`QueueStorage::take_counters`]), plus the NPDUs the core itself
    /// refused or dropped as too long to send ([`MstpCoreConfig::max_npdu`]).
    #[must_use = "the counts are gone once taken; add them to your totals"]
    pub fn take_counters(&mut self) -> MstpCounters {
        let core = core::mem::take(&mut self.counters);
        let unicast = self.queues.take_counters(QueueKind::Unicast);
        let broadcast = self.queues.take_counters(QueueKind::Broadcast);
        merge_queues(core, unicast, broadcast)
    }

    /// The counts since the counters were last taken, as
    /// [`Self::take_counters`] would return them, without taking them: for
    /// a look between takes. A core whose counters are never taken counts
    /// from its start.
    #[must_use]
    pub fn counters(&self) -> MstpCounters {
        merge_queues(
            self.counters,
            self.queues.counters(QueueKind::Unicast),
            self.queues.counters(QueueKind::Broadcast),
        )
    }

    /// Queue an NPDU with an absolute expiry derived from configuration.
    ///
    /// An NPDU longer than this station sends
    /// ([`MstpCoreConfig::max_npdu`]: 501 octets, or 1497 with
    /// extended frames on) is refused with [`QueueRefusal::Oversize`]
    /// and counted in its queue's `drops_oversize`, whatever the queue
    /// storage could hold. With extended frames on, one of 502 octets or
    /// more goes out in an extended frame.
    ///
    /// A `now` earlier than the latest time the core was given is taken as
    /// that time, as every call takes it (see the crate docs, "Time"); a
    /// later one doesn't move the core's clock.
    pub fn enqueue_npdu(
        &mut self,
        destination: u8,
        npdu: &[u8],
        priority: NetworkPriority,
        now: Instant,
    ) -> Result<(), QueueRefusal> {
        // Taken as every call takes its time, but without moving the clock.
        let now = now.max(self.last_now);
        if npdu.len() > self.config.max_npdu() {
            self.record_oversize_npdu(QueueKind::from_destination(destination));
            return Err(QueueRefusal::Oversize);
        }
        let expires = now.saturating_add(self.config.queue_max_age);
        self.queues
            .enqueue_npdu(destination, npdu, priority, now, Some(expires))
    }

    /// Service the current absolute deadline. Early/stale callbacks are no-ops.
    pub fn on_deadline(&mut self, now: Instant) {
        let now = self.observe_time(now);
        if self.abort_partial_if_late(now) {
            self.invalid_frame_event(now);
        }
        let Some(deadline) = self.next_deadline() else {
            return;
        };
        if now < deadline {
            return;
        }
        self.counters
            .record_deadline(now - deadline, self.config.late_deadline);
        if let Some(tx) = self.active_transmit {
            if now >= tx.asserted_at + self.config.transmit_watchdog {
                return self.on_transmit_aborted(tx.id, TransmitFailure::Timeout, now);
            }
            return;
        }
        if self.pending_send.is_some() {
            return self.dispatch_pending_send(now);
        }
        match self.state {
            MasterState::Idle => self.enter_no_token(now),
            MasterState::NoToken => self.generate_token_if_in_slot(now),
            MasterState::WaitForReply => {
                // Clause 9.5.6.4 ends this token use after a reply timeout;
                // queued information frames wait for the next token.
                self.frame_count = self.config.max_info_frames;
                self.set_state(MasterState::DoneWithToken);
                self.drive_token(now)
            }
            MasterState::AnswerDataRequest => self.reply_deadline(now),
            MasterState::PassToken => self.pass_token_timeout(now),
            MasterState::PollForMaster => self.poll_timeout(now),
            MasterState::UseToken | MasterState::DoneWithToken => self.drive_token(now),
        }
    }

    /// Return the next action, or [`Action::Idle`] when no action is pending.
    ///
    /// Control actions come first, then deliveries in the order they were
    /// queued. An action's octets borrow the core until the next call that
    /// needs it mutably.
    pub fn poll_action(&mut self) -> Action<'_> {
        self.actions.poll()
    }

    /// Return the earliest parser, protocol, or transmit-watchdog deadline.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        // A partial frame is dropped once the silence is longer than
        // T_frame_abort, so the deadline falls just past it: a whole
        // microsecond, the value it had when the core counted microseconds,
        // which keeps it on the whole-microsecond timeline the simulator and
        // the fuzz model check.
        let parser = self
            .last_octet
            .filter(|_| self.parser_partial)
            .map(|last_octet| last_octet + self.config.t_frame_abort + Duration::from_micros(1));
        let watchdog = self
            .active_transmit
            .map(|tx| tx.asserted_at + self.config.transmit_watchdog);
        earliest([self.protocol_deadline, parser, watchdog])
    }

    /// The time a call is taken at: `now`, or the latest time passed in
    /// when `now` is earlier (see the crate docs, "Time").
    fn observe_time(&mut self, now: Instant) -> Instant {
        self.last_now = self.last_now.max(now);
        self.last_now
    }

    /// Line activity that is not this station's own echo, worth `events`
    /// toward EventCount (Clause 9.5.6). A frame of this station's that waits
    /// to go out takes it under its own rules (see the `pending` module);
    /// otherwise the current state's timers and transitions apply.
    fn note_bus_activity(&mut self, now: Instant, events: u32) {
        self.last_activity = now;
        self.event_count = self.event_count.saturating_add(events);
        if self.pending_send.is_some() {
            return self.pending_activity(now);
        }
        match self.state {
            MasterState::NoToken => {
                self.set_state(MasterState::Idle);
                self.protocol_deadline = Some(now + T_NO_TOKEN);
            }
            MasterState::Idle => self.protocol_deadline = Some(now + T_NO_TOKEN),
            MasterState::PassToken => {
                if self.event_count > u32::from(N_MIN_OCTETS) {
                    self.set_state(MasterState::Idle);
                    self.protocol_deadline = Some(now + T_NO_TOKEN);
                } else {
                    self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                }
            }
            MasterState::PollForMaster => {
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
            }
            MasterState::WaitForReply => {
                self.protocol_deadline = Some(now + self.config.t_reply_timeout);
            }
            MasterState::UseToken | MasterState::DoneWithToken | MasterState::AnswerDataRequest => {
            }
        }
    }

    /// Go idle, with the token-loss timer running from the last activity on
    /// the line (Clause 9.5.6.2), then take up any Token that arrived while
    /// this station was answering a request.
    fn enter_idle(&mut self, now: Instant) {
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(self.last_activity + T_NO_TOKEN);
        self.take_pending_token(now)
    }

    /// Act on a Token remembered while answering a request, as IDLE acts on
    /// a received one.
    fn take_pending_token(&mut self, now: Instant) {
        if core::mem::take(&mut self.pending_token) {
            self.use_received_token(now);
        }
    }

    /// Count an NPDU refused or dropped as too long to send.
    fn record_oversize_npdu(&mut self, kind: QueueKind) {
        let queue = match kind {
            QueueKind::Unicast => &mut self.counters.unicast,
            QueueKind::Broadcast => &mut self.counters.broadcast,
        };
        queue.drops_oversize = queue.drops_oversize.saturating_add(1);
    }

    fn set_state(&mut self, state: MasterState) {
        if self.state != state {
            self.state = state;
            self.counters.record_transition();
        }
    }
}

/// The address after `current`: (current + 1) modulo (Max_Master + 1), as
/// Clause 9.5.6 computes it.
fn next_addr(current: u8, max_master: u8) -> u8 {
    match current.cmp(&max_master) {
        core::cmp::Ordering::Less => current + 1,
        core::cmp::Ordering::Equal => 0,
        // A successor above Max_Master, which a station outside the master
        // range becomes by answering a poll, and whose failure the next
        // search starts after (FindNewSuccessor, Clause 9.5.6.6).
        core::cmp::Ordering::Greater => {
            ((u32::from(current) + 1) % (u32::from(max_master) + 1)) as u8
        }
    }
}

/// The core's own counters with the queues' in place of its queue counters,
/// which hold only the NPDUs the core refused or dropped as too long, added
/// to the queues' own.
fn merge_queues(
    core: MstpCounters,
    unicast: QueueCounters,
    broadcast: QueueCounters,
) -> MstpCounters {
    let merge = |mut queue: QueueCounters, own: QueueCounters| {
        queue.drops_oversize = queue.drops_oversize.saturating_add(own.drops_oversize);
        queue
    };
    let mut counters = core;
    counters.unicast = merge(unicast, core.unicast);
    counters.broadcast = merge(broadcast, core.broadcast);
    counters
}
