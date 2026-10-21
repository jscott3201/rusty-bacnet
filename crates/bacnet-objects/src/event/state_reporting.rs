//! CHANGE_OF_STATE intrinsic reporting on an enumerated value the object
//! works out when asked, rather than one it stores (#1305).
//!
//! Binary and multi-state objects watch a stored Present_Value, and
//! `impl_builtin_intrinsic_reporting!` reads their fields directly. An Access
//! Zone watches Occupancy_State, derived on each read from the count, the
//! limits and Occupancy_Count_Enable (Clause 12.32.6), so there is no field
//! to read. [`ChangeOfStateReporting`] keeps everything such an object needs
//! for intrinsic reporting in one place:
//!
//! - the CHANGE_OF_STATE detector (Clause 13.3.2), whose Alarm_Values hold
//!   members of the watched enumeration other than its NORMAL state
//!   ([`ListedStates`]);
//! - the history behind Event_Time_Stamps and Event_Message_Texts;
//! - Event_Detection_Enable.
//!
//! It serves and takes the event rows itself, and
//! [`impl_change_of_state_reporting!`] wires the `BACnetObject` hooks to it,
//! asking the object for the watched value and its Reliability each time the
//! server evaluates or ticks. An object adopts it in three steps:
//!
//! 1. hold a `ChangeOfStateReporting`, built with the [`ListedStates`] of its
//!    enumeration;
//! 2. offer each read and write to [`ChangeOfStateReporting::read`] and
//!    [`ChangeOfStateReporting::write`], and take Event_State (and so the
//!    IN_ALARM flag) from [`ChangeOfStateReporting::event_state`];
//! 3. invoke the macro with the field and two `&self` methods returning the
//!    watched value and the Reliability served.
//!
//! The server's notification payload is told separately which property an
//! object type watches and which BACnetPropertyStates choice carries it.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use super::history::{EventHistory, EventTransitionState};
use super::options::ReportingOptions;
use super::{
    ChangeOfStateDetector, EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use crate::common::{self, read_generic_event_properties, write_generic_event_properties};

/// The event state, configuration and history of one object reporting
/// CHANGE_OF_STATE on a value it derives.
#[derive(Debug, Clone)]
pub(crate) struct ChangeOfStateReporting {
    /// The detector; named as the shared event-row macros expect.
    event_detector: ChangeOfStateDetector,
    /// Event_Time_Stamps, Event_Message_Texts and the latest transition.
    event_history: EventHistory,
    /// Event_Detection_Enable: FALSE suspends the detector (Clause 13.2.2.1).
    event_detection_enable: bool,
    /// The values Alarm_Values may hold; any other element is
    /// VALUE_OUT_OF_RANGE.
    states: ListedStates,
}

impl ChangeOfStateReporting {
    /// Detection on, no alarm values, every transition acknowledged, and
    /// Notification_Class 0, as the other built-in detectors start.
    pub(crate) fn new(states: ListedStates) -> Self {
        Self {
            event_detector: ChangeOfStateDetector::default(),
            event_history: EventHistory::default(),
            event_detection_enable: true,
            states,
        }
    }

    /// Event_State as served.
    pub(crate) fn event_state(&self) -> EventState {
        self.event_detector.event_state
    }

    /// Alarm_Values as held, raw.
    pub(crate) fn alarm_values(&self) -> &[u32] {
        &self.event_detector.alarm_values
    }

    /// Replace Alarm_Values, checking each value as a network write would;
    /// a refused list leaves the one held.
    pub(crate) fn set_alarm_values(&mut self, values: Vec<u32>) -> Result<(), Error> {
        self.event_detector.alarm_values = checked_raw_list(values, self.states)?;
        Ok(())
    }

    /// Serve one of the event rows, or `None` for any other property.
    pub(crate) fn read(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Some(Ok(PropertyValue::Boolean(self.event_detection_enable)));
        }
        if property == PropertyIdentifier::ALARM_VALUES {
            return Some(Ok(enumerated_list_value(&self.event_detector.alarm_values)));
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
            return Some(result);
        }
        self.event_history.read(property, array_index)
    }

    /// Take a network write of one of the writable event rows, or `None` for
    /// any other property. Acked_Transitions is refused with
    /// WRITE_ACCESS_DENIED; the rows nobody writes (Event_State,
    /// Event_Time_Stamps, Event_Message_Texts) are left to the object's
    /// metadata, which marks them read-only.
    pub(crate) fn write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property == PropertyIdentifier::ALARM_VALUES {
            return Some(
                enumerated_list(array_index, value, self.states).map(|values| {
                    self.event_detector.alarm_values = values;
                }),
            );
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            let PropertyValue::Boolean(enable) = *value else {
                return Some(Err(common::invalid_data_type_error()));
            };
            self.event_detection_enable = enable;
            if !enable {
                self.reset();
            }
            return Some(Ok(()));
        }
        if let Some(result) =
            self.event_history
                .write(property, array_index, value, self.event_detection_enable)
        {
            return Some(result);
        }
        write_generic_event_properties!(self, property, value.clone())
    }

    /// Back to the starting event state with detection turned off: NORMAL,
    /// every transition acknowledged, no countdown and an empty history
    /// (Clause 12.32.34 asks for the initial values while it stays off).
    fn reset(&mut self) {
        self.event_detector.event_state = EventState::NORMAL;
        self.event_detector.acked_transitions = EventTransitionBits::all();
        self.event_detector.pending = None;
        self.event_detector.fault_reliability = None;
        self.event_history.reset();
    }

    /// The per-write evaluation, suspended while detection is off.
    pub(crate) fn propose(
        &mut self,
        watched: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        if !self.event_detection_enable {
            return None;
        }
        let inhibited = self.event_history.options.inhibited();
        self.event_detector.propose(watched, reliability, inhibited)
    }

    /// The one-second tick, suspended while detection is off.
    pub(crate) fn tick(
        &mut self,
        watched: u32,
        reliability: Reliability,
    ) -> Option<TransitionOutcome> {
        if !self.event_detection_enable {
            return None;
        }
        let inhibited = self.event_history.options.inhibited();
        self.event_detector
            .tick_proposal(watched, reliability, inhibited)
    }

    /// Commit one transition through the shared kernel, then settle the
    /// detector's countdown and fault edge.
    pub(crate) fn commit(
        &mut self,
        commit: EventTransitionCommit,
        reliability: Reliability,
    ) -> Result<(), EventTransitionCommitError> {
        let change = commit.change.clone();
        EventTransitionState::new(
            &mut self.event_detector.event_state,
            &mut self.event_detector.acked_transitions,
            &mut self.event_history,
        )
        .commit(commit)?;
        self.event_detector.confirm_transition(&change, reliability);
        Ok(())
    }

    /// Acknowledge the latest transition to `event_state` stamped
    /// `timestamp`; with detection off there is nothing to acknowledge
    /// (NO_ALARM_CONFIGURED).
    pub(crate) fn acknowledge(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        if !self.event_detection_enable {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT,
                code: ErrorCode::NO_ALARM_CONFIGURED,
            });
        }
        self.event_history.acknowledge_correlated_detailed(
            &mut self.event_detector.acked_transitions,
            event_state,
            timestamp,
        )
    }

    /// What GetEnrollmentSummary reports for the object.
    pub(crate) fn enrollment_summary(&self) -> EnrollmentSummaryCapability {
        EnrollmentSummaryCapability {
            event_type: ChangeOfStateDetector::ALGORITHM,
            last_transition: self.event_history.last_transition(),
        }
    }

    /// Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair.
    pub(crate) fn options(&mut self) -> &mut ReportingOptions {
        &mut self.event_history.options
    }

    /// The property Event_Algorithm_Inhibit follows, if any.
    pub(crate) fn inhibit_reference(
        &self,
    ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
        self.event_history.options.inhibit_reference().cloned()
    }
}

/// The states a list on a CHANGE_OF_STATE object may hold, such as its
/// Alarm_Values: members of the watched enumeration, named or proprietary,
/// other than its NORMAL state.
///
/// NORMAL is the state in which nothing is wrong. Listed as an alarm value it
/// would put a quiet object in alarm, and an object's other state lists (an
/// Access Door's Fault_Values and Masked_Alarm_Values) are no place for it
/// either, so every list built on these checks refuses it with
/// VALUE_OUT_OF_RANGE naming the element (#1149, #1401). Clause 13.3.2 has
/// the algorithm's alarm values stand for offnormal conditions, but no
/// object clause bars NORMAL from a list outright, so the refusal is the
/// stack's own reading.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListedStates {
    /// The enumeration's NORMAL state, raw.
    pub(crate) normal: u32,
    /// Whether a raw value belongs to the enumeration, named or proprietary.
    pub(crate) in_range: fn(u32) -> bool,
}

impl ListedStates {
    /// Whether a list may hold `raw`.
    pub(crate) fn admits(self, raw: u32) -> bool {
        raw != self.normal && (self.in_range)(raw)
    }
}

/// The values a written list of states holds, such as Alarm_Values: a list of
/// Enumerated, where a value that isn't a list is its one element
/// (WriteProperty hands over a list, but a local write may pass one value
/// alone). An index is PROPERTY_IS_NOT_AN_ARRAY; more than
/// [`MAX_ALARM_VALUES`](crate::multistate::MAX_ALARM_VALUES) elements is
/// NO_SPACE_TO_WRITE_PROPERTY; an element of another datatype is
/// INVALID_DATA_TYPE and one `states` doesn't admit VALUE_OUT_OF_RANGE, each
/// naming the element.
pub(crate) fn enumerated_list(
    array_index: Option<u32>,
    value: &PropertyValue,
    states: ListedStates,
) -> Result<Vec<u32>, Error> {
    if array_index.is_some() {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        });
    }
    let items = match value {
        PropertyValue::List(items) => items.as_slice(),
        single => std::slice::from_ref(single),
    };
    let cap = crate::multistate::MAX_ALARM_VALUES;
    if items.len() > cap {
        let full = Error::Protocol {
            class: ErrorClass::RESOURCES,
            code: ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        };
        return Err(common::at_list_element(full, cap));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            match *item {
                PropertyValue::Enumerated(raw) if states.admits(raw) => Ok(raw),
                PropertyValue::Enumerated(_) => Err(common::value_out_of_range_error()),
                _ => Err(common::invalid_data_type_error()),
            }
            .map_err(|error| common::at_list_element(error, index))
        })
        .collect()
}

/// [`enumerated_list`] for values an application sets: the same checks, as
/// a network write of the same list would meet them.
pub(crate) fn checked_raw_list(values: Vec<u32>, states: ListedStates) -> Result<Vec<u32>, Error> {
    let values = values.into_iter().map(PropertyValue::Enumerated).collect();
    enumerated_list(None, &PropertyValue::List(values), states)
}

/// [`enumerated_list`] read back: the raw values as a list of Enumerated.
pub(crate) fn enumerated_list_value(values: &[u32]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .copied()
            .map(PropertyValue::Enumerated)
            .collect(),
    )
}

/// Implement the `BACnetObject` intrinsic-reporting hooks of an object that
/// holds a [`ChangeOfStateReporting`] in `$reporting`, with `$watched` and
/// `$reliability` paths to `&self` methods returning the raw watched value
/// and the Reliability served.
macro_rules! impl_change_of_state_reporting {
    ($reporting:ident, $watched:path, $reliability:path) => {
        fn enrollment_summary_capability_internal(
            &self,
        ) -> Option<$crate::event::EnrollmentSummaryCapability> {
            Some(self.$reporting.enrollment_summary())
        }

        fn evaluate_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            let (watched, reliability) = ($watched(self), $reliability(self));
            self.$reporting.propose(watched, reliability)
        }

        fn tick_intrinsic_reporting(&mut self) -> Option<$crate::event::TransitionOutcome> {
            let (watched, reliability) = ($watched(self), $reliability(self));
            self.$reporting.tick(watched, reliability)
        }

        fn commit_event_transition_internal(
            &mut self,
            commit: $crate::event::EventTransitionCommit,
        ) -> Result<(), $crate::event::EventTransitionCommitError> {
            let reliability = $reliability(self);
            self.$reporting.commit(commit, reliability)
        }

        fn acknowledge_alarm_correlated_internal(
            &mut self,
            event_state: bacnet_types::enums::EventState,
            timestamp: &bacnet_types::primitives::BACnetTimeStamp,
        ) -> Result<(), bacnet_types::error::Error> {
            self.$reporting
                .acknowledge(event_state, timestamp)
                .map(|_| ())
        }

        fn acknowledge_alarm_correlated_detailed_internal(
            &mut self,
            event_state: bacnet_types::enums::EventState,
            timestamp: &bacnet_types::primitives::BACnetTimeStamp,
        ) -> Result<Option<$crate::event::EventStateChange>, bacnet_types::error::Error> {
            self.$reporting.acknowledge(event_state, timestamp)
        }

        fn event_algorithm_inhibit_reference_internal(
            &self,
        ) -> Option<bacnet_types::constructed::BACnetObjectPropertyReference> {
            self.$reporting.inhibit_reference()
        }

        fn follow_event_algorithm_inhibit_internal(&mut self, inhibit: bool) -> bool {
            self.$reporting.options().follow(inhibit)
        }
    };
}

pub(crate) use impl_change_of_state_reporting;
