//! The Color Temperature object (type 64, Addendum 135-2020ca Clause 12.Y):
//! a correlated colour temperature in kelvin.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::constructed::BACnetColorCommand;
use bacnet_types::enums::{ColorOperation, ColorTransition, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::engine::Engine;
use super::{
    command, metadata, milliseconds, noncommandable_audit_policy, written_transition,
    written_unsigned,
};
use crate::audit::{AuditPolicyAuthority, ObjectAuditPolicy};
use crate::command_source::{CommandOrigin, SingleValueSource};
use crate::common::{self, read_identity_properties};
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::traits::{BACnetObject, DeadlineWaker, MonotonicClock};
use crate::transition::Transition;

/// How far a fading or ramping colour temperature moves between COV samples
/// of Tracking_Value, in kelvin. Ten kelvin is fine enough that a
/// subscriber's own increment of a few tens of kelvin is met closely, and the
/// 100 ms sample grid bounds how often a fast ramp is sampled.
const KELVIN_SAMPLE_STEP: f64 = 10.0;

/// BACnet Color Temperature object (type 64).
///
/// Holds a correlated colour temperature in kelvin, 1000 to 30000 and within
/// Min_Pres_Value and Max_Pres_Value. It isn't commandable: Present_Value
/// takes a write with no priority array, clamping a value between 1000 and
/// Min_Pres_Value up to the minimum, and one between Max_Pres_Value and
/// 30000 down to the maximum (Clause 12.Y.4).
///
/// Color_Command takes a [`BACnetColorCommand`] whose operation is
/// FADE_TO_CCT, RAMP_TO_CCT, STEP_UP_CCT, STEP_DOWN_CCT or STOP (see
/// [`set_color_command`](Self::set_color_command)) and carries it out
/// (#1474). FADE_TO_CCT and RAMP_TO_CCT set Present_Value to the target,
/// clamped to the limits, and move Tracking_Value to it from where it
/// stands, over the command's fade time or at its ramp rate (the defaults
/// when it carries none), with In_Progress FADE_ACTIVE or RAMP_ACTIVE until
/// it arrives. The step operations set Present_Value at once to
/// Tracking_Value plus or minus the step increment, clamped to the limits.
/// STOP ends a fade or ramp there, and Present_Value takes the temperature it
/// reached. A Present_Value write halts a fade or ramp too, then moves to the
/// new temperature as Transition says: at once for NONE, a fade over
/// Default_Fade_Time for FADE, or a ramp at Default_Ramp_Rate for RAMP.
///
/// Fades and ramps run on the monotonic clock the database binds, as a
/// Color object's fades do.
///
/// Cloning copies served state into an in-memory object. Tags persistence and
/// pending saves remain solely with the original; cloning or dropping the copy
/// never saves, corrects, settles, or waits on the original writer.
#[derive(Clone)]
pub struct ColorTemperatureObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value in kelvin: the temperature the output is at, or moving
    /// to.
    present_value: u32,
    /// The last command written; operation NONE until then.
    color_command: BACnetColorCommand,
    /// Default_Color_Temperature in kelvin, or 0.
    default_color_temperature: u32,
    /// Default_Fade_Time in milliseconds, within 100..=86_400_000.
    default_fade_time: u32,
    /// Default_Ramp_Rate in kelvin per second, within 1..=30_000.
    default_ramp_rate: u32,
    /// Default_Step_Increment in kelvin, within 1..=30_000.
    default_step_increment: u32,
    /// Transition: NONE, FADE or RAMP.
    transition: ColorTransition,
    /// Min_Pres_Value and Max_Pres_Value, within 1000..=30_000.
    min_pres_value: u32,
    max_pres_value: u32,
    /// Audit_Level and Auditable_Operations, once provisioned.
    audit_policy: ObjectAuditPolicy,
    /// Value_Source, once tracked (#1552).
    value_source: SingleValueSource,
    /// Tags, Profile_Location and Profile_Name, once provisioned (#1553).
    profile: ProfileState,
    engine: Engine<u32>,
}

impl ColorTemperatureObject {
    /// Create a Color Temperature object at 4000 K (neutral white), which is
    /// also its Default_Color_Temperature.
    ///
    /// Min_Pres_Value and Max_Pres_Value start at the full 1000 to 30000 K
    /// range, Default_Fade_Time at 100 ms (as a Color object's does),
    /// Default_Ramp_Rate at 100 K/s, Default_Step_Increment at 50 K and
    /// Transition at NONE.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 4000,
            color_command: BACnetColorCommand::new(ColorOperation::NONE),
            default_color_temperature: 4000,
            default_fade_time: *command::FADE_TIME_MS.start(),
            default_ramp_rate: 100,
            default_step_increment: 50,
            transition: ColorTransition::NONE,
            min_pres_value: *command::KELVIN.start(),
            max_pres_value: *command::KELVIN.end(),
            audit_policy: ObjectAuditPolicy::default(),
            value_source: SingleValueSource::default(),
            profile: ProfileState::default(),
            engine: Engine::new(KELVIN_SAMPLE_STEP),
        })
    }

    /// Track Value_Source (Clause 19.5, #1552), as a Color object's
    /// [`set_value_source_tracking`](super::ColorObject::set_value_source_tracking)
    /// does. A `set_min_max` that moves Present_Value leaves no source
    /// either.
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.value_source.set_enabled(enabled);
    }

    pub(super) fn value_source(&self) -> &SingleValueSource {
        &self.value_source
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while `set_profile` provisions the row. Loaded data must satisfy
    /// the normal Tags rules and the opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name)?;
        object.profile = ProfileState::persistent(object.oid, persistence)?;
        Ok(object)
    }

    /// Wait for queued save attempts to finish. This is not a success receipt;
    /// writes report their own save outcomes. Unstaged local writes block.
    pub fn wait_for_tag_saves(&self) {
        self.profile.wait_for_saves();
    }

    /// Provision the optional Tags, Profile_Location and Profile_Name rows
    /// before registration (#1553; see [`ObjectProfile`]). Tags takes
    /// writes; the profile rows are read-only over the network. A profile
    /// that fails [`ObjectProfile::check`] is refused and changes nothing.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    pub(super) fn profile(&self) -> &ObjectProfile {
        self.profile.profile()
    }

    /// Provision the optional Audit_Level and Auditable_Operations rows
    /// before registration, as a Color object's are (#1525). Table 12-Y has
    /// no Audit_Priority_Filter either, so a priority filter in `policy` is
    /// left out.
    pub fn set_audit_policy(&mut self, policy: ObjectAuditPolicy) {
        self.audit_policy = noncommandable_audit_policy(policy);
    }

    pub(super) fn audit_policy(&self) -> &ObjectAuditPolicy {
        &self.audit_policy
    }

    /// Set Present_Value in kelvin, as a WriteProperty of it would.
    ///
    /// A value outside 1000 to 30000 is refused with VALUE_OUT_OF_RANGE and
    /// changes nothing; one inside is clamped to Min_Pres_Value and
    /// Max_Pres_Value (Clause 12.Y.4). A fade or ramp in progress is then
    /// halted, and the output moves to the value as Transition says.
    pub fn set_present_value(&mut self, kelvin: u32) -> Result<(), Error> {
        let kelvin = self.written_kelvin(u64::from(kelvin))?;
        self.write_present_value(kelvin);
        self.value_source.forget();
        Ok(())
    }

    /// Set Min_Pres_Value and Max_Pres_Value, in kelvin.
    ///
    /// Clauses 12.Y.13 and 12.Y.14 keep them within 1000 to 30000; a minimum
    /// below 1000, a maximum above 30000 or a minimum above the maximum is
    /// refused with VALUE_OUT_OF_RANGE and changes nothing. A Present_Value
    /// outside the new limits moves to the nearer one at once, halting any
    /// fade or ramp, and a non-zero Default_Color_Temperature is clamped the
    /// same way.
    pub fn set_min_max(&mut self, min: u32, max: u32) -> Result<(), Error> {
        if !command::KELVIN.contains(&min) || !command::KELVIN.contains(&max) || min > max {
            return Err(common::value_out_of_range_error());
        }
        self.min_pres_value = min;
        self.max_pres_value = max;
        let clamped = self.present_value.clamp(min, max);
        if clamped != self.present_value {
            self.engine.halt(self.engine.now());
            self.present_value = clamped;
            self.value_source.forget();
        }
        if self.default_color_temperature != 0 {
            self.default_color_temperature = self.default_color_temperature.clamp(min, max);
        }
        Ok(())
    }

    /// The last command written to Color_Command, or operation NONE with no
    /// other field before any write.
    pub fn color_command(&self) -> BACnetColorCommand {
        self.color_command
    }

    /// Set Color_Command and carry the command out, as a WriteProperty of
    /// the encoded command would.
    ///
    /// The command is checked against the Color Temperature object's table
    /// of colour commands (Addendum 135-2020ca, Table 12-Y2): FADE_TO_CCT,
    /// RAMP_TO_CCT, STEP_UP_CCT, STEP_DOWN_CCT and STOP are taken.
    /// FADE_TO_CCT and RAMP_TO_CCT need a target colour temperature of 1000
    /// to 30000 K. A field the operation uses must be in range: fade time
    /// (FADE_TO_CCT) 100 to 86,400,000 ms, ramp rate (RAMP_TO_CCT) 1 to
    /// 30000 K/s, step increment (the step operations) 1 to 30000 K. Fields
    /// the operation doesn't use are kept unchecked. A refusal is
    /// VALUE_OUT_OF_RANGE and leaves the object unchanged.
    pub fn set_color_command(&mut self, command: BACnetColorCommand) -> Result<(), Error> {
        command::check_color_temperature(&command)?;
        if self.take_color_command(command) {
            self.value_source.forget();
        }
        Ok(())
    }

    /// A written temperature: refused outside 1000 to 30000 K, otherwise
    /// clamped to Min_Pres_Value and Max_Pres_Value (Clause 12.Y.4).
    fn written_kelvin(&self, kelvin: u64) -> Result<u32, Error> {
        u32::try_from(kelvin)
            .ok()
            .filter(|kelvin| command::KELVIN.contains(kelvin))
            .map(|kelvin| self.clamped(kelvin))
            .ok_or_else(common::value_out_of_range_error)
    }

    fn clamped(&self, kelvin: u32) -> u32 {
        kelvin.clamp(self.min_pres_value, self.max_pres_value)
    }

    /// Store a checked command and carry it out now (Table 12-Y2). Each
    /// operation but STOP replaces the fade or ramp in progress, and STOP
    /// ends it (Clause 12.Y.6.1). Whether it set Present_Value: a STOP with
    /// nothing moving doesn't.
    fn take_color_command(&mut self, command: BACnetColorCommand) -> bool {
        self.color_command = command;
        let now = self.engine.now();
        let tracking = self.engine.tracking(self.present_value, now);
        let target = command
            .target_color_temperature
            .map(|kelvin| self.clamped(kelvin));
        match (command.operation, target) {
            (ColorOperation::FADE_TO_CCT, Some(target)) => {
                let fade_time = command.fade_time.unwrap_or(self.default_fade_time);
                self.present_value = target;
                self.engine.start(Transition::fade(
                    tracking,
                    target,
                    now,
                    milliseconds(fade_time),
                ));
                true
            }
            (ColorOperation::RAMP_TO_CCT, Some(target)) => {
                let rate = command.ramp_rate.unwrap_or(self.default_ramp_rate);
                self.present_value = target;
                self.engine
                    .start(Transition::ramp(tracking, target, now, f64::from(rate)));
                true
            }
            (operation @ (ColorOperation::STEP_UP_CCT | ColorOperation::STEP_DOWN_CCT), _) => {
                let increment = command
                    .step_increment
                    .unwrap_or(self.default_step_increment);
                let stepped = if operation == ColorOperation::STEP_UP_CCT {
                    tracking.saturating_add(increment)
                } else {
                    tracking.saturating_sub(increment)
                };
                self.present_value = self.clamped(stepped);
                self.engine.start(None);
                true
            }
            (ColorOperation::STOP, _) => match self.engine.halt(now) {
                Some(reached) => {
                    self.present_value = reached;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// A Present_Value or Color_Command write, the two that can set
    /// Present_Value, and whether it did.
    fn write_output(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<bool, Error> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            let PropertyValue::Unsigned(kelvin) = value else {
                return Err(common::invalid_data_type_error());
            };
            let kelvin = self.written_kelvin(kelvin)?;
            self.write_present_value(kelvin);
            return Ok(true);
        }
        let command = command::decode_write(value, command::check_color_temperature)?;
        Ok(self.take_color_command(command))
    }

    /// Halt any fade or ramp and move to a written Present_Value as
    /// Transition says (Clause 12.Y.15): at once, a fade over
    /// Default_Fade_Time, or a ramp at Default_Ramp_Rate.
    fn write_present_value(&mut self, kelvin: u32) {
        let now = self.engine.now();
        let tracking = self.engine.tracking(self.present_value, now);
        self.present_value = kelvin;
        let transition = match self.transition {
            ColorTransition::FADE => {
                Transition::fade(tracking, kelvin, now, milliseconds(self.default_fade_time))
            }
            ColorTransition::RAMP => {
                Transition::ramp(tracking, kelvin, now, f64::from(self.default_ramp_rate))
            }
            _ => None,
        };
        self.engine.start(transition);
    }
}

impl BACnetObject for ColorTemperatureObject {
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
        if let Some(result) = read_identity_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = self.audit_policy.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.value_source.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.profile.read(property, array_index) {
            return result;
        }
        let unsigned = |value: u32| Ok(PropertyValue::Unsigned(u64::from(value)));
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::COLOR_TEMPERATURE.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => unsigned(self.present_value),
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                unsigned(self.engine.tracking(self.present_value, self.engine.now()))
            }
            p if p == PropertyIdentifier::COLOR_COMMAND => Ok(command::encode(&self.color_command)),
            p if p == PropertyIdentifier::IN_PROGRESS => Ok(PropertyValue::Enumerated(
                self.engine.in_progress(self.engine.now()).to_raw(),
            )),
            p if p == PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE => {
                unsigned(self.default_color_temperature)
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => unsigned(self.default_fade_time),
            p if p == PropertyIdentifier::DEFAULT_RAMP_RATE => unsigned(self.default_ramp_rate),
            p if p == PropertyIdentifier::DEFAULT_STEP_INCREMENT => {
                unsigned(self.default_step_increment)
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => unsigned(self.min_pres_value),
            p if p == PropertyIdentifier::MAX_PRES_VALUE => unsigned(self.max_pres_value),
            p if p == PropertyIdentifier::TRANSITION => {
                Ok(PropertyValue::Enumerated(self.transition.to_raw()))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self
            .audit_policy
            .write(property, array_index, &value, priority)
        {
            return result;
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE
                || p == PropertyIdentifier::COLOR_COMMAND =>
            {
                // A tracked source needs the writer (`write_property_from`).
                self.value_source.unsourced()?;
                self.write_output(property, value)?;
            }
            // Clause 12.Y.8 clamps a write as Present_Value's is, except 0,
            // which the restart rule of Clause 12.Y.4 gives a meaning of its
            // own: bring back the temperature from before the restart.
            p if p == PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE => {
                self.default_color_temperature = match value {
                    PropertyValue::Unsigned(0) => 0,
                    PropertyValue::Unsigned(kelvin) => self.written_kelvin(kelvin)?,
                    _ => return Err(common::invalid_data_type_error()),
                };
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                self.default_fade_time = written_unsigned(value, command::FADE_TIME_MS)?;
            }
            p if p == PropertyIdentifier::DEFAULT_RAMP_RATE => {
                self.default_ramp_rate = written_unsigned(value, command::KELVIN_STEP)?;
            }
            p if p == PropertyIdentifier::DEFAULT_STEP_INCREMENT => {
                self.default_step_increment = written_unsigned(value, command::KELVIN_STEP)?;
            }
            p if p == PropertyIdentifier::TRANSITION => {
                self.transition = written_transition(value, ColorTransition::RAMP)?;
            }
            _ => {
                return Err(common::unhandled_write_error(
                    self.property_metadata().as_ref(),
                    property,
                    array_index,
                ))
            }
        }
        Ok(())
    }

    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        match property {
            PropertyIdentifier::VALUE_SOURCE => {
                self.value_source.correct(array_index, value, origin)
            }
            PropertyIdentifier::PRESENT_VALUE | PropertyIdentifier::COLOR_COMMAND => {
                self.value_source.admit(origin)?;
                if self.write_output(property, value)? {
                    self.value_source.record(origin);
                }
                Ok(())
            }
            _ => self.write_property(property, array_index, value, priority),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_color_temperature_object(self)
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.profile.capability()
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn audit_object_policy_internal(&self) -> ObjectAuditPolicy {
        self.audit_policy
    }

    fn audit_policy_authority_internal(&mut self) -> Option<AuditPolicyAuthority<'_>> {
        Some(AuditPolicyAuthority::new(&mut self.audit_policy))
    }

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.engine.advance_by(elapsed)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.engine.bind_clock(clock);
    }

    fn bind_deadline_waker_internal(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.engine.bind_waker(waker);
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.profile.expire(now);
        self.engine.advance_to(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.engine.deadline()
    }

    fn set_tracking_cov_increment_internal(&mut self, finest: Option<f64>) {
        self.engine.set_finest_increment(finest);
    }

    /// A copy that reads as the object does now; see the Color object's.
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(Self {
            engine: self.engine.frozen(),
            ..self.clone()
        }))
    }
}
