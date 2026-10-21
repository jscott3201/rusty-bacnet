//! The [`Storage`] bundle: a core built over it, the receive window it
//! takes from it, and the buffers its sizes must cover.

use bacnet_mstp_codec::{
    Frame, FrameType, MAX_COBS_FRAME_LENGTH, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA,
};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, ConfigError, CoreBuffer, Direction, Duration, ExtendedStorage, Instant, MasterCore,
    MasterState, MstpCoreConfig, QueueKind, QueueStorage, StandardStorage, Storage,
};

const STATION: u8 = 1;
const ASKED: u8 = 3;

fn config(extended: bool) -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 3;
    if extended {
        config.with_extended_frames()
    } else {
        config
    }
}

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let length = frame.encode_into(&mut wire).unwrap();
    wire.truncate(length);
    wire
}

/// Send a data-expecting-reply frame to `ASKED` and wait for its reply.
fn wait_for_reply<Q: QueueStorage, const N: usize>(core: &mut MasterCore<'_, Q, N>) -> Instant {
    core.enqueue_npdu(ASKED, &[1, 0x04], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(Frame::new(FrameType::TOKEN, STATION, 0, &[]), Instant::ZERO);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { id, .. } = core.poll_action() else {
        panic!("expected the request transmit");
    };
    core.on_transmit_started(id, due).unwrap();
    let done = due + Duration::from_millis(1);
    core.on_transmit_complete(id, done);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::WaitForReply);
    done
}

/// The octet at which an extended frame for this station first counts as
/// invalid while the core waits for a reply.
fn counted_at<Q: QueueStorage, const N: usize>(core: &mut MasterCore<'_, Q, N>) -> usize {
    let wire = encode(Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        STATION,
        5,
        &[0x22; MAX_EXTENDED_MPDU_DATA],
    ));
    let mut now = wait_for_reply(core);
    for (index, &octet) in wire.iter().enumerate() {
        now += Duration::from_micros(100);
        core.on_octet(octet, now);
        if core.counters().parser_errors > 0 {
            return index + 1;
        }
    }
    panic!("the frame never counted");
}

/// A core over a standard bundle runs as one over the same parts: it sends
/// what it queued, and the queues it borrowed are readable once it is
/// gone.
#[test]
fn a_standard_bundle_builds_a_working_core() {
    let mut storage: StandardStorage<2, 1, 1> = StandardStorage::new();
    {
        let mut core = MasterCore::new(config(false), &mut storage, Instant::ZERO).unwrap();
        wait_for_reply(&mut core);
        core.enqueue_npdu(ASKED, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
            .unwrap();
        assert_eq!(core.counters().frames_transmitted, 1);
    }
    assert_eq!(storage.queues().depth(QueueKind::Unicast), 1);
}

/// An extended bundle serves both settings. With the flag off the core uses
/// a standard frame's share of the receive storage, so an extended frame
/// counts as invalid at its header, as on a node without COBS support; with
/// the whole storage it would count only at its last octet (see
/// `receive_filter.rs`).
#[test]
fn an_extended_bundle_without_the_flag_receives_as_a_standard_node() {
    let mut storage: Box<ExtendedStorage<2, 1, 1>> = Box::default();
    let mut core = MasterCore::new(config(false), &mut *storage, Instant::ZERO).unwrap();
    assert_eq!(counted_at(&mut core), 8, "counted at the header");

    let mut core = MasterCore::new(config(true), &mut *storage, Instant::ZERO).unwrap();
    // With the flag on, the frame is taken whole and ends the wait as data.
    let wire = encode(Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        STATION,
        5,
        &[0x22; MAX_EXTENDED_MPDU_DATA],
    ));
    assert_eq!(wire.len(), MAX_EXTENDED_FRAME_LENGTH);
    let mut now = wait_for_reply(&mut core);
    for &octet in &wire {
        now += Duration::from_micros(100);
        core.on_octet(octet, now);
    }
    assert_eq!(core.counters().parser_errors, 0);
    assert!(matches!(
        core.poll_action(),
        Action::DeliverNpdu { npdu, .. } if npdu.len() == MAX_EXTENDED_MPDU_DATA
    ));
}

/// A standard bundle can't hold extended frames, so a core with the flag on
/// is refused over it, naming the receive storage first.
#[test]
fn a_standard_bundle_refuses_extended_frames() {
    let mut storage: StandardStorage<2, 1> = StandardStorage::new();
    assert!(matches!(
        MasterCore::new(config(true), &mut storage, Instant::ZERO),
        Err(ConfigError::ExtendedStorageTooSmall {
            buffer: CoreBuffer::ReceiveStorage,
            ..
        })
    ));
}

/// A bundle around queues of the caller's own: here the allocation-free
/// ones again, built by hand.
#[test]
fn a_bundle_takes_queues_of_its_own() {
    let mut storage: Storage<bacnet_mstp_core::FixedQueues<1, 1>, 1> =
        Storage::with_queues(bacnet_mstp_core::FixedQueues::new());
    let mut core = MasterCore::new(config(false), &mut storage, Instant::ZERO).unwrap();
    wait_for_reply(&mut core);
    assert_eq!(core.counters().frames_transmitted, 1);
    assert!(
        storage
            .queues_mut()
            .take_counters(QueueKind::Unicast)
            .dequeued
            == 1
    );
}
