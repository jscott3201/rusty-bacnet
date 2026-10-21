//! A frame half received while this station's own frame waits to go out.
//!
//! The receiver is off while the driver is on (Clause 9.5.5.1), so once the
//! adapter confirms the start, a frame in progress can never complete. The
//! core drops it then, counted as a frame abort. Its T_frame_abort must not
//! fire later, during or after the station's own frame, as an invalid frame.

mod support;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant, MasterState};

use support::*;

/// A header's first octets.
const PARTIAL: [u8; 4] = [0x55, 0xFF, 0x01, 0x02];

/// Default timing: T_frame_abort (60 bit times) ends after T_turnaround (40),
/// so the partial frame's abort would land in the middle of the station's
/// own poll, which it would then take for an invalid answer to that poll.
#[test]
fn a_partial_frame_dies_when_a_poll_starts() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let last = feed(&mut core, &PARTIAL, ms(11));
    let due = last + turnaround(&core);
    assert_eq!(core.next_deadline(), Some(due));
    core.on_deadline(due);
    let (id, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    core.on_transmit_started(id, due).unwrap();

    // Where the partial frame's abort would have fallen, mid-poll.
    core.on_deadline(last + core.config().t_frame_abort + us(1));
    assert_eq!(core.snapshot().poll_station, PEER);
    assert_eq!(core.counters().frame_aborts, 1);
    assert_eq!(core.counters().parser_errors, 0);
    let watchdog = due + core.config().transmit_watchdog;
    assert_eq!(core.next_deadline(), Some(watchdog));
    let end = due + Duration::from_millis(9);
    core.on_transmit_complete(id, end);
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.next_deadline(), Some(end + Duration::from_millis(20)));
    assert_eq!(core.counters().parser_errors, 0);
}

/// A long T_frame_abort (20 ms, which validation allows): the partial
/// frame's abort would fall after a data request has gone out, and end the
/// wait for its reply as an invalid frame (Clause 9.5.6.4). The wait runs
/// its full T_reply_timeout instead.
#[test]
fn a_partial_frame_does_not_end_the_wait_for_a_reply() {
    let mut config = config();
    config.t_frame_abort = Duration::from_millis(20);
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core =
        Core::from_parts(config, Queues::new(), &mut rx, &mut actions, Instant::ZERO).unwrap();
    enqueue(&mut core, true, ms(1));
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    let last = feed(&mut core, &PARTIAL, ms(11));
    let (start, header) = send_next(&mut core);
    assert!(is(header, FrameType::BACNET_DATA_EXPECTING_REPLY, PEER));
    assert_eq!(start, last + turnaround(&core));
    assert_eq!(core.state(), MasterState::WaitForReply);
    let complete = start + Duration::from_millis(1);
    let timeout = complete + core.config().t_reply_timeout;

    core.on_deadline(last + Duration::from_millis(20) + us(1));
    assert_eq!(core.state(), MasterState::WaitForReply);
    assert_eq!(core.next_deadline(), Some(timeout));
    assert_eq!(core.counters().frame_aborts, 1);
    assert_eq!(core.counters().parser_errors, 0);
    core.on_deadline(timeout);
    assert_ne!(core.state(), MasterState::WaitForReply);
}

/// A start that comes after a partial frame's T_frame_abort fell due, that
/// deadline never serviced, counts the frame as servicing it would have:
/// one abort, and one invalid frame since a header was under way.
#[test]
fn an_overdue_abort_counts_alike_at_the_start() {
    for serviced in [true, false] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let last = feed(&mut core, &PARTIAL, ms(11));
        let due = last + turnaround(&core);
        core.on_deadline(due);
        let (id, _) = handed_over(&mut core);
        let abort = last + core.config().t_frame_abort + us(1);
        if serviced {
            assert_eq!(core.next_deadline(), Some(abort));
            core.on_deadline(abort);
        }
        core.on_transmit_started(id, abort + us(10)).unwrap();
        let counters = core.counters();
        assert_eq!(
            (counters.frame_aborts, counters.parser_errors),
            (1, 1),
            "serviced {serviced}"
        );
        assert_eq!(core.snapshot().poll_station, PEER);
    }
}
