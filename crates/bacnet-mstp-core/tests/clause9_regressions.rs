//! Public-API regressions for Clause 9 transitions and local scheduling.

use bacnet_mstp_codec::{decode_frame, Frame, FrameHeader, FrameType};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, NetworkPriority, ReplyDecision, TransmitFailure, TransmitId,
};

type Queues = HeaplessQueues<16, 16>;
type Actions = bacnet_mstp_core::ActionBuffers;

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

fn config(station: u8, max_master: u8) -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = station;
    config.max_master = max_master;
    config
}

fn validated(
    core: &mut MasterCore<'_, Queues>,
    kind: FrameType,
    destination: u8,
    source: u8,
    data: &[u8],
    now: Instant,
) {
    core.on_validated_frame(Frame::new(kind, destination, source, data), now)
        .unwrap();
}

fn take_transmit(core: &mut MasterCore<'_, Queues>, now: Instant) -> (TransmitId, FrameHeader) {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let (id, header) = match core.poll_action() {
        Action::Transmit { id, frame } => {
            let (decoded, _) = decode_frame(frame).unwrap();
            (id, decoded.header)
        }
        other => panic!("expected transmit, got {other:?}"),
    };
    core.on_transmit_started(id, now).unwrap();
    (id, header)
}

fn finish(core: &mut MasterCore<'_, Queues>, id: TransmitId, now: Instant) {
    core.on_transmit_complete(id, now).unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
}

fn discover_successor_three(core: &mut MasterCore<'_, Queues>) {
    core.on_deadline(ms(500)).unwrap();
    core.on_deadline(ms(510)).unwrap();
    let (poll_two, header) = take_transmit(core, ms(510));
    assert_eq!(header.destination, 2);
    finish(core, poll_two, ms(511));
    core.on_deadline(ms(531)).unwrap();
    let (poll_three, header) = take_transmit(core, ms(531));
    assert_eq!(header.destination, 3);
    finish(core, poll_three, ms(532));
    validated(
        core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        1,
        3,
        &[],
        ms(533),
    );
    core.on_deadline(ms(538)).unwrap();
}

#[test]
fn pass_token_retries_once_then_polls_after_failed_successor() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    discover_successor_three(&mut core);

    let (token, header) = take_transmit(&mut core, ms(538));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::TOKEN, 3)
    );
    finish(&mut core, token, ms(539));

    core.on_deadline(ms(559)).unwrap();
    let (retry, header) = take_transmit(&mut core, ms(559));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::TOKEN, 3)
    );
    finish(&mut core, retry, ms(560));
    assert_eq!(core.counters().token_retries, 1);

    core.on_deadline(ms(580)).unwrap();
    let (_poll, header) = take_transmit(&mut core, ms(580));
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(
        header.destination, 0,
        "failed successor search wraps at Max_Master"
    );
    assert_eq!(core.counters().token_retries, 1);
}

#[test]
fn regeneration_forgets_a_stale_successor_and_restarts_at_ts_plus_one() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    discover_successor_three(&mut core);
    let (token, _) = take_transmit(&mut core, ms(538));
    finish(&mut core, token, ms(539));
    validated(
        &mut core,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0,
        3,
        &[1],
        ms(540),
    );
    assert_eq!(core.state(), MasterState::Idle);

    core.on_deadline(ms(1_040)).unwrap();
    core.on_deadline(ms(1_050)).unwrap();
    let (_poll, header) = take_transmit(&mut core, ms(1_050));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 2)
    );
    assert_eq!(core.counters().token_regenerations, 2);
}

// With the request at 1 ms, the decision deadline is 226 ms (T_reply_delay
// 250 ms less the 25 ms transmit margin), and the requester may stop waiting
// from 256 ms (the shortest T_reply_timeout, 255 ms).
#[test]
fn late_request_data_is_rejected_and_deadline_sends_postponed() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        ms(1),
    );
    let request = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    let due = ms(226);
    assert_eq!(core.next_deadline(), Some(due));
    assert_eq!(
        core.complete_request(request, ReplyDecision::Data(&[9]), due),
        Err(CoreError::RequestExpired)
    );
    assert_eq!(core.snapshot().request_id, Some(request));
    core.on_deadline(due).unwrap();
    let (_tx, header) = take_transmit(&mut core, due);
    assert_eq!(header.frame_type, FrameType::REPLY_POSTPONED);
    assert_eq!(header.destination, 7);
    assert!(core.counters().state_transitions > 0);
    assert_eq!(core.counters().deadline_misses, 0);
    assert_eq!(core.counters().late_replies, 0);

    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        ms(1),
    );
    let request = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    assert_eq!(
        core.complete_request(request, ReplyDecision::Data(&[9]), ms(256)),
        Err(CoreError::RequestExpired)
    );
    assert_eq!(core.snapshot().request_id, None);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.counters().expired_requests, 1);

    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        ms(1),
    );
    let request = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    core.complete_request(
        request,
        ReplyDecision::Data(&[9]),
        ms(226) - Duration::from_micros(1),
    )
    .unwrap();
    assert_eq!(core.snapshot().request_id, None);
    assert!(core.counters().npdus_delivered > 0);
}

#[test]
fn request_ownership_expires_without_a_late_postponed_transmission() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        ms(1),
    );
    let request = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    // Intervening token traffic cannot transfer ownership away from the
    // request, but it also cannot justify a response after Treply_timeout.
    // The Token is kept for when the request ends: then it is used, here to
    // poll for a successor, and no Reply Postponed goes out.
    validated(&mut core, FrameType::TOKEN, 1, 3, &[], ms(100));
    assert_eq!(core.snapshot().request_id, Some(request));
    core.on_deadline(ms(302)).unwrap();
    assert_eq!(core.snapshot().request_id, None);
    assert_eq!(core.state(), MasterState::PollForMaster);
    let (_poll, header) = take_transmit(&mut core, ms(302));
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    assert_eq!(
        core.complete_request(request, ReplyDecision::Postponed, ms(303)),
        Err(CoreError::NoPendingRequest)
    );
    assert!(core.counters().deadline_misses > 0);
    assert_eq!(core.counters().expired_requests, 1);

    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut cfg = config(1, 3);
    cfg.t_reply_timeout = Duration::from_millis(300);
    let mut core =
        MasterCore::new(cfg, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        Instant::ZERO,
    );
    let _ = core.poll_action();
    // The requester's own reply timeout doesn't matter: any requester may
    // stop waiting from 255 ms.
    core.on_deadline(ms(280)).unwrap();
    assert_eq!(core.snapshot().request_id, None);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.counters().expired_requests, 1);
}

#[test]
fn deferred_application_reply_is_not_dispatched_after_its_deadline() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        1,
        7,
        &[1, 0],
        Instant::ZERO,
    );
    let request = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    core.complete_request(request, ReplyDecision::Data(&[9]), ms(1))
        .unwrap();
    assert_eq!(core.snapshot().request_id, None);
    core.on_deadline(ms(300)).unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
    assert!(core.counters().deadline_misses > 0);
    assert_eq!(core.counters().expired_requests, 1);
}

#[test]
fn configured_ratios_preserve_max_info_frames_and_weight_broadcasts() {
    let mut cfg = config(1, 2);
    cfg.max_info_frames = 4;
    cfg.multiple_frame_ratio = 100;
    cfg.broadcast_unicast_ratio = 2;
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core =
        MasterCore::new(cfg, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    for destination in [0xff, 0xff, 2, 2] {
        core.enqueue_npdu(destination, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
            .unwrap();
    }
    validated(&mut core, FrameType::TOKEN, 1, 2, &[], Instant::ZERO);

    let mut destinations = [0; 4];
    for destination in &mut destinations {
        let due = core.next_deadline().unwrap();
        core.on_deadline(due).unwrap();
        let (id, header) = take_transmit(&mut core, due);
        *destination = header.destination;
        assert_eq!(
            header.frame_type,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
        );
        finish(&mut core, id, due + Duration::from_millis(1));
    }
    assert_eq!(destinations, [0xff, 0xff, 2, 2]);
    assert_eq!(core.counters().frames_transmitted, 4);
}

#[test]
fn malformed_poll_response_advances_poll_and_sole_master_resumes_queued_work() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(0, 2),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.on_deadline(ms(500)).unwrap();
    let (poll, header) = take_transmit(&mut core, ms(500));
    assert_eq!(header.destination, 1);
    finish(&mut core, poll, ms(501));
    core.on_octet(0x55, ms(502)).unwrap();
    core.on_octet(0xff, ms(503)).unwrap();
    core.on_receive_error(bacnet_mstp_core::ReceiveError::Framing, ms(504))
        .unwrap();
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    let (_next_poll, header) = take_transmit(&mut core, due);
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 2)
    );

    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(0, 1),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.on_deadline(ms(500)).unwrap();
    let (search, _) = take_transmit(&mut core, ms(500));
    finish(&mut core, search, ms(501));
    core.enqueue_npdu(1, &[1, 0], NetworkPriority::Normal, ms(510))
        .unwrap();
    core.on_deadline(ms(521)).unwrap();
    let (data, header) = take_transmit(&mut core, ms(521));
    assert_eq!(
        header.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );
    finish(&mut core, data, ms(522));
    let maintenance_due = core.next_deadline().unwrap();
    core.on_deadline(maintenance_due).unwrap();
    let (maintenance, header) = take_transmit(&mut core, maintenance_due);
    assert_eq!(header.frame_type, FrameType::POLL_FOR_MASTER);
    finish(
        &mut core,
        maintenance,
        maintenance_due + Duration::from_millis(1),
    );
    core.enqueue_npdu(
        1,
        &[2, 0],
        NetworkPriority::Normal,
        maintenance_due + Duration::from_millis(2),
    )
    .unwrap();
    let usage_timeout = core.next_deadline().unwrap();
    core.on_deadline(usage_timeout).unwrap();
    let (_data, header) = take_transmit(&mut core, usage_timeout);
    assert_eq!(
        header.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );
    assert!(core.counters().state_transitions > 0);
}

#[test]
fn malformed_header_resync_bytes_do_not_create_a_second_invalid_frame() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(0, 2),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.on_deadline(ms(500)).unwrap();
    let (poll, _) = take_transmit(&mut core, ms(500));
    finish(&mut core, poll, ms(501));

    let mut bad = [0; 32];
    let length = Frame::new(FrameType::REPLY_TO_POLL_FOR_MASTER, 0, 1, &[])
        .encode_into(&mut bad)
        .unwrap();
    bad[7] ^= 0x01;
    for (index, byte) in bad[..length].iter().copied().enumerate() {
        core.on_octet(byte, ms(502) + Duration::from_micros(index as u64))
            .unwrap();
    }
    assert_eq!(core.counters().parser_errors, 1);
    let turnaround = core.next_deadline().unwrap();
    core.on_deadline(turnaround).unwrap();
    let (_next_poll, header) = take_transmit(&mut core, turnaround);
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 2)
    );
    core.on_silence(turnaround + core.config().t_frame_abort + Duration::from_micros(1))
        .unwrap();
    assert_eq!(core.counters().parser_errors, 1);
}

#[test]
fn reply_timeout_ends_token_use_and_pass_token_event_count_is_strict() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut cfg = config(1, 3);
    cfg.max_info_frames = 3;
    let mut core =
        MasterCore::new(cfg, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    discover_successor_three(&mut core);
    let (token, _) = take_transmit(&mut core, ms(538));
    finish(&mut core, token, ms(539));
    for value in 0..4 {
        core.on_octet(value, ms(540 + u64::from(value))).unwrap();
    }
    assert_eq!(core.state(), MasterState::PassToken);
    core.on_octet(4, ms(544)).unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    assert!(core.counters().state_transitions > 0);

    // Rebuild the known-successor path and prove a WaitForReply timeout passes
    // the token instead of consuming another queued information frame.
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core =
        MasterCore::new(cfg, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    discover_successor_three(&mut core);
    let (token, _) = take_transmit(&mut core, ms(538));
    finish(&mut core, token, ms(539));
    core.enqueue_npdu(3, &[1, 0x04], NetworkPriority::Normal, ms(540))
        .unwrap();
    core.enqueue_npdu(3, &[2, 0], NetworkPriority::Normal, ms(540))
        .unwrap();
    validated(&mut core, FrameType::TOKEN, 1, 3, &[], ms(541));
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    let (request, header) = take_transmit(&mut core, due);
    assert_eq!(header.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    finish(&mut core, request, due + Duration::from_millis(1));
    let reply_timeout = core.next_deadline().unwrap();
    core.on_deadline(reply_timeout).unwrap();
    let (_token, header) = take_transmit(&mut core, reply_timeout);
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::TOKEN, 3)
    );
    assert_eq!(core.counters().unicast.depth, 1);
}

#[test]
fn host_chunk_activity_prevents_retry_during_a_fragmented_successor_frame() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    discover_successor_three(&mut core);
    let (token, header) = take_transmit(&mut core, ms(538));
    assert_eq!(header.frame_type, FrameType::TOKEN);
    finish(&mut core, token, ms(539));
    assert_eq!(core.state(), MasterState::PassToken);

    core.on_host_activity(5, ms(550)).unwrap();

    assert_eq!(core.state(), MasterState::Idle);
    core.on_deadline(ms(559)).unwrap();
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.next_deadline(), Some(ms(1050)));
}

/// A frame cut short after its start still drove the line, and each octet it
/// sent restarted the silence (Clause 9.5.5.1). So the token-generation slot
/// (Clause 9.5.6.7) counts from the abort, not from the activity before the
/// frame. Found by the core fuzz target (#1640).
#[test]
fn an_aborted_frame_restarts_the_silence_before_token_generation() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(2, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    validated(&mut core, FrameType::POLL_FOR_MASTER, 2, 1, &[], ms(1));
    core.on_deadline(ms(6)).unwrap();
    let (reply, header) = take_transmit(&mut core, ms(6));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::REPLY_TO_POLL_FOR_MASTER, 1)
    );
    core.on_transmit_aborted(reply, TransmitFailure::Aborted, ms(100))
        .unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));

    // T_no_token after the abort the station waits for its own slot, two
    // slots in, rather than for a window counted from before the frame.
    core.on_deadline(ms(600)).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.next_deadline(), Some(ms(620)));
    core.on_deadline(ms(620)).unwrap();
    let (_poll, header) = take_transmit(&mut core, ms(620));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 3)
    );
    assert_eq!(core.counters().missed_generation_slots, 0);
}

/// A station above Max_Master that answers a poll becomes the successor
/// (ReceivedReplyToPFM, Clause 9.5.6.8). When it never uses the token, the
/// search for a new successor starts at (NS + 1) modulo (Max_Master + 1)
/// (FindNewSuccessor, Clause 9.5.6.6): 3 for NS 130 and Max_Master 7, not
/// 0. Found by the core fuzz target (#1640).
#[test]
fn a_successor_search_past_one_above_max_master_starts_at_ns_plus_one() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 7),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.on_deadline(ms(500)).unwrap();
    core.on_deadline(ms(510)).unwrap();
    let (poll, header) = take_transmit(&mut core, ms(510));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 2)
    );
    finish(&mut core, poll, ms(511));
    validated(
        &mut core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        1,
        130,
        &[],
        ms(512),
    );
    core.on_deadline(ms(517)).unwrap();
    let (token, header) = take_transmit(&mut core, ms(517));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::TOKEN, 130)
    );
    finish(&mut core, token, ms(518));
    core.on_deadline(ms(538)).unwrap();
    let (retry, header) = take_transmit(&mut core, ms(538));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::TOKEN, 130)
    );
    finish(&mut core, retry, ms(539));

    core.on_deadline(ms(559)).unwrap();
    let (_poll, header) = take_transmit(&mut core, ms(559));
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::POLL_FOR_MASTER, 3)
    );
}

/// With a successor above Max_Master, the address after PS never equals NS,
/// so the maintenance polls every 50 token uses (Clause 9.5.6.5) would walk
/// on round to this station itself. They poll every other address, then the
/// sweep ends as ResetMaintenancePFM ends it. Found in review of the core
/// fuzz target (#1640).
#[test]
fn maintenance_polls_never_reach_this_station_with_a_successor_above_max_master() {
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(1, 3),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.on_deadline(ms(500)).unwrap();
    core.on_deadline(ms(510)).unwrap();
    let (poll, _) = take_transmit(&mut core, ms(510));
    finish(&mut core, poll, ms(511));
    validated(
        &mut core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        1,
        130,
        &[],
        ms(512),
    );
    core.on_deadline(ms(517)).unwrap();
    let (token, _) = take_transmit(&mut core, ms(517));
    finish(&mut core, token, ms(518));

    // Station 130 hands the token straight back, round after round; after a
    // poll the wait times out and the token goes to 130 again.
    let mut polls = Vec::new();
    for round in 0..60u64 {
        let at = 600 + 100 * round;
        validated(&mut core, FrameType::TOKEN, 1, 130, &[], ms(at));
        core.on_deadline(ms(at + 5)).unwrap();
        let (first, header) = take_transmit(&mut core, ms(at + 5));
        finish(&mut core, first, ms(at + 6));
        if header.frame_type == FrameType::POLL_FOR_MASTER {
            polls.push((round, header.destination));
            core.on_deadline(ms(at + 26)).unwrap();
            let (token, header) = take_transmit(&mut core, ms(at + 26));
            assert_eq!(header.destination, 130);
            finish(&mut core, token, ms(at + 27));
        } else {
            assert_eq!(
                (header.frame_type, header.destination),
                (FrameType::TOKEN, 130)
            );
        }
    }
    assert_eq!(polls, [(50, 2), (51, 3), (52, 0)]);
}
