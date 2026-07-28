//! Clause 13.2.2 fault-precedence tests.
//!
//! Reliability had no path into event-state-detection before this (#167), so
//! `EventState::FAULT` was unreachable and every assertion here would have been
//! unwritable. The readings these pin come from a normative pass over the
//! standard; where the standard is silent, the test says so rather than
//! implying a citation exists.

use super::*;

const NO_FAULT: u32 = Reliability::NO_FAULT_DETECTED.to_raw();
const OVER_RANGE: u32 = Reliability::OVER_RANGE.to_raw();
const NO_SENSOR: u32 = Reliability::NO_SENSOR.to_raw();
const SHORTED_LOOP: u32 = Reliability::SHORTED_LOOP.to_raw();

#[test]
fn event_transition_classifier_covers_every_event_state_and_bit() {
    let cases = [
        (EventState::NORMAL, EventTransition::ToNormal, 0x04),
        (EventState::FAULT, EventTransition::ToFault, 0x02),
        (EventState::OFFNORMAL, EventTransition::ToOffnormal, 0x01),
        (EventState::HIGH_LIMIT, EventTransition::ToOffnormal, 0x01),
        (EventState::LOW_LIMIT, EventTransition::ToOffnormal, 0x01),
        (
            EventState::LIFE_SAFETY_ALARM,
            EventTransition::ToOffnormal,
            0x01,
        ),
    ];

    for (state, expected_transition, expected_bit) in cases {
        let transition = EventTransition::for_target_state(state);
        assert_eq!(transition, expected_transition, "state {}", state.to_raw());
        assert_eq!(
            transition.bit_mask(),
            expected_bit,
            "state {}",
            state.to_raw()
        );
    }
}

/// High limit 80, low limit 20, deadband 2, no delay, all transitions enabled.
fn detector() -> OutOfRangeDetector {
    OutOfRangeDetector {
        high_limit: 80.0,
        low_limit: 20.0,
        deadband: 2.0,
        limit_enable: LimitEnable::BOTH,
        notification_class: 1,
        notify_type: 0,
        event_enable: 0x07,
        time_delay: 0,
        event_state: EventState::NORMAL,
        acked_transitions: 0b111,
        pending: None,
        fault_reliability: None,
    }
}

// --- the rule itself ---

#[test]
fn fault_precedence_truth_table() {
    // Bad reliability outside FAULT: transition in.
    assert_eq!(
        fault_precedence(OVER_RANGE, None, EventState::NORMAL),
        FaultPrecedence::EnterFault
    );
    assert_eq!(
        fault_precedence(OVER_RANGE, None, EventState::HIGH_LIMIT),
        FaultPrecedence::EnterFault
    );
    // Unchanged bad reliability already in FAULT: hold.
    assert_eq!(
        fault_precedence(OVER_RANGE, Some(OVER_RANGE), EventState::FAULT),
        FaultPrecedence::HoldFault
    );
    // Changed bad reliability already in FAULT: re-enter.
    assert_eq!(
        fault_precedence(NO_SENSOR, Some(OVER_RANGE), EventState::FAULT),
        FaultPrecedence::ReenterFault
    );
    // In FAULT with no recorded value: re-enter, do not hold. The crate never
    // produces this state, but both fields are public so a downstream implementor
    // can. Re-entering is self-healing — it stores the value and the invariant
    // holds afterwards — whereas holding stores nothing, so the field would stay
    // None and every later genuine change would land here and hold again.
    assert_eq!(
        fault_precedence(OVER_RANGE, None, EventState::FAULT),
        FaultPrecedence::ReenterFault
    );
    // Recovered while in FAULT.
    assert_eq!(
        fault_precedence(NO_FAULT, Some(OVER_RANGE), EventState::FAULT),
        FaultPrecedence::RecoverToNormal
    );
    // No fault in play.
    assert_eq!(
        fault_precedence(NO_FAULT, None, EventState::NORMAL),
        FaultPrecedence::RunAlgorithm
    );
    assert_eq!(
        fault_precedence(NO_FAULT, None, EventState::HIGH_LIMIT),
        FaultPrecedence::RunAlgorithm
    );
}

#[test]
fn any_non_zero_reliability_faults_not_just_a_known_one() {
    // Clause 13.2.2 keys on "a value other than NO_FAULT_DETECTED", not on
    // membership in a list, so an unmodeled value must fault too.
    for reliability in [OVER_RANGE, SHORTED_LOOP, 9999] {
        let mut det = detector();
        assert_eq!(
            det.probe(50.0, reliability).unwrap().change.to,
            EventState::FAULT
        );
    }
}

// --- entry ---

#[test]
fn bad_reliability_drives_event_state_to_fault() {
    let mut det = detector();
    let outcome = det.probe(50.0, OVER_RANGE).expect("fault transition");
    assert_eq!(outcome.change.from, EventState::NORMAL);
    assert_eq!(outcome.change.to, EventState::FAULT);
    assert_eq!(det.event_state, EventState::FAULT);
}

#[test]
fn fault_takes_precedence_over_the_algorithm() {
    // Present value is above the high limit, so the algorithm alone would say
    // HIGH_LIMIT. Clause 13.2.2: "Fault detection takes precedence over the
    // detection of normal and offnormal states."
    let mut det = detector();
    assert_eq!(
        det.probe(99.0, OVER_RANGE).unwrap().change.to,
        EventState::FAULT
    );
}

#[test]
fn fault_entry_ignores_time_delay() {
    // Clause 13.2.2.1's ToFault transition carries no delay term; Time_Delay is
    // an event-algorithm parameter (Clause 13.3.1), and the algorithm is what
    // fault detection overrides. A delay of 10s must not defer the fault.
    let mut det = detector();
    det.time_delay = 10;
    let outcome = det
        .probe(50.0, OVER_RANGE)
        .expect("fault fires immediately");
    assert_eq!(outcome.change.to, EventState::FAULT);
    assert!(det.pending.is_none());
}

#[test]
fn fault_entry_cancels_an_in_flight_countdown() {
    // Project ruling, not a quotation: the standard is silent on the pending
    // timer's fate. Cancelling matches Clause 13.2.2.1.5's treatment of the
    // analogous inhibit case, which restarts the full delay rather than
    // resuming a partial one.
    let mut det = detector();
    det.time_delay = 5;
    assert!(det.probe(99.0, NO_FAULT).is_none(), "countdown seeded");
    assert_eq!(det.pending.as_ref().unwrap().state, EventState::HIGH_LIMIT);

    assert_eq!(
        det.probe(99.0, OVER_RANGE).unwrap().change.to,
        EventState::FAULT
    );
    assert!(
        det.pending.is_none(),
        "pre-fault countdown must be discarded"
    );
}

#[test]
fn holding_fault_blocks_a_state_independent_algorithm() {
    // `HoldFault` is what stops the event algorithm from running while the fault
    // stands. This test uses ChangeOfStateDetector deliberately: its
    // `compute_new_state` is a pure function of the present value, so without
    // the guard it would answer NORMAL for a non-alarm value and fire a
    // spurious FAULT -> NORMAL recovery while Reliability is still bad.
    //
    // OutOfRangeDetector cannot show this. Its `compute_new_state` matches on
    // `self.event_state` and falls through to `_ => self.event_state`, so it is
    // inert in FAULT and the guard is redundant *there* — which is exactly why
    // asserting the hold only against that detector would prove nothing.
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        time_delay: 0,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        det.probe(1, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    assert!(
        det.probe(0, SHORTED_LOOP).is_none(),
        "a non-alarm value must not recover the object while Reliability is bad"
    );
    assert_eq!(det.event_state, EventState::FAULT);
    assert!(det.tick(0, SHORTED_LOOP).is_none());
    assert_eq!(det.event_state, EventState::FAULT);
}

#[test]
fn holding_fault_reports_no_further_transitions() {
    // The standing condition is satisfied once FAULT holds, so re-evaluating with
    // the *same* Reliability value must not re-fire. Note every probe and tick
    // here passes OVER_RANGE unchanged — that is the whole point. Clause 13.2.2.1
    // does define a FAULT re-entry on a *changed* Reliability value, and #217
    // implements it; this test pins the other half of that rule, and is what fails
    // if the re-entry condition is widened to fire whenever the object is faulted
    // rather than only when the value changed.
    let mut det = detector();
    assert!(det.probe(50.0, OVER_RANGE).is_some());
    assert!(det.probe(50.0, OVER_RANGE).is_none());
    assert!(det.tick(50.0, OVER_RANGE).is_none());
    assert!(
        det.probe(99.0, OVER_RANGE).is_none(),
        "the standing fault must not report another transition"
    );
    assert_eq!(det.event_state, EventState::FAULT);
}

#[test]
fn out_of_range_reenters_fault_only_when_reliability_changes() {
    let mut det = detector();

    let mut outcomes = usize::from(det.probe(50.0, OVER_RANGE).is_some());
    for _ in 0..4 {
        outcomes += usize::from(det.tick(50.0, OVER_RANGE).is_some());
    }
    assert_eq!(outcomes, 1, "an unchanged fault must not flood");
    assert_eq!(det.fault_reliability, Some(OVER_RANGE));

    let reentry = det
        .probe(50.0, NO_SENSOR)
        .expect("changed reliability re-enters FAULT");
    assert_eq!(reentry.change.from, EventState::FAULT);
    assert_eq!(reentry.change.to, EventState::FAULT);
    assert_eq!(reentry.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(
        EventTransition::for_target_state(reentry.change.to),
        EventTransition::ToFault
    );
    assert_eq!(det.fault_reliability, Some(NO_SENSOR));
    assert!(det.tick(50.0, NO_SENSOR).is_none());

    assert_eq!(
        det.probe(50.0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
    assert_eq!(det.fault_reliability, None);
    assert_eq!(
        det.probe(50.0, NO_SENSOR).unwrap().change.to,
        EventState::FAULT
    );
}

#[test]
fn change_of_state_reenters_fault_only_when_reliability_changes() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        ..Default::default()
    };

    let mut outcomes = usize::from(det.probe(0, OVER_RANGE).is_some());
    for _ in 0..4 {
        outcomes += usize::from(det.tick(0, OVER_RANGE).is_some());
    }
    assert_eq!(outcomes, 1, "an unchanged fault must not flood");
    assert_eq!(det.fault_reliability, Some(OVER_RANGE));

    let reentry = det
        .probe(0, NO_SENSOR)
        .expect("changed reliability re-enters FAULT");
    assert_eq!(reentry.change.from, EventState::FAULT);
    assert_eq!(reentry.change.to, EventState::FAULT);
    assert_eq!(reentry.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(
        EventTransition::for_target_state(reentry.change.to),
        EventTransition::ToFault
    );
    assert_eq!(det.fault_reliability, Some(NO_SENSOR));
    assert!(det.tick(0, NO_SENSOR).is_none());

    assert_eq!(
        det.probe(0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
    assert_eq!(det.fault_reliability, None);
    assert_eq!(
        det.probe(0, NO_SENSOR).unwrap().change.to,
        EventState::FAULT
    );
}

#[test]
fn command_failure_reenters_fault_only_when_reliability_changes() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };

    let mut outcomes = usize::from(det.probe(0, 0, OVER_RANGE).is_some());
    for _ in 0..4 {
        outcomes += usize::from(det.tick(0, 0, OVER_RANGE).is_some());
    }
    assert_eq!(outcomes, 1, "an unchanged fault must not flood");
    assert_eq!(det.fault_reliability, Some(OVER_RANGE));

    let reentry = det
        .probe(0, 0, NO_SENSOR)
        .expect("changed reliability re-enters FAULT");
    assert_eq!(reentry.change.from, EventState::FAULT);
    assert_eq!(reentry.change.to, EventState::FAULT);
    assert_eq!(reentry.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(
        EventTransition::for_target_state(reentry.change.to),
        EventTransition::ToFault
    );
    assert_eq!(det.fault_reliability, Some(NO_SENSOR));
    assert!(det.tick(0, 0, NO_SENSOR).is_none());

    assert_eq!(
        det.probe(0, 0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
    assert_eq!(det.fault_reliability, None);
    assert_eq!(
        det.probe(0, 0, NO_SENSOR).unwrap().change.to,
        EventState::FAULT
    );
}

// --- recovery: the reading that refuted the issue's own suggestion ---

#[test]
fn recovery_from_fault_enters_normal_not_the_algorithm_state() {
    // THE keystone assertion. Clause 13.2.2.1 (Fault, ToNormal): "If
    // reliability-evaluation indicates a value of NO_FAULT_DETECTED, then
    // perform the corresponding transition actions and enter the Normal state."
    //
    // Issue #167's own "Suggested direction" proposed re-deriving the state from
    // the event algorithm here. That would yield HIGH_LIMIT, because the present
    // value never came back into range — a FAULT -> HIGH_LIMIT transition the
    // state machine does not define. This test is what fails under that design.
    let mut det = detector();
    assert_eq!(
        det.probe(99.0, OVER_RANGE).unwrap().change.to,
        EventState::FAULT
    );

    let outcome = det.probe(99.0, NO_FAULT).expect("recovery transition");
    assert_eq!(outcome.change.from, EventState::FAULT);
    assert_eq!(
        outcome.change.to,
        EventState::NORMAL,
        "recovery must enter NORMAL even though present_value is still out of range"
    );
}

#[test]
fn algorithm_reasserts_offnormal_after_recovery() {
    // Recovery lands in NORMAL, and the algorithm is then free to move the
    // object out of NORMAL under its own conditions — so the out-of-range value
    // is not swallowed, merely re-detected from the state machine's real state.
    let mut det = detector();
    det.probe(99.0, OVER_RANGE);
    det.probe(99.0, NO_FAULT);
    assert_eq!(det.event_state, EventState::NORMAL);

    let outcome = det.probe(99.0, NO_FAULT).expect("algorithm re-detects");
    assert_eq!(outcome.change.from, EventState::NORMAL);
    assert_eq!(outcome.change.to, EventState::HIGH_LIMIT);
}

#[test]
fn recovery_honors_time_delay_on_the_subsequent_offnormal_transition() {
    // The delay applies to the algorithm's transition out of NORMAL, not to the
    // recovery itself — the shape Clause 13.2.2.1.5 uses for the analogous
    // inhibit case, where the condition must hold for its regular time delay.
    let mut det = detector();
    det.time_delay = 3;
    det.probe(99.0, OVER_RANGE);

    assert_eq!(
        det.probe(99.0, NO_FAULT).unwrap().change.to,
        EventState::NORMAL,
        "recovery itself is not delayed"
    );
    assert!(det.probe(99.0, NO_FAULT).is_none(), "offnormal is delayed");
    assert_eq!(det.pending.as_ref().unwrap().remaining, 3);
}

// --- what the transition carries ---

#[test]
fn fault_transitions_report_change_of_reliability() {
    // Clause 13.2.5.3 / Table 13-3. This was implemented in #211 but was
    // unreachable until now, so this is the first test to exercise it through a
    // transition an object can actually produce.
    let mut det = detector();
    let into = det.probe(50.0, OVER_RANGE).unwrap().change;
    assert_eq!(
        into.event_type(OutOfRangeDetector::ALGORITHM),
        EventType::CHANGE_OF_RELIABILITY
    );
    assert_eq!(into.transition(), EventTransition::ToFault);

    let out = det.probe(50.0, NO_FAULT).unwrap().change;
    assert_eq!(
        out.event_type(OutOfRangeDetector::ALGORITHM),
        EventType::CHANGE_OF_RELIABILITY
    );
    assert_eq!(out.transition(), EventTransition::ToNormal);
}

#[test]
fn fault_override_wins_in_both_directions_for_every_detector_algorithm() {
    let mut out_of_range = detector();
    let into = out_of_range.probe(50.0, OVER_RANGE).unwrap();
    let out = out_of_range.probe(50.0, NO_FAULT).unwrap();
    assert_eq!(into.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(out.event_type, EventType::CHANGE_OF_RELIABILITY);

    let mut change_of_state = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        ..Default::default()
    };
    let into = change_of_state.probe(0, OVER_RANGE).unwrap();
    let out = change_of_state.probe(0, NO_FAULT).unwrap();
    assert_eq!(into.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(out.event_type, EventType::CHANGE_OF_RELIABILITY);

    let mut command_failure = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    let into = command_failure.probe(0, 0, OVER_RANGE).unwrap();
    let out = command_failure.probe(0, 0, NO_FAULT).unwrap();
    assert_eq!(into.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(out.event_type, EventType::CHANGE_OF_RELIABILITY);
}

#[test]
fn out_of_range_detector_reports_its_algorithm_for_non_fault_transition() {
    let outcome = detector().probe(99.0, NO_FAULT).unwrap();
    assert_eq!(outcome.change.to, EventState::HIGH_LIMIT);
    assert_eq!(outcome.event_type, EventType::OUT_OF_RANGE);
}

#[test]
fn change_of_state_detector_reports_its_algorithm_for_non_fault_transition() {
    let mut detector = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        ..Default::default()
    };
    let outcome = detector.probe(1, NO_FAULT).unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::CHANGE_OF_STATE);
}

#[test]
fn command_failure_offnormal_reports_command_failure_not_change_of_state() {
    let mut detector = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    let outcome = detector.probe(1, 0, NO_FAULT).unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn fault_distribution_honors_the_to_fault_event_enable_bit() {
    // Clause 13.2.5: Event_Enable scopes distribution, never detection. The
    // transition is reported either way; only `distribute` changes.
    let mut det = detector();
    det.event_enable = 0x02; // TO_FAULT only
    assert!(det.probe(50.0, OVER_RANGE).unwrap().distribute);

    let mut det = detector();
    det.event_enable = 0x05; // TO_OFFNORMAL | TO_NORMAL, no TO_FAULT
    let outcome = det.probe(50.0, OVER_RANGE).expect("still detected");
    assert!(!outcome.distribute);
    assert_eq!(
        det.event_state,
        EventState::FAULT,
        "a cleared bit must not suppress the Event_State write"
    );
}

#[test]
fn out_of_range_distribution_selects_the_to_normal_bit() {
    for (event_enable, expected) in [(0x04, true), (0x03, false)] {
        let mut det = detector();
        det.event_state = EventState::HIGH_LIMIT;
        det.event_enable = event_enable;

        let outcome = det.probe(50.0, NO_FAULT).expect("TO_NORMAL transition");
        assert_eq!(outcome.change.to, EventState::NORMAL);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

#[test]
fn out_of_range_distribution_selects_the_to_offnormal_bit() {
    for (event_enable, expected) in [(0x01, true), (0x06, false)] {
        let mut det = detector();
        det.event_enable = event_enable;

        let outcome = det.probe(90.0, NO_FAULT).expect("TO_OFFNORMAL transition");
        assert_eq!(outcome.change.to, EventState::HIGH_LIMIT);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

#[test]
fn change_of_state_distribution_selects_the_to_normal_bit() {
    for (event_enable, expected) in [(0x04, true), (0x03, false)] {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![1],
            event_enable,
            event_state: EventState::OFFNORMAL,
            ..Default::default()
        };

        let outcome = det.probe(0, NO_FAULT).expect("TO_NORMAL transition");
        assert_eq!(outcome.change.to, EventState::NORMAL);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

#[test]
fn change_of_state_distribution_selects_the_to_offnormal_bit() {
    for (event_enable, expected) in [(0x01, true), (0x06, false)] {
        let mut det = ChangeOfStateDetector {
            alarm_values: vec![1],
            event_enable,
            ..Default::default()
        };

        let outcome = det.probe(1, NO_FAULT).expect("TO_OFFNORMAL transition");
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

#[test]
fn command_failure_distribution_selects_the_to_normal_bit() {
    for (event_enable, expected) in [(0x04, true), (0x03, false)] {
        let mut det = CommandFailureDetector {
            event_enable,
            event_state: EventState::OFFNORMAL,
            ..Default::default()
        };

        let outcome = det.probe(1, 1, NO_FAULT).expect("TO_NORMAL transition");
        assert_eq!(outcome.change.to, EventState::NORMAL);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

#[test]
fn command_failure_distribution_selects_the_to_offnormal_bit() {
    for (event_enable, expected) in [(0x01, true), (0x06, false)] {
        let mut det = CommandFailureDetector {
            event_enable,
            ..Default::default()
        };

        let outcome = det.probe(1, 0, NO_FAULT).expect("TO_OFFNORMAL transition");
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.distribute, expected, "mask {event_enable:#04x}");
    }
}

// --- the other two detectors ---

#[test]
fn change_of_state_detector_applies_fault_precedence() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        time_delay: 0,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        det.probe(0, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    // Recovery enters NORMAL even though the present value is an alarm value.
    assert_eq!(
        det.probe(1, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
}

#[test]
fn command_failure_detector_applies_fault_precedence() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        time_delay: 0,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        det.probe(1, 1, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
}

#[test]
fn change_of_state_fault_entry_discards_an_in_flight_countdown() {
    // Apply the same project ruling as the out-of-range detector: a countdown
    // seeded before FAULT must not resume after recovery.
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        time_delay: 2,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert!(det.probe(1, NO_FAULT).is_none(), "countdown seeded");
    assert_eq!(det.pending.as_ref().unwrap().state, EventState::OFFNORMAL);
    assert_eq!(
        det.probe(1, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    assert!(det.pending.is_none(), "pre-fault countdown is discarded");

    assert_eq!(
        det.probe(1, NO_FAULT).unwrap().change.to,
        EventState::NORMAL
    );
    assert!(det.tick(1, NO_FAULT).is_none(), "a fresh countdown starts");
    assert_eq!(det.pending.as_ref().unwrap().remaining, 2);
}

#[test]
fn command_failure_holds_fault_while_reliability_remains_bad() {
    // Matching values make the algorithm answer NORMAL, pinning HoldFault.
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        det.probe(1, 1, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    assert!(det.probe(1, 1, SHORTED_LOOP).is_none());
    assert!(det.tick(1, 1, SHORTED_LOOP).is_none());
    assert_eq!(det.event_state, EventState::FAULT);
}

#[test]
fn command_failure_recovers_to_normal_before_rerunning_its_algorithm() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        det.probe(1, 1, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    let recovery = det
        .tick(1, 0, NO_FAULT)
        .expect("healthy reliability recovers through the periodic path");
    assert_eq!(recovery.change.from, EventState::FAULT);
    assert_eq!(recovery.change.to, EventState::NORMAL);
    assert_eq!(
        det.probe(1, 0, NO_FAULT).unwrap().change.to,
        EventState::OFFNORMAL
    );
}

#[test]
fn command_failure_fault_entry_discards_an_in_flight_countdown() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        time_delay: 2,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert!(det.probe(1, 0, NO_FAULT).is_none(), "countdown seeded");
    assert_eq!(det.pending.as_ref().unwrap().state, EventState::OFFNORMAL);
    assert_eq!(
        det.tick(1, 0, SHORTED_LOOP).unwrap().change.to,
        EventState::FAULT
    );
    assert!(det.pending.is_none(), "pre-fault countdown is discarded");
}

// --- object level: the macro wiring reaches real objects ---

#[test]
fn writing_reliability_on_an_object_drives_event_state_to_fault() {
    // Proves the `impl_intrinsic_reporting!` wiring, which is what carries
    // reliability into detection for all nine intrinsically-reporting object
    // types. Reliability arrives by the ordinary property-write route, so this
    // also covers the case the server's fault detector is not involved in at
    // all — a local or network write of Reliability.
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::PropertyValue;

    let mut ai = AnalogInputObject::new(1, "ai-1", 62).expect("construct");
    assert!(
        ai.evaluate_intrinsic_reporting().is_none(),
        "healthy object has no transition to report"
    );

    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .expect("Out_Of_Service is writable");
    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(OVER_RANGE),
        None,
    )
    .expect("reliability is writable");

    let outcome = ai
        .evaluate_intrinsic_reporting()
        .expect("reliability must drive a transition");
    assert_eq!(outcome.change.to, EventState::FAULT);
    assert_eq!(
        outcome.change.event_type(OutOfRangeDetector::ALGORITHM),
        EventType::CHANGE_OF_RELIABILITY
    );
}

#[test]
fn ticking_an_object_uses_reliability_for_fault_and_recovery() {
    // The running server drives this periodic macro arm.
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::PropertyValue;

    let mut ai = AnalogInputObject::new(3, "ai-3", 62).expect("construct");
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .expect("Out_Of_Service is writable");
    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(OVER_RANGE),
        None,
    )
    .expect("reliability is writable");
    let fault = ai
        .tick_intrinsic_reporting()
        .expect("periodic tick must observe bad reliability");
    assert_eq!(fault.change.to, EventState::FAULT);

    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(NO_FAULT),
        None,
    )
    .expect("reliability is writable");
    let recovery = ai
        .tick_intrinsic_reporting()
        .expect("periodic tick must observe recovered reliability");
    assert_eq!(recovery.change.from, EventState::FAULT);
    assert_eq!(recovery.change.to, EventState::NORMAL);
}

#[test]
fn faulted_object_reports_both_fault_and_in_alarm_status_flags() {
    // Clause 12.2 derives IN_ALARM from Event_State and FAULT from Reliability,
    // and notes "The relationship between individual flags is not defined by the
    // protocol." They are reconciled upstream: while event-state-detection is
    // enabled, Clause 13.2.2 makes a bad Reliability determine Event_State =
    // FAULT, so both flags read TRUE together. Before #167 this object reported
    // FAULT TRUE with IN_ALARM FALSE, because Reliability never reached
    // Event_State.
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::{PropertyValue, StatusFlags};

    let mut ai = AnalogInputObject::new(2, "ai-2", 62).expect("construct");
    ai.set_reliability_internal(OVER_RANGE)
        .expect("in-service reliability evaluation is supported");
    ai.evaluate_intrinsic_reporting();

    let flags = ai
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .expect("status flags readable");
    let PropertyValue::BitString { data, .. } = flags else {
        panic!("Status_Flags is a bit string");
    };
    let bits = data[0] >> 4;
    assert_ne!(
        bits & StatusFlags::FAULT.bits(),
        0,
        "FAULT follows Reliability"
    );
    assert_ne!(
        bits & StatusFlags::IN_ALARM.bits(),
        0,
        "IN_ALARM follows Event_State, which Reliability now drives"
    );
}

#[test]
fn command_failure_to_fault_distribution_is_no_longer_hardcoded_off() {
    // #200: this detector's `fire` returned `distribute: false` for FAULT
    // unconditionally, which was unobservable while FAULT was unreachable.
    let mut det = CommandFailureDetector {
        event_enable: 0x02, // TO_FAULT set
        time_delay: 0,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert!(
        det.probe(1, 1, SHORTED_LOOP).unwrap().distribute,
        "TO_FAULT distribution must follow Event_Enable like the other two directions"
    );

    let mut det = CommandFailureDetector {
        event_enable: 0x05, // TO_FAULT clear
        time_delay: 0,
        event_state: EventState::NORMAL,
        ..Default::default()
    };
    assert!(!det.probe(1, 1, SHORTED_LOOP).unwrap().distribute);
}
