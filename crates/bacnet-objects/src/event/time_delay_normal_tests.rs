//! Time_Delay_Normal tests (ASHRAE 135-2020 §13.3 pTimeDelayNormal).
//!
//! The algorithms carry TWO delays: pTimeDelay governs every indication into
//! an offnormal state (including offnormal→offnormal re-indication — OOR
//! (d)/(g), CHANGE_OF_STATE (c)), while pTimeDelayNormal gates only the
//! sustained-condition return to NORMAL (OOR (e)/(h), CHANGE_OF_STATE (b),
//! COMMAND_FAILURE (b)). Absent, pTimeDelayNormal "takes on the value of the
//! pTimeDelay parameter", so the fallback must be visible as such: `None`.

use super::*;

const NO_FAULT: u32 = bacnet_types::enums::Reliability::NO_FAULT_DETECTED.to_raw();
const FAULTED: u32 = bacnet_types::enums::Reliability::OVER_RANGE.to_raw();

/// High limit 80, low limit 20, deadband 2, all transitions enabled, and a
/// configurable `Time_Delay` — mirroring `tests.rs`'s delayed fixture.
fn make_delayed_detector(time_delay: u32) -> OutOfRangeDetector {
    OutOfRangeDetector {
        high_limit: 80.0,
        low_limit: 20.0,
        deadband: 2.0,
        limit_enable: LimitEnable::BOTH,
        event_enable: 0x07,
        time_delay,
        ..Default::default()
    }
}

fn make_asymmetric_detector(time_delay: u32, time_delay_normal: u32) -> OutOfRangeDetector {
    let mut det = make_delayed_detector(time_delay);
    det.time_delay_normal = Some(time_delay_normal);
    det
}

#[test]
fn time_delay_normal_selects_the_return_to_normal_delay() {
    // TD=5 / TDN=10: the offnormal indication fires after 5 ticks; the
    // NORMAL indication needs 10 — Clause 13.3.6 (a)/(b) versus (e).
    let mut det = make_asymmetric_detector(5, 10);
    assert!(det.probe(81.0, NO_FAULT).is_none());
    assert_eq!(
        det.pending.as_ref().expect("pending seeded").remaining,
        5,
        "the offnormal direction is seeded from Time_Delay, not Time_Delay_Normal"
    );
    for _ in 0..4 {
        assert!(det.tick(81.0, NO_FAULT).is_none());
        assert_eq!(det.event_state, EventState::NORMAL);
    }
    let change = det.tick(81.0, NO_FAULT).unwrap().change; // 5th tick fires
    assert_eq!(change.to, EventState::HIGH_LIMIT);

    // Return path: drop below (high_limit - deadband) = 78.0 and hold.
    assert!(det.probe(50.0, NO_FAULT).is_none());
    assert_eq!(
        det.pending
            .as_ref()
            .expect("return-to-normal pending seeded")
            .remaining,
        10,
        "the NORMAL direction is seeded from Time_Delay_Normal"
    );
    for _ in 0..9 {
        let outcome = det.tick(50.0, NO_FAULT);
        assert!(outcome.is_none(), "NORMAL must wait out pTimeDelayNormal");
        assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    }
    let change = det.tick(50.0, NO_FAULT).unwrap().change; // 10th tick fires
    assert_eq!(change.from, EventState::HIGH_LIMIT);
    assert_eq!(change.to, EventState::NORMAL);
    assert!(det.pending.is_none());
}

#[test]
fn time_delay_normal_absent_falls_back_to_time_delay() {
    // The normative absent case: symmetric delays in both directions without
    // any Time_Delay_Normal configured (= pre-#225 behavior).
    let mut det = make_delayed_detector(4);
    assert!(det.time_delay_normal.is_none());
    assert!(det.probe(81.0, NO_FAULT).is_none());
    for _ in 0..3 {
        assert!(det.tick(81.0, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(81.0, NO_FAULT).unwrap().change.to,
        EventState::HIGH_LIMIT
    );
    assert!(det.probe(50.0, NO_FAULT).is_none());
    assert_eq!(
        det.pending
            .as_ref()
            .expect("return pending seeded")
            .remaining,
        4,
        "absent Time_Delay_Normal takes on Time_Delay's value"
    );
    for _ in 0..3 {
        assert!(det.tick(50.0, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(50.0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
}

#[test]
fn time_delay_normal_zero_fires_the_return_to_normal_immediately() {
    // TD=5 / TDN=0: the NORMAL direction is immediate while the offnormal
    // direction still waits out pTimeDelay.
    let mut det = make_asymmetric_detector(5, 0);
    det.event_state = EventState::HIGH_LIMIT;
    let change = det.probe(50.0, NO_FAULT).unwrap().change;
    assert_eq!(change.to, EventState::NORMAL);
    assert!(det.pending.is_none());

    // ...and the offnormal direction still waits 5 ticks.
    assert!(det.probe(81.0, NO_FAULT).is_none());
    for _ in 0..4 {
        assert!(det.tick(81.0, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(81.0, NO_FAULT).unwrap().change.to,
        EventState::HIGH_LIMIT
    );
}

#[test]
fn time_delay_normal_does_not_govern_offnormal_to_offnormal() {
    // Clause 13.3.6 (d): HIGH_LIMIT → LOW_LIMIT re-cross is an offnormal
    // indication, so it is pTimeDelay-gated even with a long
    // Time_Delay_Normal configured.
    let mut det = make_asymmetric_detector(5, 30);
    det.event_state = EventState::HIGH_LIMIT;
    assert!(det.probe(19.0, NO_FAULT).is_none());
    let pending = det
        .pending
        .as_ref()
        .expect("direct re-cross seeded pending");
    assert_eq!(pending.state, EventState::LOW_LIMIT);
    assert_eq!(
        pending.remaining, 5,
        "offnormal→offnormal uses pTimeDelay, not pTimeDelayNormal"
    );
    for _ in 0..4 {
        assert!(det.tick(19.0, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(19.0, NO_FAULT).unwrap().change.to,
        EventState::LOW_LIMIT
    );
}

#[test]
fn time_delay_normal_mid_delay_reseed_uses_the_current_targets_delay() {
    // A pending offnormal re-cross (seeded with TD=5) abandoned for a fresh
    // return-to-NORMAL condition re-seeds with TDN=10 — the delay belongs to
    // the transition being counted, not to whichever was pending first.
    let mut det = make_asymmetric_detector(5, 10);
    det.event_state = EventState::HIGH_LIMIT;
    assert!(det.probe(19.0, NO_FAULT).is_none());
    let pending = det.pending.as_ref().expect("re-cross pending seeded");
    assert_eq!(pending.state, EventState::LOW_LIMIT);
    assert_eq!(pending.remaining, 5);

    assert!(det.probe(50.0, NO_FAULT).is_none());
    let pending = det
        .pending
        .as_ref()
        .expect("re-seeded toward NORMAL with pTimeDelayNormal");
    assert_eq!(pending.state, EventState::NORMAL);
    assert_eq!(pending.remaining, 10);
    for _ in 0..9 {
        assert!(det.tick(50.0, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(50.0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
}

#[test]
fn fault_transitions_ignore_both_delays() {
    // Clause 13.2.2 fault precedence carries no delay term: an in-flight
    // countdown is dropped and FAULT fires on the evaluation itself, whatever
    // Time_Delay / Time_Delay_Normal say.
    let mut det = make_asymmetric_detector(30, 30);
    assert!(det.probe(81.0, NO_FAULT).is_none());
    assert!(det.pending.is_some());
    let outcome = det.tick(81.0, FAULTED).expect("FAULT fires immediately");
    assert_eq!(outcome.change.to, EventState::FAULT);
    assert_eq!(
        outcome.event_type,
        bacnet_types::enums::EventType::CHANGE_OF_RELIABILITY
    );
    assert!(det.pending.is_none());
    assert_eq!(det.event_state, EventState::FAULT);
}

#[test]
fn time_delay_normal_change_of_state_asymmetric_round_trip() {
    // Same selection on CHANGE_OF_STATE: Clause 13.3.2 (a) is pTimeDelay,
    // (b) is pTimeDelayNormal.
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        time_delay: 2,
        time_delay_normal: Some(4),
        ..Default::default()
    };
    assert!(det.probe(1, NO_FAULT).is_none());
    assert_eq!(det.pending.as_ref().unwrap().remaining, 2);
    assert!(det.tick(1, NO_FAULT).is_none());
    assert_eq!(
        det.tick(1, NO_FAULT).unwrap().change.to,
        EventState::OFFNORMAL
    );

    assert!(det.probe(0, NO_FAULT).is_none());
    assert_eq!(
        det.pending.as_ref().unwrap().remaining,
        4,
        "Clause 13.3.2 (b) waits pTimeDelayNormal"
    );
    for _ in 0..3 {
        assert!(det.tick(0, NO_FAULT).is_none());
    }
    assert_eq!(det.tick(0, NO_FAULT).unwrap().change.to, EventState::NORMAL);
}

#[test]
fn time_delay_normal_command_failure_asymmetric_round_trip() {
    // Command failure has no offnormal→offnormal condition; its (a) is
    // pTimeDelay and (b) is pTimeDelayNormal (Clause 13.3.4).
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        time_delay: 1,
        time_delay_normal: Some(3),
        ..Default::default()
    };
    assert!(det.probe(1, 0, NO_FAULT).is_none()); // mismatch → pending OFFNORMAL
    assert_eq!(det.pending.as_ref().unwrap().remaining, 1);
    assert_eq!(
        det.tick(1, 0, NO_FAULT).unwrap().change.to,
        EventState::OFFNORMAL
    );

    assert!(det.probe(1, 1, NO_FAULT).is_none()); // agree → pending NORMAL
    assert_eq!(
        det.pending.as_ref().unwrap().remaining,
        3,
        "Clause 13.3.4 (b) waits pTimeDelayNormal"
    );
    for _ in 0..2 {
        assert!(det.tick(1, 1, NO_FAULT).is_none());
    }
    assert_eq!(
        det.tick(1, 1, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
}

#[test]
fn time_delay_normal_delayed_return_still_cancels_on_revert() {
    // The debounce contract is direction-independent: reverting to the
    // alarmed condition before pTimeDelayNormal elapses cancels the pending
    // NORMAL transition with no notification. The confirmed state is already
    // HIGH_LIMIT, so the revert leaves NOTHING pending — staying alarmed is
    // not a new indication and needs no countdown.
    let mut det = make_asymmetric_detector(5, 10);
    det.event_state = EventState::HIGH_LIMIT;
    assert!(det.probe(50.0, NO_FAULT).is_none());
    for _ in 0..5 {
        assert!(det.tick(50.0, NO_FAULT).is_none());
    }
    assert!(
        det.tick(81.0, NO_FAULT).is_none(),
        "revert cancels, no fire"
    );
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    assert!(det.pending.is_none(), "cancelled, not re-armed");
}
