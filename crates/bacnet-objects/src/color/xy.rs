//! The Color object (type 63, Addendum 135-2020ca Clause 12.X): a CIE 1931
//! xy colour.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::{ColorOperation, ColorTransition, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::engine::Engine;
use super::{
    command, metadata, milliseconds, noncommandable_audit_policy, written_transition,
    written_unsigned, written_xy, xy_value,
};
use crate::audit::{AuditPolicyAuthority, ObjectAuditPolicy};
use crate::command_source::{CommandOrigin, SingleValueSource};
use crate::common::{self, read_identity_properties};
use crate::traits::{BACnetObject, DeadlineWaker, MonotonicClock};
use crate::transition::Transition;

/// D65 white, where a new Color object starts.
const D65: BACnetXyColor = BACnetXyColor::new(0.3127, 0.3290);

/// How far a fading colour moves between COV samples of Tracking_Value: a
/// thousandth of the 0.0 to 1.0 coordinate range, measured along the fade's
/// straight line on the xy diagram. That is about the size of the smallest
/// colour differences an eye tells apart, and the 100 ms sample grid bounds
/// how often a short fade is sampled.
const XY_SAMPLE_STEP: f64 = 0.001;

/// BACnet Color object (type 63).
///
/// Holds a colour as CIE 1931 xy coordinates, each 0.0 to 1.0. It isn't
/// commandable: Present_Value takes a write with no priority array.
///
/// Color_Command takes a [`BACnetColorCommand`] whose operation is
/// FADE_TO_COLOR or STOP (see [`set_color_command`](Self::set_color_command))
/// and carries it out (#1474). FADE_TO_COLOR sets Present_Value to the target
/// at once and fades Tracking_Value to it from where it stands, over the
/// command's fade time or Default_Fade_Time, with In_Progress FADE_ACTIVE
/// until it arrives. STOP ends the fade there, and Present_Value takes the
/// colour it reached. A Present_Value write halts a fade too, then moves to
/// the new colour as Transition says: at once for NONE, or a fade over
/// Default_Fade_Time for FADE.
///
/// Fades run on the monotonic clock the database binds; the server's
/// monotonic task samples Tracking_Value for COV as it moves and ends the
/// fade on time. With no clock bound they wait on `advance_time_internal`.
#[derive(Clone)]
pub struct ColorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present_Value: the colour the output is at, or fading to.
    present_value: BACnetXyColor,
    /// The last command written; operation NONE until then.
    color_command: BACnetColorCommand,
    /// Default_Color: the colour for the output after a restart.
    default_color: BACnetXyColor,
    /// Default_Fade_Time in milliseconds, within 100..=86_400_000.
    default_fade_time: u32,
    /// Transition: NONE or FADE.
    transition: ColorTransition,
    /// Audit_Level and Auditable_Operations, once provisioned.
    audit_policy: ObjectAuditPolicy,
    /// Value_Source, once tracked (#1552).
    value_source: SingleValueSource,
    engine: Engine<BACnetXyColor>,
}

impl ColorObject {
    /// Create a Color object at D65 white (x 0.3127, y 0.3290), which is also
    /// its Default_Color.
    ///
    /// Default_Fade_Time starts at 100 ms, the shortest the addendum allows,
    /// as Lighting Output's does, and Transition at NONE.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COLOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: D65,
            color_command: BACnetColorCommand::new(ColorOperation::NONE),
            default_color: D65,
            default_fade_time: *command::FADE_TIME_MS.start(),
            transition: ColorTransition::NONE,
            audit_policy: ObjectAuditPolicy::default(),
            value_source: SingleValueSource::default(),
            engine: Engine::new(XY_SAMPLE_STEP),
        })
    }

    /// Track Value_Source (Clause 19.5, #1552), provisioned before
    /// registration as the audit policy is.
    ///
    /// On, the object serves Value_Source: the writer of the last
    /// Present_Value write, or of the last Color_Command that set
    /// Present_Value, which alone may then correct it. Such a write must
    /// name its writer (`write_property_from`, as the server's network and
    /// local writes do); one without is refused with WRITE_ACCESS_DENIED.
    /// The typed setters name none, so after one Value_Source reads NONE.
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.value_source.set_enabled(enabled);
    }

    pub(super) fn value_source(&self) -> &SingleValueSource {
        &self.value_source
    }

    /// Provision the optional Audit_Level and Auditable_Operations rows
    /// before registration, as an Analog or Binary Value's are (#1525).
    ///
    /// A field left `None` keeps its row out of the object; a present row is
    /// writable, and a DEFAULT level inherits the selected Audit Reporter's.
    /// The object has no commandable property for Audit_Priority_Filter to
    /// filter, and Table 12-X doesn't list it, so a priority filter in
    /// `policy` is left out. Provisioning doesn't install or enable a
    /// Reporter.
    pub fn set_audit_policy(&mut self, policy: ObjectAuditPolicy) {
        self.audit_policy = noncommandable_audit_policy(policy);
    }

    pub(super) fn audit_policy(&self) -> &ObjectAuditPolicy {
        &self.audit_policy
    }

    /// Set Present_Value, as a WriteProperty of the colour would.
    ///
    /// A coordinate outside 0.0 to 1.0, or NaN, is refused with
    /// VALUE_OUT_OF_RANGE and changes nothing (Clause 12.X.4). Otherwise a
    /// fade in progress is halted, and the output moves to `color` as
    /// Transition says.
    pub fn set_present_value(&mut self, color: BACnetXyColor) -> Result<(), Error> {
        if !command::xy_in_range(color) {
            return Err(common::value_out_of_range_error());
        }
        self.write_present_value(color);
        self.value_source.forget();
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
    /// The command is checked against the Color object's table of colour
    /// commands (Addendum 135-2020ca, Table 12-X2): only FADE_TO_COLOR and
    /// STOP are taken. FADE_TO_COLOR needs a target colour with both
    /// coordinates 0.0 to 1.0, and a fade time, when it carries one, of 100
    /// to 86,400,000 ms. Fields the operation doesn't use are kept unchecked.
    /// A refusal is VALUE_OUT_OF_RANGE and leaves the object unchanged.
    pub fn set_color_command(&mut self, command: BACnetColorCommand) -> Result<(), Error> {
        command::check_color(&command)?;
        if self.take_color_command(command) {
            self.value_source.forget();
        }
        Ok(())
    }

    /// Store a checked command and carry it out now (Table 12-X2). A new
    /// FADE_TO_COLOR or a STOP halts the fade in progress (Clause 12.X.6.1).
    /// Whether it set Present_Value: a STOP with no fade running doesn't.
    fn take_color_command(&mut self, command: BACnetColorCommand) -> bool {
        self.color_command = command;
        let now = self.engine.now();
        match command.operation {
            ColorOperation::FADE_TO_COLOR => {
                let Some(target) = command.target_color else {
                    return false;
                };
                let tracking = self.engine.tracking(self.present_value, now);
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
            ColorOperation::STOP => match self.engine.halt(now) {
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
            self.write_present_value(written_xy(value)?);
            return Ok(true);
        }
        let command = command::decode_write(value, command::check_color)?;
        Ok(self.take_color_command(command))
    }

    /// Halt any fade and move to a written Present_Value as Transition says
    /// (Clause 12.X.11): at once, or a fade over Default_Fade_Time.
    fn write_present_value(&mut self, color: BACnetXyColor) {
        let now = self.engine.now();
        let tracking = self.engine.tracking(self.present_value, now);
        self.present_value = color;
        let fade = (self.transition == ColorTransition::FADE)
            .then(|| Transition::fade(tracking, color, now, milliseconds(self.default_fade_time)));
        self.engine.start(fade.flatten());
    }
}

impl BACnetObject for ColorObject {
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
        // Each read takes its own instant, so one ReadPropertyMultiple that
        // straddles a fade's end may pair a Tracking_Value just short of the
        // target with In_Progress IDLE.
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::COLOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => Ok(xy_value(self.present_value)),
            p if p == PropertyIdentifier::TRACKING_VALUE => Ok(xy_value(
                self.engine.tracking(self.present_value, self.engine.now()),
            )),
            p if p == PropertyIdentifier::COLOR_COMMAND => Ok(command::encode(&self.color_command)),
            p if p == PropertyIdentifier::IN_PROGRESS => Ok(PropertyValue::Enumerated(
                self.engine.in_progress(self.engine.now()).to_raw(),
            )),
            p if p == PropertyIdentifier::DEFAULT_COLOR => Ok(xy_value(self.default_color)),
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                Ok(PropertyValue::Unsigned(u64::from(self.default_fade_time)))
            }
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
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE
                || p == PropertyIdentifier::COLOR_COMMAND =>
            {
                // A tracked source needs the writer (`write_property_from`).
                self.value_source.unsourced()?;
                self.write_output(property, value)?;
            }
            // (0, 0) is taken like any other colour: Clause 12.X.8 has it
            // mean that a restart brings back the colour from before it.
            p if p == PropertyIdentifier::DEFAULT_COLOR => {
                self.default_color = written_xy(value)?;
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => {
                self.default_fade_time = written_unsigned(value, command::FADE_TIME_MS)?;
            }
            p if p == PropertyIdentifier::TRANSITION => {
                self.transition = written_transition(value, ColorTransition::FADE)?;
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
        metadata::for_color_object(self)
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
        self.engine.advance_to(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.engine.deadline()
    }

    /// A copy that reads as the object does now: its clock stops at this
    /// instant, so a fade's Tracking_Value in a COV report is the value at
    /// the moment the report was taken.
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(Self {
            engine: self.engine.frozen(),
            ..self.clone()
        }))
    }
}
