//! Frames that arrive while this station answers a request (Clauses 9.5.6.2
//! and 9.5.6.9), and the reply window under steady line activity.

use bacnet_mstp_codec::{Frame, FrameHeader, FrameType};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Direction, Duration, FixedQueues, Instant, MasterCore, MasterState, MstpCoreConfig,
    ReceiveError, ReplyDecision, RequestId, TransmitId,
};

type Queues = FixedQueues<4, 4>;
type Actions = bacnet_mstp_core::ActionBuffers;

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 3;
    config
}

/// Deliver a request from station 7 at 1 ms and return its identifier.
fn receive_request(core: &mut MasterCore<'_, Queues>) -> RequestId {
    core.on_validated_frame(
        Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, 1, 7, &[1, 0x04]),
        ms(1),
    );
    match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    }
}

fn take_transmit(core: &mut MasterCore<'_, Queues>, now: Instant) -> (TransmitId, FrameHeader) {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let (id, header) = match core.poll_action() {
        Action::Transmit { id, header, .. } => (id, header),
        other => panic!("expected transmit, got {other:?}"),
    };
    core.on_transmit_started(id, now).unwrap();
    (id, header)
}

fn finish(core: &mut MasterCore<'_, Queues>, id: TransmitId, now: Instant) {
    core.on_transmit_complete(id, now);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
}

/// Answer the request at 30 ms and send the reply, which ends at 31 ms.
fn send_reply(core: &mut MasterCore<'_, Queues>, id: RequestId) {
    core.complete_request(id, ReplyDecision::Data(&[9]), ms(30))
        .unwrap();
    let (tx, header) = take_transmit(core, ms(30));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 7)
    );
    finish(core, tx, ms(31));
}

#[test]
fn data_is_delivered_and_a_token_kept_while_answering() {
    let token = Frame::new(FrameType::TOKEN, 1, 2, &[]);
    let data = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &[7]);
    for (first, second) in [(token, data), (data, token)] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        core.enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
            .unwrap();
        let id = receive_request(&mut core);
        core.on_validated_frame(first, ms(10));
        core.on_validated_frame(second, ms(20));
        // The data goes up at once, without disturbing the request.
        assert_eq!(
            core.poll_action(),
            Action::DeliverNpdu {
                source: 5,
                destination: 1,
                npdu: &[7],
                request_id: None,
            }
        );
        assert_eq!(core.state(), MasterState::AnswerDataRequest);
        assert_eq!(core.snapshot().request_id, Some(id));

        // After the reply, the core holds the token and sends its queued data.
        send_reply(&mut core, id);
        assert_eq!(core.state(), MasterState::UseToken);
        let due = core.next_deadline().unwrap();
        core.on_deadline(due);
        let (_, header) = take_transmit(&mut core, due);
        assert_eq!(
            (header.frame_type, header.destination),
            (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2)
        );
    }
}

#[test]
fn frames_wanting_a_prompt_answer_are_dropped_and_counted() {
    for frame in [
        Frame::new(FrameType::POLL_FOR_MASTER, 1, 5, &[]),
        Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, 1, 5, &[1, 0x04]),
        Frame::new(FrameType::TEST_REQUEST, 1, 5, &[1]),
    ] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        let id = receive_request(&mut core);
        core.on_validated_frame(frame, ms(10));
        assert_eq!(
            core.counters().discarded_while_busy,
            1,
            "{:?}",
            frame.header.frame_type
        );
        assert_eq!(core.snapshot().request_id, Some(id));
        assert_eq!(core.poll_action(), Action::Idle);
        // The reply still goes out, and nothing answers the dropped frame.
        send_reply(&mut core, id);
        assert_eq!(core.state(), MasterState::Idle);
        assert_eq!(core.poll_action(), Action::Idle);
    }
}

#[test]
fn broadcast_data_is_delivered_while_answering() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let id = receive_request(&mut core);
    for (kind, npdu) in [
        (FrameType::BACNET_DATA_EXPECTING_REPLY, [1u8, 0x04]),
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, [2u8, 0x00]),
    ] {
        core.on_validated_frame(Frame::new(kind, 0xFF, 5, &npdu), ms(10));
        assert_eq!(
            core.poll_action(),
            Action::DeliverNpdu {
                source: 5,
                destination: 0xFF,
                npdu: &npdu,
                request_id: None,
            }
        );
    }
    assert_eq!(core.counters().discarded_while_busy, 0);
    assert_eq!(core.snapshot().request_id, Some(id));
}

#[test]
fn a_later_frame_replaces_a_kept_token() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let id = receive_request(&mut core);
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 1, 2, &[]), ms(10));
    // The token moved on to station 3: the one kept for this station is stale.
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 3, 2, &[]), ms(20));
    send_reply(&mut core, id);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

#[test]
fn a_kept_token_is_used_when_the_request_is_abandoned() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let id = receive_request(&mut core);
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 1, 2, &[]), ms(10));
    core.complete_request(id, ReplyDecision::Abandoned, ms(20))
        .unwrap();
    // With no successor known, using the token starts with a poll, sent at
    // once since the line has been quiet longer than the turnaround.
    assert_eq!(core.state(), MasterState::PollForMaster);
    let (_, header) = take_transmit(&mut core, ms(20));
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
}

/// The reply times: give-up is 255 ms after the request at 1 ms, less one
/// octet at 9600 baud (1042 µs) and the 1 ms guard.
const GIVE_UP: Instant = Instant::from_micros(1_000 + 255_000 - 1_042 - 1_000);

/// Answer at 1.5 ms, inside the turnaround, then put noise on the line
/// every 2 ms from 2 ms while `until` holds.
fn reply_under_noise(
    core: &mut MasterCore<'_, Queues>,
    until: impl Fn(Instant) -> bool,
) -> Instant {
    let id = receive_request(core);
    core.complete_request(id, ReplyDecision::Data(&[9]), Instant::from_micros(1_500))
        .unwrap();
    let mut now = ms(2);
    let mut last = now;
    while until(now) {
        core.on_receive_error(ReceiveError::Framing, now);
        last = now;
        now += Duration::from_millis(2);
    }
    last
}

#[test]
fn the_reply_window_closes_under_steady_line_activity() {
    // The noise keeps the reply waiting until the requester may have given
    // up; the next noise drops the reply instead of holding the core.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let last = reply_under_noise(&mut core, |now| now <= GIVE_UP + Duration::from_millis(1));
    assert!(last >= GIVE_UP);
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
    // The requester timed out and passes this station the token, which it
    // now takes up.
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 1, 7, &[]), ms(258));
    assert_eq!(core.state(), MasterState::PollForMaster);
}

#[test]
fn the_reply_window_closes_on_time_even_while_the_turnaround_moves() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let last = reply_under_noise(&mut core, |now| now <= ms(252));
    // The turnaround would end after the window closes, so the window is
    // the deadline.
    assert!(last + core.config().t_turnaround > GIVE_UP);
    assert_eq!(core.next_deadline(), Some(GIVE_UP));
    core.on_deadline(GIVE_UP);
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}
