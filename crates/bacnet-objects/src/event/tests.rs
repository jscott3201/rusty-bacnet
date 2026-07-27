use super::*;

fn make_detector() -> OutOfRangeDetector {
    OutOfRangeDetector {
        high_limit: 80.0,
        low_limit: 20.0,
        deadband: 2.0,
        limit_enable: LimitEnable::BOTH,
        notification_class: 1,
        notify_type: 0,
        event_enable: 0x07, // all transitions
        time_delay: 0,
        event_state: EventState::NORMAL,
        acked_transitions: 0b111,
        pending: None,
    }
}

#[test]
fn normal_stays_normal_within_limits() {
    let mut det = make_detector();
    assert!(det.evaluate(50.0).is_none());
    assert_eq!(det.event_state, EventState::NORMAL);
}

#[test]
fn normal_to_high_limit() {
    let mut det = make_detector();
    let change = det.evaluate(81.0).unwrap().change;
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
}

#[test]
fn normal_to_low_limit() {
    let mut det = make_detector();
    let change = det.evaluate(19.0).unwrap().change;
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::LOW_LIMIT);
    assert_eq!(det.event_state, EventState::LOW_LIMIT);
}

#[test]
fn at_boundary_no_transition() {
    let mut det = make_detector();
    // At exactly high_limit — not exceeded, stays NORMAL
    assert!(det.evaluate(80.0).is_none());
    // At exactly low_limit — not below, stays NORMAL
    assert!(det.evaluate(20.0).is_none());
}

#[test]
fn high_limit_to_normal_with_deadband() {
    let mut det = make_detector();
    det.evaluate(81.0); // → HIGH_LIMIT

    // Still above (high_limit - deadband) = 78.0 — stay HIGH_LIMIT
    assert!(det.evaluate(79.0).is_none());

    // Drop below deadband threshold
    let change = det.evaluate(77.0).unwrap().change;
    assert_eq!(change.from, EventState::HIGH_LIMIT);
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn low_limit_to_normal_with_deadband() {
    let mut det = make_detector();
    det.evaluate(19.0); // → LOW_LIMIT

    // Still below (low_limit + deadband) = 22.0 — stay LOW_LIMIT
    assert!(det.evaluate(21.0).is_none());

    // Rise above deadband threshold
    let change = det.evaluate(23.0).unwrap().change;
    assert_eq!(change.from, EventState::LOW_LIMIT);
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn high_limit_to_low_limit_direct() {
    let mut det = make_detector();
    det.evaluate(81.0); // → HIGH_LIMIT

    // Drop directly below low_limit
    let change = det.evaluate(19.0).unwrap().change;
    assert_eq!(change.from, EventState::HIGH_LIMIT);
    assert_eq!(change.to, EventState::LOW_LIMIT);
}

#[test]
fn low_limit_to_high_limit_direct() {
    let mut det = make_detector();
    det.evaluate(19.0); // → LOW_LIMIT

    // Jump directly above high_limit
    let change = det.evaluate(81.0).unwrap().change;
    assert_eq!(change.from, EventState::LOW_LIMIT);
    assert_eq!(change.to, EventState::HIGH_LIMIT);
}

#[test]
fn high_limit_disabled_no_transition() {
    let mut det = make_detector();
    det.limit_enable.high_limit_enable = false;

    // Above high_limit but disabled — stays NORMAL
    assert!(det.evaluate(100.0).is_none());
}

#[test]
fn low_limit_disabled_no_transition() {
    let mut det = make_detector();
    det.limit_enable.low_limit_enable = false;

    // Below low_limit but disabled — stays NORMAL
    assert!(det.evaluate(0.0).is_none());
}

#[test]
fn both_limits_disabled() {
    let mut det = make_detector();
    det.limit_enable = LimitEnable::NONE;
    assert!(det.evaluate(100.0).is_none());
    assert!(det.evaluate(0.0).is_none());
}

#[test]
fn limit_enable_bits_round_trip() {
    let le = LimitEnable::BOTH;
    let bits = le.to_bits();
    let decoded = LimitEnable::from_bits(bits);
    assert_eq!(decoded, le);

    let le = LimitEnable {
        low_limit_enable: true,
        high_limit_enable: false,
    };
    let bits = le.to_bits();
    let decoded = LimitEnable::from_bits(bits);
    assert_eq!(decoded, le);
}

#[test]
fn deadband_at_exact_boundary() {
    let mut det = make_detector();
    det.evaluate(81.0); // → HIGH_LIMIT

    // At exactly (high_limit - deadband) = 78.0 — still HIGH_LIMIT (need to be below)
    assert!(det.evaluate(78.0).is_none());

    // Just below
    let change = det.evaluate(77.99).unwrap().change;
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn event_state_change_derives_event_type() {
    use bacnet_types::enums::EventType;

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    assert_eq!(change.event_type(), EventType::OUT_OF_RANGE);
}

#[test]
fn event_state_change_to_normal_from_high() {
    use bacnet_types::enums::EventType;

    let change = EventStateChange {
        from: EventState::HIGH_LIMIT,
        to: EventState::NORMAL,
    };
    assert_eq!(change.event_type(), EventType::OUT_OF_RANGE);
}

#[test]
fn event_enable_zero_suppresses_distribution_not_the_transition() {
    // Clause 12.12 scopes Event_Enable to distribution, and Clause 13.2.2.1.4
    // requires the transition actions regardless. So every transition is still
    // reported and Event_State still advances; only `distribute` goes false.
    let mut det = make_detector();
    det.event_enable = 0x00; // all disabled

    for (pv, expected) in [
        (81.0, EventState::HIGH_LIMIT),
        (50.0, EventState::NORMAL),
        (19.0, EventState::LOW_LIMIT),
    ] {
        let outcome = det
            .evaluate(pv)
            .expect("transition is reported even when suppressed");
        assert!(!outcome.distribute);
        assert_eq!(outcome.change.to, expected);
        assert_eq!(det.event_state, expected);
    }
}

#[test]
fn event_enable_to_normal_only() {
    let mut det = make_detector();
    det.event_enable = 0x04; // only TO_NORMAL

    // NORMAL → HIGH_LIMIT: TO_OFFNORMAL not enabled, so not distributed —
    // but still reported, and Event_State still advances.
    let outcome = det.evaluate(81.0).unwrap();
    assert!(!outcome.distribute);
    assert_eq!(outcome.change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);

    // HIGH_LIMIT → NORMAL: TO_NORMAL enabled, fires
    let change = det.evaluate(50.0).unwrap().change;
    assert_eq!(change.from, EventState::HIGH_LIMIT);
    assert_eq!(change.to, EventState::NORMAL);

    // NORMAL → LOW_LIMIT: TO_OFFNORMAL not enabled, so not distributed.
    let outcome = det.evaluate(19.0).unwrap();
    assert!(!outcome.distribute);
    assert_eq!(outcome.change.to, EventState::LOW_LIMIT);
    assert_eq!(det.event_state, EventState::LOW_LIMIT);

    // LOW_LIMIT → NORMAL: TO_NORMAL enabled, fires
    let change = det.evaluate(50.0).unwrap().change;
    assert_eq!(change.from, EventState::LOW_LIMIT);
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn event_enable_to_offnormal_only() {
    let mut det = make_detector();
    det.event_enable = 0x01; // only TO_OFFNORMAL

    // NORMAL → HIGH_LIMIT: TO_OFFNORMAL enabled, fires
    let change = det.evaluate(81.0).unwrap().change;
    assert_eq!(change.to, EventState::HIGH_LIMIT);

    // HIGH_LIMIT → NORMAL: TO_NORMAL not enabled, so not distributed.
    let outcome = det.evaluate(50.0).unwrap();
    assert!(!outcome.distribute);
    assert_eq!(outcome.change.to, EventState::NORMAL);
    assert_eq!(det.event_state, EventState::NORMAL);
}

#[test]
fn event_state_change_generic() {
    use bacnet_types::enums::EventType;

    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::NORMAL,
    };
    assert_eq!(change.event_type(), EventType::CHANGE_OF_STATE);
}

// --- ChangeOfStateDetector tests ---

#[test]
fn cos_normal_when_no_alarm_values() {
    let mut det = ChangeOfStateDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    assert!(det.evaluate(0).is_none()); // empty alarm_values → always NORMAL
}

#[test]
fn cos_normal_to_offnormal() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1], // ACTIVE (1) is alarm
        event_enable: 0x07,
        ..Default::default()
    };
    let change = det.evaluate(1).unwrap().change;
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::OFFNORMAL);
}

#[test]
fn cos_offnormal_to_normal() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        ..Default::default()
    };
    det.evaluate(1); // → OFFNORMAL
    let change = det.evaluate(0).unwrap().change; // back to NORMAL
    assert_eq!(change.from, EventState::OFFNORMAL);
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn cos_stays_offnormal_while_in_alarm() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        ..Default::default()
    };
    det.evaluate(1); // → OFFNORMAL
    assert!(det.evaluate(1).is_none()); // still alarm value, no change
}

#[test]
fn cos_multistate_alarm_values() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![3, 5, 7], // multiple alarm states
        event_enable: 0x07,
        ..Default::default()
    };
    assert!(det.evaluate(1).is_none()); // not an alarm state
    let change = det.evaluate(5).unwrap().change;
    assert_eq!(change.to, EventState::OFFNORMAL);
    assert!(det.evaluate(3).is_none()); // still offnormal (different alarm value)
    let change = det.evaluate(2).unwrap().change;
    assert_eq!(change.to, EventState::NORMAL);
}

// --- CommandFailureDetector tests ---

#[test]
fn cmdfail_matching_stays_normal() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    assert!(det.evaluate(1, 1).is_none()); // present == feedback
}

#[test]
fn cmdfail_mismatch_goes_offnormal() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    let change = det.evaluate(1, 0).unwrap().change; // present != feedback
    assert_eq!(change.to, EventState::OFFNORMAL);
}

#[test]
fn cmdfail_match_restores_normal() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        ..Default::default()
    };
    det.evaluate(1, 0); // → OFFNORMAL
    let change = det.evaluate(1, 1).unwrap().change; // match → NORMAL
    assert_eq!(change.to, EventState::NORMAL);
}

// --- Time_Delay tests (AS-HRAE 135-2020 §13.2.4) ---
//
// `probe` is the per-write entry; `tick` is the 1 Hz periodic entry. The
// invariant under test: the countdown advances per tick (elapsed second),
// never per probe, so repeated writes to the same out-of-range value cannot
// shorten the delay. Event_State stays at the confirmed (old) state until
// the delay elapses; reverting mid-delay cancels with no notification.

fn make_delayed_detector(time_delay: u32) -> OutOfRangeDetector {
    let mut det = make_detector();
    det.time_delay = time_delay;
    det
}

#[test]
fn time_delay_zero_fires_immediately_on_probe() {
    let mut det = make_delayed_detector(0);
    let change = det.probe(81.0).unwrap().change;
    assert_eq!(change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    assert!(det.pending.is_none());
}

#[test]
fn time_delay_nonzero_probe_seeds_pending_without_firing() {
    let mut det = make_delayed_detector(3);
    assert!(det.probe(81.0).is_none());
    // Event_State stays at the confirmed (old) state during the delay.
    assert_eq!(det.event_state, EventState::NORMAL);
    let pending = det.pending.expect("pending seeded");
    assert_eq!(pending.state, EventState::HIGH_LIMIT);
    assert_eq!(pending.remaining, 3);
}

#[test]
fn time_delay_repeated_probes_do_not_accelerate_countdown() {
    // The regression test for the original design flaw: calling the per-write
    // entry repeatedly with the same out-of-range value must NOT decrement the
    // countdown. Three probes then three more — still pending, full delay.
    let mut det = make_delayed_detector(3);
    for _ in 0..6 {
        assert!(det.probe(81.0).is_none());
    }
    assert_eq!(det.event_state, EventState::NORMAL);
    let pending = det.pending.expect("pending still seeded");
    assert_eq!(
        pending.remaining, 3,
        "writes must not advance the countdown"
    );
}

#[test]
fn time_delay_redundant_probe_does_not_reset_countdown() {
    // Regression for the symmetric bug class: a redundant write of the SAME
    // qualifying value while a pending transition already exists must NOT
    // re-seed (which would reset `remaining` to the full delay). Per
    // ASHRAE 135-2020 §13.2.4 Time_Delay is a debounce timer — writes faster
    // than the 1s tick must not pin the transition forever. Interleave probe
    // and tick to prove the elapsed countdown survives a redundant probe.
    let mut det = make_delayed_detector(3);
    det.probe(81.0); // seed HIGH_LIMIT, remaining = 3
    assert!(det.tick(81.0).is_none()); // remaining 2

    // A redundant probe of the same out-of-range value: the elapsed second
    // must NOT be erased. `remaining` stays 2, not reset to 3.
    assert!(det.probe(81.0).is_none());
    assert_eq!(
        det.pending.as_ref().unwrap().remaining,
        2,
        "redundant probe must not re-seed the countdown"
    );

    // And the transition still fires after the remaining ticks, not 3 more.
    assert!(det.tick(81.0).is_none()); // remaining 1
    let change = det.tick(81.0).unwrap().change; // remaining 0 → fire
    assert_eq!(change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
}

#[test]
fn time_delay_chattering_input_still_fires() {
    // The full pinning scenario: writes faster than the 1s tick. A 3s delay
    // must still elapse after exactly 3 ticks despite sub-tick redundant writes.
    let mut det = make_delayed_detector(3);
    det.probe(81.0); // remaining 3
    for _ in 0..3 {
        // Chatter: a redundant probe before each tick.
        assert!(det.probe(81.0).is_none());
        let r = det.tick(81.0);
        // First two ticks: no fire. Third tick (remaining 0): fire.
        if det.pending.is_some() {
            assert!(r.is_none());
        } else {
            assert!(r.is_some(), "transition must fire on the 3rd tick");
        }
    }
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    assert!(det.pending.is_none());
}

#[test]
fn time_delay_fires_after_exact_tick_count() {
    let mut det = make_delayed_detector(3);
    det.probe(81.0); // seed, remaining = 3
    assert_eq!(det.event_state, EventState::NORMAL);

    assert!(det.tick(81.0).is_none()); // remaining 2
    assert!(det.tick(81.0).is_none()); // remaining 1
    let change = det.tick(81.0).unwrap().change; // remaining 0 → fire
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
    assert!(det.pending.is_none());
}

#[test]
fn time_delay_cancelled_on_revert_before_expiry() {
    let mut det = make_delayed_detector(3);
    det.probe(81.0); // seed pending HIGH_LIMIT
    assert!(det.tick(81.0).is_none()); // remaining 2

    // Condition clears mid-delay: pending cancelled, no notification, no state change.
    assert!(det.tick(50.0).is_none());
    assert!(det.pending.is_none());
    assert_eq!(det.event_state, EventState::NORMAL);

    // A subsequent tick with no out-of-range condition fires nothing.
    assert!(det.tick(50.0).is_none());
    assert_eq!(det.event_state, EventState::NORMAL);
}

#[test]
fn time_delay_reseeded_when_target_changes_mid_delay() {
    let mut det = make_delayed_detector(2);
    det.probe(81.0); // pending HIGH_LIMIT, remaining 2
    assert!(det.tick(81.0).is_none()); // remaining 1

    // Jump to a different out-of-range target (LOW_LIMIT): re-seed the delay
    // for the new target without firing the old one.
    assert!(det.tick(19.0).is_none());
    let pending = det.pending.expect("re-seeded for new target");
    assert_eq!(pending.state, EventState::LOW_LIMIT);
    assert_eq!(pending.remaining, 2);
    assert_eq!(det.event_state, EventState::NORMAL);
}

#[test]
fn time_delay_change_of_state_seeds_and_fires() {
    let mut det = ChangeOfStateDetector {
        alarm_values: vec![1],
        event_enable: 0x07,
        time_delay: 2,
        ..Default::default()
    };
    assert!(det.probe(1).is_none());
    assert_eq!(det.event_state, EventState::NORMAL);
    assert!(det.tick(1).is_none()); // remaining 1
    let change = det.tick(1).unwrap().change; // fire
    assert_eq!(change.to, EventState::OFFNORMAL);
    assert_eq!(det.event_state, EventState::OFFNORMAL);
}

#[test]
fn time_delay_command_failure_seeds_and_fires() {
    let mut det = CommandFailureDetector {
        event_enable: 0x07,
        time_delay: 1,
        ..Default::default()
    };
    assert!(det.probe(1, 0).is_none()); // mismatch → pending OFFNORMAL
    assert_eq!(det.event_state, EventState::NORMAL);
    let change = det.tick(1, 0).unwrap().change; // remaining 0 → fire
    assert_eq!(change.to, EventState::OFFNORMAL);
}

#[test]
fn time_delay_event_enable_gates_distribution_not_state_during_delay() {
    // event_enable==0 for the offnormal transition: the delay still counts,
    // Event_State still advances, and the transition is still reported — with
    // `distribute` false so the notification is suppressed at the send site.
    let mut det = make_delayed_detector(1);
    det.event_enable = 0x00;
    assert!(det.probe(81.0).is_none(), "delay seeded, nothing fired yet");
    let outcome = det
        .tick(81.0)
        .expect("delay elapsed: transition is reported");
    assert!(!outcome.distribute);
    assert_eq!(outcome.change.to, EventState::HIGH_LIMIT);
    assert_eq!(det.event_state, EventState::HIGH_LIMIT);
}
