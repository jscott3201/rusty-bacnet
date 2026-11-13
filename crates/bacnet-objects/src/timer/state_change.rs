//! State_Change_Values, List_Of_Object_Property_References and
//! Priority_For_Writing (Clauses 12.57.20 to 12.57.22, #2206), and the writes
//! each transition makes with them.
//!
//! The three come as a set (Table 12-75 footnote 5): a Timer has all of them
//! once [`TimerObject::set_state_change`] has run, and none before. A
//! transition looks up its element of State_Change_Values, whose index is the
//! transition's BACnetTimerTransition value, and queues one write per
//! reference with that value at Priority_For_Writing. A `no-value` element
//! writes nothing.
//!
//! The writes are the bundled server's, made exactly as a Command's action
//! list is: the object leaves a [`CommandRun`] under the guard that made the
//! transition, the server takes it there and makes the writes once that
//! guard is released, local targets through the local write path and other
//! devices' through a confirmed WriteProperty, with the Timer as the
//! initiating object. Each write is attempted whatever the others did.
//!
//! # Overlapping transitions: the newer one supersedes
//!
//! Every transition starts a fresh run generation and replaces any run not
//! yet taken, a `no-value` one included, which leaves none. Queueing instead
//! would spend writes on a state the Timer has already left, and a burst of
//! transitions against a slow remote target would queue without bound. The
//! rule is the same whether the transitions share a guard or not: a write
//! that first applies a due expiry and then restarts the timer makes only the
//! restart's writes.
//!
//! Unlike a Command or Channel, which refuse a new Present_Value write while
//! a run is in progress, a Timer can't refuse a transition, so two of its runs
//! can be live at once. The older one goes stale and stops before its next
//! write, but the generation check comes before the write without a guard
//! held across both, so a write it had already started, local or remote, can
//! land after the newer run's write to the same target and leave the older
//! value there. Writing the same target from two transitions in quick
//! succession is the case to avoid.
//!
//! A run is never left untaken. A write that refuses after applying a due
//! expiry leaves that expiry's run with nothing to take it; the server's
//! operation task, woken by the same deadline, takes it on its next pass,
//! since `Timed::advance_monotonic_time` reports a Timer with a run waiting.
//!
//! # Reliability
//!
//! In service Reliability reads CONFIGURATION_ERROR while the non-NULL values
//! of State_Change_Values differ in datatype (Clause 12.57.20). That is the
//! only half of the clause's configuration rule checked up front: whether each
//! referenced property takes the datatype shows when a run writes it, and a
//! refusal then sets CONFIGURATION_ERROR as the run's verdict. Constructed
//! values all count as one datatype for the comparison
//! ([`BACnetTimerStateChangeValue::datatype`]). Otherwise it
//! reports how the last run ended: NO_FAULT_DETECTED, or the kind of its
//! first failed write ([`WriteFailure::reliability`]), as a Channel does.
//! Out of service it is the client's to simulate.

use crate::prelude::*;
use alloc::sync::Arc;
use bacnet_encoding::constructed::{
    decode_timer_state_change_value, encode_lighting_command, encode_timer_state_change_value,
};
use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetDeviceObjectPropertyReference, BACnetTimerStateChangeValue,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, PropertyIdentifier, Reliability, TimerTransition,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::TimerObject;
use crate::command::{CommandRun, RunPlan, WriteFailure, next_generation};
use crate::common;
use crate::device_reference;
use crate::durable::PendingWrite;
use crate::traits::CommandRunner;

/// The most references List_Of_Object_Property_References holds; a write or
/// setter past it is refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY.
pub const MAX_TIMER_REFERENCES: usize = 1024;

/// One element per transition, IDLE_TO_RUNNING (1) to EXPIRED_TO_RUNNING
/// (7).
pub type StateChangeValues = [BACnetTimerStateChangeValue; 7];

/// The three properties, as a write leaves them.
#[derive(Clone)]
pub(crate) struct TimerSet {
    pub(super) values: StateChangeValues,
    pub(super) references: Vec<BACnetDeviceObjectPropertyReference>,
    pub(super) priority: u8,
    /// A write set State_Change_Values or the references, now or before a
    /// restart: with storage, it keeps the set (`saving`).
    pub(super) written: bool,
}

impl TimerSet {
    /// Whether the set already holds these, as the application gave them:
    /// the values bit for bit, as their encoding holds them (`==` on a REAL
    /// fails for a NaN and passes for the two zeros). A set a write made
    /// isn't the application's, so it never matches (#2552).
    fn is(
        &self,
        values: &StateChangeValues,
        references: &[BACnetDeviceObjectPropertyReference],
        priority: u8,
    ) -> bool {
        let encoded = |values: &StateChangeValues| {
            let mut octets = BytesMut::new();
            // `check_set` has seen every value encode.
            values
                .iter()
                .all(|value| encode_timer_state_change_value(&mut octets, value).is_ok())
                .then_some(octets)
        };
        !self.written
            && self.priority == priority
            && self.references == references
            && encoded(&self.values).is_some_and(|held| encoded(values) == Some(held))
    }
}

/// The run the three properties drive. A Timer without them never starts
/// one.
pub(super) struct StateChangeRun {
    /// How the last run ended, as Reliability.
    verdict: Reliability,
    /// A fresh `next_generation` at every transition.
    generation: u64,
    /// The number of writes in the run of `generation`, while it runs.
    running: Option<usize>,
    pending: Option<CommandRun>,
}

impl TimerObject {
    /// Give the Timer State_Change_Values, List_Of_Object_Property_References
    /// and Priority_For_Writing, which it serves together from then on.
    ///
    /// `values[0]` is written on IDLE_TO_RUNNING, through `values[6]` on
    /// EXPIRED_TO_RUNNING, to every reference, at `priority`. A reference may
    /// name another Device, which the server writes over the network.
    /// Refused, changing nothing, with PROPERTY / VALUE_OUT_OF_RANGE for a
    /// priority outside 1..=16, a value with no encoding or a reference whose
    /// device identifier isn't a Device, and with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY past [`MAX_TIMER_REFERENCES`] references.
    ///
    /// On a Timer [`with_persistence`](Self::with_persistence) whose storage
    /// holds a set a write made ([`state_change_saved`](Self::state_change_saved)),
    /// the saved set wins, so an application that sets its configuration at
    /// each start keeps a peer's write: the arguments are checked but not
    /// taken, and [`clear_written_config`](Self::clear_written_config)
    /// lets them apply again.
    pub fn set_state_change(
        &mut self,
        values: StateChangeValues,
        references: Vec<BACnetDeviceObjectPropertyReference>,
        priority: u8,
    ) -> Result<(), Error> {
        check_set(&values, &references, priority)?;
        if self.state_change_saved() {
            log_debug!(timer = %self.oid, "Saved State_Change_Values kept over a configured set");
            return Ok(());
        }
        // The run restarts either way. It isn't saved and no staged step
        // is worked out from it, so only a different set is a counted change.
        let unchanged = self
            .config
            .set
            .as_ref()
            .is_some_and(|set| set.is(&values, &references, priority));
        self.config.set = Some(TimerSet {
            values,
            references,
            priority,
            written: false,
        });
        self.run = StateChangeRun::new();
        if !unchanged {
            self.config_changed();
        }
        Ok(())
    }

    /// Reliability as served: see the module documentation.
    pub(super) fn served_reliability(&self) -> Reliability {
        match &self.config.set {
            Some(set) if !self.out_of_service => {
                if mixed_datatypes(&set.values) {
                    Reliability::CONFIGURATION_ERROR
                } else {
                    self.run.verdict
                }
            }
            _ => self.reliability,
        }
    }

    /// Queue the writes `transition` makes, superseding any run still going.
    pub(super) fn plan_writes(&mut self, transition: TimerTransition) {
        let oid = self.oid;
        let Some(set) = &self.config.set else {
            return;
        };
        let change = &mut self.run;
        let mut commands = Vec::new();
        let index = (transition.to_raw() as usize).wrapping_sub(1);
        if let Some(value) = set.values.get(index).and_then(write_value) {
            commands.extend(set.references.iter().map(|reference| BACnetActionCommand {
                device_identifier: reference.device_identifier,
                object_identifier: reference.object_identifier,
                property_identifier: PropertyIdentifier::from_raw(reference.property_identifier),
                property_array_index: reference.property_array_index,
                property_value: value.clone(),
                priority: Some(set.priority),
                post_delay: None,
                quit_on_failure: false,
                write_successful: false,
            }));
        }
        change.generation = next_generation();
        change.running = (!commands.is_empty()).then_some(commands.len());
        change.pending = change.running.map(|_| CommandRun {
            source: oid,
            generation: change.generation,
            plan: RunPlan::Actions(commands),
            chain: Arc::from([]),
        });
    }

    /// Whether a transition's writes wait to be taken.
    pub(super) fn has_pending_writes(&self) -> bool {
        self.run.pending.is_some()
    }

    /// Serve one of the three; `None` for any other property, or before
    /// [`Self::set_state_change`].
    pub(super) fn read_state_change(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let set = self.config.set.as_ref()?;
        Some(match property {
            PropertyIdentifier::STATE_CHANGE_VALUES => set
                .values
                .iter()
                .map(read_value)
                .collect::<Result<Vec<_>, _>>()
                .and_then(|values| common::read_array(values, array_index)),
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
                Ok(device_reference::reference_list(&set.references))
            }
            PropertyIdentifier::PRIORITY_FOR_WRITING => {
                Ok(PropertyValue::Unsigned(set.priority.into()))
            }
            _ => return None,
        })
    }

    /// Take a write of State_Change_Values or List_Of_Object_Property_References;
    /// `None` for any other property, or before [`Self::set_state_change`].
    /// With storage the set is saved first (`saving`).
    pub(super) fn write_state_change(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.config.set.as_ref()?;
        if !TimerSet::written_by(property) {
            return None;
        }
        Some(crate::write_config::saving::write(
            self,
            PendingWrite {
                property,
                array_index,
                value: value.clone(),
            },
        ))
    }
}

impl StateChangeRun {
    /// No run yet, and NO_FAULT_DETECTED.
    pub(super) fn new() -> Self {
        Self {
            verdict: Reliability::NO_FAULT_DETECTED,
            generation: next_generation(),
            running: None,
            pending: None,
        }
    }
}

impl TimerSet {
    /// Whether a write of `property` changes the set.
    pub(super) fn written_by(property: PropertyIdentifier) -> bool {
        matches!(
            property,
            PropertyIdentifier::STATE_CHANGE_VALUES
                | PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
        )
    }

    /// The set a write of `value` to `property` (at `array_index`) leaves,
    /// or the Timer's refusal.
    pub(super) fn after_write(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Result<Self, Error> {
        let mut next = self.clone();
        if property == PropertyIdentifier::STATE_CHANGE_VALUES {
            write_values(&mut next.values, array_index, value)?;
        } else {
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            let references = device_reference::decode_references(value)?;
            check_references(&references)?;
            next.references = references;
        }
        next.written = true;
        Ok(next)
    }
}

/// Refuse a set the Timer can't serve, as [`TimerObject::set_state_change`]
/// documents.
pub(super) fn check_set(
    values: &StateChangeValues,
    references: &[BACnetDeviceObjectPropertyReference],
    priority: u8,
) -> Result<(), Error> {
    if !(1..=16).contains(&priority) {
        return Err(common::value_out_of_range_error());
    }
    let mut scratch = BytesMut::new();
    for value in values {
        encode_timer_state_change_value(&mut scratch, value)
            .map_err(|_| common::value_out_of_range_error())?;
    }
    check_references(references)
}

/// The writes a run makes for its Timer. A run supersedes the one before, so
/// each write is attempted whatever the others did; the run's outcome is its
/// first failure.
impl CommandRunner for TimerObject {
    fn take_command_run(&mut self) -> Option<CommandRun> {
        self.run.pending.take()
    }

    fn command_generation(&self) -> u64 {
        self.run.generation
    }

    fn record_command_write(&mut self, generation: u64, command: usize, _success: bool) -> bool {
        let change = &self.run;
        change.generation == generation && change.running.is_some_and(|len| command < len)
    }

    fn complete_command_run(&mut self, generation: u64, outcome: Result<(), WriteFailure>) -> bool {
        let change = &mut self.run;
        if change.generation != generation || change.running.take().is_none() {
            return false;
        }
        let verdict = outcome.map_or_else(WriteFailure::reliability, |()| {
            Reliability::NO_FAULT_DETECTED
        });
        core::mem::replace(&mut change.verdict, verdict) != verdict
    }
}

/// The value a write carries for `value`, or `None` for `no-value`: a
/// primitive as itself, a date-time as its Date and Time, and the other
/// constructed values as their encoded octets, so the target decodes them
/// as it would a WriteProperty's.
fn write_value(value: &BACnetTimerStateChangeValue) -> Option<PropertyValue> {
    Some(match value {
        BACnetTimerStateChangeValue::NoValue => return None,
        BACnetTimerStateChangeValue::Primitive(value) => value.clone(),
        BACnetTimerStateChangeValue::Constructed(octets) => {
            PropertyValue::ApplicationData(octets.clone())
        }
        BACnetTimerStateChangeValue::DateTime(date, time) => {
            PropertyValue::List(vec![PropertyValue::Date(*date), PropertyValue::Time(*time)])
        }
        BACnetTimerStateChangeValue::LightingCommand(command) => {
            let mut encoded = BytesMut::new();
            encode_lighting_command(&mut encoded, command);
            PropertyValue::ApplicationData(encoded.to_vec())
        }
    })
}

/// One element as a read serves it: a primitive as itself, anything else as
/// its encoded octets.
fn read_value(value: &BACnetTimerStateChangeValue) -> Result<PropertyValue, Error> {
    if let BACnetTimerStateChangeValue::Primitive(value) = value {
        return Ok(value.clone());
    }
    let mut encoded = BytesMut::new();
    encode_timer_state_change_value(&mut encoded, value)?;
    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
}

/// Whether two non-NULL values differ in datatype (Clause 12.57.20).
fn mixed_datatypes(values: &StateChangeValues) -> bool {
    let mut datatypes = values
        .iter()
        .filter_map(BACnetTimerStateChangeValue::datatype)
        .filter(|&datatype| datatype != 0);
    datatypes
        .next()
        .is_some_and(|first| datatypes.any(|other| other != first))
}

fn check_references(references: &[BACnetDeviceObjectPropertyReference]) -> Result<(), Error> {
    if references.len() > MAX_TIMER_REFERENCES {
        return Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ));
    }
    device_reference::check_device_members(references)
}

/// A State_Change_Values write: the whole array of seven, or one element by
/// index. The size is fixed, so index 0 is WRITE_ACCESS_DENIED and a whole
/// value of another length VALUE_OUT_OF_RANGE.
fn write_values(
    values: &mut StateChangeValues,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    match array_index {
        None => {
            let elements = match value {
                PropertyValue::ApplicationData(octets) => decode_run(octets)?,
                PropertyValue::List(elements) => {
                    elements.iter().map(element).collect::<Result<_, _>>()?
                }
                single => vec![element(single)?],
            };
            *values = elements
                .try_into()
                .map_err(|_| common::value_out_of_range_error())?;
        }
        Some(0) => return Err(common::write_access_denied_error()),
        Some(index @ 1..=7) => values[index as usize - 1] = element(value)?,
        Some(_) => return Err(common::invalid_array_index_error()),
    }
    Ok(())
}

/// One written element: a primitive, or the octets of exactly one value.
fn element(value: &PropertyValue) -> Result<BACnetTimerStateChangeValue, Error> {
    match value {
        PropertyValue::ApplicationData(octets) => match decode_run(octets)?.as_slice() {
            [one] => Ok(one.clone()),
            _ => Err(common::invalid_data_encoding_error()),
        },
        value if value.is_primitive() => Ok(BACnetTimerStateChangeValue::Primitive(value.clone())),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// The values `octets` holds back to back. A context tag past `[3]` names a
/// datatype this Timer doesn't support (DATATYPE_NOT_SUPPORTED, Clause
/// 12.57.20); anything else that doesn't decode is INVALID_DATA_ENCODING.
fn decode_run(octets: &[u8]) -> Result<Vec<BACnetTimerStateChangeValue>, Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let (value, end) =
            decode_timer_state_change_value(octets, offset).map_err(|_| match tags::decode_tag(
                octets, offset,
            ) {
                Ok((tag, _))
                    if tag.class == TagClass::Context && tag.number > 3 && !tag.is_closing =>
                {
                    common::protocol_error(ErrorClass::PROPERTY, ErrorCode::DATATYPE_NOT_SUPPORTED)
                }
                _ => common::invalid_data_encoding_error(),
            })?;
        values.push(value);
        offset = end;
    }
    Ok(values)
}
