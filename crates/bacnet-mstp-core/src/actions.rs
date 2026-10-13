//! Actions the MS/TP core hands its adapter, and the storage behind them.
//!
//! Two kinds of action wait to be polled, and control actions always come
//! first, so a backlog of received data never delays the transceiver:
//!
//! - **Control actions** turn the transceiver around and put a frame on the
//!   wire. They need no ring: see [`ActionQueue`] for why at most three can
//!   wait, in a fixed order. The frame's octets live in the one frame buffer
//!   of [`ActionBuffers`].
//! - **Deliveries** hand received data up. They wait in the delivery slots of
//!   [`ActionBuffers`], first in, first out. Their payloads are copied in when
//!   they are queued, because the stream decoder reuses its receive buffer.

use core::fmt;

use bacnet_mstp_codec::{
    Frame, FrameError, FrameHeader, FrameType, HEADER_LENGTH, MAX_EXTENDED_FRAME_LENGTH,
    MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

/// The physical direction requested for the serial transceiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Receive bytes from the bus: the line driver is off.
    Receive,
    /// Prepare to drive the bus for the [`Action::Transmit`] that follows.
    ///
    /// Do not enable the line driver yet: only once
    /// [`MasterCore::on_transmit_started`](crate::MasterCore::on_transmit_started)
    /// returns `Ok`. On a transceiver whose receiver and driver enables are
    /// tied, enabling the driver early also turns the receiver off, and the
    /// core stops hearing the very activity that must refuse the start.
    Transmit,
}

/// A generation-bound identifier for an application request.
///
/// The core creates a fresh generation for each request.  Consumers should
/// return the identifier they received rather than manufacturing a new one;
/// the explicit constructor is provided so adapters can persist or restore
/// identifiers without depending on the representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(u32);

impl RequestId {
    /// Construct an identifier from a generation value.
    #[must_use]
    pub const fn new(generation: u32) -> Self {
        Self(generation)
    }

    /// Return the generation represented by this identifier.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.0
    }
}

/// A generation-bound identifier for a transmit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransmitId(u32);

impl TransmitId {
    /// Construct an identifier from a generation value.
    #[must_use]
    pub const fn new(generation: u32) -> Self {
        Self(generation)
    }

    /// Return the generation represented by this identifier.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.0
    }
}

/// Reason a platform could not complete a transmit operation.
///
/// The core does not know how a serial driver failed.  These deliberately
/// small, allocation-free reasons let the driver report the protocol-relevant
/// distinction without coupling the core to an operating-system error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransmitFailure {
    /// The driver stopped the transfer before the final stop bit was sent.
    Aborted,
    /// The transfer did not complete before its deadline.
    Timeout,
    /// The driver reported an I/O failure.
    Io,
    /// The driver rejected the transfer before it started.
    Rejected,
}

/// An operation emitted by the core.
///
/// The byte slices borrow the core only until the next call that needs it
/// mutably. A platform should copy a transmit frame into its UART/DMA storage
/// before polling the next action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an action the adapter doesn't carry out is lost: a frame unsent, an NPDU undelivered"]
pub enum Action<'a> {
    /// Change the serial transceiver direction.
    SetDirection(Direction),
    /// Send one complete MS/TP frame.
    ///
    /// Enable the line driver for it (prepared by the
    /// [`SetDirection`](Self::SetDirection) polled just before) only once
    /// [`MasterCore::on_transmit_started`](crate::MasterCore::on_transmit_started)
    /// has returned `Ok`, and send only then. Until then keep the receiver on
    /// and feed the core what it hears. Right before that call, drain the
    /// receive FIFO or DMA buffer and feed the core every octet and receive
    /// error in it: activity the core has not been given cannot refuse the
    /// start. The core may also take the frame back first, which it shows by
    /// queueing `SetDirection(Direction::Receive)` before the start; drop the
    /// frame then. After an error for this id, or an abort, send nothing
    /// more for it. The exception is
    /// [`CoreError::TransmitNotPolled`](crate::CoreError::TransmitNotPolled),
    /// an adapter bug that leaves the id live: poll this action, then start.
    Transmit {
        /// Generation-bound operation identifier.
        id: TransmitId,
        /// Complete encoded frame, including preamble and CRCs.
        frame: &'a [u8],
    },
    /// Deliver a validated NPDU to the network layer, from a BACnet data
    /// frame: standard (Types 5 and 6) or, with
    /// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
    /// on, extended (Types 32 and 33).
    DeliverNpdu {
        /// Source MS/TP station.
        source: u8,
        /// Destination MS/TP station.
        destination: u8,
        /// NPDU bytes (without the MS/TP frame header and CRC).
        npdu: &'a [u8],
        /// Request awaiting an application decision, if this was a data-
        /// expecting-reply frame.
        request_id: Option<RequestId>,
    },
    /// Deliver a received frame whose data is not an NPDU, for this station
    /// or broadcast: a Test_Response, or a proprietary frame (types 128-255).
    ///
    /// Clause 9.5.6.2 passes both up as frames that need no reply. A
    /// Test_Response for this station that ends a wait for a reply comes
    /// here too (Clause 9.5.6.4). The core knows no proprietary type that
    /// asks for a reply, so it never answers one; one without the 2-octet
    /// vendor identifier Clause 9.3.11 requires is not delivered.
    ///
    /// What the core cannot judge is left to the application, which should
    /// drop a proprietary frame whose vendor type it does not know, and a
    /// broadcast one of a type that expects a reply: Clause 9.5.6.2 counts
    /// both as unwanted (ReceivedUnwantedFrame, cases (c) and (b)).
    DeliverFrame {
        /// [`FrameType::TEST_RESPONSE`], or a proprietary type (128-255),
        /// which no [`FrameType`] constant names.
        frame_type: FrameType,
        /// Source MS/TP station.
        source: u8,
        /// Destination MS/TP station: this station, or 255 for broadcast.
        destination: u8,
        /// The data field: a Test_Response's test data, or a proprietary
        /// frame's vendor identifier (two octets, most significant first)
        /// followed by its content.
        data: &'a [u8],
    },
    /// No action is pending.
    Idle,
}

/// The delivery slots [`ActionBuffers`] has unless the caller picks another
/// number.
pub const DEFAULT_DELIVERY_SLOTS: usize = 8;

/// The storage behind the actions a [`MasterCore`](crate::MasterCore)
/// returns, which the core borrows for as long as it lives, as it borrows its
/// receive storage.
///
/// It holds the frame this station is sending, at most one at a time, in a
/// buffer of `FRAME` octets, and `DELIVERIES` slots for received data
/// waiting to be polled, each with room for `NPDU` octets. That is the
/// core's only large state besides its queues and receive storage, which is
/// why the caller owns it: [`Self::new`] is a `const fn` that zeroes
/// everything, so on a microcontroller the buffers can live in a
/// zero-initialized static (`.bss`), and the core itself stays small enough
/// to build on the stack (see the crate docs).
///
/// `DELIVERIES` must be at least one, or [`Self::new`] fails to compile. One
/// is enough for an adapter that polls every action after each call into the
/// core, since a call completes at most one frame. More let an adapter leave
/// deliveries waiting while it handles other events. A delivery that finds
/// every slot full, or is longer than a slot, is dropped and counted in
/// [`MstpCounters::delivery_drops`](crate::MstpCounters::delivery_drops); a
/// data request dropped that way is answered with Reply Postponed.
///
/// # Sizes
///
/// The defaults are a standard frame's: `NPDU` 501 octets and `FRAME`
/// [`MAX_STANDARD_FRAME_LENGTH`]. A core with
/// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
/// on needs room for extended frames, [`ExtendedActionBuffers`]: `NPDU`
/// [`MAX_EXTENDED_MPDU_DATA`] and `FRAME` [`MAX_EXTENDED_FRAME_LENGTH`].
/// `NPDU` is also the longest queued NPDU the core sends, which is why the
/// core carries it as its own parameter. `FRAME` must hold any standard
/// frame and, when `NPDU` is more than 501, an extended frame carrying
/// `NPDU` octets, or [`Self::new`] fails to compile.
pub struct ActionBuffers<
    const DELIVERIES: usize = DEFAULT_DELIVERY_SLOTS,
    const NPDU: usize = MAX_STANDARD_MPDU_DATA,
    const FRAME: usize = MAX_STANDARD_FRAME_LENGTH,
> {
    frame: [u8; FRAME],
    deliveries: [DeliverySlot<NPDU>; DELIVERIES],
}

/// [`ActionBuffers`] sized for extended frames, as
/// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
/// needs: delivery slots of [`MAX_EXTENDED_MPDU_DATA`] octets and a frame
/// buffer of [`MAX_EXTENDED_FRAME_LENGTH`].
pub type ExtendedActionBuffers<const DELIVERIES: usize = DEFAULT_DELIVERY_SLOTS> =
    ActionBuffers<DELIVERIES, MAX_EXTENDED_MPDU_DATA, MAX_EXTENDED_FRAME_LENGTH>;

impl<const DELIVERIES: usize, const NPDU: usize, const FRAME: usize>
    ActionBuffers<DELIVERIES, NPDU, FRAME>
{
    /// Zeroed buffers with `DELIVERIES` delivery slots.
    #[must_use]
    pub const fn new() -> Self {
        const {
            assert!(
                DELIVERIES > 0,
                "an MS/TP core needs one delivery slot at least"
            );
            assert!(
                FRAME >= frame_room(NPDU),
                "FRAME can't hold the longest frame the core may send with NPDU-octet slots"
            );
        };
        Self {
            frame: [0; FRAME],
            deliveries: [DeliverySlot::EMPTY; DELIVERIES],
        }
    }
}

impl<const DELIVERIES: usize, const NPDU: usize, const FRAME: usize> Default
    for ActionBuffers<DELIVERIES, NPDU, FRAME>
{
    fn default() -> Self {
        Self::new()
    }
}

/// The sizes, not the octets: the core overwrites the buffers before it
/// reads them.
impl<const DELIVERIES: usize, const NPDU: usize, const FRAME: usize> fmt::Debug
    for ActionBuffers<DELIVERIES, NPDU, FRAME>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionBuffers")
            .field("deliveries", &DELIVERIES)
            .field("npdu", &NPDU)
            .field("frame", &FRAME)
            .finish()
    }
}

/// The longest frame a core with `npdu`-octet delivery slots may send: any
/// standard frame (a reply or a Test_Response carries up to 501 octets),
/// and an extended frame carrying `npdu` octets when that is more: its
/// encoded data grows by at most one octet in 254, and its CRC field takes
/// five.
const fn frame_room(npdu: usize) -> usize {
    if npdu <= MAX_STANDARD_MPDU_DATA {
        MAX_STANDARD_FRAME_LENGTH
    } else {
        PREAMBLE.len() + HEADER_LENGTH + npdu + npdu.div_ceil(254) + 5
    }
}

/// One received data field waiting to be polled.
#[derive(Clone, Copy)]
struct DeliverySlot<const NPDU: usize> {
    /// The request bound to an NPDU that asks for a reply.
    request_id: Option<RequestId>,
    /// The raw type of the frame the data came in, which says how it is
    /// delivered: BACnet data as an NPDU, any other type whole.
    frame_type: u8,
    source: u8,
    destination: u8,
    length: u16,
    data: [u8; NPDU],
}

impl<const NPDU: usize> DeliverySlot<NPDU> {
    /// All zero bits, so zeroed buffers can live in `.bss`.
    const EMPTY: Self = Self {
        request_id: None,
        frame_type: 0,
        source: 0,
        destination: 0,
        length: 0,
        data: [0; NPDU],
    };
}

/// A delivery was dropped: every slot was full, or the data was longer than
/// a slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeliveryRefused;

/// A frame of this station's whose actions have not all been polled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedTransmit {
    id: TransmitId,
    /// Its `SetDirection(Transmit)` was polled; the frame itself is next.
    direction_polled: bool,
}

/// The actions waiting to be polled, over the caller's [`ActionBuffers`].
///
/// # Why the control actions need no ring
///
/// At most one frame of this station's waits or is on the wire at a time:
/// the core makes a frame only when none is pending or active, and hands it
/// over only when no transmit is active. So at most one transmit, its switch
/// to transmit and its frame, waits to be polled, and its octets fit the one
/// frame buffer, which nothing rewrites until that transmit is over.
///
/// A switch back to receive is queued in two places only. Completion needs a
/// confirmed start, and the core refuses to start a transmit whose frame has
/// not been polled, so its actions are gone by then. Taking a frame back
/// withdraws its actions that have not been polled first. Either way no
/// transmit waits behind it, so a waiting switch to receive always comes
/// first. Two in a row would ask for the same thing, so a second one is
/// absorbed: an adapter that falls behind finds one switch to receive, not a
/// backlog. That is why a control action can never be refused for want of
/// room.
pub(crate) struct ActionQueue<'b, const NPDU: usize> {
    /// A switch back to receive waits, ahead of any transmit.
    receive: bool,
    transmit: Option<QueuedTransmit>,
    frame: &'b mut [u8],
    frame_length: usize,
    deliveries: &'b mut [DeliverySlot<NPDU>],
    /// The oldest waiting delivery.
    head: usize,
    /// How many deliveries wait.
    waiting: usize,
}

/// What waits to be polled, not the octets.
impl<const NPDU: usize> fmt::Debug for ActionQueue<'_, NPDU> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionQueue")
            .field("receive", &self.receive)
            .field("transmit", &self.transmit)
            .field("frame_length", &self.frame_length)
            .field("deliveries", &self.waiting)
            .finish()
    }
}

impl<'b, const NPDU: usize> ActionQueue<'b, NPDU> {
    /// An empty queue over the caller's buffers. Whatever they held is
    /// overwritten before it is read.
    pub(crate) fn new<const DELIVERIES: usize, const FRAME: usize>(
        buffers: &'b mut ActionBuffers<DELIVERIES, NPDU, FRAME>,
    ) -> Self {
        let ActionBuffers { frame, deliveries } = buffers;
        Self {
            receive: false,
            transmit: None,
            frame: &mut frame[..],
            frame_length: 0,
            deliveries,
            head: 0,
            waiting: 0,
        }
    }

    /// Encode the frame this station sends next into the frame buffer.
    pub(crate) fn encode_frame(&mut self, frame: Frame<'_>) -> Result<(), FrameError> {
        debug_assert!(
            self.transmit.is_none(),
            "a queued transmit still needs the frame buffer"
        );
        self.frame_length = frame.encode_into(self.frame)?;
        Ok(())
    }

    /// Queue the switch back to receive.
    pub(crate) fn queue_receive(&mut self) {
        debug_assert!(
            self.transmit.is_none(),
            "a receive switch behind a transmit"
        );
        self.receive = true;
    }

    /// Queue the encoded frame: a switch to transmit, then the frame.
    pub(crate) fn queue_transmit(&mut self, id: TransmitId) {
        debug_assert!(self.transmit.is_none(), "two transmits queued");
        self.transmit = Some(QueuedTransmit {
            id,
            direction_polled: false,
        });
    }

    /// Whether `id`'s frame has yet to be polled.
    pub(crate) fn transmit_queued(&self, id: TransmitId) -> bool {
        self.transmit.is_some_and(|transmit| transmit.id == id)
    }

    /// Withdraw a transmit that has not been polled yet, with its direction
    /// change if that has not been polled either.
    pub(crate) fn purge_transmit(&mut self, id: TransmitId) {
        if self.transmit_queued(id) {
            self.transmit = None;
        }
    }

    /// Queue received data for delivery, copying it in. `header` says whom it
    /// came from and how it goes up: see [`DeliverySlot::frame_type`].
    pub(crate) fn enqueue_delivery(
        &mut self,
        header: FrameHeader,
        data: &[u8],
        request_id: Option<RequestId>,
    ) -> Result<(), DeliveryRefused> {
        let length = u16::try_from(data.len()).map_err(|_| DeliveryRefused)?;
        if data.len() > NPDU || self.waiting == self.deliveries.len() {
            return Err(DeliveryRefused);
        }
        let slot = &mut self.deliveries[wrap(self.head + self.waiting, self.deliveries.len())];
        slot.request_id = request_id;
        slot.frame_type = header.frame_type.to_raw();
        slot.source = header.source;
        slot.destination = header.destination;
        slot.length = length;
        slot.data[..data.len()].copy_from_slice(data);
        self.waiting += 1;
        Ok(())
    }

    /// Remove the next action and borrow its payload, or return
    /// [`Action::Idle`].
    ///
    /// A transmit borrows the frame buffer and a delivery its slot. Both
    /// stay as they are until a later call that needs `&mut self`, so no
    /// action outlives its payload. Control actions always come before
    /// deliveries: a backlog of deliveries must not delay restoring the
    /// receive direction or sending the next token.
    pub(crate) fn poll(&mut self) -> Action<'_> {
        if core::mem::take(&mut self.receive) {
            return Action::SetDirection(Direction::Receive);
        }
        if let Some(transmit) = self.transmit.as_mut() {
            if !transmit.direction_polled {
                transmit.direction_polled = true;
                return Action::SetDirection(Direction::Transmit);
            }
            let id = transmit.id;
            self.transmit = None;
            return Action::Transmit {
                id,
                frame: &self.frame[..self.frame_length],
            };
        }
        if self.waiting == 0 {
            return Action::Idle;
        }
        let index = self.head;
        self.head = wrap(index + 1, self.deliveries.len());
        self.waiting -= 1;
        let slot = &self.deliveries[index];
        let data = &slot.data[..usize::from(slot.length)];
        match FrameType::from_raw(slot.frame_type) {
            frame_type if carries_npdu(frame_type) => Action::DeliverNpdu {
                source: slot.source,
                destination: slot.destination,
                npdu: data,
                request_id: slot.request_id,
            },
            frame_type => Action::DeliverFrame {
                frame_type,
                source: slot.source,
                destination: slot.destination,
                data,
            },
        }
    }
}

/// Whether a frame of this type carries a BACnet NPDU: BACnet data expecting
/// a reply or not, standard (Types 5 and 6) or extended (32 and 33).
pub(crate) const fn carries_npdu(frame_type: FrameType) -> bool {
    matches!(
        frame_type,
        FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    )
}

/// `index` wrapped into a ring of `length` slots, for an index less than
/// twice the length (no division).
fn wrap(index: usize, length: usize) -> usize {
    if index >= length {
        index - length
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPDU: FrameType = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;

    fn header(frame_type: FrameType, source: u8, destination: u8) -> FrameHeader {
        FrameHeader::new(frame_type, destination, source)
    }

    /// Encode a one-octet test frame and queue it as transmit `id`.
    fn queue_frame(actions: &mut ActionQueue<'_, MAX_STANDARD_MPDU_DATA>, id: u32, octet: u8) {
        actions
            .encode_frame(Frame::new(FrameType::TEST_REQUEST, 2, 1, &[octet]))
            .unwrap();
        actions.queue_transmit(TransmitId::new(id));
    }

    fn polled_frame(actions: &mut ActionQueue<'_, MAX_STANDARD_MPDU_DATA>) -> (TransmitId, u8) {
        match actions.poll() {
            Action::Transmit { id, frame } => (id, frame[8]),
            other => panic!("expected a transmit, got {other:?}"),
        }
    }

    #[test]
    fn controls_preempt_deliveries_and_delivery_order_is_stable() {
        let mut buffers = ActionBuffers::<2>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        actions
            .enqueue_delivery(header(NPDU, 1, 2), &[1], None)
            .unwrap();
        queue_frame(&mut actions, 9, 0xA5);
        let request = Some(RequestId::new(7));
        let expecting = FrameType::BACNET_DATA_EXPECTING_REPLY;
        actions
            .enqueue_delivery(header(expecting, 3, 4), &[2], request)
            .unwrap();

        assert_eq!(actions.poll(), Action::SetDirection(Direction::Transmit));
        assert_eq!(polled_frame(&mut actions), (TransmitId::new(9), 0xA5));
        assert_eq!(
            actions.poll(),
            Action::DeliverNpdu {
                source: 1,
                destination: 2,
                npdu: &[1],
                request_id: None,
            }
        );
        assert_eq!(
            actions.poll(),
            Action::DeliverNpdu {
                source: 3,
                destination: 4,
                npdu: &[2],
                request_id: request,
            }
        );
        assert_eq!(actions.poll(), Action::Idle);
    }

    #[test]
    fn a_switch_to_receive_comes_first_and_repeats_are_absorbed() {
        let mut buffers = ActionBuffers::<1>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        for _ in 0..3 {
            actions.queue_receive();
        }
        queue_frame(&mut actions, 4, 0x11);
        assert_eq!(actions.poll(), Action::SetDirection(Direction::Receive));
        assert_eq!(actions.poll(), Action::SetDirection(Direction::Transmit));
        assert_eq!(polled_frame(&mut actions), (TransmitId::new(4), 0x11));
        assert_eq!(actions.poll(), Action::Idle);
    }

    #[test]
    fn a_purge_withdraws_what_has_not_been_polled() {
        let mut buffers = ActionBuffers::<1>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        queue_frame(&mut actions, 1, 0);
        actions.purge_transmit(TransmitId::new(2));
        assert!(actions.transmit_queued(TransmitId::new(1)));
        actions.purge_transmit(TransmitId::new(1));
        assert_eq!(actions.poll(), Action::Idle);

        // Once the direction change is polled, the purge takes the frame.
        queue_frame(&mut actions, 3, 0);
        assert_eq!(actions.poll(), Action::SetDirection(Direction::Transmit));
        assert!(actions.transmit_queued(TransmitId::new(3)));
        actions.purge_transmit(TransmitId::new(3));
        assert_eq!(actions.poll(), Action::Idle);

        // A polled frame is no longer queued.
        queue_frame(&mut actions, 5, 0x77);
        assert_eq!(actions.poll(), Action::SetDirection(Direction::Transmit));
        assert_eq!(polled_frame(&mut actions), (TransmitId::new(5), 0x77));
        assert!(!actions.transmit_queued(TransmitId::new(5)));
    }

    #[test]
    fn whole_frames_go_up_as_frames() {
        let mut buffers = ActionBuffers::<2>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        let proprietary = FrameType::from_raw(0x80);
        actions
            .enqueue_delivery(header(FrameType::TEST_RESPONSE, 5, 1), &[9, 9], None)
            .unwrap();
        actions
            .enqueue_delivery(header(proprietary, 6, 0xFF), &[0x01, 0x04, 3], None)
            .unwrap();
        assert_eq!(
            actions.poll(),
            Action::DeliverFrame {
                frame_type: FrameType::TEST_RESPONSE,
                source: 5,
                destination: 1,
                data: &[9, 9],
            }
        );
        assert_eq!(
            actions.poll(),
            Action::DeliverFrame {
                frame_type: proprietary,
                source: 6,
                destination: 0xFF,
                data: &[0x01, 0x04, 3],
            }
        );
    }

    #[test]
    fn deliveries_wrap_around_and_a_full_ring_refuses_without_change() {
        let mut buffers = ActionBuffers::<2>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        for round in 0..5u8 {
            actions
                .enqueue_delivery(header(NPDU, round, 1), &[round], None)
                .unwrap();
            actions
                .enqueue_delivery(header(NPDU, round, 2), &[round, round], None)
                .unwrap();
            assert_eq!(
                actions.enqueue_delivery(header(NPDU, round, 3), &[0xEE], None),
                Err(DeliveryRefused)
            );
            assert!(matches!(
                actions.poll(),
                Action::DeliverNpdu { source, destination: 1, npdu: &[octet], .. }
                    if source == round && octet == round
            ));
            // One slot free again: the next round's first delivery wraps.
            if round % 2 == 0 {
                assert!(matches!(
                    actions.poll(),
                    Action::DeliverNpdu { destination: 2, npdu, .. } if npdu == [round, round]
                ));
            } else {
                actions
                    .enqueue_delivery(header(NPDU, round, 4), &[4], None)
                    .unwrap();
                assert!(matches!(
                    actions.poll(),
                    Action::DeliverNpdu { destination: 2, .. }
                ));
                assert!(matches!(
                    actions.poll(),
                    Action::DeliverNpdu { destination: 4, .. }
                ));
            }
            assert_eq!(actions.poll(), Action::Idle);
        }
    }

    #[test]
    fn queued_payload_is_stable_after_source_changes() {
        let mut buffers = ActionBuffers::<1>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        let mut npdu = [4, 5, 6];
        actions
            .enqueue_delivery(header(NPDU, 4, 5), &npdu, None)
            .unwrap();
        npdu.fill(0xEE);
        assert_eq!(
            actions.poll(),
            Action::DeliverNpdu {
                source: 4,
                destination: 5,
                npdu: &[4, 5, 6],
                request_id: None,
            }
        );
    }

    #[test]
    fn rejects_oversize_payloads_without_consuming_slots() {
        let mut buffers = ActionBuffers::<1>::new();
        let mut actions = ActionQueue::new(&mut buffers);
        let npdu = [0u8; MAX_STANDARD_MPDU_DATA + 1];
        assert_eq!(
            actions.enqueue_delivery(header(NPDU, 0, 1), &npdu, None),
            Err(DeliveryRefused)
        );
        actions
            .enqueue_delivery(header(NPDU, 0, 1), &npdu[..MAX_STANDARD_MPDU_DATA], None)
            .unwrap();
    }
}
