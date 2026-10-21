//! The clock rule (#1684): a `now` earlier than the latest one passed in is
//! taken as that latest one. Each test moves the core's clock forward with a
//! call that changes nothing else (a transmit event for an id the core
//! doesn't hold, refused as stale, or the station's own echo), then makes a
//! call stamped earlier, at a moment where the latest time and the stamp
//! lead to different outcomes. The test pins the outcome of the latest time.

mod support;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{
    Action, Duration, MasterState, ReplyDecision, RequestError, TransmitFailure, TransmitId,
};

use support::*;

/// An id the core never handed out in these tests.
const STRANGER: TransmitId = TransmitId::new(9_999);

/// Move the core's clock to `at` without changing anything else: a
/// completion for an id it doesn't hold is ignored, but its time is taken.
fn move_clock(core: &mut Core<'_>, at: bacnet_mstp_core::Instant) {
    let before = (core.snapshot(), core.counters());
    core.on_transmit_complete(STRANGER, at);
    assert_eq!((core.snapshot(), core.counters()), before);
}

/// `on_deadline` stamped before a deadline that the core's clock has passed
/// services it: the lost-token wait of 500 ms is over at 600 ms.
#[test]
fn a_deadline_call_stamped_early_services_what_is_due_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    move_clock(&mut core, ms(600));
    assert_eq!(core.state(), MasterState::Idle);
    core.on_deadline(ms(100));
    assert_eq!(core.state(), MasterState::NoToken);
    // Serviced 100 ms late, at 600 ms rather than at the stamp.
    assert_eq!(core.counters().late_deadlines, 1);
    let lateness = core.counters().deadline_lateness;
    assert_eq!(lateness.buckets, [0, 0, 0, 0, 0, 1]);
    assert_eq!(lateness.max, bacnet_mstp_core::Duration::from_millis(100));
}

/// `on_silence` stamped early drops a partial frame whose T_frame_abort the
/// core's clock has passed.
#[test]
fn a_silence_stamped_early_aborts_a_partial_frame_that_is_late_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let token = encode(FrameType::TOKEN, STATION, PEER, &[]);
    let last = feed(&mut core, &token[..4], ms(10));
    let abort = core.config().t_frame_abort;
    move_clock(&mut core, last + abort + Duration::from_millis(5));
    core.on_silence(last + Duration::from_millis(1));
    let counters = core.counters();
    assert_eq!((counters.frame_aborts, counters.parser_errors), (1, 1));
}

/// A data answer stamped before the decision deadline, given once the
/// core's clock has passed it, is too late: the core keeps the request and
/// will send Reply Postponed itself.
#[test]
fn an_answer_stamped_early_is_judged_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        STATION,
        PEER,
        &[0x01, 0x04, 0x07],
        ms(10),
    );
    let id = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected a request, got {other:?}"),
    };
    // T_reply_delay 250 ms less the 25 ms margin: the answer was due by
    // 235 ms. The requester waits until about 263 ms.
    move_clock(&mut core, ms(240));
    assert_eq!(
        core.complete_request(id, ReplyDecision::Data(&[0x01, 0x00, 0x07]), ms(20)),
        Err(RequestError::Expired)
    );
    assert_eq!(core.snapshot().request_id, Some(id));
    assert_eq!(core.state(), MasterState::AnswerDataRequest);
}

/// A start stamped within T_turnaround of line activity, reported once the
/// core's clock is past that turnaround, is taken at the latest time: the
/// line has been quiet long enough, so the start goes ahead.
#[test]
fn a_start_stamped_early_is_judged_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    // An octet after the hand-over pushes the earliest start back by
    // T_turnaround from it.
    let octet = handed + Duration::from_millis(1);
    core.on_octet(0x00, octet);
    let quiet = octet + turnaround(&core);
    move_clock(&mut core, quiet + Duration::from_millis(1));
    core.on_transmit_started(id, octet + Duration::from_millis(1))
        .unwrap();
    assert_eq!(core.counters().deferred_starts, 0);
    assert_eq!(core.snapshot().transmit_id, Some(id));
}

/// The poll handed over and started at `start`, with this station's echo
/// heard at `echo`, which moves the clock while the driver is on.
fn started_with_echo(
    core: &mut Core<'_>,
    echo: bacnet_mstp_core::Instant,
) -> (TransmitId, bacnet_mstp_core::Instant) {
    poll_waiting(core);
    let start = core.next_deadline().unwrap();
    core.on_deadline(start);
    let (id, _) = handed_over(core);
    core.on_transmit_started(id, start).unwrap();
    // Echo while the driver is on: taken for the station's own, ignored.
    core.on_octet(0x55, echo);
    (id, start)
}

/// A completion stamped before the latest time ends the frame at the latest
/// time: the driver-on time and the wait for the poll's answer run from
/// there.
#[test]
fn a_completion_stamped_early_ends_the_frame_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let echo = ms(40);
    let (id, start) = started_with_echo(&mut core, echo);
    core.on_transmit_complete(id, start + Duration::from_millis(5));
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(
        core.next_deadline(),
        Some(echo + core.config().t_usage_timeout)
    );
    assert_eq!(core.counters().de_assertion, echo - start);
}

/// An abort stamped before the latest time ends the frame at the latest
/// time: the driver-on time and the wait for a lost token run from there.
#[test]
fn an_abort_stamped_early_ends_the_frame_at_the_latest_time() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let echo = ms(40);
    let (id, start) = started_with_echo(&mut core, echo);
    core.on_transmit_aborted(id, TransmitFailure::Io, start + Duration::from_millis(5));
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(
        core.next_deadline(),
        Some(echo + Duration::from_millis(500))
    );
    assert_eq!(core.counters().de_assertion, echo - start);
}
