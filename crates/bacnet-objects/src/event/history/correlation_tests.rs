use super::*;

use bacnet_types::primitives::{Date, Time};

use crate::event::{EventStateChange, EventTransitionCommit};

fn sequence(value: u16) -> BACnetTimeStamp {
    BACnetTimeStamp::SequenceNumber(value)
}

fn time(hour: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::Time(Time {
        hour,
        minute: 2,
        second: 3,
        hundredths: 4,
    })
}

fn date_time(day: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 9,
            day,
            day_of_week: 3,
        },
        time: Time {
            hour: 5,
            minute: 6,
            second: 7,
            hundredths: 8,
        },
    }
}

fn commit(
    state: &mut EventState,
    acked: &mut u8,
    history: &mut EventHistory,
    to: EventState,
    timestamp: BACnetTimeStamp,
) {
    let from = *state;
    EventTransitionState::new(state, acked, history)
        .commit(EventTransitionCommit {
            change: EventStateChange { from, to },
            coordinate: EventTransition::for_target_state(to),
            ack_required: true,
            timestamp,
            message_text: None,
        })
        .unwrap();
}

fn assert_service_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::SERVICES.to_raw() as u32
                && code == expected.to_raw() as u32
    ));
}

#[test]
fn exact_timestamp_choices_acknowledge_prior_coordinates_one_bit_at_a_time() {
    let mut state = EventState::NORMAL;
    let mut acked = 0b111;
    let mut history = EventHistory::default();
    let stamps = [time(1), sequence(22), date_time(2)];

    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::HIGH_LIMIT,
        stamps[0].clone(),
    );
    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::FAULT,
        stamps[1].clone(),
    );
    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::NORMAL,
        stamps[2].clone(),
    );
    assert_eq!(state, EventState::NORMAL);
    assert_eq!(acked, 0);

    let history_before = history.clone();
    for (requested_state, stamp, expected) in [
        (EventState::HIGH_LIMIT, &stamps[0], 0b001),
        (EventState::FAULT, &stamps[1], 0b011),
        (EventState::NORMAL, &stamps[2], 0b111),
    ] {
        history
            .acknowledge_correlated(&mut acked, requested_state, stamp)
            .unwrap();
        assert_eq!(acked, expected);
        assert_eq!(history, history_before);
    }
}

#[test]
fn latest_same_coordinate_timestamp_replaces_the_older_timestamp() {
    let mut state = EventState::NORMAL;
    let mut acked = 0b111;
    let mut history = EventHistory::default();
    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::HIGH_LIMIT,
        sequence(1),
    );
    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::LOW_LIMIT,
        sequence(2),
    );
    let before = (acked, history.clone());

    assert_service_error(
        history.acknowledge_correlated(&mut acked, EventState::LOW_LIMIT, &sequence(1)),
        ErrorCode::INVALID_TIME_STAMP,
    );
    assert_eq!((acked, history.clone()), before);
    assert_service_error(
        history.acknowledge_correlated(&mut acked, EventState::HIGH_LIMIT, &sequence(1)),
        ErrorCode::INVALID_EVENT_STATE,
    );
    assert_eq!((acked, history.clone()), before);
    history
        .acknowledge_correlated(&mut acked, EventState::LOW_LIMIT, &sequence(2))
        .unwrap();
    assert_eq!(acked, before.0 | 0b001);
}

#[test]
fn request_side_offnormal_wildcard_matches_every_residual_offnormal_state() {
    for stored in [
        EventState::HIGH_LIMIT,
        EventState::LOW_LIMIT,
        EventState::LIFE_SAFETY_ALARM,
        EventState::from_raw(65_535),
    ] {
        let mut state = EventState::NORMAL;
        let mut acked = 0b110;
        let mut history = EventHistory::default();
        commit(&mut state, &mut acked, &mut history, stored, sequence(9));

        history
            .acknowledge_correlated(&mut acked, EventState::OFFNORMAL, &sequence(9))
            .unwrap();
        assert_eq!(acked, 0b111, "stored state {stored:?}");
    }
}

#[test]
fn concrete_state_matching_is_exact_and_precedes_timestamp_validation() {
    for (stored, requested) in [
        (EventState::LOW_LIMIT, EventState::HIGH_LIMIT),
        (EventState::OFFNORMAL, EventState::HIGH_LIMIT),
        (EventState::from_raw(60_001), EventState::from_raw(60_002)),
    ] {
        let mut state = EventState::NORMAL;
        let mut acked = 0b101;
        let mut history = EventHistory::default();
        commit(&mut state, &mut acked, &mut history, stored, sequence(12));
        let before = (acked, history.clone());

        assert_service_error(
            history.acknowledge_correlated(&mut acked, requested, &sequence(99)),
            ErrorCode::INVALID_EVENT_STATE,
        );
        assert_eq!((acked, history.clone()), before);
    }
}

#[test]
fn exact_generic_and_proprietary_states_match_their_committed_identity() {
    for stored in [EventState::OFFNORMAL, EventState::from_raw(60_001)] {
        let mut state = EventState::NORMAL;
        let mut acked = 0b110;
        let mut history = EventHistory::default();
        commit(&mut state, &mut acked, &mut history, stored, sequence(14));

        history
            .acknowledge_correlated(&mut acked, stored, &sequence(14))
            .unwrap();
        assert_eq!(acked, 0b111);
    }
}

#[test]
fn uninitialized_sequence_zero_slot_is_not_a_committed_transition() {
    let history = EventHistory::default();
    let mut acked = 0b010;
    let before = (acked, history.clone());

    assert_service_error(
        history.acknowledge_correlated(&mut acked, EventState::HIGH_LIMIT, &sequence(0)),
        ErrorCode::INVALID_TIME_STAMP,
    );
    assert_eq!((acked, history), before);
}

#[test]
fn already_set_bit_is_idempotent_and_preserves_all_history() {
    let mut state = EventState::NORMAL;
    let mut acked = 0b111;
    let mut history = EventHistory::default();
    commit(
        &mut state,
        &mut acked,
        &mut history,
        EventState::HIGH_LIMIT,
        sequence(7),
    );
    acked |= 0b001;
    let before = (state, acked, history.clone());

    history
        .acknowledge_correlated(&mut acked, EventState::HIGH_LIMIT, &sequence(7))
        .unwrap();
    assert_eq!((state, acked, history), before);
}
