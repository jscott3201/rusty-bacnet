//! Multi-State Input (type 13), Multi-State Output (type 14), and
//! Multi-State Value (type 19) objects per ASHRAE 135-2020 Clauses 12.18,
//! 12.19, and 12.20.
//!
//! Each holds the state every I/O object shares (`IoCore`), its event
//! detector (CHANGE_OF_STATE on the Input and the Value, COMMAND_FAILURE on
//! the Output), the states the three share (`MultiStateProperties`:
//! Number_Of_States, State_Text and the Reliability they work out from
//! them) and its own Present_Value; the Input and the Value add
//! Alarm_Values, the Output a Feedback_Value, and the Output and the Value
//! a priority array.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
#[cfg(test)]
use bacnet_types::primitives::BACnetTimeStamp;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::event::ChangeOfStateDetector;
use crate::io_core::IoCore;
use crate::traits::{BACnetObject, ReliabilityEvaluation};

mod metadata;

/// Resource cap consistent with bounded server tables such as
/// `MAX_COV_SUBSCRIPTIONS`.
pub(crate) const MAX_ALARM_VALUES: usize = 1024;

/// The refusal of an Alarm_Values list `len` entries long: past
/// [`MAX_ALARM_VALUES`] it is RESOURCES / NO_SPACE_TO_WRITE_PROPERTY, naming
/// the first entry that doesn't fit (#1048).
fn alarm_values_fit(len: usize) -> Result<(), Error> {
    if len > MAX_ALARM_VALUES {
        return Err(common::at_list_element(
            Error::protocol(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY),
            MAX_ALARM_VALUES,
        ));
    }
    Ok(())
}

/// An Alarm_Values entry names one of the object's `number_of_states`
/// states; one past them could never match Present_Value, so it is
/// VALUE_OUT_OF_RANGE rather than held for Reliability to report as
/// CONFIGURATION_ERROR (#1429).
fn alarm_state(state: u64, number_of_states: u32) -> Result<u32, Error> {
    if (1..=u64::from(number_of_states)).contains(&state) {
        Ok(state as u32)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Check a list `set_alarm_values` is given as a write of it is checked
/// (#2415): at most [`MAX_ALARM_VALUES`] entries, each one of the object's
/// `number_of_states` states. A refusal names its entry.
fn check_alarm_values(values: &[u32], number_of_states: u32) -> Result<(), Error> {
    alarm_values_fit(values.len())?;
    for (index, state) in values.iter().enumerate() {
        alarm_state(u64::from(*state), number_of_states)
            .map_err(|error| common::at_list_element(error, index))?;
    }
    Ok(())
}

/// Decode an Alarm_Values write on a Multi-state Input or Value with
/// `number_of_states` states: a list of Unsigned states, checked as
/// [`check_alarm_values`] checks a local one, each refusal naming its
/// element.
fn decode_alarm_values_write(
    array_index: Option<u32>,
    value: PropertyValue,
    number_of_states: u32,
) -> Result<Vec<u32>, Error> {
    if array_index.is_some() {
        return Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }
    let PropertyValue::List(values) = value else {
        return Err(common::invalid_data_type_error());
    };
    alarm_values_fit(values.len())?;
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            match value {
                PropertyValue::Unsigned(state) => alarm_state(state, number_of_states),
                _ => Err(common::invalid_data_type_error()),
            }
            .map_err(|error| common::at_list_element(error, index))
        })
        .collect()
}

/// Take a write of Alarm_Values on a Multi-state Input or Value with
/// `number_of_states` states ([`decode_alarm_values_write`]); a refused
/// list leaves the one held. A staged write is checked by the same code
/// (`write_config::reporting`).
pub(crate) fn write_alarm_values(
    detector: &mut ChangeOfStateDetector,
    array_index: Option<u32>,
    value: PropertyValue,
    number_of_states: u32,
) -> Result<(), Error> {
    detector.alarm_values = decode_alarm_values_write(array_index, value, number_of_states)?;
    Ok(())
}

/// Alarm_Values of a Multi-state Input or Value: the detector's list.
fn read_alarm_values(detector: &ChangeOfStateDetector) -> PropertyValue {
    PropertyValue::List(
        detector
            .alarm_values
            .iter()
            .map(|v| PropertyValue::Unsigned(*v as u64))
            .collect(),
    )
}

/// Reject a `number_of_states` of zero.
///
/// Every multi-state object starts `PRESENT_VALUE` (and, for Output/Value, the
/// relinquish default) at 1, and accepts writes only in `1..=number_of_states`.
/// With zero states that initial value is out of range with no attainable value,
/// so the constructors centralize this guard here.
fn require_nonzero_states(number_of_states: u32) -> Result<(), Error> {
    if number_of_states == 0 {
        return Err(Error::OutOfRange(
            "number_of_states must be at least 1".into(),
        ));
    }
    Ok(())
}

/// The label a new object gives `state`: `State <state>`, written without
/// `core::fmt`, which a no_std image building a multi-state object would
/// otherwise link (#2445).
fn default_state_label(state: u32) -> String {
    common::numbered_name("State ", state)
}

/// The State_Text a new object with `number_of_states` states starts with.
fn default_state_text(number_of_states: u32) -> Vec<String> {
    (1..=number_of_states).map(default_state_label).collect()
}

/// Resize an object's State_Text with the repository's local count-change policy.
///
/// Validation precedes mutation. Shrink truncates the tail, while growth keeps
/// the retained prefix and appends the same `State {n}` labels as construction.
fn resize_state_text(
    current_number_of_states: &mut u32,
    state_text: &mut Vec<String>,
    number_of_states: u32,
) -> Result<(), Error> {
    require_nonzero_states(number_of_states)?;
    let new_len = number_of_states as usize;
    if new_len < state_text.len() {
        state_text.truncate(new_len);
    } else {
        // The labels held number fewer than `number_of_states`, a u32.
        let first = state_text.len() as u32 + 1;
        state_text.extend((first..=number_of_states).map(default_state_label));
    }
    *current_number_of_states = number_of_states;
    Ok(())
}

/// What a multi-state object takes only from a CreateObject initial value
/// (#1429): Number_Of_States. WriteProperty refuses it; a whole State_Text
/// write changes it instead (#1443).
const CREATION_ONLY: &[PropertyIdentifier] = &[PropertyIdentifier::NUMBER_OF_STATES];

/// The most states a client may give a multi-state object, by a CreateObject
/// initial value or by State_Text written whole. State_Text keeps a label
/// for every state, so the count a client picks is bounded like the other
/// tables a client sizes.
pub const MAX_NUMBER_OF_STATES: u32 = 1024;

/// Check a state count a client asks for, given the `current` one and
/// `held`, the values the object keeps that name a state (Present_Value,
/// Relinquish_Default, the commands in Priority_Array, Alarm_Values).
///
/// The count runs from 1 to [`MAX_NUMBER_OF_STATES`]. Clause 12.20.10 and
/// its Multi-state Input and Output counterparts leave it to the device
/// whether a shrink repairs the values that name a state past it, or keeps
/// them and reports a fault; this one refuses a count that would strand a
/// held state that names one of the `current` states. Both refusals are
/// VALUE_OUT_OF_RANGE, the code WriteProperty gives a value the property
/// can't take (Clause 15.9.1.3.1). A held state already past the current
/// count doesn't block a new count. A local shrink can leave one, and so
/// can an Alarm_Values list restored from storage (#2331) on an object
/// whose storage holds no written State_Text.
fn check_state_count(
    current: u32,
    count: u64,
    held: impl IntoIterator<Item = u32>,
) -> Result<u32, Error> {
    let count = u32::try_from(count)
        .ok()
        .filter(|count| (1..=MAX_NUMBER_OF_STATES).contains(count))
        .ok_or_else(common::value_out_of_range_error)?;
    let stranded = |state: u32| (1..=current).contains(&state) && state > count;
    if held.into_iter().any(stranded) {
        return Err(common::value_out_of_range_error());
    }
    Ok(count)
}

/// Apply a CreateObject initial value to Number_Of_States (#1429), checked
/// before anything changes: [`check_state_count`], then State_Text resized
/// as [`resize_state_text`] does (Clause 12.20.10 ties the two sizes
/// together). Any other property is WRITE_ACCESS_DENIED, as the trait
/// default answers.
fn initialize_states(
    number_of_states: &mut u32,
    state_text: &mut Vec<String>,
    held: impl IntoIterator<Item = u32>,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    if property != PropertyIdentifier::NUMBER_OF_STATES {
        return Err(common::write_access_denied_error());
    }
    let PropertyValue::Unsigned(count) = value else {
        return Err(common::invalid_data_type_error());
    };
    let count = check_state_count(*number_of_states, count, held)?;
    resize_state_text(number_of_states, state_text, count)
}

/// Take State_Text written whole, by WriteProperty, WritePropertyMultiple or
/// a CreateObject initial value (#1443): one CharacterString per state,
/// where a single label arrives as that CharacterString, the way a
/// one-element array value decodes. The labels replace the old ones, and
/// their number becomes Number_Of_States: Clause 12.20.11 and its
/// Multi-state Input and Output counterparts tie the two sizes together in
/// both directions. A new count is checked as [`check_state_count`] says;
/// nothing changes on a refusal.
fn write_whole_state_text(
    number_of_states: &mut u32,
    state_text: &mut Vec<String>,
    held: impl IntoIterator<Item = u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    let labels = match value {
        PropertyValue::CharacterString(label) => vec![label],
        PropertyValue::List(values) => values
            .into_iter()
            .map(|value| match value {
                PropertyValue::CharacterString(label) => Ok(label),
                _ => Err(common::invalid_data_type_error()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(common::invalid_data_type_error()),
    };
    let count = whole_state_text_count(*number_of_states, labels.len(), held)?;
    *state_text = labels;
    *number_of_states = count;
    Ok(())
}

/// The Number_Of_States `len` labels given as the whole State_Text leave an
/// object with `current` states, by a write ([`write_whole_state_text`]) or
/// a `set_state_texts` (#2445): `current` when the count stays, otherwise
/// a new count checked as [`check_state_count`] says.
fn whole_state_text_count(
    current: u32,
    len: usize,
    held: impl IntoIterator<Item = u32>,
) -> Result<u32, Error> {
    if len == current as usize {
        Ok(current)
    } else {
        check_state_count(current, len as u64, held)
    }
}

/// The check a `set_state_texts` makes of `texts` when a State_Text a write
/// saved wins over it (#2414): only a count no object takes, none or past
/// [`MAX_NUMBER_OF_STATES`], is refused, since the saved texts keep their
/// own count.
#[cfg(feature = "event-reporting")]
fn kept_state_texts_check(texts: &[String]) -> Result<(), Error> {
    check_state_count(0, texts.len() as u64, core::iter::empty()).map(|_| ())
}

/// The refusal `set_state_texts` gives labels on an object whose `store`
/// keeps its reporting settings: storage keeps only what
/// [`state_text_fits`] takes, so a label past
/// [`MAX_KEPT_STATE_TEXT_OCTETS`] is RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY, as a State_Text write of it is. Taken, it
/// would leave every later State_Text write refused (#2445).
#[cfg(feature = "event-reporting")]
fn storable_state_texts(
    store: &crate::write_config::reporting::ReportingStore,
    texts: &[String],
) -> Result<(), Error> {
    if store.attached() && !state_text_fits(texts) {
        return Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ));
    }
    Ok(())
}

/// Write one State_Text element, the whole array
/// ([`write_whole_state_text`]), or its size at index 0; an index past the
/// states is INVALID_ARRAY_INDEX.
///
/// A whole write can change the array's size, so Clause 12.1.5.1 has index
/// 0 take a write too (#1443). The Unsigned written there is a new state
/// count, checked as [`check_state_count`] says, and it resizes State_Text
/// and Number_Of_States together as [`resize_state_text`] does: a shrink
/// drops the last labels, and a grow, whose new labels the clause leaves to
/// the device, appends the `State {n}` labels a new object starts with.
fn write_state_text(
    number_of_states: &mut u32,
    state_text: &mut Vec<String>,
    held: impl IntoIterator<Item = u32>,
    array_index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    match array_index {
        None => write_whole_state_text(number_of_states, state_text, held, value),
        Some(0) => {
            let PropertyValue::Unsigned(count) = value else {
                return Err(common::invalid_data_type_error());
            };
            let count = check_state_count(*number_of_states, count, held)?;
            resize_state_text(number_of_states, state_text, count)
        }
        Some(index) if (index as usize) <= state_text.len() => {
            let PropertyValue::CharacterString(label) = value else {
                return Err(common::invalid_data_type_error());
            };
            state_text[(index - 1) as usize] = label;
            Ok(())
        }
        Some(_) => Err(common::invalid_array_index_error()),
    }
}

/// The most octets of UTF-8 a label of State_Text may hold on an object
/// that keeps it in storage (#2414). A write takes any label, but storage
/// keeps the whole array, up to [`MAX_NUMBER_OF_STATES`] labels, in a file
/// of at most 1 MiB beside the object's other kept rows: 512 octets a
/// label, which leaves room for them, is far past any label a head end
/// gives a state.
pub const MAX_KEPT_STATE_TEXT_OCTETS: usize = 512;

/// Whether storage keeps `texts` as a State_Text a write leaves: 1 to
/// [`MAX_NUMBER_OF_STATES`] labels, each at most
/// [`MAX_KEPT_STATE_TEXT_OCTETS`] octets. A write to an object with storage
/// that would leave other texts is refused, and a file holding them is
/// damage: this one rule is both checks, so what a write saves reopens.
pub(crate) fn state_text_fits(texts: &[String]) -> bool {
    (1..=MAX_NUMBER_OF_STATES as usize).contains(&texts.len())
        && texts
            .iter()
            .all(|text| text.len() <= MAX_KEPT_STATE_TEXT_OCTETS)
}

/// What a write of State_Text leaves the kept `rows` of an object with
/// storage (#2414), or the object's refusal of it: [`write_state_text`] on
/// the texts `rows` hold (`served`, the object's own, until a step of the
/// request changes them), with `held` the states the object keeps besides
/// Alarm_Values and the Alarm_Values `rows` hold. A write the object takes
/// that would leave texts [`state_text_fits`] refuses is RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY, before anything is saved, as a Description
/// past its cap is (#2364).
#[cfg(feature = "event-reporting")]
pub(crate) fn state_text_after(
    served: &[String],
    held: impl IntoIterator<Item = u32>,
    rows: &mut crate::write_config::AlgorithmWriteConfig,
    write: &crate::durable::PendingWrite,
) -> Result<(), Error> {
    // Taken, not copied (#2526): on a refusal `rows`, a working copy, are
    // dropped with it.
    let mut texts = rows.state_text.take().unwrap_or_else(|| served.to_vec());
    let mut count = u32::try_from(texts.len()).unwrap_or(u32::MAX);
    let alarm_values = rows.alarm_values.iter().flatten().copied();
    let held = held.into_iter().chain(alarm_values);
    write_state_text(
        &mut count,
        &mut texts,
        held,
        write.array_index,
        write.value.clone(),
    )?;
    if !state_text_fits(&texts) {
        return Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ));
    }
    rows.state_text = Some(texts);
    Ok(())
}

/// Take a saved State_Text (#2414) as a write of it would: the labels, and
/// as many states. Nothing that names a state is repaired, as a local
/// `set_number_of_states` repairs nothing; the caller works Reliability
/// out again. `saved` has passed [`state_text_fits`].
#[cfg(feature = "event-reporting")]
pub(crate) fn restore_state_text(
    number_of_states: &mut u32,
    state_text: &mut Vec<String>,
    saved: &[String],
) {
    saved.clone_into(state_text);
    *number_of_states = saved.len() as u32;
}

/// A fault the object's own evaluation reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedMultiStateFault {
    ConfigurationError,
    MultiStateOutOfRange,
}

impl OwnedMultiStateFault {
    fn reliability(self) -> Reliability {
        match self {
            Self::ConfigurationError => Reliability::CONFIGURATION_ERROR,
            Self::MultiStateOutOfRange => Reliability::MULTI_STATE_OUT_OF_RANGE,
        }
    }
}

/// Number_Of_States and State_Text, one label per state, and the fault the
/// object's own Reliability evaluation over them set: what the three share
/// beyond their [`IoCore`].
struct MultiStateProperties {
    number_of_states: u32,
    state_text: Vec<String>,
    /// The fault Reliability shows because the evaluation set it, until
    /// `set_reliability` takes Reliability over (first-stage ownership).
    owned_fault: Option<OwnedMultiStateFault>,
}

impl MultiStateProperties {
    /// `number_of_states` states labelled `State 1` on; zero is refused.
    fn new(number_of_states: u32) -> Result<Self, Error> {
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            number_of_states,
            state_text: default_state_text(number_of_states),
            owned_fault: None,
        })
    }

    /// Work `reliability` out: CONFIGURATION_ERROR while
    /// `configuration_invalid`, else MULTI_STATE_OUT_OF_RANGE for a
    /// `present_value` outside the states. A fault is set only over
    /// NO_FAULT_DETECTED, and cleared only when the evaluation set it.
    fn evaluate(
        &mut self,
        configuration_invalid: bool,
        present_value: u32,
        reliability: &mut Reliability,
    ) -> ReliabilityEvaluation {
        let observed_fault = if configuration_invalid {
            Some(OwnedMultiStateFault::ConfigurationError)
        } else if !(1..=self.number_of_states).contains(&present_value) {
            Some(OwnedMultiStateFault::MultiStateOutOfRange)
        } else {
            None
        };

        let (new_reliability, new_owner) = if self.owned_fault.is_some() {
            (
                observed_fault
                    .map(OwnedMultiStateFault::reliability)
                    .unwrap_or(Reliability::NO_FAULT_DETECTED),
                observed_fault,
            )
        } else if *reliability == Reliability::NO_FAULT_DETECTED {
            let Some(fault) = observed_fault else {
                return ReliabilityEvaluation::Unchanged;
            };
            (fault.reliability(), Some(fault))
        } else {
            return ReliabilityEvaluation::Unchanged;
        };

        // Ownership can change while Reliability is already zero, notably when
        // inhibit normalized an owned fault and its source recovered before the
        // gate reopened. Keep the private state current even without a public
        // Reliability transition.
        self.owned_fault = new_owner;
        if new_reliability == *reliability {
            return ReliabilityEvaluation::Unchanged;
        }
        let old_reliability = *reliability;
        *reliability = new_reliability;
        ReliabilityEvaluation::Changed {
            old_reliability,
            new_reliability,
        }
    }

    /// Read Number_Of_States or State_Text, whole, by element or its size
    /// at index 0; `None` for any other property.
    fn read(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let texts = &self.state_text;
        Some(match property {
            PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            PropertyIdentifier::STATE_TEXT => match array_index {
                None => Ok(PropertyValue::List(
                    texts
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(texts.len() as u64)),
                Some(idx) if idx >= 1 && (idx as usize) <= texts.len() => Ok(
                    PropertyValue::CharacterString(texts[(idx - 1) as usize].clone()),
                ),
                _ => Err(common::invalid_array_index_error()),
            },
            _ => return None,
        })
    }
}

/// Give `$object`, a multi-state object holding an [`IoCore`] in `core`
/// and [`MultiStateProperties`] in `multistate`, what the three share
/// around them: the state count and State_Text setters and writes,
/// Number_Of_States as a CreateObject initial value, the follow-up of a
/// write the core took, and the Reliability it works out. The object names
/// the values that name a state (`held_states`, given `alarm_values`, its
/// own Alarm_Values) and says whether one it holds is outside its states
/// (`configuration_invalid`).
macro_rules! impl_multistate {
    ($object:ty) => {
        impl $object {
            /// Change the locally configured number of states.
            ///
            /// The BACnet `Number_Of_States` property remains read-only. A
            /// successful local change retains the State_Text prefix,
            /// truncates its tail on shrink, appends constructor-style
            /// labels on growth, and immediately re-evaluates Reliability;
            /// the values that name a state (Present_Value, and the
            /// commands, Relinquish_Default, Feedback_Value and Alarm_Values
            /// of a type that has them) are not repaired. On an object
            /// keeping its reporting settings, a State_Text a write saved
            /// wins, and with it its count: zero is still refused, and the
            /// saved texts stay, until `clear_written_reporting` (#2414).
            pub fn set_number_of_states(
                &mut self,
                number_of_states: u32,
            ) -> Result<(), bacnet_types::error::Error> {
                $crate::multistate::require_nonzero_states(number_of_states)?;
                #[cfg(feature = "event-reporting")]
                if self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::STATE_TEXT) {
                    return Ok(());
                }
                $crate::multistate::resize_state_text(
                    &mut self.multistate.number_of_states,
                    &mut self.multistate.state_text,
                    number_of_states,
                )?;
                let _ = self.recompute_reliability();
                Ok(())
            }

            /// Set State_Text, one label per state, state 1 first (#2445).
            ///
            /// The number of labels becomes Number_Of_States, checked as a
            /// write of the whole array is: 1 to
            /// [`MAX_NUMBER_OF_STATES`](crate::multistate::MAX_NUMBER_OF_STATES)
            /// labels, and no fewer than a state the object holds names
            /// (Present_Value, and the commands, Relinquish_Default and
            /// Alarm_Values of a type that has them), else
            /// VALUE_OUT_OF_RANGE with nothing changed. Reliability is then
            /// worked out again. On an object keeping its reporting
            /// settings, labels its storage can't keep (one past
            /// [`MAX_KEPT_STATE_TEXT_OCTETS`](crate::multistate::MAX_KEPT_STATE_TEXT_OCTETS)
            /// octets) are RESOURCES / NO_SPACE_TO_WRITE_PROPERTY, as a
            /// write of them is, and a State_Text a write saved wins: this
            /// then refuses only a count no object takes and leaves the
            /// saved texts, until `clear_written_reporting` (#2414).
            pub fn set_state_texts(
                &mut self,
                texts: alloc::vec::Vec<alloc::string::String>,
            ) -> Result<(), bacnet_types::error::Error> {
                #[cfg(feature = "event-reporting")]
                if self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::STATE_TEXT) {
                    return $crate::multistate::kept_state_texts_check(&texts);
                }
                let count = $crate::multistate::whole_state_text_count(
                    self.multistate.number_of_states,
                    texts.len(),
                    self.held_states(self.alarm_values()),
                )?;
                #[cfg(feature = "event-reporting")]
                $crate::multistate::storable_state_texts(&self.core.reporting_store, &texts)?;
                self.multistate.state_text = texts;
                self.multistate.number_of_states = count;
                let _ = self.recompute_reliability();
                Ok(())
            }

            /// Take a State_Text write: whole, by element or by its size
            /// ([`write_state_text`](crate::multistate::write_state_text)),
            /// Reliability then worked out again.
            fn write_state_text(
                &mut self,
                array_index: Option<u32>,
                value: bacnet_types::primitives::PropertyValue,
            ) -> Result<(), bacnet_types::error::Error> {
                // Written whole, State_Text sets Number_Of_States too (#1443).
                let held: alloc::vec::Vec<u32> = self.held_states(self.alarm_values()).collect();
                $crate::multistate::write_state_text(
                    &mut self.multistate.number_of_states,
                    &mut self.multistate.state_text,
                    held,
                    array_index,
                    value,
                )?;
                let _ = self.recompute_reliability();
                Ok(())
            }

            /// Finish a write the core took: Reliability_Evaluation_Inhibit
            /// or a Boolean Out_Of_Service may hand Reliability back, so it
            /// is worked out again; a relinquish changes nothing.
            fn core_written(
                &mut self,
                written: Result<$crate::io_core::CoreWrite, bacnet_types::error::Error>,
            ) -> Result<(), bacnet_types::error::Error> {
                if matches!(
                    written?,
                    $crate::io_core::CoreWrite::Inhibit | $crate::io_core::CoreWrite::OutOfService
                ) {
                    let _ = self.recompute_reliability();
                }
                Ok(())
            }

            /// Work Reliability out from the states, while the object owns
            /// it.
            fn recompute_reliability(&mut self) -> $crate::traits::ReliabilityEvaluation {
                if !self.core.owns_reliability() {
                    return $crate::traits::ReliabilityEvaluation::Unchanged;
                }
                let configuration_invalid = self.configuration_invalid();
                self.multistate.evaluate(
                    configuration_invalid,
                    self.present_value,
                    &mut self.core.reliability,
                )
            }
        }

        impl $crate::creation::Creatable for $object {
            const CREATION_ONLY_PROPERTIES: &'static [bacnet_types::enums::PropertyIdentifier] =
                $crate::multistate::CREATION_ONLY;

            fn initialize_property(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                value: bacnet_types::primitives::PropertyValue,
            ) -> Result<(), bacnet_types::error::Error> {
                let held: alloc::vec::Vec<u32> = self.held_states(self.alarm_values()).collect();
                $crate::multistate::initialize_states(
                    &mut self.multistate.number_of_states,
                    &mut self.multistate.state_text,
                    held,
                    property,
                    value,
                )?;
                let _ = self.recompute_reliability();
                Ok(())
            }
        }

        impl $crate::traits::ReliabilityEval for $object {
            fn set_reliability(
                &mut self,
                reliability: bacnet_types::enums::Reliability,
            ) -> Result<(), bacnet_types::error::Error> {
                self.core.set_reliability(reliability)?;
                self.multistate.owned_fault = None;
                Ok(())
            }

            fn evaluate_reliability(
                &mut self,
            ) -> Result<$crate::traits::ReliabilityEvaluation, bacnet_types::error::Error> {
                Ok(self.recompute_reliability())
            }

            fn reliability_evaluation_inhibited(&self) -> bool {
                self.core.reliability_inhibit.enabled()
            }
        }
    };
}
use impl_multistate;

mod input;
mod output;
mod value;
pub use input::*;
pub use output::*;
pub use value::*;
#[cfg(test)]
mod tests;
