//! Test_Response and proprietary frames go up whole (#1617), as
//! `Action::DeliverFrame`, where Clause 9.5.6 passes them to the layers
//! above.

mod support;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC};
use bacnet_mstp_core::{Action, Duration, MasterState, ReplyDecision};

use support::*;

const PROPRIETARY: FrameType = FrameType::from_raw(0x80);

fn whole(frame_type: FrameType, source: u8, destination: u8, data: &[u8]) -> Action<'_> {
    Action::DeliverFrame {
        frame_type,
        source,
        destination,
        data,
    }
}

/// Clause 9.5.6.2 (ReceivedDataNoReply): in IDLE a Test_Response, or a
/// proprietary frame with its vendor identifier (Clause 9.3.11), for this
/// station or broadcast goes up. One for another station, or a proprietary
/// frame too short for its vendor identifier, goes nowhere.
#[test]
fn test_responses_and_proprietary_frames_go_up_from_idle() {
    let cases: [(FrameType, u8, &[u8], bool); 8] = [
        (FrameType::TEST_RESPONSE, STATION, &[7, 7], true),
        (FrameType::TEST_RESPONSE, BROADCAST_MAC, &[], true),
        (FrameType::TEST_RESPONSE, 3, &[7], false),
        (PROPRIETARY, STATION, &[0x01, 0x04, 9], true),
        (
            FrameType::from_raw(0xFF),
            BROADCAST_MAC,
            &[0x01, 0x04],
            true,
        ),
        (PROPRIETARY, 3, &[0x01, 0x04], false),
        (PROPRIETARY, STATION, &[0x01], false),
        (PROPRIETARY, STATION, &[], false),
    ];
    for host in [false, true] {
        for (frame_type, destination, data, delivered) in cases {
            let mut rx = [0; 64];
            let mut actions = Actions::new();
            let mut core = new_core(&mut rx, &mut actions);
            if host {
                validated(&mut core, frame_type, destination, 3, data, ms(10));
            } else if data.len() >= 2 || !frame_type.is_proprietary() {
                // The encoder refuses a proprietary frame without its vendor
                // identifier; only a host assembler can hand one over.
                feed(&mut core, &encode(frame_type, destination, 3, data), ms(10));
            } else {
                continue;
            }
            let case = format!("{frame_type:?} for {destination}, host {host}");
            if delivered {
                assert_eq!(
                    core.poll_action(),
                    whole(frame_type, 3, destination, data),
                    "{case}"
                );
            }
            assert_eq!(core.poll_action(), Action::Idle, "{case}");
            let counters = core.counters();
            assert_eq!(counters.frames_delivered, u32::from(delivered), "{case}");
            assert_eq!(counters.npdus_delivered, 0);
            assert_eq!(core.state(), MasterState::Idle, "{case}");
        }
    }
}

/// Clause 9.5.6.4 (ReceivedReply): a Test_Response for this station ends a
/// wait for a reply, whichever station sent it, and goes up.
#[test]
fn a_test_response_that_ends_a_wait_goes_up() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let waiting = waiting_for_reply(&mut core);
    let replied = waiting + Duration::from_millis(5);
    validated(
        &mut core,
        FrameType::TEST_RESPONSE,
        STATION,
        3,
        &[4, 2],
        replied,
    );
    assert_eq!(
        core.poll_action(),
        whole(FrameType::TEST_RESPONSE, 3, STATION, &[4, 2])
    );
    // DONE_WITH_TOKEN, which polls for a successor here.
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.counters().frames_delivered, 1);
}

/// Clause 9.5.6.4 (ReceivedUnexpectedFrame): the core knows no proprietary
/// reply, so a proprietary frame while waiting for one is unexpected: it
/// sends the station to IDLE, dropping the token, and does not go up. A
/// broadcast Test_Response is unexpected there too.
#[test]
fn a_frame_that_is_not_the_reply_does_not_go_up_while_waiting() {
    for (frame_type, destination, data) in [
        (PROPRIETARY, STATION, &[0x01, 0x04][..]),
        (FrameType::TEST_RESPONSE, BROADCAST_MAC, &[][..]),
    ] {
        let mut rx = [0; 64];
        let mut actions = Actions::new();
        let mut core = new_core(&mut rx, &mut actions);
        let waiting = waiting_for_reply(&mut core);
        let heard = waiting + Duration::from_millis(5);
        validated(&mut core, frame_type, destination, 3, data, heard);
        assert_eq!(core.state(), MasterState::Idle, "{frame_type:?}");
        assert_eq!(core.poll_action(), Action::Idle, "{frame_type:?}");
        assert_eq!(core.counters().frames_delivered, 0);
    }
}

/// Clause 9.5.6.8 (ReceivedUnexpectedFrame): any frame but a Reply To Poll
/// For Master for this station sends a poller to IDLE, unread.
#[test]
fn nothing_goes_up_while_a_poll_waits_for_its_answer() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let (sent, _) = send_next(&mut core);
    let heard = sent + Duration::from_millis(5);
    validated(&mut core, FrameType::TEST_RESPONSE, STATION, 3, &[], heard);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
}

/// While the station answers a request, or waits to send a frame of its
/// own, a frame that needs no reply goes up at once (Clause 9.5.6.9 leaves
/// it for IDLE, which would pass it up).
#[test]
fn frames_that_need_no_reply_go_up_while_the_station_is_busy() {
    // Answering a request.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    validated(
        &mut core,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        STATION,
        PEER,
        &[1, 4, 7],
        ms(10),
    );
    assert_eq!(core.state(), MasterState::AnswerDataRequest);
    validated(&mut core, PROPRIETARY, BROADCAST_MAC, 3, &[0, 9], ms(11));
    let Action::DeliverNpdu {
        request_id: Some(request),
        ..
    } = core.poll_action()
    else {
        panic!("expected the request");
    };
    assert_eq!(
        core.poll_action(),
        whole(PROPRIETARY, 3, BROADCAST_MAC, &[0, 9])
    );
    core.complete_request(request, ReplyDecision::Data(&[1, 0, 8]), ms(12))
        .unwrap();
    assert_eq!(core.counters().frames_delivered, 1);

    // A data frame of this station's waiting out its turnaround.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    enqueue(&mut core, false, ms(0));
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(10));
    assert_eq!(core.state(), MasterState::UseToken);
    let heard = ms(10) + us(1_000);
    validated(&mut core, FrameType::TEST_RESPONSE, STATION, 3, &[5], heard);
    assert_eq!(
        core.poll_action(),
        whole(FrameType::TEST_RESPONSE, 3, STATION, &[5])
    );
    assert_eq!(core.next_deadline(), Some(heard + turnaround(&core)));
    let (_, header) = send_next(&mut core);
    assert!(is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER));
    assert_eq!(core.counters().cancelled_sends, 0);
}
