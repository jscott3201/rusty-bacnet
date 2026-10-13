//! Reply timing for a received data-expecting-reply frame: the decision
//! deadline keeps a transmit margin inside T_reply_delay, and a late deadline
//! call or transmit start still answers with Reply Postponed (Clause 9.5.6.9).

use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, ReceiveError, ReplyDecision, RequestId, TransmitId, LATE_DEADLINE,
};

type Queues = HeaplessQueues<4, 4>;
type Actions = bacnet_mstp_core::ActionBuffers;

/// The request arrives at 1 ms.
const REQUEST_AT: Instant = Instant::from_micros(1_000);
/// T_reply_delay (250 ms) less the default 25 ms margin, after the request.
const DECIDE_BY: Instant = Instant::from_micros(226_000);
/// The end of T_reply_delay.
const SEND_BY: Instant = Instant::from_micros(251_000);
/// The shortest T_reply_timeout (255 ms) after the request, less one octet
/// at 9600 baud (1042 µs, rounded up) and the 1 ms guard.
const GIVE_UP: Instant = Instant::from_micros(1_000 + 255_000 - 1_042 - 1_000);

fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 3;
    config
}

/// Deliver a request from station 7 and return its identifier.
fn receive_request(core: &mut MasterCore<'_, Queues>) -> RequestId {
    core.on_validated_frame(
        Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, 1, 7, &[1, 0x04]),
        REQUEST_AT,
    )
    .unwrap();
    match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    }
}

/// Take the queued transmit and check it is a Reply Postponed to station 7.
fn take_postponed(core: &mut MasterCore<'_, Queues>) -> TransmitId {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { id, frame } => {
            let header = decode_frame(frame).unwrap().0.header;
            assert_eq!(
                (header.frame_type, header.destination),
                (FrameType::REPLY_POSTPONED, 7)
            );
            id
        }
        other => panic!("expected transmit, got {other:?}"),
    }
}

#[test]
fn decision_deadline_leaves_the_transmit_margin() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    assert_eq!(core.next_deadline(), Some(DECIDE_BY));

    // Platform latency inside the margin is not late.
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, SEND_BY).unwrap();
    core.on_transmit_complete(id, SEND_BY + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    let counters = core.counters();
    assert_eq!(
        (
            counters.deadline_misses,
            counters.late_replies,
            counters.frames_transmitted
        ),
        (0, 0, 1)
    );

    // The margin is configurable.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut config = config();
    config.reply_transmit_margin = Duration::from_millis(50);
    let mut core =
        MasterCore::new(config, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    receive_request(&mut core);
    assert_eq!(
        core.next_deadline(),
        Some(SEND_BY - Duration::from_millis(50))
    );
}

#[test]
fn deadline_call_one_microsecond_late_still_sends_postponed() {
    // Lateness under LATE_DEADLINE is ordinary tick jitter: sent, not counted.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    let late = DECIDE_BY + Duration::from_micros(1);
    core.on_deadline(late).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, late).unwrap();
    assert_eq!(core.counters().deadline_misses, 0);
    assert_eq!(core.counters().late_replies, 0);

    // At LATE_DEADLINE it is counted as a miss, and still answered.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    let late = DECIDE_BY + LATE_DEADLINE;
    core.on_deadline(late).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, late).unwrap();
    assert_eq!(core.counters().deadline_misses, 1);
    assert_eq!(core.counters().late_replies, 0);

    // A deadline call even past T_reply_delay still answers, and the late
    // start is counted too.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    let late = SEND_BY + Duration::from_micros(1);
    core.on_deadline(late).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, late).unwrap();
    assert_eq!(core.counters().deadline_misses, 1);
    assert_eq!(core.counters().late_replies, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

#[test]
fn transmit_start_200_microseconds_late_still_sends_postponed() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    let start = SEND_BY + Duration::from_micros(200);
    core.on_transmit_started(id, start).unwrap();
    assert_eq!(core.counters().late_replies, 1);
    core.on_transmit_complete(id, start + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().frames_transmitted, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

#[test]
fn nothing_is_sent_once_the_requester_may_have_stopped_waiting() {
    // A Reply Postponed that would start at the requester's shortest timeout
    // is refused: the requester may be transmitting again.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    assert_eq!(
        core.on_transmit_started(id, GIVE_UP),
        Err(CoreError::RequestExpired)
    );
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.counters().late_replies, 0);

    // One microsecond earlier it still goes out, counted late.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    receive_request(&mut core);
    core.on_deadline(DECIDE_BY).unwrap();
    let id = take_postponed(&mut core);
    core.on_transmit_started(id, GIVE_UP - Duration::from_micros(1))
        .unwrap();
    assert_eq!(core.counters().late_replies, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

#[test]
fn a_reply_waiting_out_the_turnaround_survives_line_activity() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let id = receive_request(&mut core);
    // The answer comes inside the turnaround, so the reply waits for it.
    core.complete_request(
        id,
        ReplyDecision::Data(&[9]),
        REQUEST_AT + Duration::from_micros(500),
    )
    .unwrap();
    let noise = REQUEST_AT + Duration::from_millis(1);
    core.on_receive_error(ReceiveError::Framing, noise).unwrap();
    // The reply waits a fresh turnaround from the noise instead of vanishing.
    let due = noise + core.config().t_turnaround;
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { frame, .. } = core.poll_action() else {
        panic!("expected the reply transmit");
    };
    let reply = decode_frame(frame).unwrap().0;
    assert_eq!(
        (
            reply.header.frame_type,
            reply.header.destination,
            reply.data
        ),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 7, &[9][..])
    );
    assert_eq!(core.counters().cancelled_sends, 0);

    // A frame for this station meanwhile is ignored rather than answered in
    // the reply's place.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let id = receive_request(&mut core);
    core.complete_request(
        id,
        ReplyDecision::Data(&[9]),
        REQUEST_AT + Duration::from_micros(500),
    )
    .unwrap();
    core.on_validated_frame(Frame::new(FrameType::POLL_FOR_MASTER, 1, 5, &[]), noise)
        .unwrap();
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { frame, .. } = core.poll_action() else {
        panic!("expected the reply transmit");
    };
    let reply = decode_frame(frame).unwrap().0.header;
    assert_eq!(
        (reply.frame_type, reply.destination),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 7)
    );
}
