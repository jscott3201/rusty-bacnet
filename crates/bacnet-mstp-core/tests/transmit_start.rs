//! From the hand-over to the confirmed start (#1625). Octets and receive
//! errors in that time are line input, not echo; a start within
//! T_turnaround of such activity is refused and the frame waits again
//! (Clause 9.5.5.1). Only after the confirmed start is input taken for the
//! station's own echo.

mod support;

use bacnet_mstp_codec::{BROADCAST_MAC, FrameType};
use bacnet_mstp_core::{
    Action, Direction, Duration, MasterState, ReceiveError, ReplyDecision, TransmitError,
    TransmitFailure, TransmitId,
};

use support::*;

/// A whole frame that arrives between the hand-over and the start is
/// received and delivered, and the start it overlaps is refused: the poll
/// waits T_turnaround after the frame, then comes back under a new id.
#[test]
fn a_frame_heard_before_the_confirmed_start_is_received() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    let (id, _) = handed_over(&mut core);
    let frame = encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        BROADCAST_MAC,
        3,
        &[1, 0, 7],
    );
    let last = feed(&mut core, &frame, handed + us(300));
    assert_eq!(core.counters().frames_received, 2);
    assert_eq!(
        core.on_transmit_started(id, last + us(300)),
        Err(TransmitError::Deferred)
    );
    assert_eq!(core.counters().deferred_starts, 1);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert!(matches!(
        core.poll_action(),
        Action::DeliverNpdu { source: 3, .. }
    ));
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.state(), MasterState::PollForMaster);
    let due = last + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due);
    let (again, header) = handed_over(&mut core);
    assert_ne!(again, id);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    assert_eq!(core.on_transmit_started(id, due), Err(TransmitError::Stale));
    core.on_transmit_started(again, due).unwrap();
    assert_eq!(core.counters().cancelled_sends, 0);
}

/// A receive error, or octets a host adapter reports, between the
/// hand-over and the start are line activity just as an octet is.
#[test]
fn other_input_before_the_confirmed_start_defers_it() {
    for host in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let handed = core.next_deadline().unwrap();
        core.on_deadline(handed);
        let (id, _) = handed_over(&mut core);
        let heard = handed + Duration::from_millis(1);
        if host {
            core.on_host_activity(2, heard);
        } else {
            core.on_receive_error(ReceiveError::Framing, heard);
        }
        assert_eq!(
            core.on_transmit_started(id, heard + us(100)),
            Err(TransmitError::Deferred),
            "host {host}"
        );
        assert_eq!(core.next_deadline(), Some(heard + turnaround(&core)));
    }
}

/// Only activity within T_turnaround of the start refuses it: a start that
/// comes a full turnaround after the activity goes ahead.
#[test]
fn a_start_a_full_turnaround_after_the_activity_goes_ahead() {
    for (late, refused) in [(Duration::ZERO, false), (us(1), true)] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let handed = core.next_deadline().unwrap();
        core.on_deadline(handed);
        let (id, _) = handed_over(&mut core);
        let noise = handed + Duration::from_millis(1);
        core.on_octet(0x00, noise);
        let start = noise + turnaround(&core) - late;
        let result = core.on_transmit_started(id, start);
        if refused {
            assert_eq!(result, Err(TransmitError::Deferred));
        } else {
            assert!(result.is_ok());
            assert_eq!(core.snapshot().transmit_id, Some(id));
        }
    }
}

/// After the confirmed start, octets, receive errors and host-reported
/// octets are the station's own echo: they change no count, state or
/// deadline. Here a whole Token echoes back during the station's own Token
/// pass.
#[test]
fn input_after_the_confirmed_start_is_echo() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    pass_waiting(&mut core);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    let (id, _) = handed_over(&mut core);
    core.on_transmit_started(id, due).unwrap();
    let before = (core.counters(), core.state(), core.next_deadline());
    let echo = encode(FrameType::TOKEN, PEER, STATION, &[]);
    let last = feed(&mut core, &echo, due + us(100));
    for step in 1..=5 {
        core.on_receive_error(ReceiveError::Framing, last + us(step));
    }
    core.on_host_activity(8, last + us(6));
    assert_eq!(
        (core.counters(), core.state(), core.next_deadline()),
        before
    );
    let end = last + us(10);
    core.on_transmit_complete(id, end);
    assert_eq!(core.state(), MasterState::PassToken);
    assert_eq!(core.next_deadline(), Some(end + Duration::from_millis(20)));
}

/// An answer handed over and kept waiting by activity is dropped once its
/// requester may have stopped waiting, and taken back from the adapter.
/// Earlier activity only defers it.
#[test]
fn a_reply_handed_over_waits_within_its_window() {
    let request = ms(1);
    let give_up = request + us(255_000 - 1_042 - 1_000);
    for activity in [ms(10), give_up] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        validated(
            &mut core,
            FrameType::BACNET_DATA_EXPECTING_REPLY,
            STATION,
            7,
            &[1, 4],
            request,
        );
        let Action::DeliverNpdu {
            request_id: Some(asked),
            ..
        } = core.poll_action()
        else {
            panic!("expected the request");
        };
        core.complete_request(asked, ReplyDecision::Data(&[9]), ms(2))
            .unwrap();
        let handed = core.next_deadline().unwrap();
        core.on_deadline(handed);
        let (id, header) = handed_over(&mut core);
        assert!(is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 7));
        core.on_octet(0x00, activity);
        if activity == give_up {
            assert_eq!(core.counters().expired_requests, 1);
            assert_eq!(core.state(), MasterState::Idle);
            expect_receive_only(&mut core);
            assert_eq!(
                core.on_transmit_started(id, activity + us(1)),
                Err(TransmitError::Stale)
            );
        } else {
            assert_eq!(
                core.on_transmit_started(id, activity + us(1)),
                Err(TransmitError::Deferred)
            );
            expect_receive_only(&mut core);
            let due = activity + turnaround(&core);
            core.on_deadline(due);
            let (again, _) = handed_over(&mut core);
            core.on_transmit_started(again, due).unwrap();
            assert_eq!(core.counters().late_replies, 0);
            assert_eq!(core.counters().expired_requests, 0);
        }
    }
}

/// An adapter abort of a transmit it never started also gives up the
/// waiting frame: nothing comes back for it, and it is counted as given up.
#[test]
fn an_aborted_unstarted_transmit_is_not_sent_again() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    let (id, _) = handed_over(&mut core);
    core.on_transmit_aborted(id, TransmitFailure::Rejected, handed);
    expect_receive_only(&mut core);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(
        core.next_deadline(),
        Some(handed + Duration::from_millis(500))
    );
    assert_eq!(core.counters().cancelled_sends, 1);
    assert_eq!(core.counters().expired_requests, 0);
}

/// An answer the adapter aborts before starting it leaves its request
/// unanswered, so it counts as an expired request.
#[test]
fn an_aborted_unstarted_answer_counts_as_expired() {
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
    core.on_deadline(due);
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::REPLY_TO_POLL_FOR_MASTER, 7));
    core.on_transmit_aborted(id, TransmitFailure::Io, due);
    expect_receive_only(&mut core);
    assert_eq!(core.counters().expired_requests, 1);
    assert_eq!(core.counters().cancelled_sends, 0);
    assert_eq!(core.state(), MasterState::Idle);
}

/// The transmit watchdog aborts a transmit the adapter never completes. One
/// it never started added no driver-on time and is counted as given up; a
/// started one adds the time its driver was on.
#[test]
fn only_a_started_transmit_adds_driver_on_time() {
    for started in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let handed = core.next_deadline().unwrap();
        core.on_deadline(handed);
        let (id, _) = handed_over(&mut core);
        if started {
            core.on_transmit_started(id, handed).unwrap();
        }
        let watchdog = handed + core.config().transmit_watchdog;
        assert_eq!(core.next_deadline(), Some(watchdog));
        core.on_deadline(watchdog);
        expect_receive_only(&mut core);
        let on = if started {
            core.config().transmit_watchdog
        } else {
            Duration::ZERO
        };
        assert_eq!(core.counters().de_assertion, on, "started {started}");
        assert_eq!(core.counters().cancelled_sends, u32::from(!started));
        // The core's own watchdog aborts as a timeout.
        assert_eq!(core.counters().transmit_timeouts, 1);
        assert_eq!(core.state(), MasterState::Idle);
    }
}

/// An abort counts under its reason only when it aborts the transmit: one
/// for an id the core doesn't hold is refused as stale and counts nothing,
/// and neither does a start or a completion for such an id.
#[test]
fn only_an_abort_of_the_transmit_counts_its_reason() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    let (id, _) = handed_over(&mut core);
    let stranger = TransmitId::new(id.generation().wrapping_add(7));
    let before = core.counters();
    for failure in [
        TransmitFailure::Aborted,
        TransmitFailure::Timeout,
        TransmitFailure::Io,
        TransmitFailure::Rejected,
    ] {
        core.on_transmit_aborted(stranger, failure, handed);
    }
    assert_eq!(
        core.on_transmit_started(stranger, handed),
        Err(TransmitError::Stale)
    );
    core.on_transmit_complete(stranger, handed);
    assert_eq!(core.counters(), before);
    assert_eq!(core.snapshot().transmit_id, Some(id));

    core.on_transmit_aborted(id, TransmitFailure::Rejected, handed);
    let counters = core.counters();
    assert_eq!(
        (
            counters.transmit_aborts,
            counters.transmit_timeouts,
            counters.transmit_io_errors,
            counters.transmit_rejections,
        ),
        (0, 0, 0, 1)
    );
}

/// The octets come with the confirmed start and only with it (decision D10,
/// #1769), and the turnaround contract holds as before. The action names
/// the frame by header and length; a start within T_turnaround of line
/// activity is refused, with nothing to send, and the frame waits a full
/// T_turnaround of quiet line from that activity (Clause 9.5.5.1); it then
/// comes back under a new id as the same frame, and its confirmed start
/// hands over exactly its encoding. From there on, input is echo.
#[test]
fn only_a_confirmed_start_hands_out_the_frame() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { id, header, length } = core.poll_action() else {
        panic!("expected the poll");
    };
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));

    let noise = handed + us(100);
    core.on_octet(0x00, noise);
    assert_eq!(
        core.on_transmit_started(id, noise + us(1)),
        Err(TransmitError::Deferred)
    );
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));

    let due = noise + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due - us(1));
    assert_eq!(
        core.poll_action(),
        Action::Idle,
        "not before a quiet turnaround"
    );
    core.on_deadline(due);
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit {
        id: again,
        header: same_header,
        length: same_length,
    } = core.poll_action()
    else {
        panic!("expected the poll again");
    };
    assert_ne!(again, id);
    assert_eq!((same_header, same_length), (header, length));
    let frame = core.on_transmit_started(again, due).unwrap().to_vec();
    assert_eq!(
        frame,
        encode(FrameType::POLL_FOR_MASTER, PEER, STATION, &[])
    );
    assert_eq!(frame.len(), length);

    let before = (core.snapshot(), core.counters());
    feed(&mut core, &frame, due + us(10));
    assert_eq!((core.snapshot(), core.counters()), before, "its own echo");
}
