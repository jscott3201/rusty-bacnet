//! Load Control object (type 28) per ASHRAE 135-2020 Clause 12.28.
//!
//! The Load Control object is a standard interface for demand-response load
//! shedding. A client asks for a shed by writing Requested_Shed_Level,
//! Shed_Duration and Duty_Window, then Start_Time; the object runs the shed
//! state machine of Figure 12-5 and reports, through Present_Value,
//! Expected_Shed_Level and Actual_Shed_Level, how the loads stand.
//!
//! # The application route
//!
//! The loads, and what they can shed, belong to the application. It
//! implements [`LoadShedder`] and installs it with
//! [`LoadControlObject::set_shedder`]: the object hands it the request as it
//! stands and takes back whether the loads comply, the expected level and
//! the actual level, and tells it when a request ends. Without one the
//! object sheds nothing: a request waits for Start_Time and then stays
//! SHED_NON_COMPLIANT until its Shed_Duration has run.
//!
//! Time is the Device clock: the object needs one to start a request, and
//! the server lends its own. A request's moments are checked on each write
//! and on the server's once-a-second operation pass, so a client writes
//! Start_Time last ([`LoadControlObject::set_start_time`]).
//!
//! # Across a restart
//!
//! An object built with [`LoadControlObject::with_persistence`] keeps its
//! request in storage the application gives it ([`LoadControlPersistence`],
//! or the file backend `FileLoadControlPersistence` with std), and a rebuilt
//! one looks at it again as though Start_Time had just been written (Clause
//! 12.28, #2479). The `saving` module says when it saves. An object from
//! [`new`](LoadControlObject::new) keeps nothing: it starts SHED_INACTIVE.
//! Shed_Levels, Full_Duty_Baseline and State_Description are configuration the
//! application sets each time it builds the object, or changes while it runs
//! ([`LoadControlSetting`], #2497); storage keeps none of them.

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use bacnet_encoding::constructed::{decode_shed_level, encode_shed_level};
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier, Reliability, ShedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use bytes::BytesMut;

use crate::clock::{ClockReader, UNSPECIFIED_DATETIME, local_moment};
use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod metadata;
mod persistence;
mod saving;
mod shed;
#[cfg(test)]
mod shed_tests;
#[cfg(test)]
mod tests;

#[cfg(feature = "std")]
pub use persistence::FileLoadControlPersistence;
pub use persistence::{LoadControlPersistence, LoadControlSnapshot};
pub use shed::{LoadShedder, ShedProgress, ShedRequest, ShedStatus};

/// One configuration property of a running Load Control, as the application
/// changes it through the server (#2497). Each variant is the argument of
/// the matching setter ([`LoadControlObject::set_shed_levels`],
/// [`set_full_duty_baseline`](LoadControlObject::set_full_duty_baseline),
/// [`set_state_description`](LoadControlObject::set_state_description)), and
/// is checked and takes effect as that setter does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LoadControlSetting {
    /// Shed_Levels with their Shed_Level_Descriptions, a pair for each level.
    ShedLevels(Vec<(u64, String)>),
    /// Full_Duty_Baseline in kilowatts, or `None` to clear it.
    FullDutyBaseline(Option<f32>),
    /// State_Description, or `None` to clear it.
    StateDescription(Option<String>),
}

/// BACnet Load Control object — demand-response load shedding.
///
/// Requested_Shed_Level, Expected_Shed_Level and Actual_Shed_Level are
/// [`BACnetShedLevel`] values and go out in that CHOICE's context-tagged form
/// (Clause 21). Shed_Levels and Shed_Level_Descriptions start empty; the
/// embedder sets them with [`set_shed_levels`](Self::set_shed_levels). The
/// optional Full_Duty_Baseline and State_Description are served only once
/// the embedder sets them ([`set_full_duty_baseline`](Self::set_full_duty_baseline),
/// [`set_state_description`](Self::set_state_description)). The module docs
/// describe the state machine and the application route.
#[derive(Clone)]
pub struct LoadControlObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value: the shed state.
    state: ShedState,
    requested_shed_level: BACnetShedLevel,
    expected_shed_level: BACnetShedLevel,
    actual_shed_level: BACnetShedLevel,
    /// Minutes.
    shed_duration: u64,
    /// Minutes.
    duty_window: u64,
    /// The Duty_Window a request takes when no client wrote one, and the one
    /// the object returns to when a request ends (Clause 12.28.13).
    agreed_duty_window: u64,
    start_time: (Date, Time),
    enable: bool,
    shed_levels: Vec<u64>,
    shed_level_descriptions: Vec<String>,
    /// Full_Duty_Baseline in kilowatts; the row is absent while `None`.
    full_duty_baseline: Option<f32>,
    /// State_Description; the row is absent while `None`.
    state_description: Option<String>,
    status_flags: StatusFlags,
    /// Event_State.
    event_state: EventState,
    reliability: Reliability,
    clock: Option<Arc<dyn ClockReader>>,
    shedder: Option<Arc<dyn LoadShedder>>,
    /// Whether the request under way has been offered to the shedder, so
    /// that its end is reported to it.
    offered: bool,
    /// The shed state a restored request was saved in, while the
    /// application may still hold loads shed for it before the restart:
    /// COMPLIANT or NON_COMPLIANT (#2479). Its end owes a stop though the
    /// shedder was never asked about it here. Cleared once the request is
    /// offered or ends; storage keeps reading this state meanwhile, so a
    /// second restart still owes the stop.
    shed_before_restart: Option<ShedState>,
    /// Where the request is kept, with persistence (#2479).
    durable: saving::Durable,
}

impl LoadControlObject {
    /// Create a new Load Control object with default values: enabled,
    /// SHED_INACTIVE, no Shed_Levels, an agreed Duty_Window of zero and no
    /// [`LoadShedder`].
    ///
    /// The three shed levels start at level 0: LEVEL is the one choice every
    /// Load Control must take (Clause 12.28.10), and 0 is its no-shed default.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            state: ShedState::SHED_INACTIVE,
            requested_shed_level: BACnetShedLevel::Level(0),
            expected_shed_level: BACnetShedLevel::Level(0),
            actual_shed_level: BACnetShedLevel::Level(0),
            shed_duration: 0,
            duty_window: 0,
            agreed_duty_window: 0,
            start_time: UNSPECIFIED_DATETIME,
            enable: true,
            shed_levels: Vec::new(),
            shed_level_descriptions: Vec::new(),
            full_duty_baseline: None,
            state_description: None,
            status_flags: StatusFlags::empty(),
            event_state: EventState::NORMAL,
            reliability: Reliability::NO_FAULT_DETECTED,
            clock: None,
            shedder: None,
            offered: false,
            shed_before_restart: None,
            durable: saving::Durable::default(),
        })
    }

    /// Install the application's [`LoadShedder`], replacing any earlier one.
    /// The object asks it from the next look at a request on.
    ///
    /// The shedder replaced is not told anything, even with a request under
    /// way, and the new one hears of that request's end only once it has
    /// been asked about it, or when the request was restored as it was
    /// being shed and no shedder has been asked about it since (#2479).
    pub fn set_shedder(&mut self, shedder: Arc<dyn LoadShedder>) {
        self.shedder = Some(shedder);
        self.offered = false;
    }

    /// Present_Value, the shed state.
    pub fn shed_state(&self) -> ShedState {
        self.state
    }

    /// Set Requested_Shed_Level, with the checks and effects of a
    /// WriteProperty of it.
    ///
    /// A shed request asks for less load, so a percent above 100, or an amount
    /// that is negative or not finite, would ask for more than the baseline
    /// (Table 12-33) and fails with PROPERTY / VALUE_OUT_OF_RANGE, changing
    /// nothing. Any level is accepted. While no request is pending or being
    /// shed, Expected_Shed_Level and Actual_Shed_Level take the default of the
    /// new level's choice: 100 for a percent, 0 for a level, 0.0 for an
    /// amount. During a request the write reconfigures it, and the choice's
    /// default cancels it.
    pub fn set_requested_shed_level(&mut self, level: BACnetShedLevel) -> Result<(), Error> {
        check_requested_level(&level)?;
        self.requested_shed_level = level;
        self.settle(shed::Prompt::Parameters);
        Ok(())
    }

    /// Set Start_Time, with the checks and effects of a WriteProperty of it:
    /// an actual local date and time starts a request, or reconfigures the
    /// one under way, and the wholly unspecified value cancels it (and while
    /// no request is under way changes nothing).
    ///
    /// The object looks at the request on each write, so Start_Time goes
    /// last: write Requested_Shed_Level, a non-zero Shed_Duration and
    /// Duty_Window first, then Start_Time. A Start_Time behind the requested
    /// choice's default level, or one whose Shed_Duration has already run,
    /// is taken and ends the request at once, so it reads unspecified again.
    ///
    /// The seconds and hundredths may be unspecified, and count as zero, and
    /// the weekday is taken as written. Any other partly unspecified value
    /// names no moment and fails with PROPERTY / VALUE_OUT_OF_RANGE, changing
    /// nothing. While Enable is FALSE an acceptable value is taken but not
    /// kept: Start_Time stays unspecified.
    pub fn set_start_time(&mut self, start_time: (Date, Time)) -> Result<(), Error> {
        check_start_time(start_time)?;
        if self.enable {
            self.start_time = start_time;
            self.settle(shed::Prompt::StartTime);
        }
        Ok(())
    }

    /// Set Shed_Duration, in minutes, with the effects of a WriteProperty of
    /// it: during a request, it reconfigures the request. While Enable is
    /// FALSE it is taken but not kept: Shed_Duration stays zero.
    pub fn set_shed_duration(&mut self, minutes: u64) {
        if self.enable {
            self.shed_duration = minutes;
            self.settle(shed::Prompt::Parameters);
        }
    }

    /// Set Duty_Window, in minutes, with the effects of a WriteProperty of it:
    /// during a request, it reconfigures the request.
    pub fn set_duty_window(&mut self, minutes: u64) {
        self.duty_window = minutes;
        self.settle(shed::Prompt::Parameters);
    }

    /// Set the agreed Duty_Window, in minutes: the one a request takes when
    /// no client wrote one, and the one the object returns to when a request
    /// ends (Clause 12.28.13). While no request is pending or being shed,
    /// Duty_Window takes it at once.
    pub fn set_agreed_duty_window(&mut self, minutes: u64) {
        self.agreed_duty_window = minutes;
        if self.state == ShedState::SHED_INACTIVE {
            self.duty_window = minutes;
        }
        self.follow_storage();
    }

    /// Set Enable, with the effects of a WriteProperty of it: FALSE ends any
    /// request and takes no new one until it is TRUE again (Clause
    /// 12.28.14).
    ///
    /// While it is FALSE, a write of Start_Time or Shed_Duration is
    /// acknowledged but not kept, so both read their idle values (unspecified
    /// and zero), a Shed_Duration written before it went FALSE included;
    /// Requested_Shed_Level and Duty_Window are kept as plain configuration. Turning it back on starts nothing by itself: the next
    /// Start_Time write does, with the level and duty window held and the
    /// Shed_Duration written since.
    pub fn set_enable(&mut self, enable: bool) {
        self.enable = enable;
        self.settle(shed::Prompt::Look);
    }

    /// Set Shed_Levels and Shed_Level_Descriptions together: for each level
    /// the object can take, its value and a description of what it sheds,
    /// in order of increasing shed (Clauses 12.28.18 and 12.28.19). The
    /// order isn't checked.
    ///
    /// Level 0 is the no-shed level, which the array doesn't hold (Clause
    /// 12.28.10): a 0 fails with PROPERTY / VALUE_OUT_OF_RANGE and changes
    /// nothing. Clients may rewrite the values but not change their number,
    /// and read the descriptions only.
    pub fn set_shed_levels(&mut self, levels: Vec<(u64, String)>) -> Result<(), Error> {
        if levels.iter().any(|&(level, _)| level == 0) {
            return Err(common::value_out_of_range_error());
        }
        (self.shed_levels, self.shed_level_descriptions) = levels.into_iter().unzip();
        self.settle(shed::Prompt::Look);
        Ok(())
    }

    /// Set Full_Duty_Baseline, in kilowatts: the fixed baseline a PERCENT or
    /// AMOUNT request is reckoned from (Clauses 12.28.10 and 12.28.15, Table
    /// 12-33), or `None` for none. The property is optional and read-only to
    /// clients: the object serves it, and lists it in Property_List, only
    /// while one is set.
    ///
    /// A negative or non-finite baseline fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing. The [`LoadShedder`] reads it
    /// as [`ShedRequest::full_duty_baseline`], and a request under way is
    /// put to it again with the new value.
    pub fn set_full_duty_baseline(&mut self, kilowatts: Option<f32>) -> Result<(), Error> {
        if let Some(kilowatts) = kilowatts {
            common::reject_non_finite(kilowatts)?;
            if kilowatts < 0.0 {
                return Err(common::value_out_of_range_error());
            }
        }
        self.full_duty_baseline = kilowatts;
        self.settle(shed::Prompt::Look);
        Ok(())
    }

    /// Set State_Description, a note for operators on the shed state
    /// (Clause 12.28.6), or `None` for none. Like Full_Duty_Baseline it is
    /// optional and read-only to clients, served and listed in
    /// Property_List only while set.
    pub fn set_state_description(&mut self, text: Option<String>) {
        self.state_description = text;
    }

    /// A WriteProperty of a property storage keeps, through its setter:
    /// one of the four request properties or Enable, or, with storage, a
    /// written Description.
    fn write_request(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match property {
            p if p == PropertyIdentifier::ENABLE => {
                self.set_enable(boolean_write(value)?);
                Ok(())
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                self.description = crate::write_config::description::written(&value)?;
                self.durable.description_written();
                Ok(())
            }
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                self.set_requested_shed_level(decode_shed_level_write(value)?)
            }
            p if p == PropertyIdentifier::START_TIME => {
                self.set_start_time(decode_date_time_write(&value)?)
            }
            p if p == PropertyIdentifier::SHED_DURATION => {
                self.set_shed_duration(unsigned_write(value)?);
                Ok(())
            }
            _ => {
                self.set_duty_window(unsigned_write(value)?);
                Ok(())
            }
        }
    }

    /// A WriteProperty of Shed_Levels: the array whole, keeping its size, or
    /// one element (Table 12-32 footnote 1). Index 0, the size, is
    /// WRITE_ACCESS_DENIED, and a level of 0 VALUE_OUT_OF_RANGE, as for
    /// [`set_shed_levels`](Self::set_shed_levels).
    fn write_shed_levels(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let mut levels = self.shed_levels.clone();
        match array_index {
            None => {
                // A one-element array arrives as its element alone.
                let elements = match value {
                    PropertyValue::List(elements) => elements,
                    element => vec![element],
                };
                levels = elements
                    .into_iter()
                    .map(unsigned_write)
                    .collect::<Result<Vec<_>, _>>()?;
                if levels.len() != self.shed_levels.len() {
                    return Err(common::value_out_of_range_error());
                }
            }
            Some(0) => return Err(common::write_access_denied_error()),
            Some(index) => {
                let position = usize::try_from(index - 1).unwrap_or(usize::MAX);
                let Some(level) = levels.get_mut(position) else {
                    return Err(common::invalid_array_index_error());
                };
                *level = unsigned_write(value)?;
            }
        }
        if levels.contains(&0) {
            return Err(common::value_out_of_range_error());
        }
        self.shed_levels = levels;
        self.settle(shed::Prompt::Look);
        Ok(())
    }
}

/// Refuse a Start_Time that names no moment and isn't wholly unspecified.
fn check_start_time(start_time: (Date, Time)) -> Result<(), Error> {
    if start_time != UNSPECIFIED_DATETIME && local_moment(start_time).is_none() {
        return Err(common::value_out_of_range_error());
    }
    Ok(())
}

/// Decode a write of a BOOLEAN, or PROPERTY / INVALID_DATA_TYPE.
fn boolean_write(value: PropertyValue) -> Result<bool, Error> {
    match value {
        PropertyValue::Boolean(value) => Ok(value),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Decode a write of an Unsigned, or PROPERTY / INVALID_DATA_TYPE.
fn unsigned_write(value: PropertyValue) -> Result<u64, Error> {
    match value {
        PropertyValue::Unsigned(value) => Ok(value),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Refuse a requested shed level that asks for more load than the baseline:
/// a percent above 100, or an amount that is negative or not finite
/// (Table 12-33).
fn check_requested_level(level: &BACnetShedLevel) -> Result<(), Error> {
    match *level {
        BACnetShedLevel::Percent(percent) if percent > 100 => {
            Err(common::value_out_of_range_error())
        }
        BACnetShedLevel::Amount(amount) => {
            common::reject_non_finite(amount)?;
            if amount < 0.0 {
                return Err(common::value_out_of_range_error());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The Table 12-33 default for the choice `level` uses, the value that also
/// cancels a shed request when written.
fn choice_default(level: &BACnetShedLevel) -> BACnetShedLevel {
    match level {
        BACnetShedLevel::Percent(_) => BACnetShedLevel::Percent(100),
        BACnetShedLevel::Level(_) => BACnetShedLevel::Level(0),
        BACnetShedLevel::Amount(_) => BACnetShedLevel::Amount(0.0),
    }
}

/// A shed level in its Clause 21 CHOICE form.
fn shed_level_value(level: &BACnetShedLevel) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_shed_level(&mut buf, level);
    PropertyValue::ApplicationData(buf.to_vec())
}

/// Decode a Requested_Shed_Level write: exactly one `BACnetShedLevel` CHOICE,
/// which the WriteProperty decoder passes on as `ApplicationData`. Any other
/// value, the bare Unsigned or REAL and the one-element list taken before
/// #1133 included, fails with PROPERTY / INVALID_DATA_TYPE.
fn decode_shed_level_write(value: PropertyValue) -> Result<BACnetShedLevel, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    match decode_shed_level(&bytes, 0) {
        Ok((level, end)) if end == bytes.len() => Ok(level),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Decode a BACnetDateTime write: an application Date then an application
/// Time, or PROPERTY / INVALID_DATA_TYPE.
fn decode_date_time_write(value: &PropertyValue) -> Result<(Date, Time), Error> {
    match value {
        PropertyValue::List(items) => match items.as_slice() {
            [PropertyValue::Date(date), PropertyValue::Time(time)] => Ok((*date, *time)),
            _ => Err(common::invalid_data_type_error()),
        },
        _ => Err(common::invalid_data_type_error()),
    }
}

impl BACnetObject for LoadControlObject {
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
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        // Table 12-32 has no Out_Of_Service (#1064), and Clause 12.28 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::LOAD_CONTROL.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.state.to_raw()))
            }
            p if p == PropertyIdentifier::STATE_DESCRIPTION => match &self.state_description {
                Some(text) => Ok(PropertyValue::CharacterString(text.clone())),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::FULL_DUTY_BASELINE => match self.full_duty_baseline {
                Some(kilowatts) => Ok(PropertyValue::Real(kilowatts)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                Ok(shed_level_value(&self.requested_shed_level))
            }
            p if p == PropertyIdentifier::EXPECTED_SHED_LEVEL => {
                Ok(shed_level_value(&self.expected_shed_level))
            }
            p if p == PropertyIdentifier::ACTUAL_SHED_LEVEL => {
                Ok(shed_level_value(&self.actual_shed_level))
            }
            p if p == PropertyIdentifier::SHED_DURATION => {
                Ok(PropertyValue::Unsigned(self.shed_duration))
            }
            p if p == PropertyIdentifier::DUTY_WINDOW => {
                Ok(PropertyValue::Unsigned(self.duty_window))
            }
            p if p == PropertyIdentifier::START_TIME => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.start_time.0),
                PropertyValue::Time(self.start_time.1),
            ])),
            p if p == PropertyIdentifier::ENABLE => Ok(PropertyValue::Boolean(self.enable)),
            p if p == PropertyIdentifier::SHED_LEVELS => common::read_array(
                self.shed_levels
                    .iter()
                    .map(|&level| PropertyValue::Unsigned(level))
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::SHED_LEVEL_DESCRIPTIONS => common::read_array(
                self.shed_level_descriptions
                    .iter()
                    .cloned()
                    .map(PropertyValue::CharacterString)
                    .collect(),
                array_index,
            ),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        // With storage, the request, Enable and a written Description are
        // saved before they are served (#2479).
        if self.durable.is_kept() && saving::kept(property, &value) {
            return self.write_kept(property, value);
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if saving::kept(p, &value) => self.write_request(p, value),
            p if p == PropertyIdentifier::SHED_LEVELS => self.write_shed_levels(array_index, value),
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        Some(self)
    }

    fn timed(&mut self) -> Option<&mut dyn crate::traits::Timed> {
        Some(self)
    }

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }

    // Table 13-1 lists Load Control, so it takes SubscribeCOV; the shed
    // rows it reports are that row's (the default).
    crate::traits::lend_cov_reporting!();
}

impl crate::traits::CovReporting for LoadControlObject {}

/// The application's route to the configuration of a running object: the
/// setters, behind the server's local routes (#2497). They are not storage's
/// business (the module docs of `saving` say what it keeps), but a change
/// that moves a request under way is saved like any the state machine makes.
impl crate::traits::LocalInput for LoadControlObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::LoadControl(setting) = input else {
            return Err(crate::traits::not_supported());
        };
        match setting {
            LoadControlSetting::ShedLevels(levels) => self.set_shed_levels(levels),
            LoadControlSetting::FullDutyBaseline(kilowatts) => {
                self.set_full_duty_baseline(kilowatts)
            }
            LoadControlSetting::StateDescription(text) => {
                self.set_state_description(text);
                Ok(())
            }
        }
    }
}

impl crate::traits::Timed for LoadControlObject {
    fn bind_clocks(&mut self, clocks: &crate::traits::Clocks) {
        self.clock = clocks.clock.clone();
    }

    /// Look at the request against the Device clock: its Start_Time and end
    /// are wall-clock moments, so the monotonic `now` isn't used.
    fn advance_monotonic_time(&mut self, _now: core::time::Duration) -> bool {
        self.settle(shed::Prompt::Look)
    }

    /// The device is starting: look at a request restored from storage as
    /// though Start_Time had just been written (Clause 12.28, #2479). A
    /// look at an object with no request under way changes nothing.
    fn start_output(&mut self) {
        self.settle(shed::Prompt::Look);
    }

    /// Leaving its database: a request under way ends as a cancelled one
    /// does, so the application is told to stop and the object reads
    /// SHED_INACTIVE (#2479). Removed, its storage saves the ended request;
    /// released (replaced, or let go after a failed start), it leaves
    /// storage as it was, for the object or start that comes next.
    fn retire(&mut self, retirement: crate::traits::Retirement) {
        if retirement == crate::traits::Retirement::Removed {
            self.settle(shed::Prompt::Retire);
            return;
        }
        let storage = self.durable.detach();
        self.settle(shed::Prompt::Retire);
        self.durable.attach(storage);
    }

    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(Self {
            clock: None,
            shedder: None,
            ..self.clone()
        }))
    }
}
