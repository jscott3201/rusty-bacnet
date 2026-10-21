//! Schedule (type 17) and Calendar (type 6) objects per ASHRAE 135-2020.

use bacnet_encoding::constructed::{
    encode_daily_schedule, encode_date_range, encode_special_event,
};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetDateRange, BACnetObjectPropertyReference, BACnetSpecialEvent, BACnetTimeValue,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use bytes::BytesMut;
use std::borrow::Cow;

use crate::common::{self, read_property_list_property};
use crate::traits::BACnetObject;

mod calendar;
mod calendar_metadata;
mod date_list;
mod evaluation;
mod metadata;
mod out_of_service;
mod reliability;
mod targets;
mod writes;

pub use calendar::CalendarObject;
pub use evaluation::{ScheduleTargetOutcome, ScheduleWrite};

// ---------------------------------------------------------------------------
// Schedule (type 17)
// ---------------------------------------------------------------------------

/// BACnet Schedule object.
///
/// Present_Value is calculated as Clause 12.24.4 describes (see
/// [`evaluate`](Self::evaluate)): within Effective_Period, the best special
/// event in effect, then today's weekly entry, then Schedule_Default. The
/// bundled server evaluates every Schedule once a minute against the Device
/// clock and writes a changed value, in its own datatype, to every member of
/// List_Of_Object_Property_References at Priority_For_Writing. Entering the
/// Effective_Period, the first evaluation after start-up included, writes the
/// value even when it has not changed.
///
/// Time-values and Schedule_Default hold values of a primitive datatype; the
/// setters refuse anything else. Weekly_Schedule, Exception_Schedule,
/// Effective_Period, Schedule_Default, List_Of_Object_Property_References and
/// Priority_For_Writing are network-writable, through the same checks as the
/// setters; after such a write the bundled server runs the evaluation again
/// at once. AddListElement and RemoveListElement edit the reference list and
/// write it back whole the same way. A change to the references or the
/// priority sends the current value to the new targets and relinquishes the
/// slots it leaves behind (see
/// [`set_object_property_references`](Self::set_object_property_references)).
///
/// Reliability is CONFIGURATION_ERROR while the non-NULL values in the two
/// schedules and Schedule_Default are not all of one datatype, or while a
/// referenced property refused the schedule's datatype, or the reference
/// itself, at its last write (Clause 12.24.13). Such a schedule still
/// evaluates and writes its references, and each pass that has nothing else
/// to send offers the current value again to the references that refused it,
/// so the fault clears once its cause is gone.
///
/// While Out_Of_Service is TRUE the calculation leaves Present_Value alone and
/// a client may write it instead; each such write goes on to the references
/// as a calculated change would (Clause 12.24.14). Back in service, the
/// calculation takes over again.
pub struct ScheduleObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: PropertyValue,
    schedule_default: PropertyValue,
    out_of_service: bool,
    reliability: Reliability,
    /// Evaluated Reliability saved while a client simulation owns the property
    /// (Out_Of_Service TRUE); restored on the return to service.
    reliability_before_out_of_service: Option<Reliability>,
    /// Whether the current CONFIGURATION_ERROR was raised by this object's
    /// own consistency check, either half, which may then clear it.
    owns_configuration_error: bool,
    /// References whose last write of a value in the schedule's datatype
    /// was refused, for its datatype or as a reference: the reference half of
    /// the consistency check (`reliability.rs`, #1086, #1433), and what a
    /// pass with nothing else to send retries (#1436).
    refusing_references: Vec<BACnetObjectPropertyReference>,
    status_flags: StatusFlags,
    /// 7-day weekly schedule: index 0 = Monday, index 6 = Sunday.
    weekly_schedule: [Vec<BACnetTimeValue>; 7],
    exception_schedule: Vec<BACnetSpecialEvent>,
    effective_period: BACnetDateRange,
    list_of_object_property_references: Vec<BACnetObjectPropertyReference>,
    /// Priority for writing to referenced objects (1-16).
    priority_for_writing: u8,
    /// Whether the last evaluation found today inside Effective_Period. The
    /// first evaluation inside it after one outside it (or after start-up)
    /// writes the references even if Present_Value is unchanged (Clause
    /// 12.24.6).
    in_effective_period: bool,
    /// A Present_Value written while Out_Of_Service is TRUE that has not been
    /// sent to the references yet.
    simulated_write: Option<PropertyValue>,
    /// The slots this Schedule's value holds on its targets (`targets.rs`).
    held: Option<targets::HeldCommand>,
    /// NULL writes owed to the slots a change of the references or the
    /// priority left behind.
    relinquish_owed: Vec<ScheduleWrite>,
    /// A change of the references or the priority owes the current
    /// Present_Value to the new list at the new priority.
    rewrite_owed: bool,
}

impl ScheduleObject {
    /// Create a new Schedule object; `schedule_default` is both Schedule_Default and the initial
    /// Present_Value, and must be of a primitive datatype (INVALID_DATA_TYPE otherwise).
    /// Effective_Period starts with both dates unspecified, a range that covers every date, and
    /// Priority_For_Writing at 16.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        schedule_default: PropertyValue,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::SCHEDULE, instance)?;
        if !schedule_default.is_primitive() {
            return Err(common::invalid_data_type_error());
        }
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: schedule_default.clone(),
            schedule_default,
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            owns_configuration_error: false,
            refusing_references: Vec::new(),
            status_flags: StatusFlags::empty(),
            weekly_schedule: [vec![], vec![], vec![], vec![], vec![], vec![], vec![]],
            exception_schedule: Vec::new(),
            effective_period: BACnetDateRange {
                start_date: unspecified_date(),
                end_date: unspecified_date(),
            },
            list_of_object_property_references: Vec::new(),
            priority_for_writing: 16, // default: lowest priority
            in_effective_period: false,
            simulated_write: None,
            held: None,
            relinquish_owed: Vec::new(),
            rewrite_owed: false,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set time-value entries for a given day (0=Monday .. 6=Sunday).
    ///
    /// Refuses, leaving the day unchanged, a day index past 6 or a time that
    /// is not specific (VALUE_OUT_OF_RANGE), a value that is not of a
    /// primitive datatype (INVALID_DATA_TYPE), and a time given twice
    /// (DUPLICATE_ENTRY).
    pub fn set_weekly_schedule(
        &mut self,
        day_index: usize,
        entries: Vec<BACnetTimeValue>,
    ) -> Result<(), Error> {
        let Some(day) = self.weekly_schedule.get_mut(day_index) else {
            return Err(common::value_out_of_range_error());
        };
        evaluation::check_time_values(&entries)?;
        *day = entries;
        self.contents_changed();
        Ok(())
    }

    /// Append a special event to the exception schedule.
    ///
    /// Refuses an event priority outside 1 to 16 or an inline calendar entry
    /// with an out-of-range value (VALUE_OUT_OF_RANGE), time-values that
    /// [`set_weekly_schedule`](Self::set_weekly_schedule) would refuse, and an
    /// event past the 1,024-event cap (RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY).
    pub fn add_exception(&mut self, event: BACnetSpecialEvent) -> Result<(), Error> {
        evaluation::check_special_event(&event)?;
        if self.exception_schedule.len() >= writes::MAX_EXCEPTIONS {
            return Err(writes::no_space_error());
        }
        self.exception_schedule.push(event);
        self.contents_changed();
        Ok(())
    }

    /// Set the effective period for this schedule.
    ///
    /// Each date must be a specific date or wholly unspecified, an open end
    /// (VALUE_OUT_OF_RANGE otherwise).
    pub fn set_effective_period(&mut self, period: BACnetDateRange) -> Result<(), Error> {
        if !period.is_valid() {
            return Err(common::value_out_of_range_error());
        }
        self.effective_period = period;
        Ok(())
    }

    /// Set Priority_For_Writing, 1 (highest) to 16 (VALUE_OUT_OF_RANGE
    /// otherwise).
    ///
    /// The current value then goes to the references at the new priority,
    /// and the slots held at the old one are relinquished, as for
    /// [`set_object_property_references`](Self::set_object_property_references).
    pub fn set_priority_for_writing(&mut self, priority: u8) -> Result<(), Error> {
        if !(1..=16).contains(&priority) {
            return Err(common::value_out_of_range_error());
        }
        self.priority_for_writing = priority;
        self.targets_changed();
        Ok(())
    }

    /// Append a local target reference, retaining its optional array index.
    ///
    /// Refuses a reference past the 1,024-member cap (RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY). The new member gets the current value as
    /// [`set_object_property_references`](Self::set_object_property_references)
    /// describes.
    pub fn add_object_property_reference(
        &mut self,
        r: BACnetObjectPropertyReference,
    ) -> Result<(), Error> {
        if self.list_of_object_property_references.len() >= targets::MAX_REFERENCES {
            return Err(writes::no_space_error());
        }
        self.list_of_object_property_references.push(r);
        self.targets_changed();
        Ok(())
    }

    /// Replace List_Of_Object_Property_References with local target
    /// references, each retaining its optional array index.
    ///
    /// Refuses more than 1,024 references (RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY), leaving the list unchanged. Once
    /// accepted, the next schedule pass sends the current Present_Value to
    /// every reference at Priority_For_Writing, if the object is writing at
    /// all: in service only inside Effective_Period. It also relinquishes, with
    /// a NULL at the priority it filled, each slot this Schedule holds on a
    /// reference the change drops. A Schedule holds a slot from a write of a
    /// value other than NULL until it leaves its Effective_Period; one outside
    /// it relinquishes nothing, so as not to clear the command of another
    /// Schedule in season on the same targets (Clause 12.24.6).
    pub fn set_object_property_references(
        &mut self,
        references: Vec<BACnetObjectPropertyReference>,
    ) -> Result<(), Error> {
        if references.len() > targets::MAX_REFERENCES {
            return Err(writes::no_space_error());
        }
        self.list_of_object_property_references = references;
        self.targets_changed();
        Ok(())
    }

    /// Read the current present_value.
    pub fn present_value(&self) -> &PropertyValue {
        &self.present_value
    }

    /// Weekly_Schedule, Exception_Schedule or Schedule_Default changed:
    /// re-check Reliability.
    fn contents_changed(&mut self) {
        let _ = self.recompute_reliability();
    }
}

/// A Date with every octet unspecified.
fn unspecified_date() -> Date {
    Date {
        year: Date::UNSPECIFIED,
        month: Date::UNSPECIFIED,
        day: Date::UNSPECIFIED,
        day_of_week: Date::UNSPECIFIED,
    }
}

/// One Weekly_Schedule element: a BACnetDailySchedule.
fn daily_schedule(time_values: &[BACnetTimeValue]) -> Result<PropertyValue, Error> {
    let mut encoded = BytesMut::new();
    encode_daily_schedule(&mut encoded, time_values)?;
    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
}

/// One Exception_Schedule element: a BACnetSpecialEvent.
fn special_event(event: &BACnetSpecialEvent) -> Result<PropertyValue, Error> {
    let mut encoded = BytesMut::new();
    encode_special_event(&mut encoded, event)?;
    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
}

impl BACnetObject for ScheduleObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::SCHEDULE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => Ok(self.present_value.clone()),
            p if p == PropertyIdentifier::SCHEDULE_DEFAULT => Ok(self.schedule_default.clone()),
            // FAULT follows Reliability and OUT_OF_SERVICE follows
            // Out_Of_Service; IN_ALARM follows the fixed NORMAL Event_State
            // this object reports.
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                EventState::NORMAL,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability.to_raw()))
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            // Clause 21 wire forms, one list element per array element so
            // the services concatenate them for a whole-array read.
            p if p == PropertyIdentifier::WEEKLY_SCHEDULE => match array_index {
                None => Ok(PropertyValue::List(
                    self.weekly_schedule
                        .iter()
                        .map(|day| daily_schedule(day))
                        .collect::<Result<_, _>>()?,
                )),
                Some(0) => Ok(PropertyValue::Unsigned(7)),
                Some(idx) if (1..=7).contains(&idx) => {
                    daily_schedule(&self.weekly_schedule[(idx - 1) as usize])
                }
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::EXCEPTION_SCHEDULE => match array_index {
                None => Ok(PropertyValue::List(
                    self.exception_schedule
                        .iter()
                        .map(special_event)
                        .collect::<Result<_, _>>()?,
                )),
                Some(0) => Ok(PropertyValue::Unsigned(self.exception_schedule.len() as u64)),
                Some(i) => (i as usize)
                    .checked_sub(1)
                    .and_then(|idx| self.exception_schedule.get(idx))
                    .ok_or_else(common::invalid_array_index_error)
                    .and_then(special_event),
            },
            p if p == PropertyIdentifier::EFFECTIVE_PERIOD => {
                let mut encoded = BytesMut::new();
                encode_date_range(&mut encoded, &self.effective_period);
                Ok(PropertyValue::ApplicationData(encoded.to_vec()))
            }
            // The local references back to back, each with no Device member,
            // in the run form the server's list services edit.
            p if p == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
                Ok(crate::device_reference::reference_run(
                    &self
                        .list_of_object_property_references
                        .iter()
                        .map(crate::device_reference::local_property_reference)
                        .collect::<Vec<_>>(),
                ))
            }
            p if p == PropertyIdentifier::PRIORITY_FOR_WRITING => {
                Ok(PropertyValue::Unsigned(self.priority_for_writing as u64))
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            return self.write_present_value(array_index, value);
        }
        if property == PropertyIdentifier::WEEKLY_SCHEDULE {
            return self.write_weekly_schedule(array_index, value);
        }
        if property == PropertyIdentifier::EXCEPTION_SCHEDULE {
            return self.write_exception_schedule(array_index, value);
        }
        if property == PropertyIdentifier::EFFECTIVE_PERIOD {
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            return self.write_effective_period(value);
        }
        if property == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES {
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            return self.write_object_property_references(value);
        }
        if property == PropertyIdentifier::PRIORITY_FOR_WRITING {
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            return self.write_priority_for_writing(value);
        }
        if property == PropertyIdentifier::SCHEDULE_DEFAULT {
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            // Clause 12.24.9: any primitive datatype, NULL included.
            if !value.is_primitive() {
                return Err(common::invalid_data_type_error());
            }
            self.schedule_default = value;
            self.contents_changed();
            return Ok(());
        }
        // Clause 12.24 Table 12-28 carries no writable footnote on Reliability
        // (plain R), but the object text still anticipates client simulation:
        // the Reliability_Evaluation_Inhibit description states the property
        // holds NO_FAULT_DETECTED while evaluation is disabled, except when a
        // client has supplied a replacement Reliability value while Out_Of_Service
        // is TRUE. In service the property reports the object's
        // own consistency evaluation (CONFIGURATION_ERROR et al.), so a network
        // write is refused; the internal route is `set_reliability_internal`
        // with the complementary guard.
        if property == PropertyIdentifier::RELIABILITY {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Enumerated(raw) = value {
                let v = Reliability::from_raw(raw);
                if !common::is_reliability_value_valid(v) {
                    return Err(common::value_out_of_range_error());
                }
                self.reliability = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::INVALID_DATA_TYPE,
            });
        }
        if let Some(result) = common::write_out_of_service_with_reliability_restore(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            result?;
            // Back in service, the restored value may predate changes made
            // while the client owned Reliability.
            let _ = self.recompute_reliability();
            return Ok(());
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        // While Out_Of_Service is TRUE the client owns the simulated value;
        // refusing here keeps the internal consistency evaluation from
        // clobbering the simulation.
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        // The caller owns this value; the consistency check leaves it alone.
        self.owns_configuration_error = false;
        Ok(())
    }

    fn evaluate_reliability_internal(
        &mut self,
    ) -> Result<crate::traits::ReliabilityEvaluation, Error> {
        Ok(self.recompute_reliability())
    }

    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        self.take_owed_writes()
    }

    fn complete_schedule_write(
        &mut self,
        write: &ScheduleWrite,
        outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        self.complete_write(write, outcomes)
    }

    fn retry_refusals_naming(&self, target: ObjectIdentifier) -> Option<ScheduleWrite> {
        self.retry_refused_naming(target)
    }

    fn tick_schedule(
        &mut self,
        today: SpecificDate,
        time: Time,
        calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<ScheduleWrite> {
        // Out_Of_Service decouples Present_Value from the calculation; a
        // client's write takes its place (`out_of_service.rs`).
        if self.out_of_service {
            return None;
        }
        let Some(value) = self.evaluate(today, time, calendar_active) else {
            // Inactive: nothing is written, and the slots the last value
            // filled are no longer this Schedule's to relinquish
            // (`targets.rs`). Entering the period writes anyway.
            self.in_effective_period = false;
            self.rewrite_owed = false;
            self.held = None;
            return None;
        };
        let entered = !std::mem::replace(&mut self.in_effective_period, true);
        if !entered && !self.rewrite_owed && value == self.present_value {
            // Nothing new for the list; offer the value again only to the
            // references that refused it (`reliability.rs`, #1436).
            return self.retry_refused();
        }
        self.present_value = value.clone();
        self.command(value)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod calendar_tests;

#[cfg(test)]
mod evaluation_tests;

#[cfg(test)]
mod out_of_service_tests;

#[cfg(test)]
mod reliability_tests;

#[cfg(test)]
mod retry_tests;

#[cfg(test)]
mod targets_tests;

#[cfg(test)]
mod write_tests;
