//! The host input path (#1617). An adapter that assembles frames itself
//! reports raw octets, valid frames and invalid frames, and they must drive
//! the transitions the core's own decoder drives. Each test runs both paths
//! where they can differ.

mod support;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC};
use bacnet_mstp_core::{Action, Direction, Duration, Instant, MasterState, TransmitError};

use support::*;

/// Clause 9.5.6.4 (InvalidFrame): an invalid frame while waiting for a reply
/// ends the token use. Station 1 knows no successor yet, so it polls for one
/// once the line has been quiet for T_turnaround after the bad frame.
#[test]
fn an_invalid_reply_ends_the_wait_on_either_path() {
    for host in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let waiting = waiting_for_reply(&mut core);
        let found = invalid_frame(&mut core, host, waiting + Duration::from_millis(10));
        assert_eq!(core.counters().parser_errors, 1, "host {host}");
        assert_eq!(core.state(), MasterState::PollForMaster, "host {host}");
        assert_eq!(core.next_deadline(), Some(found + turnaround(&core)));
        let (_, header) = send_next(&mut core);
        assert!(is(header, FrameType::POLL_FOR_MASTER, PEER), "host {host}");
    }
}

/// Clause 9.5.6.8: an invalid frame answers a poll as silence does. With no
/// successor known, the next address is polled (SendNextPFM) until the
/// polls come round to this station, which then declares itself the sole
/// master (DeclareSoleMaster). A sole master's next poll answered by an
/// invalid frame sends it back to USE_TOKEN (SoleMaster).
#[test]
fn an_invalid_frame_answers_a_poll_as_silence_does() {
    for host in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let (mut sent, header) = send_next(&mut core);
        assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
        // Each poll is answered by an invalid frame 2 ms after it ended.
        let answer = |sent: Instant| sent + Duration::from_millis(3);
        // Stations 3 and 0 are polled in turn, then station 1 is alone.
        for next in [3, 0] {
            let found = invalid_frame(&mut core, host, answer(sent));
            assert_eq!(core.next_deadline(), Some(found + turnaround(&core)));
            let header;
            (sent, header) = send_next(&mut core);
            assert!(is(header, FrameType::POLL_FOR_MASTER, next), "host {host}");
        }
        invalid_frame(&mut core, host, answer(sent));
        assert!(core.snapshot().sole_master, "host {host}");
        // Its next maintenance poll goes out; an NPDU waits meanwhile.
        let (sent, header) = send_next(&mut core);
        assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
        enqueue(&mut core, false, answer(sent));
        invalid_frame(&mut core, host, answer(sent));
        assert_eq!(core.state(), MasterState::UseToken, "host {host}");
        let (_, header) = send_next(&mut core);
        assert!(
            is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER),
            "host {host}"
        );
        assert_eq!(core.counters().parser_errors, 4);
    }
}

/// Clause 9.5.6.8 (DoneWithPFM): with a successor known, an invalid frame
/// answering the maintenance poll passes the token to the successor.
#[test]
fn an_invalid_answer_to_a_maintenance_poll_passes_the_token() {
    for host in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        maintenance_poll_sent(&mut core);
        // The poll ended T_usage_timeout before the deadline.
        let ended = core.next_deadline().unwrap() - Duration::from_millis(20);
        let found = invalid_frame(&mut core, host, ended + Duration::from_millis(2));
        assert_eq!(core.state(), MasterState::PassToken, "host {host}");
        assert_eq!(core.next_deadline(), Some(found + turnaround(&core)));
        let (_, header) = send_next(&mut core);
        assert!(is(header, FrameType::TOKEN, 3), "host {host}");
    }
}

/// While a frame of this station's waits, an invalid frame is only line
/// activity for it (see the core's `pending` module): the frame waits
/// T_turnaround again, and its state takes no transition. Handed over and
/// not yet started, its start is refused.
#[test]
fn an_invalid_frame_only_delays_a_waiting_frame() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let found = ms(10) + us(2_000);
    core.on_invalid_frame(found);
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.next_deadline(), Some(found + turnaround(&core)));
    let handed = core.next_deadline().unwrap();
    core.on_deadline(handed);
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    let again = handed + us(300);
    core.on_invalid_frame(again);
    assert_eq!(
        core.on_transmit_started(id, again + us(300)),
        Err(TransmitError::Deferred)
    );
    assert_eq!(core.next_deadline(), Some(again + turnaround(&core)));
    let counters = core.counters();
    assert_eq!(counters.parser_errors, 2);
    assert_eq!(counters.cancelled_sends, 0);
}

/// Host octets count toward EventCount one event each, and an invalid frame
/// counts one more: together more than Nmin_octets is the SawTokenUser
/// evidence that drops a waiting token retry (Clause 9.5.6.6). Up to
/// Nmin_octets only delays it.
#[test]
fn host_events_count_toward_saw_token_user() {
    for (octets, dropped) in [(3, false), (4, true)] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let (handed, id) = retry_handed_over(&mut core);
        let heard = handed + us(500);
        core.on_host_activity(octets, heard);
        core.on_invalid_frame(heard);
        let counters = core.counters();
        assert_eq!(counters.cancelled_sends, u32::from(dropped), "{octets}");
        if dropped {
            assert_eq!(core.state(), MasterState::Idle);
            expect_receive_only(&mut core);
        } else {
            assert_eq!(core.state(), MasterState::PassToken);
            assert_eq!(
                core.on_transmit_started(id, heard + us(100)),
                Err(TransmitError::Deferred)
            );
            assert_eq!(core.next_deadline(), Some(heard + turnaround(&core)));
        }
    }
}

/// From the confirmed start to completion, every host input is the
/// station's own echo: a whole frame (here its own Token heard back), an
/// invalid frame or raw octets change no count, state or deadline.
#[test]
fn host_input_after_the_confirmed_start_is_echo() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    pass_waiting(&mut core);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TOKEN, PEER));
    core.on_transmit_started(id, due).unwrap();
    let before = core.counters();
    let deadline = core.next_deadline();
    let echo = due + Duration::from_millis(1);
    validated(&mut core, FrameType::TOKEN, PEER, STATION, &[], echo);
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], echo);
    core.on_invalid_frame(echo);
    core.on_host_activity(8, echo);
    assert_eq!(core.counters(), before);
    assert_eq!(core.next_deadline(), deadline);
    assert_eq!(core.state(), MasterState::PassToken);
    assert_eq!(core.poll_action(), Action::Idle);
    let done = echo + us(100);
    core.on_transmit_complete(id, done);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(
        core.counters().frames_transmitted,
        before.frames_transmitted + 1
    );
}

/// A frame whose source is this station's address, outside its own
/// transmissions, comes from another station with the same address. Clause
/// 9 has no rule for it, so the core counts it and handles it like any
/// other frame: here a broadcast is delivered and a Token is taken up.
#[test]
fn a_frame_from_this_stations_address_is_counted_and_handled() {
    for host in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let broadcast = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
        // A broadcast from station 3 first, which is not counted; then a
        // broadcast and a Token from this station's address.
        let frames = [
            (broadcast, BROADCAST_MAC, 3, &[1, 0, 6][..], 0),
            (broadcast, BROADCAST_MAC, STATION, &[1, 0, 7][..], 1),
            (FrameType::TOKEN, STATION, STATION, &[][..], 2),
        ];
        let mut at = ms(10);
        for (kind, destination, source, data, counted) in frames {
            if host {
                validated(&mut core, kind, destination, source, data, at);
            } else {
                at = feed(&mut core, &encode(kind, destination, source, data), at);
            }
            assert_eq!(
                core.counters().own_address_frames,
                counted,
                "host {host}, from {source}"
            );
            at += Duration::from_millis(5);
        }
        assert_eq!(core.counters().frames_received, 3);
        for source in [3, STATION] {
            assert!(matches!(
                core.poll_action(),
                Action::DeliverNpdu { source: from, destination: BROADCAST_MAC, .. }
                    if from == source
            ));
        }
        // The Token: no successor known, so a poll for one waits.
        assert_eq!(core.state(), MasterState::PollForMaster, "host {host}");
    }
}
