//! Integration tests for the portable MS/TP master core.

#![cfg(feature = "heapless")]

use bacnet_mstp_codec::{Frame, FrameType, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::{
    Action, CoreError, Direction, Duration, HeaplessQueues, Instant, MasterCore, MasterState,
    NetworkPriority, ReceiveError, ReplyDecision, RequestId, TransmitId,
};
use proptest::prop_assert;

type Queues = HeaplessQueues<8, 8>;

fn at(millis: u64) -> Instant {
    Instant::from_micros(millis * 1_000)
}

fn frame(kind: FrameType, destination: u8, source: u8, data: &[u8]) -> Frame<'_> {
    Frame::new(kind, destination, source, data)
}

fn standard_config(station: u8, max_master: u8) -> bacnet_mstp_core::MstpCoreConfig {
    let mut config = bacnet_mstp_core::MstpCoreConfig::default();
    config.station = station;
    config.max_master = max_master;
    config
}

fn take_transmit<'a, Q: bacnet_mstp_core::QueueStorage>(
    core: &'a mut MasterCore<'_, Q>,
    now: Instant,
) -> TransmitId {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let id = match core.poll_action() {
        Action::Transmit { id, frame } => {
            assert!(frame.len() >= 8);
            id
        }
        other => panic!("expected transmit, got {other:?}"),
    };
    core.on_transmit_started(id, now).unwrap();
    id
}

fn complete_transmit<Q: bacnet_mstp_core::QueueStorage>(
    core: &mut MasterCore<'_, Q>,
    id: TransmitId,
    now: Instant,
) {
    core.on_transmit_complete(id, now).unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
}

#[test]
fn public_events_reach_all_clause_states_and_counters_are_observable() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    assert_eq!(core.state(), MasterState::Idle);

    // Lost-token recovery has a distinct NoToken station slot before its
    // PollForMaster transmission is due.
    core.on_deadline(at(500)).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    core.on_deadline(at(510)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    let poll = take_transmit(&mut core, at(510));
    complete_transmit(&mut core, poll, at(511));

    // No ReplyToPollForMaster arrives, so the usage timeout advances polling.
    core.on_deadline(at(531)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    let poll_next = take_transmit(&mut core, at(531));
    complete_transmit(&mut core, poll_next, at(532));

    // A received token enters UseToken while queued work is available.
    core.enqueue_npdu(2, &[0, 0x00], NetworkPriority::Normal, at(532))
        .unwrap();
    core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), at(533))
        .unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), at(534))
        .unwrap();
    let state_after_token = core.state();
    assert!(
        matches!(
            state_after_token,
            MasterState::UseToken | MasterState::DoneWithToken | MasterState::PollForMaster
        ),
        "state after token: {state_after_token:?}"
    );
    assert_ne!(core.state(), MasterState::Idle);
    let counters = core.counters();
    assert!(counters.state_transitions > 0);
    assert!(counters.frames_received >= 2);
}

#[test]
fn frame_abort_is_strictly_greater_than_timing_and_next_deadline_is_plus_one() {
    let mut rx = [0; 1024];
    let core_config = standard_config(0, 1);
    let abort = core_config.t_frame_abort;
    let mut core = MasterCore::new(core_config, Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_octet(0x55, Instant::ZERO).unwrap();
    assert_eq!(
        core.next_deadline(),
        Some(Instant::ZERO + abort + Duration::from_micros(1))
    );
    core.on_silence(Instant::ZERO + abort).unwrap();
    assert_eq!(core.counters().frame_aborts, 0);
    core.on_silence(Instant::ZERO + abort + Duration::from_micros(1))
        .unwrap();
    assert_eq!(core.counters().frame_aborts, 1);
}

#[test]
fn no_token_uses_station_slot_and_resets_after_new_silence_epoch() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(2, 4), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_deadline(at(500)).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    assert_eq!(core.next_deadline(), Some(at(520)));
    core.on_silence(at(519)).unwrap();
    assert_eq!(core.state(), MasterState::NoToken);
    core.on_deadline(at(520)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert!(core.counters().token_regenerations >= 1);

    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(2, 4), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_deadline(at(500)).unwrap();
    core.on_octet(0x00, at(501)).unwrap();
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.next_deadline(), Some(at(1001)));
}

#[test]
fn poll_for_master_uses_usage_timeout_not_slot() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(0, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_deadline(at(500)).unwrap();
    core.on_deadline(at(500)).unwrap();
    // Station zero's slot is the beginning of the epoch.
    let poll = take_transmit(&mut core, at(500));
    complete_transmit(&mut core, poll, at(501));
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.next_deadline(), Some(at(521)));
    core.on_deadline(at(511)).unwrap();
    assert_eq!(core.poll_action(), Action::Idle);
    core.on_deadline(at(521)).unwrap();
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert!(core.counters().deadline_misses == 0);
}

#[test]
fn request_ids_reject_stale_duplicate_oversize_and_allow_cancel() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_validated_frame(
        frame(FrameType::BACnetDataExpectingReply, 1, 7, &[0, 0x01]),
        at(1),
    )
    .unwrap();
    let id = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    let stale = RequestId::new(id.raw().wrapping_add(1));
    assert_eq!(
        core.complete_request(stale, ReplyDecision::Cancelled, at(2)),
        Err(CoreError::StaleRequest)
    );
    assert_eq!(
        core.complete_request(
            id,
            ReplyDecision::Data(&[0; MAX_STANDARD_MPDU_DATA + 1]),
            at(2)
        ),
        Err(CoreError::ReplyOversize {
            length: MAX_STANDARD_MPDU_DATA + 1,
            maximum: MAX_STANDARD_MPDU_DATA,
        })
    );
    core.complete_request(id, ReplyDecision::Cancelled, at(2))
        .unwrap();
    assert_eq!(core.snapshot().request_id, None);
    assert_eq!(
        core.complete_request(id, ReplyDecision::Cancelled, at(3)),
        Err(CoreError::NoPendingRequest)
    );
    assert!(core.counters().npdus_delivered >= 1);
}

#[test]
fn abandoned_request_returns_idle_without_wire_actions() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_validated_frame(
        frame(FrameType::BACnetDataExpectingReply, 1, 7, &[0, 0x01]),
        at(1),
    )
    .unwrap();
    let id = match core.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };

    core.complete_request(id, ReplyDecision::Abandoned, at(2))
        .unwrap();

    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.snapshot().request_id, None);
    assert_eq!(core.poll_action(), Action::Idle);
}

#[test]
fn transmit_ids_are_generation_bound_and_watchdog_recovers_receive_direction() {
    let mut rx = [0; 1024];
    let queues = Queues::new();
    let mut core = MasterCore::new(standard_config(1, 2), queues, &mut rx, Instant::ZERO).unwrap();
    core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), Instant::ZERO)
        .unwrap();
    core.on_deadline(at(5)).unwrap();
    let id = take_transmit(&mut core, at(5));
    assert_eq!(
        core.on_transmit_started(id, at(6)),
        Err(CoreError::TransmitAlreadyStarted)
    );
    assert_eq!(
        core.on_transmit_complete(TransmitId::new(id.raw().wrapping_add(1)), at(7)),
        Err(CoreError::StaleTransmit)
    );
    let watchdog = at(6) + core.config().transmit_watchdog;
    core.on_deadline(watchdog).unwrap();
    assert_eq!(core.snapshot().transmit_id, None);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert!(core.counters().frames_transmitted == 0);
}

#[test]
fn broadcasts_are_delivered_without_reply_and_invalid_wait_reply_finishes_token() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.on_validated_frame(
        frame(FrameType::BACnetDataExpectingReply, 0xff, 2, &[1, 2]),
        at(1),
    )
    .unwrap();
    assert_eq!(
        core.poll_action(),
        Action::DeliverNpdu {
            source: 2,
            destination: 0xff,
            npdu: &[1, 2],
            request_id: None,
        }
    );

    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.enqueue_npdu(2, &[0, 0x04], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), Instant::ZERO)
        .unwrap();
    core.on_deadline(at(5)).unwrap();
    let tx = take_transmit(&mut core, at(5));
    complete_transmit(&mut core, tx, at(6));
    assert_eq!(core.state(), MasterState::WaitForReply);
    core.on_receive_error(ReceiveError::Framing, at(7)).unwrap();
    assert_eq!(core.state(), MasterState::WaitForReply);
    core.on_octet(0x55, at(8)).unwrap();
    core.on_octet(0xff, at(9)).unwrap();
    core.on_receive_error(ReceiveError::Framing, at(10))
        .unwrap();
    let post_error_state = core.state();
    assert!(
        matches!(
            post_error_state,
            MasterState::DoneWithToken | MasterState::PollForMaster | MasterState::PassToken
        ),
        "post receive-error state: {post_error_state:?}"
    );
    assert!(core.counters().parser_errors >= 1);
}

#[test]
fn queue_overflow_does_not_prevent_control_progress_and_validated_frames_match_octets() {
    let mut rx = [0; 1024];
    let mut core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    for source in 2..=9 {
        core.on_validated_frame(
            frame(FrameType::BACnetDataNotExpectingReply, 1, source, &[source]),
            at(u64::from(source)),
        )
        .unwrap();
    }
    assert_eq!(core.counters().npdus_delivered, 8);
    core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), at(10))
        .unwrap();
    core.on_deadline(at(15)).unwrap();
    assert!(matches!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    ));

    let mut encoded = [0; 512];
    let len = frame(FrameType::BACnetDataNotExpectingReply, 1, 2, &[9, 8])
        .encode_into(&mut encoded)
        .unwrap();
    let mut rx = [0; 1024];
    let mut stream_core =
        MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    for (index, byte) in encoded[..len].iter().copied().enumerate() {
        stream_core
            .on_octet(byte, at(u64::try_from(index).unwrap() + 1))
            .unwrap();
    }
    assert_eq!(stream_core.counters().frames_received, 1);
    assert_eq!(stream_core.counters().npdus_delivered, 1);
    assert_eq!(stream_core.state(), MasterState::Idle);
}

#[test]
fn monotone_random_events_never_panic_and_deadlines_are_causal() {
    proptest::proptest!(|(steps in proptest::collection::vec(0u8..6, 1..80))| {
        let mut rx = [0; 1024];
        let mut core = MasterCore::new(standard_config(1, 2), Queues::new(), &mut rx, Instant::ZERO)
            .unwrap();
        let mut now = Instant::ZERO;
        for step in steps {
            now = now + Duration::from_micros(u64::from(step));
            match step {
                0 => { core.on_silence(now).unwrap(); }
                1 => { core.on_receive_error(ReceiveError::Overrun, now).unwrap(); }
                2 => { core.on_octet(0x55, now).unwrap(); }
                3 => { core.on_octet(0xff, now).unwrap(); }
                _ => { core.on_deadline(now).unwrap(); }
            }
            if let Some(deadline) = core.next_deadline() {
                prop_assert!(deadline >= now || core.counters().deadline_misses > 0);
            }
            let _ = core.counters();
        }
    });
}

#[cfg(feature = "alloc")]
mod allocation_test {
    extern crate std;

    use super::*;
    use bacnet_mstp_core::AllocQueues;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    std::thread_local! {
        static THREAD_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    struct CountingAllocator;

    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            THREAD_ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
            // SAFETY: forwarded unchanged to the platform allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: forwarded unchanged to the platform allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAllocator = CountingAllocator;

    #[test]
    fn alloc_queues_and_core_do_not_allocate_after_construction() {
        let mut rx = [0; 1024];
        let queues = AllocQueues::new(4, 4);
        let mut core =
            MasterCore::new(standard_config(1, 2), queues, &mut rx, Instant::ZERO).unwrap();
        // Initialize the thread-local counter before beginning the measured
        // interval; construction above is the final permitted allocation.
        THREAD_ALLOCATIONS.with(|count| count.set(0));
        core.enqueue_npdu(2, &[1, 2], NetworkPriority::Normal, at(0))
            .unwrap();
        core.on_validated_frame(frame(FrameType::Token, 1, 2, &[]), at(2))
            .unwrap();
        core.on_deadline(at(7)).unwrap();
        assert_eq!(
            core.poll_action(),
            Action::SetDirection(Direction::Transmit)
        );
        let id = match core.poll_action() {
            Action::Transmit { id, .. } => id,
            other => panic!("expected transmit, got {other:?}"),
        };
        core.on_transmit_started(id, at(7)).unwrap();
        core.on_transmit_complete(id, at(8)).unwrap();
        assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
        let measured = THREAD_ALLOCATIONS.with(Cell::get);
        assert_eq!(measured, 0);
        assert!(core.counters().unicast.enqueued >= 1);
    }
}
