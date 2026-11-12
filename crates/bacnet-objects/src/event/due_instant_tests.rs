//! Each detector's delay runs from the instant its condition was first
//! seen, to the instant (#2303): seeded by a write at 0.9 s with a delay of
//! 2, a transition is due at 2.9 s, both into an offnormal state
//! (Time_Delay) and back to NORMAL (Time_Delay_Normal). The one-second
//! countdown fired these at the 2 s tick.

use super::*;

const NO_FAULT: Reliability = Reliability::NO_FAULT_DETECTED;

fn ms(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

/// Seeded at 0.9 s: nothing at 2 s or just before 2.9 s, then at 2.9 s.
fn fires_at_2900(mut tick: impl FnMut(Instant) -> Option<TransitionOutcome>) -> EventState {
    for millis in [1_000, 2_000, 2_899] {
        assert!(
            tick(ms(millis)).is_none(),
            "fired at {millis} ms, before its due instant"
        );
    }
    tick(ms(2_900)).expect("due at 2.9 s").change.to
}

#[test]
fn change_of_state_delays_run_from_the_write() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: EventTransitionBits::all(),
        time_delay: 2,
        time_delay_normal: Some(2),
        ..Default::default()
    };
    assert!(det.probe(1, NO_FAULT, ms(900)).is_none());
    assert_eq!(det.next_due(), Some(ms(2_900)));
    let to = fires_at_2900(|now| det.tick(1, NO_FAULT, now));
    assert_eq!(to, EventState::OFFNORMAL);

    // Back to NORMAL under Time_Delay_Normal, timed the same way from a
    // write 0.9 s into a fresh clock.
    let mut det = ChangeOfStateDetector {
        event_state: EventState::OFFNORMAL,
        ..det
    };
    assert!(det.probe(0, NO_FAULT, ms(900)).is_none());
    assert_eq!(det.next_due(), Some(ms(2_900)));
    let to = fires_at_2900(|now| det.tick(0, NO_FAULT, now));
    assert_eq!(to, EventState::NORMAL);
}

#[test]
fn command_failure_delays_run_from_the_write() {
    let mut det = CommandFailureDetector {
        event_enable: EventTransitionBits::all(),
        time_delay: 2,
        time_delay_normal: Some(2),
        ..Default::default()
    };
    assert!(det.probe(1, 0, NO_FAULT, ms(900)).is_none());
    assert_eq!(det.next_due(), Some(ms(2_900)));
    let to = fires_at_2900(|now| det.tick(1, 0, NO_FAULT, now));
    assert_eq!(to, EventState::OFFNORMAL);

    let mut det = CommandFailureDetector {
        event_state: EventState::OFFNORMAL,
        ..det
    };
    assert!(det.probe(1, 1, NO_FAULT, ms(900)).is_none());
    assert_eq!(det.next_due(), Some(ms(2_900)));
    let to = fires_at_2900(|now| det.tick(1, 1, NO_FAULT, now));
    assert_eq!(to, EventState::NORMAL);
}

#[test]
fn out_of_range_time_delay_normal_runs_from_the_write() {
    let mut det = OutOfRangeDetector {
        high_limit: 80.0,
        low_limit: 20.0,
        deadband: 2.0,
        limit_enable: LimitEnable::all(),
        event_enable: EventTransitionBits::all(),
        time_delay: 5,
        time_delay_normal: Some(2),
        event_state: EventState::HIGH_LIMIT,
        ..Default::default()
    };
    assert!(det.probe(50.0, NO_FAULT, ms(900)).is_none());
    assert_eq!(det.next_due(), Some(ms(2_900)));
    let to = fires_at_2900(|now| det.tick(50.0, NO_FAULT, now));
    assert_eq!(to, EventState::NORMAL);
}
