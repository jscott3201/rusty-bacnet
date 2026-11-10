//! The Color object (type 63, Addendum 135-2020ca Clause 12.X): a CIE 1931
//! xy colour.

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::{ColorOperation, ColorTransition, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::engine::Engine;
use super::persistence::{ColorOutputPersistence, ColorOutputValue};
use super::saving::{ColorOutputSaveCounters, OutputStorage, impl_color_durable};
use super::{
    command, metadata, milliseconds, noncommandable_audit_policy, written_transition,
    written_unsigned, written_xy, xy_value,
};
use crate::audit::ObjectAuditPolicy;
use crate::command_source::{CommandOrigin, SingleValueSource};
use crate::common::{self, read_identity_properties};
use crate::durable::StageStep;
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::traits::BACnetObject;
use crate::transition::Transition;

/// A written Default_Color as storage keeps it.
fn written_default(value: &PropertyValue) -> Result<ColorOutputValue, Error> {
    written_xy(value.clone()).map(ColorOutputValue::Xy)
}

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
/// fade on time. With no clock bound they wait on the test-only
/// `advance_time`.
///
/// Cloning copies served state into an in-memory object. Tags persistence and
/// pending saves remain solely with the original; cloning or dropping the copy
/// never saves, corrects, settles, or waits on the original writer.
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
    /// Tags, Profile_Location and Profile_Name, once provisioned (#1553).
    profile: ProfileState,
    /// Present_Value storage, once taken (#1526).
    output: OutputStorage,
    /// Whether Present_Value came back from that storage.
    restored: bool,
    /// The colour storage brought back, until a start takes it.
    to_restore: Option<BACnetXyColor>,
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
            profile: ProfileState::default(),
            output: OutputStorage::default(),
            restored: false,
            to_restore: None,
            engine: Engine::new(XY_SAMPLE_STEP),
        })
    }

    /// Keep Present_Value and a written Default_Color in `persistence`
    /// (#1526, #1956), and take what it saved for this object, if anything:
    /// the colour as Present_Value, and a default a write set as
    /// Default_Color. Call it right after building the object, before
    /// anything writes Present_Value.
    ///
    /// The object then saves each new Present_Value on its own writer
    /// thread; a change doesn't wait for its save, and a save that fails is
    /// logged and tried again (see [`ColorOutputPersistence`]). A
    /// Default_Color write saves first and is refused when the save fails,
    /// and so does a Description write (#2364), whose saved text is served
    /// from the start.
    /// With Default_Color (0, 0), the device's start keeps the saved colour
    /// (Clause 12.X.8).
    ///
    /// Fails with storage's error when what it holds can't be loaded, and
    /// when it holds a colour temperature, a coordinate outside 0.0 to 1.0
    /// or a Description past 16 KiB.
    pub fn with_output_persistence(
        mut self,
        persistence: Arc<dyn ColorOutputPersistence>,
    ) -> Result<Self, Error> {
        let Some(saved) = self.output.open(self.oid, persistence)? else {
            return Ok(self);
        };
        let xy = |value| match value {
            Some(ColorOutputValue::Xy(color)) if command::xy_in_range(color) => Ok(Some(color)),
            None => Ok(None),
            Some(_) => Err(Error::Encoding(
                "saved Color output is not a colour the object can hold".into(),
            )),
        };
        if let Some(color) = xy(saved.present_value)? {
            self.present_value = color;
            self.restored = true;
            self.to_restore = Some(color);
        }
        if let Some(color) = xy(saved.default)? {
            self.default_color = color;
        }
        if let Some(text) = saved.description {
            self.description = text;
        }
        Ok(self)
    }

    /// Whether Present_Value came back from storage
    /// ([`with_output_persistence`](Self::with_output_persistence)).
    pub fn output_restored(&self) -> bool {
        self.restored
    }

    /// The Present_Value save counters, shared with the object, or `None`
    /// without storage. Take them before the object joins a database.
    pub fn output_save_counters(&self) -> Option<ColorOutputSaveCounters> {
        self.output.counters()
    }

    /// Block until the Present_Value saves queued so far have run. Dropping
    /// the object waits for them too.
    pub fn wait_for_output_saves(&self) {
        self.output.wait_for_saves();
    }

    /// Default_Color: the colour the device's start drives the output to,
    /// or (0, 0) to keep the colour from before the restart.
    pub fn default_color(&self) -> BACnetXyColor {
        self.default_color
    }

    /// Set Default_Color, as a WriteProperty of it would. (0, 0) is taken
    /// like any other colour and keeps the saved colour at the device's
    /// start (Clause 12.X.8). A coordinate outside 0.0 to 1.0, or NaN, is
    /// refused with VALUE_OUT_OF_RANGE.
    ///
    /// This configures the default; it is not saved. Once storage holds a
    /// default a write set ([`with_output_persistence`](Self::with_output_persistence)),
    /// that one wins, so an application that sets its configuration at each
    /// start keeps a peer's write: the colour is checked but not taken, and
    /// [`clear_written_default`](Self::clear_written_default) lets it apply
    /// again.
    pub fn set_default_color(&mut self, color: BACnetXyColor) -> Result<(), Error> {
        if !command::xy_in_range(color) {
            return Err(common::value_out_of_range_error());
        }
        if !self.output.default_saved() {
            self.default_color = color;
        }
        Ok(())
    }

    /// Whether storage holds a Default_Color a write set, now or before a
    /// restart, which [`set_default_color`](Self::set_default_color) then
    /// leaves alone. Always false without storage.
    pub fn default_color_saved(&self) -> bool {
        self.output.default_saved()
    }

    /// Drop the Default_Color a write saved (#1956): storage keeps none
    /// from now on, so [`set_default_color`](Self::set_default_color)
    /// applies again. The default served now stays until it, or a write,
    /// changes it. Saves before it returns; a save that fails keeps the
    /// written default and returns storage's error. Does nothing without
    /// storage or a written default.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_default(&mut self) -> Result<(), Error> {
        self.output.clear_written_default()
    }

    /// Drop the Description a write saved (#2364): storage keeps none from
    /// now on, and a rebuilt object starts without it. The text served now
    /// stays until a write changes it. Saves before it returns; a save that
    /// fails keeps the written Description and returns storage's error.
    /// Does nothing without storage or a saved Description.
    /// [`clear_written_default`](Self::clear_written_default) leaves a
    /// saved Description alone.
    /// Call it before the object joins the database, not under the database
    /// guard: it waits for its save.
    pub fn clear_written_description(&mut self) -> Result<(), Error> {
        self.output.clear_written_description()
    }

    /// Stage the request's Default_Color and Description writes on the
    /// output storage.
    fn stage_default(&mut self, writes: &[crate::durable::PendingWrite]) -> StageStep {
        self.output
            .stage_kept(writes, PropertyIdentifier::DEFAULT_COLOR, written_default)
    }

    /// Drive the output as the device's start does (#1526); the server
    /// calls this for every Color object in the database as it starts. An
    /// application running the object without a server, or adding it to a
    /// server already running, calls it itself.
    ///
    /// A non-zero Default_Color becomes Present_Value at once, whatever was
    /// set before the start, with no fade, since the addendum gives the
    /// start no transition. With Default_Color (0, 0) the output keeps, in
    /// order: the colour restored from storage; else a colour set before the
    /// start; else nothing, and In_Progress reads NOT_CONTROLLED until a
    /// Present_Value or Color_Command write drives it (Clauses 12.X.4 and
    /// 12.X.8). Values set before the start are configuration, since no
    /// peer can write until then, so a first boot and later ones agree.
    ///
    /// It is safe to call again, as an in-process restart would: the second
    /// call keeps the current colour for a zero default, since the restored
    /// one was taken by the first.
    pub fn start_output(&mut self) {
        let restored = self.to_restore.take();
        let target = if self.default_color != BACnetXyColor::new(0.0, 0.0) {
            Some(self.default_color)
        } else {
            restored
        };
        // Without a target, a colour set before the start stays, and with
        // none the output stays undriven.
        if let Some(color) = target {
            if color != self.present_value {
                self.value_source.forget();
            }
            self.present_value = color;
            self.engine.start(None);
            self.output_changed();
        }
    }

    /// Present_Value may have changed: save it, if the object keeps it.
    fn output_changed(&mut self) {
        self.output.note(ColorOutputValue::Xy(self.present_value));
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

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while `set_profile` provisions the row. Loaded data must satisfy
    /// the normal Tags rules and the opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name)?;
        object.profile = ProfileState::persistent(object.oid, persistence, false)?;
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

    pub(super) fn audit_overrides(&self) -> &ObjectAuditPolicy {
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
        let set = self.carry_out(command);
        if set {
            self.output_changed();
        }
        set
    }

    /// [`take_color_command`](Self::take_color_command) without the save.
    fn carry_out(&mut self, command: BACnetColorCommand) -> bool {
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
        self.output_changed();
    }
}

impl BACnetObject for ColorObject {
    crate::traits::lend_audit_policy!();

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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        if property == PropertyIdentifier::DESCRIPTION
            && let Some(result) = self.output.write_description(&mut self.description, &value)
        {
            return result;
        }
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
            // (0, 0) is taken like any other colour: Clause 12.X.8 has it
            // mean that a restart brings back the colour from before it. With
            // storage it is saved first (#1956).
            p if p == PropertyIdentifier::DEFAULT_COLOR => {
                let written = self
                    .output
                    .write_default(p, &value, || written_default(&value))?;
                if let ColorOutputValue::Xy(color) = written {
                    self.default_color = color;
                }
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
                ));
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
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

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        if self.output.is_some() {
            return Some(self);
        }
        self.profile.capability()
    }

    crate::traits::lend_cov_reporting!();

    fn timed(&mut self) -> Option<&mut dyn crate::traits::Timed> {
        Some(self)
    }
}

impl crate::traits::CovReporting for ColorObject {}

crate::traits::impl_audit_policy!(ColorObject);

impl crate::traits::Timed for ColorObject {
    fn bind_clocks(&mut self, clocks: &crate::traits::Clocks) {
        self.engine.bind_clock(clocks.monotonic.clone());
        self.engine.bind_waker(clocks.deadline_waker.clone());
    }

    fn start_output(&mut self) {
        self.start_output();
    }

    fn advance_monotonic_time(&mut self, now: Duration) -> bool {
        self.engine.advance_to(now)
    }

    fn next_monotonic_deadline(&self) -> Option<Duration> {
        self.engine.deadline()
    }

    /// A copy that reads as the object does now: its clock stops at this
    /// instant, so a fade's Tracking_Value in a COV report is the value at
    /// the moment the report was taken.
    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(Self {
            engine: self.engine.frozen(),
            ..self.clone()
        }))
    }
}

#[cfg(test)]
impl ColorObject {
    pub(crate) fn advance_time(&mut self, elapsed: Duration) -> bool {
        self.engine.advance_by(elapsed)
    }

    /// Whether the output writer's thread has ended.
    #[cfg(feature = "std")]
    pub(super) fn writer_thread_ended(&self) -> bool {
        self.output.writer_thread_ended()
    }
}

impl_color_durable!(ColorObject);
