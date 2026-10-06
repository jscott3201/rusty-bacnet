//! Lighting Output (type 54) and Binary Lighting Output (type 55) objects per
//! ASHRAE 135-2020 Clauses 12.54 and 12.55.

use bacnet_types::constructed::BACnetLightingCommand;
use bacnet_types::enums::{LightingOperation, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{self, read_common_properties, read_priority_array};
use crate::object_profile::ObjectProfile;
use crate::traits::{BACnetObject, DeadlineWaker, MonotonicClock};

// ---------------------------------------------------------------------------
// LightingOutput (type 54)
// ---------------------------------------------------------------------------

/// BACnet Lighting Output object.
///
/// Commandable output with a 16-level priority array controlling a
/// floating-point present-value (0.0 to 100.0 percent). A commanded level
/// above 0.0 and below 1.0 is stored as 1.0, and so is such a
/// Relinquish_Default (see [`set_relinquish_default`](Self::set_relinquish_default)).
///
/// Lighting_Command takes a [`BACnetLightingCommand`] checked against its
/// operation (see [`set_lighting_command`](Self::set_lighting_command)),
/// serves the last one taken and carries it out (#1384): fades and ramps move
/// Tracking_Value to a new level over time with In_Progress showing which,
/// steps change the level at once, and the warn operations blink and hold
/// the level for Egress_Time before relinquishing or turning it off.
/// Present_Value takes the blink-warn values -1.0, -2.0 and -3.0 as the
/// three warn commands. Tracking_Value equals Present_Value whenever no fade
/// or ramp is moving it and no trim holds it.
///
/// Color_Reference, Color_Override and Override_Color_Reference are absent
/// until [`set_color_link`](Self::set_color_link) links the output to its
/// colour objects (#1527).
///
/// High_End_Trim, Low_End_Trim and Trim_Fade_Time are absent until set
/// (#1528; see [`set_high_end_trim`](Self::set_high_end_trim)). Once set,
/// Tracking_Value is held between the trims, In_Progress reads TRIM_ACTIVE
/// while that keeps it from Present_Value, and a trim change takes
/// Trim_Fade_Time to show.
///
/// Fades, ramps and egress timers run on the monotonic clock the database
/// binds; the server's monotonic task advances them and fans their COV out.
/// With no clock bound they wait on `advance_time_internal`.
#[derive(Clone)]
pub struct LightingOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    /// The last command written; operation NONE until then.
    lighting_command: BACnetLightingCommand,
    lighting_command_default_priority: u32,
    /// The fade, ramp or egress in progress, if any.
    operation: Option<engine::Operation>,
    /// Blink-warn notifications requested so far, for tests to observe.
    blink_request_count: u64,
    blink_warn_enable: bool,
    /// Egress_Time in seconds.
    egress_time: u32,
    /// Default_Fade_Time in milliseconds, within 100..=86_400_000.
    default_fade_time: u32,
    /// Default_Ramp_Rate in percent per second, within 0.1..=100.0.
    default_ramp_rate: f32,
    /// Default_Step_Increment in percent, within 0.1..=100.0.
    default_step_increment: f32,
    /// COV_Increment: the Present_Value change that triggers a notification.
    cov_increment: f32,
    /// The finest COV increment a Tracking_Value subscriber asks for, as the
    /// server last passed it (#1510).
    finest_tracking_increment: Option<f64>,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// High_End_Trim, Low_End_Trim and Trim_Fade_Time, and a trim change
    /// under way.
    trims: trim::Trims,
    /// Color_Reference, with Color_Override and Override_Color_Reference
    /// when overridable; absent until set.
    color_link: Option<ColorLink>,
    /// Tags, Profile_Location and Profile_Name, once provisioned (#1553).
    profile: ObjectProfile,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    deadline_waker: Option<Arc<DeadlineWaker>>,
    /// The time an object with no clock bound has been advanced to.
    logical_now: Duration,
}

impl LightingOutputObject {
    /// Create a new Lighting Output object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            lighting_command: BACnetLightingCommand::new(LightingOperation::NONE),
            lighting_command_default_priority: 16,
            operation: None,
            blink_request_count: 0,
            blink_warn_enable: false,
            egress_time: 0,
            default_fade_time: *DEFAULT_FADE_TIME_MS.start(),
            default_ramp_rate: 100.0,
            default_step_increment: 1.0,
            cov_increment: 0.0,
            finest_tracking_increment: None,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            priority_array: [None; 16],
            relinquish_default: 0.0,
            trims: trim::Trims::NONE,
            color_link: None,
            profile: ObjectProfile::default(),
            monotonic_clock: None,
            deadline_waker: None,
            logical_now: Duration::ZERO,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Recalculate present-value from the priority array.
    ///
    /// Tracking_Value isn't stored: it reads as Present_Value while
    /// In_Progress is IDLE and as the fade or ramp's current value while one
    /// runs (Clause 12.54.5), so a new Present_Value shows in it only once
    /// nothing is moving it.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Checked the same way a commanded Present_Value is: a value outside
    /// 0.0 to 100.0, or NaN, is refused with VALUE_OUT_OF_RANGE, and one
    /// above 0.0 and below 1.0 is stored as 1.0, so Present_Value never falls
    /// back to a level between off and the dimmest on level. After the store,
    /// Present_Value is resolved anew from the priority array so an empty
    /// array falls back to the new default immediately.
    pub fn set_relinquish_default(&mut self, value: f32) -> Result<(), Error> {
        self.relinquish_default = normalized_level(value)?;
        self.recalculate_present_value();
        Ok(())
    }

    /// The last command written to Lighting_Command, or operation NONE with no
    /// other field before any write.
    pub fn lighting_command(&self) -> BACnetLightingCommand {
        self.lighting_command
    }

    /// Set Lighting_Command and carry the command out, as a WriteProperty of
    /// the encoded command would.
    ///
    /// The command is checked against its operation (Clause 12.54, Table
    /// 12-67). NONE, a reserved operation (11 to 255) and anything past
    /// 65,535 are refused; FADE_TO and RAMP_TO need a target level. A field
    /// the operation uses must be in range: target level 0.0 to 100.0, fade
    /// time 100 to 86,400,000 ms, ramp rate and step increment 0.1 to 100.0,
    /// priority 1 to 16. Fields it doesn't use are kept unchecked, and a
    /// proprietary operation (256 to 65,535) has only its priority checked. A
    /// refusal is VALUE_OUT_OF_RANGE and leaves the object unchanged.
    ///
    /// A command taken is stored as written and carried out at its priority,
    /// or at Lighting_Command_Default_Priority when it has none. A level it
    /// puts in a priority slot is normalized as a commanded Present_Value is
    /// (Clause 12.54.4), so a FADE_TO 0.5 fades to 1.0 while Lighting_Command
    /// still reads 0.5. A proprietary operation is stored and does nothing
    /// else.
    pub fn set_lighting_command(&mut self, command: BACnetLightingCommand) -> Result<(), Error> {
        command::check(&command)?;
        self.take_lighting_command(command);
        Ok(())
    }

    /// Store a checked command and carry it out now.
    fn take_lighting_command(&mut self, command: BACnetLightingCommand) {
        self.lighting_command = command;
        let now = self.now();
        self.execute(&command, now);
    }

    /// Link the output to the colour objects that set its colour, or take
    /// the link away with `None` (#1527, Addendum 135-2020ca part 4).
    ///
    /// A link serves Color_Reference, and with a [`ColorOverride`] also
    /// Color_Override and Override_Color_Reference; all of them take
    /// writes. Each reference must name a colour object, Color or Color
    /// Temperature (instance 4194303 names none); anything else is refused
    /// with VALUE_OUT_OF_RANGE and changes nothing. The object only stores
    /// the references:
    /// [`ObjectDatabase::lighting_color`](crate::database::ObjectDatabase::lighting_color)
    /// follows them.
    pub fn set_color_link(&mut self, link: Option<ColorLink>) -> Result<(), Error> {
        self.color_link = link.map(ColorLink::checked).transpose()?;
        Ok(())
    }

    /// The colour link, if one is set.
    pub fn color_link(&self) -> Option<&ColorLink> {
        self.color_link.as_ref()
    }

    /// Provision the optional Tags, Profile_Location and Profile_Name rows
    /// before registration (#1553), as a Color object's
    /// [`set_profile`](crate::color::ColorObject::set_profile) does.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        profile.check()?;
        self.profile = profile;
        Ok(())
    }

    /// Set Default_Fade_Time, the milliseconds a fade request without its own
    /// fade time takes. A new object uses 100, the shortest fade the clause
    /// allows, as Default_Ramp_Rate starts at its fastest rate.
    ///
    /// Clause 12.54.16 bounds it to 100..=86_400_000 (one day); a value
    /// outside that range is refused with VALUE_OUT_OF_RANGE and the property
    /// is left unchanged. WriteProperty applies the same check.
    pub fn set_default_fade_time(&mut self, milliseconds: u32) -> Result<(), Error> {
        if !DEFAULT_FADE_TIME_MS.contains(&milliseconds) {
            return Err(common::value_out_of_range_error());
        }
        self.default_fade_time = milliseconds;
        Ok(())
    }

    /// Set Default_Ramp_Rate, the percent-per-second rate a ramp request
    /// without its own rate uses. A new object uses 100.0.
    ///
    /// Clause 12.54.17 bounds it to 0.1..=100.0; a value outside that range,
    /// or a non-finite one, is refused with VALUE_OUT_OF_RANGE and the
    /// property is left unchanged. WriteProperty applies the same check.
    pub fn set_default_ramp_rate(&mut self, value: f32) -> Result<(), Error> {
        self.default_ramp_rate = lighting_percent(value)?;
        Ok(())
    }

    /// Set Default_Step_Increment, the percent a step request without its own
    /// increment adds. A new object uses 1.0.
    ///
    /// Clause 12.54.18 bounds it to 0.1..=100.0; a value outside that range,
    /// or a non-finite one, is refused with VALUE_OUT_OF_RANGE and the
    /// property is left unchanged. WriteProperty applies the same check.
    pub fn set_default_step_increment(&mut self, value: f32) -> Result<(), Error> {
        self.default_step_increment = lighting_percent(value)?;
        Ok(())
    }
}

/// Check a light level written to Present_Value or Relinquish_Default.
///
/// Both are percentages on the Clause 12.54 normalized scale: 0.0 is off,
/// 1.0 is the dimmest on level and 100.0 the brightest, and no level lies
/// strictly between 0.0 and 1.0. Clause 12.54.4 has a Present_Value write in
/// that gap taken as 1.0, so such a level comes back as 1.0. 0.0 and levels
/// from 1.0 to 100.0 come back unchanged. A level below 0.0 or above 100.0,
/// NaN included, is VALUE_OUT_OF_RANGE; a Present_Value write tells the
/// blink-warn values -1.0 to -3.0 apart before it gets here. -0.0 is off, so
/// it comes back as 0.0 rather than keeping its sign on the wire.
fn normalized_level(value: f32) -> Result<f32, Error> {
    if !(0.0..=100.0).contains(&value) {
        return Err(common::value_out_of_range_error());
    }
    Ok(if value == 0.0 {
        0.0
    } else if value < 1.0 {
        1.0
    } else {
        value
    })
}

/// The Default_Fade_Time range of Clause 12.54.16, in milliseconds, which a
/// lighting command's fade time shares (Table 12-66).
const DEFAULT_FADE_TIME_MS: std::ops::RangeInclusive<u32> = 100..=86_400_000;

/// Check a Default_Ramp_Rate or Default_Step_Increment value, or a lighting
/// command's ramp rate or step increment, which share the 0.1..=100.0 range.
fn lighting_percent(value: f32) -> Result<f32, Error> {
    if (0.1..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err(common::value_out_of_range_error())
    }
}

impl BACnetObject for LightingOutputObject {
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::LIGHTING_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            // Each read takes its own instant, so one ReadPropertyMultiple
            // that straddles a fade's end may pair a Tracking_Value just short
            // of the level with In_Progress IDLE.
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Real(self.tracking_value_at(self.now())))
            }
            p if p == PropertyIdentifier::LIGHTING_COMMAND => {
                Ok(command::encode(&self.lighting_command))
            }
            p if p == PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY => Ok(
                PropertyValue::Unsigned(self.lighting_command_default_priority as u64),
            ),
            p if p == PropertyIdentifier::IN_PROGRESS => Ok(PropertyValue::Enumerated(
                self.in_progress_at(self.now()).to_raw(),
            )),
            p if p == PropertyIdentifier::BLINK_WARN_ENABLE => {
                Ok(PropertyValue::Boolean(self.blink_warn_enable))
            }
            p if p == PropertyIdentifier::EGRESS_TIME => {
                Ok(PropertyValue::Unsigned(self.egress_time as u64))
            }
            p if p == PropertyIdentifier::EGRESS_ACTIVE => {
                Ok(PropertyValue::Boolean(self.egress_active()))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                read_priority_array!(self, array_index, PropertyValue::Real)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Real(self.relinquish_default))
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                Ok(PropertyValue::Unsigned(u64::from(self.default_fade_time)))
            }
            p if p == PropertyIdentifier::DEFAULT_RAMP_RATE => {
                Ok(PropertyValue::Real(self.default_ramp_rate))
            }
            p if p == PropertyIdentifier::DEFAULT_STEP_INCREMENT => {
                Ok(PropertyValue::Real(self.default_step_increment))
            }
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p => color_link::read(self.color_link.as_ref(), p)
                .or_else(|| self.read_trim(p))
                .or_else(|| self.profile.read(p, array_index))
                .unwrap_or_else(|| Err(common::unknown_property_error())),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // A level is normalized before it reaches the slot, so the slot,
        // Present_Value, Tracking_Value and a COV report all see 1.0 for a
        // write between 0.0 and 1.0. -1.0, -2.0 and -3.0 are the warn
        // commands (Table 12-65) and never reach a slot themselves.
        if property == PropertyIdentifier::PRESENT_VALUE {
            let priority = priority.unwrap_or(16);
            if !(1..=16).contains(&priority) {
                return Err(common::value_out_of_range_error());
            }
            let write = engine::PresentValueWrite::decode(value)?;
            let now = self.now();
            self.write_present_value(priority, write, now);
            return Ok(());
        }

        // LIGHTING_COMMAND: a BACnetLightingCommand, checked against its
        // operation, then carried out.
        if property == PropertyIdentifier::LIGHTING_COMMAND {
            let command = command::decode_write(value)?;
            self.take_lighting_command(command);
            return Ok(());
        }

        // LIGHTING_COMMAND_DEFAULT_PRIORITY: 1 to 16, but not 6. Clause
        // 12.54.27 keeps 6 out: Table 19-1 gives that slot to Minimum On/Off,
        // and a command written without a priority acts at this one.
        if property == PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY {
            if let PropertyValue::Unsigned(v) = value {
                if !(1..=16).contains(&v) || v == 6 {
                    return Err(common::value_out_of_range_error());
                }
                self.lighting_command_default_priority = v as u32;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // RELINQUISH_DEFAULT — writable per Table 12-64 (R; the standard
        // permits writability), validated the same way a commanded
        // Present_Value is by the shared setter.
        if property == PropertyIdentifier::RELINQUISH_DEFAULT {
            if let PropertyValue::Real(f) = value {
                return self.set_relinquish_default(f);
            }
            return Err(common::invalid_data_type_error());
        }

        // BLINK_WARN_ENABLE
        if property == PropertyIdentifier::BLINK_WARN_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.blink_warn_enable = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // EGRESS_TIME
        if property == PropertyIdentifier::EGRESS_TIME {
            if let PropertyValue::Unsigned(v) = value {
                self.egress_time = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // DEFAULT_FADE_TIME, DEFAULT_RAMP_RATE and DEFAULT_STEP_INCREMENT go
        // through the range-checked setters. A fade time too large for the
        // setter's u32 is past the range too.
        if property == PropertyIdentifier::DEFAULT_FADE_TIME {
            if let PropertyValue::Unsigned(v) = value {
                return self.set_default_fade_time(common::u64_to_u32(v)?);
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::DEFAULT_RAMP_RATE {
            if let PropertyValue::Real(v) = value {
                return self.set_default_ramp_rate(v);
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::DEFAULT_STEP_INCREMENT {
            if let PropertyValue::Real(v) = value {
                return self.set_default_step_increment(v);
            }
            return Err(common::invalid_data_type_error());
        }

        // HIGH_END_TRIM, LOW_END_TRIM and TRIM_FADE_TIME, and the colour
        // links, once present.
        if let Some(result) = self.write_trim(property, &value) {
            return result;
        }
        if let Some(result) = color_link::write(&mut self.color_link, property, &value) {
            return result;
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
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

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_lighting_output_object(self)
    }

    fn cov_increment(&self) -> Option<f64> {
        Some(f64::from(self.cov_increment))
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.advance_to(self.logical_now)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic_clock = clock;
    }

    fn bind_deadline_waker_internal(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.deadline_waker = waker;
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.advance_to(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.next_deadline()
    }

    fn set_tracking_cov_increment_internal(&mut self, finest: Option<f64>) {
        self.finest_tracking_increment = finest;
    }

    /// A copy that reads as the object does now: its clock stops at this
    /// instant, so a fade's Tracking_Value in a COV report is the value at
    /// the moment the report was taken.
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        let mut snapshot = self.clone();
        snapshot.logical_now = self.now();
        snapshot.monotonic_clock = None;
        snapshot.deadline_waker = None;
        Some(Box::new(snapshot))
    }

    fn lighting_blink_count_internal(&self) -> u64 {
        self.blink_request_count
    }
}

mod binary;
mod color_link;
mod command;
mod engine;
mod metadata;
mod trim;
pub use binary::BinaryLightingOutputObject;
pub use color_link::{ColorLink, ColorOverride, LightingColor, OutputColor};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

#[cfg(test)]
mod required_rows_tests;

#[cfg(test)]
mod command_tests;

#[cfg(test)]
mod present_value_tests;

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod warn_tests;

#[cfg(test)]
mod trim_tests;

#[cfg(test)]
mod color_link_tests;
