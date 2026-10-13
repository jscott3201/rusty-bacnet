use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};

use super::color_link::{self, ColorLink};
use crate::common::{self, read_common_properties, read_priority_array};
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::traits::{BACnetObject, MonotonicClock};

const OFF: u32 = 0;
const ON: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    WarnOff,
    WarnRelinquish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveOperation {
    kind: OperationKind,
    priority: u8,
    deadline: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentValueCommand {
    Set(Option<u32>),
    Warn,
    WarnOff,
    WarnRelinquish,
    Stop,
}

/// BACnet Binary Lighting Output object.
///
/// The priority array stores only steady OFF/ON values. WARN, WARN_OFF,
/// WARN_RELINQUISH, and STOP are command operations interpreted at write time.
///
/// Cloning copies served state into an in-memory object. Tags persistence and
/// pending saves remain solely with the original; cloning or dropping the copy
/// never saves, corrects, settles, or waits on the original writer.
#[derive(Clone)]
pub struct BinaryLightingOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    blink_warn_enable: bool,
    egress_time: u32,
    active_operation: Option<ActiveOperation>,
    blink_request_count: u64,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    logical_now: Duration,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Color_Reference, with Color_Override and Override_Color_Reference
    /// when overridable; absent until set.
    color_link: Option<ColorLink>,
    /// Tags, Profile_Location and Profile_Name, once provisioned (#1553).
    profile: ProfileState,
}

impl BinaryLightingOutputObject {
    /// Create a new Binary Lighting Output object with no reconstructed egress.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::BINARY_LIGHTING_OUTPUT, instance)?,
            name: name.into(),
            description: String::new(),
            present_value: OFF,
            blink_warn_enable: false,
            egress_time: 0,
            active_operation: None,
            blink_request_count: 0,
            monotonic_clock: None,
            logical_now: Duration::ZERO,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            priority_array: [None; 16],
            relinquish_default: OFF,
            color_link: None,
            profile: ProfileState::default(),
        })
    }

    /// Link the output to the colour objects that set its colour, or take
    /// the link away with `None`, as a Lighting Output's
    /// [`set_color_link`](super::LightingOutputObject::set_color_link) does
    /// (#1527).
    pub fn set_color_link(&mut self, link: Option<ColorLink>) -> Result<(), Error> {
        self.color_link = link.map(ColorLink::checked).transpose()?;
        Ok(())
    }

    /// The colour link, if one is set.
    pub fn color_link(&self) -> Option<&ColorLink> {
        self.color_link.as_ref()
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
    /// before registration (#1553), as a Color object's
    /// [`set_profile`](crate::color::ColorObject::set_profile) does.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    pub(super) fn profile(&self) -> &ObjectProfile {
        self.profile.profile()
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set Relinquish_Default to OFF (0) or ON (1) (#270, #283).
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if !matches!(value, OFF | ON) {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    fn validate_present_value_command(
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(u8, PresentValueCommand), Error> {
        let priority = priority.unwrap_or(16);
        if !(1..=16).contains(&priority) {
            return Err(common::value_out_of_range_error());
        }
        let command = match value {
            PropertyValue::Null => PresentValueCommand::Set(None),
            PropertyValue::Enumerated(OFF) => PresentValueCommand::Set(Some(OFF)),
            PropertyValue::Enumerated(ON) => PresentValueCommand::Set(Some(ON)),
            PropertyValue::Enumerated(2) => PresentValueCommand::Warn,
            PropertyValue::Enumerated(3) => PresentValueCommand::WarnOff,
            PropertyValue::Enumerated(4) => PresentValueCommand::WarnRelinquish,
            PropertyValue::Enumerated(5) => PresentValueCommand::Stop,
            PropertyValue::Enumerated(_) => return Err(common::value_out_of_range_error()),
            _ => return Err(common::invalid_data_type_error()),
        };
        Ok((priority, command))
    }

    fn highest_active_priority(&self) -> Option<u8> {
        self.priority_array
            .iter()
            .position(Option::is_some)
            .map(|index| index as u8 + 1)
    }

    fn priority_is_highest_on(&self, priority: u8) -> bool {
        self.highest_active_priority() == Some(priority)
            && self.priority_array[(priority - 1) as usize] == Some(ON)
    }

    fn next_effective_value(&self, priority: u8) -> u32 {
        self.priority_array[priority as usize..]
            .iter()
            .flatten()
            .next()
            .copied()
            .unwrap_or(self.relinquish_default)
    }

    fn request_blink(&mut self) {
        self.blink_request_count = self.blink_request_count.saturating_add(1);
    }

    fn apply_terminal(&mut self, operation: ActiveOperation) {
        let slot = &mut self.priority_array[(operation.priority - 1) as usize];
        match operation.kind {
            OperationKind::WarnOff => *slot = Some(OFF),
            OperationKind::WarnRelinquish => *slot = None,
        }
        self.recalculate_present_value();
    }

    fn complete_active_operation(&mut self) {
        if let Some(operation) = self.active_operation.take() {
            self.apply_terminal(operation);
        }
    }

    fn complete_active_operation_for_write(&mut self, priority: u8) {
        if self
            .active_operation
            .is_some_and(|active| priority <= active.priority)
        {
            self.complete_active_operation();
        }
    }

    fn arm_operation(&mut self, kind: OperationKind, priority: u8) {
        let duration = Duration::from_secs(self.egress_time as u64);
        let operation = ActiveOperation {
            kind,
            priority,
            deadline: self.monotonic_now().saturating_add(duration),
        };
        if duration.is_zero() {
            self.apply_terminal(operation);
        } else {
            self.active_operation = Some(operation);
        }
    }

    fn write_present_value(&mut self, priority: u8, command: PresentValueCommand) {
        if command != PresentValueCommand::Stop {
            self.complete_active_operation_for_write(priority);
        }

        let index = (priority - 1) as usize;
        match command {
            PresentValueCommand::Set(value) => {
                self.priority_array[index] = value;
                self.recalculate_present_value();
            }
            PresentValueCommand::Warn => {
                if self.blink_warn_enable && self.priority_is_highest_on(priority) {
                    self.request_blink();
                }
            }
            PresentValueCommand::WarnOff => {
                if self.blink_warn_enable && self.priority_is_highest_on(priority) {
                    self.request_blink();
                    self.arm_operation(OperationKind::WarnOff, priority);
                } else {
                    self.priority_array[index] = Some(OFF);
                    self.recalculate_present_value();
                }
            }
            PresentValueCommand::WarnRelinquish => {
                let eligible = self.blink_warn_enable
                    && self.priority_is_highest_on(priority)
                    && self.next_effective_value(priority) != ON;
                if eligible {
                    self.request_blink();
                    self.arm_operation(OperationKind::WarnRelinquish, priority);
                } else {
                    self.priority_array[index] = None;
                    self.recalculate_present_value();
                }
            }
            PresentValueCommand::Stop => {
                if self
                    .active_operation
                    .is_some_and(|active| active.priority == priority)
                {
                    self.active_operation = None;
                }
            }
        }
    }

    fn monotonic_now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    fn expire_at(&mut self, now: Duration) -> bool {
        let Some(operation) = self.active_operation else {
            return false;
        };
        if now < operation.deadline {
            return false;
        }
        self.active_operation = None;
        self.apply_terminal(operation);
        true
    }
}

impl BACnetObject for BinaryLightingOutputObject {
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
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::BINARY_LIGHTING_OUTPUT.to_raw(),
            )),
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::Enumerated(self.present_value)),
            PropertyIdentifier::BLINK_WARN_ENABLE => {
                Ok(PropertyValue::Boolean(self.blink_warn_enable))
            }
            PropertyIdentifier::EGRESS_TIME => Ok(PropertyValue::Unsigned(self.egress_time as u64)),
            PropertyIdentifier::EGRESS_ACTIVE => {
                Ok(PropertyValue::Boolean(self.active_operation.is_some()))
            }
            PropertyIdentifier::PRIORITY_ARRAY => {
                read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default))
            }
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p => color_link::read(self.color_link.as_ref(), p)
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
        if property == PropertyIdentifier::PRESENT_VALUE {
            let (priority, command) = Self::validate_present_value_command(value, priority)?;
            self.write_present_value(priority, command);
            return Ok(());
        }
        if property == PropertyIdentifier::BLINK_WARN_ENABLE {
            if let PropertyValue::Boolean(value) = value {
                self.blink_warn_enable = value;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::EGRESS_TIME {
            if let PropertyValue::Unsigned(value) = value {
                self.egress_time = common::u64_to_u32(value)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::RELINQUISH_DEFAULT {
            if let PropertyValue::Enumerated(value) = value {
                return self.set_relinquish_default(value);
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = color_link::write(&mut self.color_link, property, &value) {
            return result;
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
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

    fn durable_writes_internal(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.profile.capability()
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata::for_binary_lighting_output_object(self)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.expire_at(self.logical_now)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic_clock = clock;
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.profile.expire(now);
        self.expire_at(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.active_operation.map(|operation| operation.deadline)
    }

    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(self.clone()))
    }

    fn lighting_blink_count_internal(&self) -> u64 {
        self.blink_request_count
    }
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "binary_direct_tests.rs"]
mod direct_tests;
