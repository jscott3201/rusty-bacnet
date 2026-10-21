//! A Reply To Poll For Master that arrives after the poller has moved on
//! (Clause 9.5.6.8), while the poller's next frame has not started yet.

mod support;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Action, Direction, Duration, MasterState, TransmitError};

use support::*;

/// Station 1 polls station 2, which says nothing for T_usage_timeout, so
/// the poll for station 3 follows (SendNextPFM). Station 2's answer then
/// arrives after all, while that poll still waits: before it was handed
/// over, or handed over with its start unconfirmed. As when the answer is
/// pending on entry to POLL_FOR_MASTER (ReceivedReplyToPFM), station 2
/// becomes the successor and gets the Token; the poll for station 3 is
/// taken back and counted.
#[test]
fn a_late_answer_to_a_poll_still_finds_the_successor() {
    for handed in [false, true] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        poll_waiting(&mut core);
        let (_, header) = send_next(&mut core);
        assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
        let timeout = core.next_deadline().unwrap();
        let late = if handed {
            core.on_deadline(timeout);
            timeout + us(500)
        } else {
            // A damaged answer first: the next poll waits out its tail.
            let mut bad = encode(FrameType::REPLY_TO_POLL_FOR_MASTER, STATION, 5, &[]);
            bad[7] ^= 0x01;
            let last = feed(&mut core, &bad, timeout - Duration::from_millis(10));
            assert_eq!(core.next_deadline(), Some(last + turnaround(&core)));
            last + us(500)
        };
        let poll = if handed {
            let (poll, header) = handed_over(&mut core);
            assert!(is(header, FrameType::POLL_FOR_MASTER, 3));
            Some(poll)
        } else {
            None
        };
        validated(
            &mut core,
            FrameType::REPLY_TO_POLL_FOR_MASTER,
            STATION,
            PEER,
            &[],
            late,
        );
        assert_eq!(core.state(), MasterState::PassToken, "handed {handed}");
        assert_eq!(core.snapshot().next_station, PEER);
        assert_eq!(core.counters().cancelled_sends, 1);
        if let Some(poll) = poll {
            assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
            assert_eq!(
                core.on_transmit_started(poll, late),
                Err(TransmitError::Stale)
            );
        }
        let due = late + turnaround(&core);
        assert_eq!(core.next_deadline(), Some(due));
        core.on_deadline(due);
        let (_, header) = handed_over(&mut core);
        assert!(is(header, FrameType::TOKEN, PEER));
    }
}

/// Station 2 never answers the maintenance poll, so station 1 passes the
/// token on to its successor (DoneWithPFM). A late answer from station 2
/// while that Token waits for its adapter is too late: the successor is
/// settled. It is dropped and counted, and the Token still goes to 3.
#[test]
fn a_late_answer_is_dropped_while_the_token_goes_to_the_successor() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    maintenance_poll_sent(&mut core);
    let timeout = core.next_deadline().unwrap();
    core.on_deadline(timeout);
    let (token, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TOKEN, 3));
    let late = timeout + us(500);
    validated(
        &mut core,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        STATION,
        PEER,
        &[],
        late,
    );
    assert_eq!(core.counters().discarded_while_busy, 1);
    assert_eq!(core.counters().cancelled_sends, 0);
    assert_eq!(core.snapshot().next_station, 3);
    assert_eq!(
        core.on_transmit_started(token, late + us(100)),
        Err(TransmitError::Deferred)
    );
    expect_receive_only(&mut core);
    let due = late + turnaround(&core);
    core.on_deadline(due);
    let (again, header) = handed_over(&mut core);
    assert!(is(header, FrameType::TOKEN, 3));
    core.on_transmit_started(again, due).unwrap();
}
