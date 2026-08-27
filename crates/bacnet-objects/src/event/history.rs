//! Shared storage for intrinsic-reporting event history properties.

use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_types::enums::{EventState, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};
use bytes::BytesMut;

use super::{EventTransition, EventTransitionCommit, EventTransitionCommitError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventHistory {
    /// Transition slots ordered `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`.
    pub(crate) time_stamps: [BACnetTimeStamp; 3],
    /// Transition slots ordered `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`.
    pub(crate) message_texts: [String; 3],
}

/// Borrowed object-owned state for one atomic event-transition commit.
///
/// This aggregate is staged for object-family adoption: it keeps the generic
/// commit policy centralized while each object remains responsible for
/// lending its own three property stores through its trait hook.
pub(crate) struct EventTransitionState<'a> {
    event_state: &'a mut EventState,
    acked_transitions: &'a mut u8,
    history: &'a mut EventHistory,
}

impl<'a> EventTransitionState<'a> {
    pub(crate) fn new(
        event_state: &'a mut EventState,
        acked_transitions: &'a mut u8,
        history: &'a mut EventHistory,
    ) -> Self {
        Self {
            event_state,
            acked_transitions,
            history,
        }
    }

    /// Validate and commit all event properties for exactly one coordinate.
    pub(crate) fn commit(
        self,
        commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        let expected_coordinate = EventTransition::for_target_state(commit.change.to);
        if commit.coordinate != expected_coordinate {
            return Err(EventTransitionCommitError::CoordinateTargetMismatch {
                coordinate: commit.coordinate,
                target: commit.change.to,
            });
        }

        if *self.event_state != commit.change.from {
            return Err(EventTransitionCommitError::CurrentStateMismatch {
                expected: commit.change.from,
                actual: *self.event_state,
            });
        }

        let index = commit.coordinate.index();
        let bit = commit.coordinate.bit_mask();
        let acknowledged = !commit.ack_required;

        *self.event_state = commit.change.to;
        if acknowledged {
            *self.acked_transitions |= bit;
        } else {
            *self.acked_transitions &= !bit;
        }
        self.history.time_stamps[index] = commit.timestamp;
        if let Some(message_text) = commit.message_text {
            self.history.message_texts[index] = message_text;
        }

        Ok(())
    }
}

/// Implement proposal-and-commit intrinsic reporting for built-in object families.
///
/// The evaluation hooks only propose transitions. The commit hook lends the
/// detector and object history to the shared kernel and finalizes private
/// detector state only after that kernel succeeds. The gated arms enforce
/// Clause 13.2.2.1's Event_Detection_Enable state-machine gate.
macro_rules! impl_builtin_intrinsic_reporting {
    (
        $detector_field:ident,
        $history_field:ident,
        $present_value_field:ident,
        $feedback_value_field:ident,
        $reliability_field:ident,
        $event_detection_enable_field:ident
    ) => {
        fn intrinsic_reporting_requires_atomic_commit(&self) -> bool {
            true
        }

        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field.propose(
                self.$present_value_field,
                self.$feedback_value_field,
                self.$reliability_field,
            )
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field.tick_proposal(
                self.$present_value_field,
                self.$feedback_value_field,
                self.$reliability_field,
            )
        }

        fn commit_event_transition_internal(
            &mut self,
            commit: $crate::event::EventTransitionCommit,
        ) -> Result<(), $crate::event::EventTransitionCommitError> {
            let change = commit.change.clone();
            $crate::event::history::EventTransitionState::new(
                &mut self.$detector_field.event_state,
                &mut self.$detector_field.acked_transitions,
                &mut self.$history_field,
            )
            .commit(commit)?;
            self.$detector_field
                .confirm_transition(&change, self.$reliability_field);
            Ok(())
        }
    };
    (
        $detector_field:ident,
        $history_field:ident,
        $present_value_field:ident,
        $reliability_field:ident,
        $event_detection_enable_field:ident
    ) => {
        fn intrinsic_reporting_requires_atomic_commit(&self) -> bool {
            true
        }

        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field
                .propose(self.$present_value_field, self.$reliability_field)
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            if !self.$event_detection_enable_field {
                return None;
            }
            self.$detector_field
                .tick_proposal(self.$present_value_field, self.$reliability_field)
        }

        fn commit_event_transition_internal(
            &mut self,
            commit: $crate::event::EventTransitionCommit,
        ) -> Result<(), $crate::event::EventTransitionCommitError> {
            let change = commit.change.clone();
            $crate::event::history::EventTransitionState::new(
                &mut self.$detector_field.event_state,
                &mut self.$detector_field.acked_transitions,
                &mut self.$history_field,
            )
            .commit(commit)?;
            self.$detector_field
                .confirm_transition(&change, self.$reliability_field);
            Ok(())
        }
    };
}

pub(crate) use impl_builtin_intrinsic_reporting;

/// Commit a built-in proposal with fixed, explicit test policy.
///
/// Object-level tests that need a later state transition use no-acknowledgment
/// policy and sequence timestamp zero. Production resolves both values from
/// the Notification Class and database-local timestamp source instead.
#[cfg(test)]
pub(crate) fn commit_test_proposal<O>(
    object: &mut O,
    outcome: super::TransitionOutcome,
) -> super::TransitionOutcome
where
    O: crate::traits::BACnetObject + ?Sized,
{
    object
        .commit_event_transition_internal(EventTransitionCommit {
            coordinate: outcome.change.transition(),
            change: outcome.change.clone(),
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(0),
            message_text: None,
        })
        .expect("built-in test proposal must commit");
    outcome
}

impl Default for EventHistory {
    fn default() -> Self {
        Self {
            time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            message_texts: [String::new(), String::new(), String::new()],
        }
    }
}

impl EventHistory {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Reads the fixed-size event arrays with their BACnet array semantics.
    pub(crate) fn read(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        match property {
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Some(match array_index {
                None => Ok(PropertyValue::List(
                    self.time_stamps.iter().map(timestamp_value).collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(index @ 1..=3) => Ok(timestamp_value(&self.time_stamps[index as usize - 1])),
                Some(_) => Err(crate::common::invalid_array_index_error()),
            }),
            p if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS => Some(match array_index {
                None => Ok(PropertyValue::List(
                    self.message_texts
                        .iter()
                        .cloned()
                        .map(PropertyValue::CharacterString)
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(index @ 1..=3) => Ok(PropertyValue::CharacterString(
                    self.message_texts[index as usize - 1].clone(),
                )),
                Some(_) => Err(crate::common::invalid_array_index_error()),
            }),
            _ => None,
        }
    }
}

fn timestamp_value(stamp: &BACnetTimeStamp) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_timestamp_choice(&mut encoded, stamp)
        .expect("BACnetTimeStamp CHOICE encoding is infallible");
    PropertyValue::ApplicationData(encoded.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    use bacnet_types::enums::{EventState, ObjectType};
    use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

    use crate::event::{
        EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
    };
    use crate::traits::BACnetObject;

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
                month: 8,
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
        change: EventStateChange,
        coordinate: EventTransition,
        ack_required: bool,
        timestamp: BACnetTimeStamp,
        message_text: Option<&str>,
    ) -> Result<(), EventTransitionCommitError> {
        EventTransitionState::new(state, acked, history).commit(EventTransitionCommit {
            change,
            coordinate,
            ack_required,
            timestamp,
            message_text: message_text.map(str::to_owned),
        })
    }

    #[test]
    fn reset_restores_mutated_history_to_default() {
        let mut history = EventHistory::default();
        history.time_stamps[1] = BACnetTimeStamp::SequenceNumber(42);
        history.message_texts[2] = "transition".into();

        history.reset();

        assert_eq!(history, EventHistory::default());
    }

    #[test]
    fn commit_updates_each_coordinate_and_preserves_exact_timestamp_choices() {
        let mut state = EventState::NORMAL;
        let mut acked = 0b101;
        let mut history = EventHistory {
            time_stamps: [
                BACnetTimeStamp::SequenceNumber(10),
                BACnetTimeStamp::SequenceNumber(20),
                BACnetTimeStamp::SequenceNumber(30),
            ],
            message_texts: [
                "old-offnormal".into(),
                "old-fault".into(),
                "old-normal".into(),
            ],
        };

        commit(
            &mut state,
            &mut acked,
            &mut history,
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventTransition::ToOffnormal,
            true,
            time(1),
            Some("high-limit"),
        )
        .unwrap();
        assert_eq!(state, EventState::HIGH_LIMIT);
        assert_eq!(acked, 0b100, "only TO_OFFNORMAL is cleared");
        assert_eq!(history.time_stamps[0], time(1));
        assert_eq!(history.time_stamps[1], BACnetTimeStamp::SequenceNumber(20));
        assert_eq!(history.time_stamps[2], BACnetTimeStamp::SequenceNumber(30));
        assert_eq!(
            history.message_texts,
            ["high-limit", "old-fault", "old-normal"]
        );

        commit(
            &mut state,
            &mut acked,
            &mut history,
            EventStateChange {
                from: EventState::HIGH_LIMIT,
                to: EventState::FAULT,
            },
            EventTransition::ToFault,
            false,
            BACnetTimeStamp::SequenceNumber(0),
            Some("fault"),
        )
        .unwrap();
        assert_eq!(state, EventState::FAULT);
        assert_eq!(acked, 0b110, "TO_FAULT is set and other bits are untouched");
        assert_eq!(history.time_stamps[0], time(1));
        assert_eq!(history.time_stamps[1], BACnetTimeStamp::SequenceNumber(0));
        assert_eq!(history.time_stamps[2], BACnetTimeStamp::SequenceNumber(30));
        assert_eq!(history.message_texts, ["high-limit", "fault", "old-normal"]);

        commit(
            &mut state,
            &mut acked,
            &mut history,
            EventStateChange {
                from: EventState::FAULT,
                to: EventState::NORMAL,
            },
            EventTransition::ToNormal,
            true,
            date_time(27),
            Some("normal"),
        )
        .unwrap();
        assert_eq!(state, EventState::NORMAL);
        assert_eq!(acked, 0b010, "only TO_NORMAL is cleared");
        assert_eq!(
            history.time_stamps,
            [time(1), BACnetTimeStamp::SequenceNumber(0), date_time(27)]
        );
        assert_eq!(history.message_texts, ["high-limit", "fault", "normal"]);
    }

    #[test]
    fn same_state_reindication_commits_and_none_preserves_every_message() {
        let mut state = EventState::NORMAL;
        let mut acked = 0b001;
        let mut history = EventHistory {
            time_stamps: [
                BACnetTimeStamp::SequenceNumber(1),
                BACnetTimeStamp::SequenceNumber(2),
                BACnetTimeStamp::SequenceNumber(3),
            ],
            message_texts: ["offnormal".into(), "fault".into(), "normal".into()],
        };
        let messages_before = history.message_texts.clone();

        commit(
            &mut state,
            &mut acked,
            &mut history,
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::NORMAL,
            },
            EventTransition::ToNormal,
            false,
            BACnetTimeStamp::SequenceNumber(u16::MAX),
            None,
        )
        .unwrap();

        assert_eq!(state, EventState::NORMAL);
        assert_eq!(
            acked, 0b101,
            "TO_NORMAL is set and other bits are untouched"
        );
        assert_eq!(
            history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(1),
                BACnetTimeStamp::SequenceNumber(2),
                BACnetTimeStamp::SequenceNumber(u16::MAX),
            ]
        );
        assert_eq!(history.message_texts, messages_before);
    }

    #[test]
    fn coordinate_mismatch_rejects_without_mutating_any_state() {
        let mut state = EventState::HIGH_LIMIT;
        let mut acked = 0b101;
        let mut history = EventHistory {
            time_stamps: [time(9), BACnetTimeStamp::SequenceNumber(44), date_time(26)],
            message_texts: ["offnormal".into(), "fault".into(), "normal".into()],
        };
        let expected_state = state;
        let expected_acked = acked;
        let expected_history = history.clone();

        let error = commit(
            &mut state,
            &mut acked,
            &mut history,
            EventStateChange {
                from: EventState::HIGH_LIMIT,
                to: EventState::NORMAL,
            },
            EventTransition::ToOffnormal,
            false,
            BACnetTimeStamp::SequenceNumber(99),
            Some("must-not-commit"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EventTransitionCommitError::CoordinateTargetMismatch {
                coordinate: EventTransition::ToOffnormal,
                target: EventState::NORMAL,
            }
        );
        assert_eq!(state, expected_state);
        assert_eq!(acked, expected_acked);
        assert_eq!(history, expected_history);
    }

    #[test]
    fn stale_state_changing_replay_rejects_without_mutating_first_commit() {
        let mut state = EventState::NORMAL;
        let mut acked = 0b111;
        let mut history = EventHistory::default();
        let first = EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::LOW_LIMIT,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(77),
            message_text: Some("low-limit".into()),
        };

        EventTransitionState::new(&mut state, &mut acked, &mut history)
            .commit(first.clone())
            .unwrap();
        let expected_acked = acked;
        let expected_history = history.clone();

        let error = EventTransitionState::new(&mut state, &mut acked, &mut history)
            .commit(first)
            .unwrap_err();

        assert_eq!(
            error,
            EventTransitionCommitError::CurrentStateMismatch {
                expected: EventState::NORMAL,
                actual: EventState::LOW_LIMIT,
            }
        );
        assert_eq!(state, EventState::LOW_LIMIT);
        assert_eq!(acked, expected_acked);
        assert_eq!(history, expected_history);
    }

    #[test]
    fn object_trait_default_rejects_event_transition_commit() {
        struct DefaultOnly {
            oid: ObjectIdentifier,
        }

        impl BACnetObject for DefaultOnly {
            fn object_identifier(&self) -> ObjectIdentifier {
                self.oid
            }

            fn object_name(&self) -> &str {
                "default-only"
            }

            fn read_property(
                &self,
                _property: PropertyIdentifier,
                _array_index: Option<u32>,
            ) -> Result<PropertyValue, Error> {
                Err(Error::Encoding("not used by this test".into()))
            }

            fn write_property(
                &mut self,
                _property: PropertyIdentifier,
                _array_index: Option<u32>,
                _value: PropertyValue,
                _priority: Option<u8>,
            ) -> Result<(), Error> {
                Err(Error::Encoding("not used by this test".into()))
            }

            fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
                Cow::Borrowed(&[])
            }
        }

        let mut object = DefaultOnly {
            oid: ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap(),
        };
        let commit = EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::FAULT,
            },
            coordinate: EventTransition::ToFault,
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(1),
            message_text: None,
        };

        assert_eq!(
            object.commit_event_transition_internal(commit),
            Err(EventTransitionCommitError::Unsupported)
        );
    }
}
