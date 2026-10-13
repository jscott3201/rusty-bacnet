//! A frame that waits out its turnaround (Clause 9.5.5.1): one test per row
//! of the table in the core's `pending` module. Line activity only delays
//! it; the evidence its sending state watches for, or its limit, drops it.

mod support;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, Instant, MasterState, ReceiveError,
};

use support::*;

/// Rows "data frame" (Clause 9.5.6.3): a data frame waiting out its
/// turnaround is pushed back by activity from every ingress, not dropped,
/// and goes out T_turnaround after it.
#[test]
fn a_data_frame_waits_out_activity_from_every_ingress() {
    for ingress in 0..4 {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        enqueue(&mut core, false, ms(1));
        validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
        assert_eq!(core.next_deadline(), Some(ms(10) + turnaround(&core)));
        let activity = ms(12);
        match ingress {
            0 => core.on_octet(0x00, activity).unwrap(),
            1 => core
                .on_receive_error(ReceiveError::Framing, activity)
                .unwrap(),
            2 => core.on_host_activity(3, activity).unwrap(),
            _ => validated(
                &mut core,
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                BROADCAST_MAC,
                3,
                &[1, 0, 7],
                activity,
            ),
        }
        assert_eq!(core.state(), MasterState::UseToken, "ingress {ingress}");
        let due = activity + turnaround(&core);
        assert_eq!(core.next_deadline(), Some(due), "ingress {ingress}");
        if ingress == 3 {
            assert!(matches!(core.poll_action(), Action::DeliverNpdu { .. }));
        }
        core.on_deadline(due).unwrap();
        let (_, header) = handed_over(&mut core);
        assert!(is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER));
        assert_eq!(core.counters().cancelled_sends, 0);
    }
}

/// Row "data frame", its limit: activity that never leaves T_turnaround of
/// quiet for T_no_token after the frame was made drops it, counted, and the
/// station lets the token go.
#[test]
fn a_data_frame_kept_waiting_for_t_no_token_is_dropped() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    enqueue(&mut core, false, ms(1));
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    let mut at = ms(10);
    while at < ms(506) {
        at += Duration::from_millis(4);
        core.on_receive_error(ReceiveError::Framing, at).unwrap();
        assert_eq!(core.state(), MasterState::UseToken);
    }
    assert_eq!(core.counters().cancelled_sends, 0);
    // 510 ms: T_no_token after the frame was made.
    core.on_receive_error(ReceiveError::Framing, ms(510))
        .unwrap();
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.next_deadline(), Some(ms(1_010)));
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.counters().unicast.dequeued, 1);
}

/// Row "data frame", frames that complete meanwhile (Clause 9.5.5.1): data
/// needing no answer is delivered, frames asking this station for something
/// (and a Reply To Poll For Master nothing waits for) are dropped and
/// counted, and the waiting frame still goes out.
#[test]
fn frames_that_complete_while_a_holder_frame_waits() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    enqueue(&mut core, false, ms(1));
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    let frames = [
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, BROADCAST_MAC),
        (FrameType::TOKEN, STATION),
        (FrameType::POLL_FOR_MASTER, STATION),
        (FrameType::BACNET_DATA_EXPECTING_REPLY, STATION),
        (FrameType::TEST_REQUEST, STATION),
        (FrameType::BACNET_DATA_EXPECTING_REPLY, BROADCAST_MAC),
        (FrameType::REPLY_TO_POLL_FOR_MASTER, STATION),
        (FrameType::TOKEN, 0),
    ];
    let mut at = ms(10);
    for (kind, destination) in frames {
        at += Duration::from_millis(1);
        validated(&mut core, kind, destination, 3, &[1, 4, 7], at);
    }
    assert_eq!(core.counters().discarded_while_busy, 5);
    assert_eq!(core.state(), MasterState::UseToken);
    for _ in 0..2 {
        assert!(matches!(
            core.poll_action(),
            Action::DeliverNpdu {
                request_id: None,
                ..
            }
        ));
    }
    assert_eq!(core.poll_action(), Action::Idle);
    let due = at + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let header = send(&mut core, due);
    assert!(is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER));
    // The token use goes on: no successor is known, so the station polls.
    let (_, header) = send_next(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
}

/// Row "Token" (Clauses 9.5.6.4 and 9.5.6.5), the case #1618 reproduced: a
/// reply whose header CRC fails ends the wait at once, and the Token pass
/// that follows waits for the rest of the damaged frame, then goes out.
#[test]
fn the_token_pass_after_an_invalid_reply_waits_for_its_tail() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    pass_waiting(&mut core);
    let (passed, _) = send_next(&mut core);
    let used = passed + Duration::from_millis(5);
    validated(
        &mut core,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        BROADCAST_MAC,
        PEER,
        &[1, 0, 7],
        used,
    );
    assert_eq!(core.state(), MasterState::Idle);
    enqueue(&mut core, true, used);
    validated(
        &mut core,
        FrameType::TOKEN,
        STATION,
        PEER,
        &[],
        used + us(100),
    );
    let (_, header) = send_next(&mut core);
    assert!(is(header, FrameType::BACNET_DATA_EXPECTING_REPLY, PEER));
    assert_eq!(core.state(), MasterState::WaitForReply);

    let mut reply = encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        STATION,
        PEER,
        &[1, 0, 9],
    );
    reply[7] ^= 0x01;
    let first = core.next_deadline().unwrap() - Duration::from_millis(250);
    feed(&mut core, &reply[..8], first);
    assert_eq!(core.counters().parser_errors, 1);
    assert_eq!(core.state(), MasterState::PassToken);
    let ninth = first + Duration::from_micros(core.config().octet_time().as_micros() * 8);
    let tail = feed(&mut core, &reply[8..], ninth);
    let due = tail + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (_, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TOKEN, PEER));
    assert_eq!(core.counters().cancelled_sends, 0);
}

/// Row "Poll For Master" (Clause 9.5.6.5, NextStationUnknown): a poll a
/// token holder makes waits out noise in its turnaround.
#[test]
fn a_poll_waits_out_noise_in_its_turnaround() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    core.on_octet(0x00, ms(12)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    let due = ms(12) + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (_, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    assert_eq!(core.counters().cancelled_sends, 0);
}

/// Row "Test_Response" (Clauses 9.5.6.2 and 9.5.6.9): it waits out noise,
/// and is dropped once the requester may have stopped waiting: 255 ms less
/// one octet (1042 µs) and 1 ms after the request.
#[test]
fn a_test_response_waits_within_the_reply_window() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(&mut core, FrameType::TEST_REQUEST, STATION, 7, &[5], ms(1));
    core.on_octet(0x00, ms(2)).unwrap();
    let due = ms(2) + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (_, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TEST_RESPONSE, 7));

    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(&mut core, FrameType::TEST_REQUEST, STATION, 7, &[5], ms(1));
    let give_up = ms(1) + us(255_000 - 1_042 - 1_000);
    let mut at = ms(1);
    while at + Duration::from_millis(4) < give_up {
        at += Duration::from_millis(4);
        core.on_octet(0x00, at).unwrap();
    }
    assert_eq!(core.next_deadline(), Some(give_up));
    assert_eq!(core.counters().expired_requests, 0);
    core.on_deadline(give_up).unwrap();
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.counters().cancelled_sends, 0);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

/// Row "Reply To Poll For Master" (Clauses 9.5.6.2 and 9.5.6.8): it waits
/// out noise, counts as late after T_usage_delay (15 ms), and is dropped
/// once the poller may have moved on: 20 ms less one octet and 1 ms.
#[test]
fn a_reply_to_a_poll_waits_within_the_pollers_patience() {
    let give_up = ms(1) + us(20_000 - 1_042 - 1_000);
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::POLL_FOR_MASTER,
        STATION,
        7,
        &[],
        ms(1),
    );
    core.on_octet(0x00, ms(2)).unwrap();
    let due = ms(2) + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::REPLY_TO_POLL_FOR_MASTER, 7));
    core.on_transmit_started(id, ms(16)).unwrap();
    assert_eq!(core.counters().late_replies, 0);

    // Started after T_usage_delay: sent, counted late.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::POLL_FOR_MASTER,
        STATION,
        7,
        &[],
        ms(1),
    );
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    let (id, _) = handed_over(&mut core);
    core.on_transmit_started(id, ms(16) + us(1)).unwrap();
    assert_eq!(core.counters().late_replies, 1);

    // Started at the cutoff: refused.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::POLL_FOR_MASTER,
        STATION,
        7,
        &[],
        ms(1),
    );
    let due = core.next_deadline().unwrap();
    core.on_deadline(due).unwrap();
    let (id, _) = handed_over(&mut core);
    assert_eq!(
        core.on_transmit_started(id, give_up),
        Err(CoreError::RequestExpired)
    );
    assert_eq!(core.counters().expired_requests, 1);
    expect_receive_only(&mut core);

    // Kept waiting by noise until the cutoff: dropped.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::POLL_FOR_MASTER,
        STATION,
        7,
        &[],
        ms(1),
    );
    for at in [2, 6, 10, 14, 18] {
        core.on_octet(0x00, ms(at)).unwrap();
    }
    assert_eq!(core.next_deadline(), Some(give_up));
    core.on_deadline(give_up).unwrap();
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

/// Rows "Token retry" (Clause 9.5.6.6): a retry the adapter has not started
/// yet is pushed back by up to Nmin_octets (4) octets. The fifth shows the
/// successor using the token (SawTokenUser): the retry is dropped and the
/// station goes idle to take up what follows.
#[test]
fn a_token_retry_yields_to_a_successor_that_uses_the_token() {
    for octets in [4usize, 5] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let (handed, id) = retry_handed_over(&mut core);
        let last = feed(&mut core, &vec![0x00; octets], handed + us(500));
        if octets == 4 {
            assert_eq!(
                core.on_transmit_started(id, last + us(500)),
                Err(CoreError::TransmitDeferred)
            );
            expect_receive_only(&mut core);
            let due = last + turnaround(&core);
            assert_eq!(core.next_deadline(), Some(due));
            core.on_deadline(due).unwrap();
            let (again, header) = handed_over(&mut core);
            assert!(is(header, FrameType::TOKEN, PEER));
            assert_ne!(again, id);
            core.on_transmit_started(again, due).unwrap();
            assert_eq!(core.counters().cancelled_sends, 0);
        } else {
            assert_eq!(core.state(), MasterState::Idle);
            assert_eq!(core.counters().cancelled_sends, 1);
            expect_receive_only(&mut core);
            assert_eq!(
                core.on_transmit_started(id, last + us(500)),
                Err(CoreError::StaleTransmit)
            );
            assert_eq!(
                core.next_deadline(),
                Some(last + Duration::from_millis(500))
            );
        }
    }
}

/// Row "Token retry", a whole frame: a valid frame from the successor is
/// the same evidence, and IDLE takes it up.
#[test]
fn a_valid_frame_ends_a_waiting_token_retry() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let (handed, id) = retry_handed_over(&mut core);
    validated(
        &mut core,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        BROADCAST_MAC,
        PEER,
        &[1, 0, 7],
        handed + us(500),
    );
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert!(matches!(core.poll_action(), Action::DeliverNpdu { .. }));
    assert_eq!(
        core.on_transmit_started(id, handed + us(600)),
        Err(CoreError::StaleTransmit)
    );
}

/// Row "Token retry", its limit: with the longest T_turnaround (15 ms), two
/// octets push the retry past T_usage_timeout (20 ms) after it was made,
/// and it is dropped.
#[test]
fn a_token_retry_kept_waiting_past_t_usage_timeout_is_dropped() {
    let mut config = config();
    config.t_turnaround = Duration::from_millis(15);
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = Core::new(config, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    let (made, id) = retry_handed_over(&mut core);
    core.on_octet(0x00, made + Duration::from_millis(1))
        .unwrap();
    assert_eq!(
        core.on_transmit_started(id, made + Duration::from_millis(2)),
        Err(CoreError::TransmitDeferred)
    );
    expect_receive_only(&mut core);
    let due = made + Duration::from_millis(16);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (again, _) = handed_over(&mut core);
    core.on_octet(0x00, made + Duration::from_millis(17))
        .unwrap();
    assert_eq!(
        core.on_transmit_started(again, made + Duration::from_millis(18)),
        Err(CoreError::TransmitDeferred)
    );
    expect_receive_only(&mut core);
    let limit = made + Duration::from_millis(20);
    assert_eq!(core.next_deadline(), Some(limit));
    core.on_deadline(limit).unwrap();
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

/// Rows "token-generation poll" (Clause 9.5.6.7): the generating poll, not
/// yet started, is pushed back by up to Nmin_octets octets. The fifth shows
/// another station is active (SawFrame): the poll is dropped and the station
/// goes idle.
#[test]
fn a_generating_poll_yields_to_another_active_station() {
    for octets in [4usize, 5] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let id = generation_handed_over(&mut core);
        let last = feed(&mut core, &vec![0x00; octets], ms(511));
        if octets == 4 {
            assert_eq!(
                core.on_transmit_started(id, last + us(500)),
                Err(CoreError::TransmitDeferred)
            );
            expect_receive_only(&mut core);
            let due = last + turnaround(&core);
            core.on_deadline(due).unwrap();
            let (again, header) = handed_over(&mut core);
            assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
            core.on_transmit_started(again, due).unwrap();
            assert_eq!(core.counters().cancelled_sends, 0);
        } else {
            assert_eq!(core.state(), MasterState::Idle);
            assert_eq!(core.counters().cancelled_sends, 1);
            expect_receive_only(&mut core);
            assert_eq!(
                core.next_deadline(),
                Some(last + Duration::from_millis(500))
            );
        }
    }
}

/// Row "token-generation poll", a whole frame: a Token for this station
/// ends the poll's wait, and IDLE takes it up.
#[test]
fn a_valid_frame_ends_a_waiting_generating_poll() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let id = generation_handed_over(&mut core);
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(511));
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    // The Token is used: with no successor known, a poll waits its turn.
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.next_deadline(), Some(ms(511) + turnaround(&core)));
    assert_eq!(
        core.on_transmit_started(id, ms(512)),
        Err(CoreError::StaleTransmit)
    );
}

/// Row "poll for a new successor" (Clause 9.5.6.6, FindNewSuccessor): it
/// counts events since the retry went out, so octets heard after the retry
/// carry over. Three then and two more while the poll waits for its adapter
/// make five: the successor is using the token (SawTokenUser), and the poll
/// is dropped. One more alone only defers it.
#[test]
fn a_new_successor_poll_yields_to_a_successor_that_uses_the_token() {
    for (later, dropped) in [(1usize, false), (2, true)] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let (handed, retry) = retry_handed_over(&mut core);
        core.on_transmit_started(retry, handed).unwrap();
        let sent = handed + Duration::from_millis(1);
        core.on_transmit_complete(retry, sent).unwrap();
        assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
        let heard = feed(&mut core, &[0x00; 3], sent + Duration::from_millis(1));
        assert_eq!(core.state(), MasterState::PassToken);
        let timeout = heard + Duration::from_millis(20);
        assert_eq!(core.next_deadline(), Some(timeout));
        core.on_deadline(timeout).unwrap();
        let (poll, header) = handed_over(&mut core);
        assert!(is(header, FrameType::POLL_FOR_MASTER, 3));
        let last = feed(&mut core, &vec![0x00; later], timeout + us(500));
        if dropped {
            assert_eq!(core.state(), MasterState::Idle);
            assert_eq!(core.counters().cancelled_sends, 1);
            expect_receive_only(&mut core);
        } else {
            assert_eq!(
                core.on_transmit_started(poll, last + us(500)),
                Err(CoreError::TransmitDeferred)
            );
            assert_eq!(core.state(), MasterState::PollForMaster);
            assert_eq!(core.counters().cancelled_sends, 0);
        }
    }
}

/// Row "Token retry": the retry counts events from when it is made. Four
/// octets heard before it, too few for SawTokenUser, do not carry over, so
/// four more while it waits for its adapter only defer it.
#[test]
fn a_token_retry_counts_events_from_when_it_is_made() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    pass_waiting(&mut core);
    let (passed, _) = send_next(&mut core);
    let heard = feed(&mut core, &[0x00; 4], passed + Duration::from_millis(2));
    assert_eq!(core.state(), MasterState::PassToken);
    let timeout = heard + Duration::from_millis(20);
    assert_eq!(core.next_deadline(), Some(timeout));
    core.on_deadline(timeout).unwrap();
    let (retry, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TOKEN, PEER));
    let last = feed(&mut core, &[0x00; 4], timeout + us(500));
    assert_eq!(
        core.on_transmit_started(retry, last + us(500)),
        Err(CoreError::TransmitDeferred)
    );
    assert_eq!(core.state(), MasterState::PassToken);
    assert_eq!(core.counters().cancelled_sends, 0);
}

/// Row "token-generation poll": it counts events from when NO_TOKEN began
/// (LostToken zeroes EventCount, Clause 9.5.6.2), not the octets of a frame
/// heard long before. One octet while it waits only defers it.
#[test]
fn a_generating_poll_counts_events_from_no_token() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let heard = feed(&mut core, &encode(FrameType::TOKEN, 3, 0, &[]), ms(1));
    let silent = heard + Duration::from_millis(500);
    assert_eq!(core.next_deadline(), Some(silent));
    core.on_deadline(silent).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    let slot = silent + Duration::from_millis(10);
    core.on_deadline(slot).unwrap();
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    core.on_octet(0x00, slot + us(500)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(
        core.on_transmit_started(id, slot + us(600)),
        Err(CoreError::TransmitDeferred)
    );
    assert_eq!(core.counters().cancelled_sends, 0);
}

/// Row "token-generation poll", its limit: an adapter that takes almost
/// T_no_token to get to the poll, with an octet just before, defers it into
/// its limit, T_no_token after it was made, where it is dropped.
#[test]
fn a_generating_poll_kept_waiting_for_t_no_token_is_dropped() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let id = generation_handed_over(&mut core);
    core.on_octet(0x00, ms(1_008)).unwrap();
    assert_eq!(
        core.on_transmit_started(id, ms(1_009)),
        Err(CoreError::TransmitDeferred)
    );
    expect_receive_only(&mut core);
    assert_eq!(core.next_deadline(), Some(ms(1_010)));
    core.on_deadline(ms(1_010)).unwrap();
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

/// Row "Poll For Master", an invalid frame meanwhile: a header that fails
/// its CRC while the poll waits is activity for the poll, not an invalid
/// answer to a poll that has not gone out (Clause 9.5.6.8).
#[test]
fn an_invalid_frame_while_a_poll_waits_only_delays_it() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let mut bad = encode(FrameType::REPLY_TO_POLL_FOR_MASTER, STATION, 3, &[]);
    bad[7] ^= 0x01;
    let last = feed(&mut core, &bad, ms(11));
    assert_eq!(core.counters().parser_errors, 1);
    assert_eq!(core.snapshot().poll_station, PEER);
    let due = last + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due).unwrap();
    let (_, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
}

/// Row "token-generation poll", its limit at a refused start: a start the
/// line refuses once the limit has passed drops the poll at once, rather
/// than leave it waiting on a deadline already due.
#[test]
fn a_start_refused_past_the_limit_drops_the_frame_at_once() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let id = generation_handed_over(&mut core);
    core.on_octet(0x00, ms(1_008)).unwrap();
    assert_eq!(
        core.on_transmit_started(id, ms(1_011)),
        Err(CoreError::TransmitDeferred)
    );
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.counters().deferred_starts, 1);
    expect_receive_only(&mut core);
    assert_eq!(core.next_deadline(), Some(ms(1_508)));
}
