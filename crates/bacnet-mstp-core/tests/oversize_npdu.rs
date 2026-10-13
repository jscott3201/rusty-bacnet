//! An NPDU too long for a standard frame never holds the token: it is
//! refused at the queue, or dropped and counted when it gets in anyway.

use bacnet_mstp_codec::{decode_frame, Frame, FrameHeader, FrameType, MAX_EXTENDED_MPDU_DATA};
use bacnet_mstp_core::{
    Action, DequeuedNpdu, Direction, ExtendedActionBuffers, HeaplessQueues, Instant, MasterCore,
    MasterState, MstpCoreConfig, NetworkPriority, QueueCounters, QueueItemInfo, QueueKind,
    QueueRefusal, QueueStorage,
};

/// Queues whose slots hold more than a standard frame's data.
type BigQueues = HeaplessQueues<4, 4, 1500>;
type Actions = bacnet_mstp_core::ActionBuffers;

fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 2;
    config
}

/// Take the token and return the header of the frame it leads to.
fn next_frame_after_token<Q: QueueStorage, const NPDU: usize>(
    core: &mut MasterCore<'_, Q, NPDU>,
) -> FrameHeader {
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 1, 2, &[]), Instant::ZERO)
        .unwrap();
    let due = core.next_deadline().unwrap();
    assert!(due > Instant::ZERO, "the next deadline is in the future");
    core.on_deadline(due).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { frame, .. } => decode_frame(frame).unwrap().0.header,
        other => panic!("expected transmit, got {other:?}"),
    }
}

#[test]
fn enqueue_refuses_an_npdu_longer_than_a_standard_frame() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        BigQueues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    for destination in [2, 0xFF] {
        assert_eq!(
            core.enqueue_npdu(
                destination,
                &[0; 502],
                NetworkPriority::Normal,
                Instant::ZERO
            ),
            Err(QueueRefusal::Oversize)
        );
    }
    let counters = core.counters();
    assert_eq!(counters.unicast.drops_oversize, 1);
    assert_eq!(counters.broadcast.drops_oversize, 1);
    assert_eq!(counters.unicast.depth, 0);
}

#[test]
fn an_oversize_npdu_already_queued_is_dropped_and_the_token_moves_on() {
    // Queued straight into the storage before the core owns it.
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    queues
        .enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(config(), queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2)
    );
    assert_eq!(core.counters().unicast.drops_oversize, 1);

    // With nothing else queued the token use simply ends: here it polls for
    // a successor.
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(config(), queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}

/// The queue itself drops an NPDU longer than the core takes, its `NPDU`,
/// and counts it in its own counters, not the core's.
#[test]
fn the_queue_counts_an_npdu_longer_than_the_core_takes() {
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core =
            MasterCore::new(config(), &mut queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
        let header = next_frame_after_token(&mut core);
        assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
        assert_eq!(core.counters().unicast.drops_oversize, 1);
    }
    let stats = queues.counters(QueueKind::Unicast);
    assert_eq!(
        (stats.depth, stats.dequeued, stats.drops_oversize),
        (0, 1, 1)
    );
}

/// A core without extended frames sends no NPDU longer than a standard
/// frame carries, even with queues and delivery slots that hold one: it
/// drops it and counts it.
#[test]
fn a_core_without_extended_frames_never_sends_a_long_npdu() {
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    let mut rx = [0; 64];
    let mut actions = Box::new(ExtendedActionBuffers::<1>::new());
    let mut core: MasterCore<'_, _, MAX_EXTENDED_MPDU_DATA> =
        MasterCore::new(config(), queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}

/// A queue storage that lends more octets than it was asked for: `L`, from
/// a buffer of its own, for each item it takes off its queue.
struct Overstating<const L: usize>(HeaplessQueues<4, 4>, [u8; L]);

impl<const L: usize> QueueStorage for Overstating<L> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.0.set_starvation_limit(limit);
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        self.0
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: Instant) -> usize {
        self.0.expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek_kind(kind, now)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        let item = self.0.dequeue_kind(kind, now, max_len)?;
        Ok(item.map(|item| {
            DequeuedNpdu::new(
                item.destination,
                item.kind,
                item.priority,
                item.sequence,
                &self.1,
            )
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.0.depth(kind)
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.0.counters(kind)
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.0.take_counters(kind)
    }
}

#[test]
fn a_queue_overstating_an_npdu_length_cannot_hold_the_token() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Overstating(HeaplessQueues::new(), [0; 600]),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}

/// The same for a core that takes NPDUs of at most 100 octets, its
/// delivery slots' size: lent 200, which a standard frame could carry, the
/// NPDU is still dropped and counted, as one too long for the core.
#[test]
fn a_queue_lending_more_than_the_core_takes_cannot_hold_the_token() {
    let mut rx = [0; 64];
    let mut actions = bacnet_mstp_core::ActionBuffers::<1, 100>::new();
    let mut core: MasterCore<'_, _, 100> = MasterCore::new(
        config(),
        Overstating(HeaplessQueues::new(), [0; 200]),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}
