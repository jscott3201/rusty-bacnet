//! The actions' side of the adapter contract (#1618): control actions never
//! run out of room, a transmit is polled before it starts, and only
//! deliveries can be dropped, counted, when the adapter falls behind.

mod support;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, ActionBuffers, Direction, Duration, Instant, MasterCore, MasterState, QueueKind,
    QueueStorage, TransmitError,
};

use support::*;

/// An adapter that stops polling: each time station 1 gets the token it
/// dequeues an NPDU and hands its frame over, nobody polls it, and the
/// transmit watchdog takes it back, counting it. Every one of those calls
/// succeeds, however long this goes on: the switches back to receive that
/// pile up are one action, and the next frame always finds room. (With an
/// eight-slot control ring each cycle left a switch behind, and the eighth
/// dequeued NPDU was lost with an error.) Once the adapter polls again, the
/// next NPDU goes out.
#[test]
fn an_adapter_that_falls_behind_never_costs_a_dequeued_npdu() {
    const CYCLES: u32 = 12;
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    let mut now = ms(10);
    for _ in 0..CYCLES {
        enqueue(&mut core, false, now);
        validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], now);
        let handed = core.next_deadline().unwrap();
        core.on_deadline(handed);
        assert!(core.snapshot().transmit_id.is_some());
        let watchdog = core.next_deadline().unwrap();
        assert_eq!(watchdog, handed + core.config().transmit_watchdog);
        core.on_deadline(watchdog);
        assert_eq!(core.state(), MasterState::Idle);
        now = watchdog + Duration::from_millis(1);
    }
    let counters = core.counters();
    assert_eq!(counters.unicast.dequeued, CYCLES);
    assert_eq!(counters.cancelled_sends, CYCLES);
    assert_eq!(counters.delivery_drops, 0);
    expect_receive_only(&mut core);

    enqueue(&mut core, false, now);
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], now);
    let (_, header) = send_next(&mut core);
    assert!(is(header, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, PEER));
    let counters = core.counters();
    assert_eq!(counters.unicast.dequeued, CYCLES + 1);
    assert_eq!(counters.frames_transmitted, 1);
}

/// A start for a transmit whose frame has not been polled is refused and
/// counted, and leaves the transmit as it was: the adapter cannot have its
/// octets, and the control actions stay within one transmit only because
/// each is polled before it starts.
#[test]
fn a_start_before_the_frame_is_polled_is_refused() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = new_core(&mut rx, &mut actions);
    poll_waiting(&mut core);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    let id = core.snapshot().transmit_id.unwrap();
    assert_eq!(
        core.on_transmit_started(id, due),
        Err(TransmitError::NotPolled)
    );
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    assert_eq!(
        core.on_transmit_started(id, due),
        Err(TransmitError::NotPolled)
    );
    assert_eq!(core.counters().adapter_errors, 2);
    assert!(matches!(core.poll_action(), Action::Transmit { id: polled, .. } if polled == id));
    core.on_transmit_started(id, due).unwrap();
    core.on_transmit_complete(id, due + Duration::from_millis(1));
    assert_eq!(core.counters().frames_transmitted, 1);
}

/// With one delivery slot, a second broadcast that arrives before the first
/// is polled is dropped and counted. Control actions never wait behind a
/// delivery: the poll the Token leads to comes out first.
#[test]
fn full_delivery_slots_drop_data_but_never_controls() {
    let mut rx = [0; 64];
    let mut actions = ActionBuffers::<1>::new();
    let mut core = new_core(&mut rx, &mut actions);
    let broadcast = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
    validated(&mut core, broadcast, BROADCAST_MAC, 3, &[1, 0, 1], ms(10));
    validated(&mut core, broadcast, BROADCAST_MAC, 3, &[1, 0, 2], ms(15));
    assert_eq!(core.counters().delivery_drops, 1);
    validated(&mut core, FrameType::TOKEN, STATION, PEER, &[], ms(20));
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    let (_, header) = handed_over(&mut core);
    assert!(is(header, FrameType::POLL_FOR_MASTER, PEER));
    assert!(matches!(
        core.poll_action(),
        Action::DeliverNpdu {
            npdu: &[1, 0, 1],
            ..
        }
    ));
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.counters().npdus_delivered, 1);
}

/// A data request that finds every delivery slot full never reaches the
/// application, so the core answers it with Reply Postponed itself.
#[test]
fn a_request_that_finds_no_slot_is_postponed() {
    let mut rx = [0; 64];
    let mut actions = ActionBuffers::<1>::new();
    let mut core = new_core(&mut rx, &mut actions);
    let broadcast = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
    validated(&mut core, broadcast, BROADCAST_MAC, 3, &[1, 0, 1], ms(10));
    let request = FrameType::BACNET_DATA_EXPECTING_REPLY;
    validated(&mut core, request, STATION, PEER, &[1, 4, 7], ms(15));
    assert_eq!(core.counters().delivery_drops, 1);
    assert_eq!(core.snapshot().request_id, None);
    let (_, header) = send_next(&mut core);
    assert!(is(header, FrameType::REPLY_POSTPONED, PEER));
}

/// The core can borrow its queues, as firmware keeping them in a static
/// does; what it queued stays in them.
#[test]
fn the_core_can_borrow_its_queues() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut queues = Queues::new();
    {
        let mut core =
            MasterCore::from_parts(config(), &mut queues, &mut rx, &mut actions, Instant::ZERO)
                .unwrap();
        for (at, npdu) in [(ms(1), [1, 0]), (ms(2), [1, 4])] {
            core.enqueue_npdu(PEER, &npdu, NetworkPriority::Normal, at)
                .unwrap();
        }
        assert_eq!(core.counters().unicast.depth, 2);
    }
    assert_eq!(queues.depth(QueueKind::Unicast), 2);
}
