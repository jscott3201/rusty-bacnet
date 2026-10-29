//! The adapter's side of the transmit contract, and the receive storage the
//! core needs.

use bacnet_mstp_codec::{Frame, FrameType};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, ConfigError, Direction, FixedQueues, Instant, MIN_RX_STORAGE, MasterCore, MasterState,
    MstpCoreConfig, TransmitError, TransmitId,
};

type Queues = FixedQueues<4, 4>;
type Actions = bacnet_mstp_core::ActionBuffers;

fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 2;
    config
}

/// Take the token with one NPDU queued and wait out the turnaround, so a
/// transmit is queued as actions. Returns when it was dispatched.
fn queue_a_transmit(core: &mut MasterCore<'_, Queues>) -> Instant {
    core.enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(Frame::new(FrameType::TOKEN, 1, 2, &[]), Instant::ZERO);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    due
}

fn poll_transmit(core: &mut MasterCore<'_, Queues>) -> TransmitId {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { id, .. } => id,
        other => panic!("expected transmit, got {other:?}"),
    }
}

#[test]
fn completion_without_a_confirmed_start_is_ignored_and_counted() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let due = queue_a_transmit(&mut core);
    let id = poll_transmit(&mut core);
    core.on_transmit_complete(id, due);
    assert_eq!(core.counters().adapter_errors, 1);
    assert_eq!(core.snapshot().transmit_id, Some(id));
    assert_eq!(core.counters().frames_transmitted, 0);

    // Starting it first is accepted as usual; starting it twice is counted.
    core.on_transmit_started(id, due).unwrap();
    assert_eq!(
        core.on_transmit_started(id, due),
        Err(TransmitError::AlreadyStarted)
    );
    assert_eq!(core.counters().adapter_errors, 2);
    core.on_transmit_complete(id, due);
    assert_eq!(core.counters().frames_transmitted, 1);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
}

#[test]
fn an_aborted_transmit_leaves_no_stale_actions() {
    // The adapter never polls the transmit; the watchdog aborts it.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    queue_a_transmit(&mut core);
    let watchdog = core.next_deadline().unwrap();
    core.on_deadline(watchdog);
    assert_eq!(core.snapshot().transmit_id, None);
    assert_eq!(core.state(), MasterState::Idle);
    // Only the switch back to receive is left: no driver enable and no frame
    // for the aborted id ahead of it.
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.poll_action(), Action::Idle);

    // Polled up to the direction change only: the frame is still withdrawn.
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    queue_a_transmit(&mut core);
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let watchdog = core.next_deadline().unwrap();
    core.on_deadline(watchdog);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.poll_action(), Action::Idle);
}

#[test]
fn receive_storage_must_hold_a_header() {
    assert_eq!(MIN_RX_STORAGE, 8);
    for length in [0, MIN_RX_STORAGE - 1] {
        let mut rx = vec![0u8; length];
        let mut actions = Actions::new();
        assert!(matches!(
            MasterCore::from_parts(config(), Queues::new(), &mut rx, &mut actions, Instant::ZERO),
            Err(ConfigError::ReceiveStorageTooSmall { length: got, minimum: 8 }) if got == length
        ));
    }
    let mut rx = [0u8; MIN_RX_STORAGE];
    let mut actions = Actions::new();
    assert!(
        MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO
        )
        .is_ok()
    );
}
