//! Local-target Staging object (type 60) per ASHRAE 135-2020 Clause 12.62.
//!
//! The object owns threshold/hysteresis evaluation only. It queues guarded
//! local target-write plans for the server, which executes them after releasing
//! the source-object database mutation lock.
//!
//! **On removal** (#2265) the object gives up the Present_Value slots its
//! stage writes filled, at each priority it wrote them (`held`). Clause 12.62
//! doesn't cover deletion; a Staging object holds its targets' slots for as
//! long as it exists, as a Schedule does, so it follows the Schedule's
//! release (#2256, #2264), with the same check that the slot is still its own.
//! A plan still being written when the object goes stops at its next target
//! (each write checks the generation under its own guard).

use crate::prelude::*;
use alloc::borrow::Cow;

use bacnet_encoding::constructed::{decode_stage_limit_value, encode_stage_limit_value};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

use crate::common::{self, read_common_properties};
use crate::device_reference;
use crate::property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability};
use crate::traits::{BACnetObject, StagingSource};

mod held;
mod metadata;
use metadata::STAGING_PROPERTY_METADATA;

/// Complete atomic configuration for a [`StagingObject`].
///
/// Target references are deliberately local-only. A reference carrying a
/// `device_identifier` is rejected rather than initiating remote BACnet I/O.
/// The bundled server drops a Device identifier naming its own Device from a
/// written reference before the object sees it (#1136).
#[derive(Debug, Clone, PartialEq)]
pub struct StagingConfig {
    /// Initial REAL `Present_Value`; finite values are clamped to the ladder.
    pub present_value: f32,
    /// `Min_Pres_Value`, strictly below the first stage's lower deadband edge.
    pub min_present_value: f32,
    /// Engineering-units enumeration stored by `Units`.
    pub units: EngineeringUnits,
    /// Command priority used for local target writes (1..=16).
    pub priority_for_writing: u8,
    /// At least two ordered stage-limit values.
    pub stages: Vec<BACnetStageLimitValue>,
    /// Local Binary Output, Binary Value, or Binary Lighting Output targets.
    pub target_references: Vec<BACnetDeviceObjectReference>,
    /// Optional names, exactly one per stage when present.
    pub stage_names: Option<Vec<String>>,
}

/// One server-owned local target mutation selected by a Staging transition.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingTargetWrite {
    /// Local target object.
    pub object_identifier: ObjectIdentifier,
    /// Whether the target receives ACTIVE rather than INACTIVE.
    pub active: bool,
}

/// Guarded work emitted after a Staging source mutation.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingWritePlan {
    /// Staging source whose generation guards completion.
    pub source: ObjectIdentifier,
    /// Source generation that selected this work.
    pub generation: u64,
    /// BACnet priority used for every target write.
    pub priority: u8,
    /// Initialized local targets in configured order.
    pub writes: Vec<StagingTargetWrite>,
}

/// BACnet Staging object with an explicit, validated stage ladder.
pub struct StagingObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    present_stage: u64,
    stages: Vec<BACnetStageLimitValue>,
    stage_names: Option<Vec<String>>,
    target_references: Vec<BACnetDeviceObjectReference>,
    priority_for_writing: u8,
    min_present_value: f32,
    units: EngineeringUnits,
    cov_increment: f32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: Reliability,
    /// The in-service Reliability, set aside while a client simulates one
    /// out of service and put back when Out_Of_Service returns to FALSE.
    reliability_before_out_of_service: Option<Reliability>,
    generation: u64,
    pending_plan: Option<StagingWritePlan>,
    held: held::HeldSlots,
}

impl StagingObject {
    /// Construct an atomically validated Staging object.
    ///
    /// The object begins with `Present_Stage` uninitialized (0). The bundled
    /// server initializes and applies it during startup; a direct
    /// `Present_Value` write also evaluates it. No default ladder or targets
    /// are invented.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        config: StagingConfig,
    ) -> Result<Self, Error> {
        validate_config(&config)?;
        let oid = ObjectIdentifier::new(ObjectType::STAGING, instance)?;
        let present_value = config.present_value.clamp(
            config.min_present_value,
            config.stages.last().unwrap().limit,
        );
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value,
            present_stage: 0,
            stages: config.stages,
            stage_names: config.stage_names,
            target_references: config.target_references,
            priority_for_writing: config.priority_for_writing,
            min_present_value: config.min_present_value,
            units: config.units,
            cov_increment: 0.0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            generation: 0,
            pending_plan: None,
            held: held::HeldSlots::default(),
        })
    }

    fn max_present_value(&self) -> f32 {
        self.stages
            .last()
            .expect("validated Staging configuration has at least two stages")
            .limit
    }

    fn select_stage(&self, value: f32) -> u64 {
        self.stages
            .iter()
            .position(|stage| stage.limit >= value)
            .map_or(self.stages.len() as u64, |index| index as u64 + 1)
    }

    fn retained_by_hysteresis(&self, value: f32) -> bool {
        let Some(index) = self
            .present_stage
            .checked_sub(1)
            .map(|stage| stage as usize)
        else {
            return false;
        };
        let Some(stage) = self.stages.get(index) else {
            return false;
        };
        let lower = if index == 0 {
            self.min_present_value
        } else {
            let prior = &self.stages[index - 1];
            prior.limit - prior.deadband
        };
        value >= lower && value <= stage.limit + stage.deadband
    }

    fn evaluate(&mut self, force_reapply: bool) {
        let previous = self.present_stage;
        if previous != 0 && self.retained_by_hysteresis(self.present_value) && !force_reapply {
            return;
        }
        let selected = self.select_stage(self.present_value);
        self.present_stage = selected;
        if previous != selected || force_reapply {
            self.queue_current_stage_plan();
        }
    }

    fn queue_current_stage_plan(&mut self) {
        self.generation = crate::command::next_generation();
        if self.out_of_service || self.present_stage == 0 {
            self.pending_plan = None;
            return;
        }
        let values = &self.stages[self.present_stage as usize - 1].values;
        let writes = self
            .target_references
            .iter()
            .zip(values)
            .filter(|(reference, _)| {
                reference.object_identifier.instance_number() != ObjectIdentifier::MAX_INSTANCE
            })
            .map(|(reference, active)| StagingTargetWrite {
                object_identifier: reference.object_identifier,
                active: *active,
            })
            .collect();
        let plan = StagingWritePlan {
            source: self.oid,
            generation: self.generation,
            priority: self.priority_for_writing,
            writes,
        };
        self.held.record(&plan);
        self.pending_plan = Some(plan);
    }

    fn reset_and_reevaluate(&mut self) {
        self.present_stage = 0;
        self.present_value = self
            .present_value
            .clamp(self.min_present_value, self.max_present_value());
        self.evaluate(true);
    }

    fn replace_stages(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let mut candidate = self.stages.clone();
        match array_index {
            None => {
                let PropertyValue::List(values) = value else {
                    return Err(common::invalid_data_type_error());
                };
                if values.len() != candidate.len() {
                    return Err(common::value_out_of_range_error());
                }
                candidate = values
                    .into_iter()
                    .map(decode_stage_property_value)
                    .collect::<Result<_, _>>()?;
            }
            Some(0) => return Err(common::write_access_denied_error()),
            Some(index) => {
                let Some(stage) = candidate.get_mut((index - 1) as usize) else {
                    return Err(common::invalid_array_index_error());
                };
                *stage = decode_stage_property_value(value)?;
            }
        }
        validate_ladder(
            &candidate,
            self.min_present_value,
            self.target_references.len(),
        )?;
        if candidate != self.stages {
            self.stages = candidate;
            self.reset_and_reevaluate();
        }
        Ok(())
    }

    fn replace_target_references(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let mut candidate = self.target_references.clone();
        match array_index {
            None => {
                let references: Vec<BACnetDeviceObjectReference> =
                    device_reference::decode_references(&value)?;
                if references.len() != candidate.len() {
                    return Err(common::value_out_of_range_error());
                }
                candidate = references;
            }
            Some(0) => return Err(common::write_access_denied_error()),
            Some(index) => {
                let Some(reference) = candidate.get_mut((index - 1) as usize) else {
                    return Err(common::invalid_array_index_error());
                };
                *reference = device_reference::decode_reference(&value)?;
            }
        }
        validate_target_references(&candidate)?;
        if candidate != self.target_references {
            self.target_references = candidate;
            self.reset_and_reevaluate();
        }
        Ok(())
    }

    fn replace_stage_names(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let Some(names) = &self.stage_names else {
            return Err(if array_index.is_none() {
                common::unknown_property_error()
            } else {
                common::write_access_denied_error()
            });
        };
        let mut candidate = names.clone();
        match array_index {
            None => {
                let PropertyValue::List(values) = value else {
                    return Err(common::invalid_data_type_error());
                };
                if values.len() != candidate.len() {
                    return Err(common::value_out_of_range_error());
                }
                candidate = values
                    .into_iter()
                    .map(|value| match value {
                        PropertyValue::CharacterString(name) => Ok(name),
                        _ => Err(common::invalid_data_type_error()),
                    })
                    .collect::<Result<_, _>>()?;
            }
            Some(0) => return Err(common::write_access_denied_error()),
            Some(index) => {
                let Some(name) = candidate.get_mut((index - 1) as usize) else {
                    return Err(common::invalid_array_index_error());
                };
                let PropertyValue::CharacterString(value) = value else {
                    return Err(common::invalid_data_type_error());
                };
                *name = value;
            }
        }
        self.stage_names = Some(candidate);
        Ok(())
    }

    fn metadata(&self) -> Vec<PropertyMetadata> {
        let mut rows = STAGING_PROPERTY_METADATA.to_vec();
        if self.stage_names.is_some() {
            let target_index = rows
                .iter()
                .position(|row| row.property_identifier == PropertyIdentifier::TARGET_REFERENCES)
                .unwrap();
            rows.insert(
                target_index,
                PropertyMetadata::new(
                    PropertyIdentifier::STAGE_NAMES,
                    PropertyConformance::Optional,
                    None,
                    PropertyWriteCapability::Always,
                ),
            );
        }
        rows
    }
}

impl BACnetObject for StagingObject {
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
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::STAGING.to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::Real(self.present_value)),
            PropertyIdentifier::PRESENT_STAGE if self.present_stage == 0 => Err(protocol_error(
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_NOT_INITIALIZED,
            )),
            PropertyIdentifier::PRESENT_STAGE => Ok(PropertyValue::Unsigned(self.present_stage)),
            PropertyIdentifier::STAGES => common::read_array(
                self.stages
                    .iter()
                    .map(|stage| {
                        let mut encoded = BytesMut::new();
                        encode_stage_limit_value(&mut encoded, stage);
                        PropertyValue::ApplicationData(encoded.to_vec())
                    })
                    .collect(),
                array_index,
            ),
            PropertyIdentifier::STAGE_NAMES => {
                let Some(names) = &self.stage_names else {
                    return Err(common::unknown_property_error());
                };
                common::read_array(
                    names
                        .iter()
                        .cloned()
                        .map(PropertyValue::CharacterString)
                        .collect(),
                    array_index,
                )
            }
            PropertyIdentifier::TARGET_REFERENCES => common::read_array(
                device_reference::reference_elements(&self.target_references),
                array_index,
            ),
            PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units.to_raw())),
            PropertyIdentifier::PRIORITY_FOR_WRITING => {
                Ok(PropertyValue::Unsigned(self.priority_for_writing.into()))
            }
            PropertyIdentifier::MIN_PRES_VALUE => Ok(PropertyValue::Real(self.min_present_value)),
            PropertyIdentifier::MAX_PRES_VALUE => Ok(PropertyValue::Real(self.max_present_value())),
            PropertyIdentifier::COV_INCREMENT => Ok(PropertyValue::Real(self.cov_increment)),
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
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        // A Reliability simulated out of service (Clause 12.62.11) ends with
        // it: back in service, the object reports the one it held before, as
        // the other objects that simulate Reliability do, until the reference
        // writes queued on the way back in judge it afresh (Clause 12.62.4.1,
        // #2258).
        let was_out_of_service = self.out_of_service;
        if let Some(result) = common::write_out_of_service_with_reliability_restore(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            match (was_out_of_service, self.out_of_service) {
                (false, true) => {
                    self.generation = crate::command::next_generation();
                    self.pending_plan = None;
                }
                (true, false) => self.evaluate(true),
                _ => {}
            }
            return result;
        }
        match property {
            PropertyIdentifier::PRESENT_VALUE => {
                let PropertyValue::Real(value) = value else {
                    return Err(common::invalid_data_type_error());
                };
                common::reject_non_finite(value)?;
                self.present_value = value.clamp(self.min_present_value, self.max_present_value());
                self.evaluate(false);
                Ok(())
            }
            PropertyIdentifier::RELIABILITY => {
                if !self.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                let PropertyValue::Enumerated(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let value = Reliability::from_raw(raw);
                if !common::is_reliability_value_valid(value) {
                    return Err(common::value_out_of_range_error());
                }
                self.reliability = value;
                Ok(())
            }
            PropertyIdentifier::STAGES => self.replace_stages(array_index, value),
            PropertyIdentifier::STAGE_NAMES => self.replace_stage_names(array_index, value),
            PropertyIdentifier::TARGET_REFERENCES => {
                self.replace_target_references(array_index, value)
            }
            PropertyIdentifier::PRIORITY_FOR_WRITING => {
                let PropertyValue::Unsigned(value) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let priority = u8::try_from(value)
                    .ok()
                    .filter(|value| (1..=16).contains(value))
                    .ok_or_else(common::value_out_of_range_error)?;
                if priority != self.priority_for_writing {
                    self.priority_for_writing = priority;
                    self.reset_and_reevaluate();
                }
                Ok(())
            }
            PropertyIdentifier::MIN_PRES_VALUE => {
                let PropertyValue::Real(value) = value else {
                    return Err(common::invalid_data_type_error());
                };
                common::reject_non_finite(value)?;
                validate_min_present_value(value, &self.stages)?;
                if value != self.min_present_value {
                    self.min_present_value = value;
                    self.reset_and_reevaluate();
                }
                Ok(())
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        // Borrowed unless Stage_Names adds its row, so the array and list
        // checks that read the rows allocate nothing for most objects.
        match self.stage_names {
            None => Cow::Borrowed(STAGING_PROPERTY_METADATA),
            Some(_) => Cow::Owned(self.metadata()),
        }
    }

    // Table 13-1 Staging row: Present_Value by COV_Increment, Status_Flags and
    // Present_Stage trigger, and Present_Stage is reported (the type's row, the default).
    crate::traits::lend_cov_reporting!();

    fn staging_source(&mut self) -> Option<&mut dyn StagingSource> {
        Some(self)
    }
}

impl crate::traits::CovReporting for StagingObject {
    fn cov_increment(&self) -> Option<f64> {
        Some(f64::from(self.cov_increment))
    }
}

impl StagingSource for StagingObject {
    fn take_staging_write_plan(&mut self) -> Option<StagingWritePlan> {
        if self.present_stage == 0 {
            self.evaluate(false);
        }
        self.pending_plan.take()
    }

    fn staging_generation(&self) -> u64 {
        self.generation
    }

    fn complete_staging_write_plan(&mut self, generation: u64, success: bool) -> bool {
        if generation != self.generation || self.out_of_service {
            return false;
        }
        let reliability = if success {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::UNRELIABLE_OTHER
        };
        if reliability == self.reliability {
            return false;
        }
        self.reliability = reliability;
        true
    }

    fn take_relinquish_writes(&mut self) -> Vec<crate::schedule::ScheduleWrite> {
        // A plan never taken wrote nothing, but its slots are recorded; the
        // ownership check skips them.
        self.pending_plan = None;
        self.held.take_relinquish_writes()
    }
}

fn decode_stage_property_value(value: PropertyValue) -> Result<BACnetStageLimitValue, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    let (stage, consumed) =
        decode_stage_limit_value(&bytes, 0).map_err(|_| common::invalid_data_encoding_error())?;
    if consumed != bytes.len() {
        return Err(common::invalid_data_encoding_error());
    }
    Ok(stage)
}

fn validate_config(config: &StagingConfig) -> Result<(), Error> {
    common::reject_non_finite(config.present_value)?;
    if !(1..=16).contains(&config.priority_for_writing) {
        return Err(common::value_out_of_range_error());
    }
    if let Some(names) = &config.stage_names
        && names.len() != config.stages.len()
    {
        return Err(common::value_out_of_range_error());
    }
    validate_target_references(&config.target_references)?;
    validate_ladder(
        &config.stages,
        config.min_present_value,
        config.target_references.len(),
    )
}

fn validate_target_references(references: &[BACnetDeviceObjectReference]) -> Result<(), Error> {
    for reference in references {
        // A device member that isn't a Device makes no reference (#1285).
        // The object can't tell which Device holds it, so any other Device
        // member is refused too; the server localizes one naming itself
        // (#1136).
        device_reference::check_local_member(reference.device_identifier)?;
        if !matches!(
            reference.object_identifier.object_type(),
            ObjectType::BINARY_OUTPUT
                | ObjectType::BINARY_VALUE
                | ObjectType::BINARY_LIGHTING_OUTPUT
        ) {
            return Err(common::value_out_of_range_error());
        }
    }
    Ok(())
}

fn validate_ladder(
    stages: &[BACnetStageLimitValue],
    min_present_value: f32,
    target_count: usize,
) -> Result<(), Error> {
    if stages.len() <= 1 {
        return Err(common::value_out_of_range_error());
    }
    for stage in stages {
        if !stage.limit.is_finite() || !stage.deadband.is_finite() || stage.deadband < 0.0 {
            return Err(common::value_out_of_range_error());
        }
        if stage.values.len() != target_count {
            return Err(common::value_out_of_range_error());
        }
    }
    for adjacent in stages.windows(2) {
        if adjacent[0].limit >= adjacent[1].limit {
            return Err(common::value_out_of_range_error());
        }
        if adjacent[0].limit + adjacent[0].deadband > adjacent[1].limit - adjacent[1].deadband {
            return Err(common::value_out_of_range_error());
        }
    }
    validate_min_present_value(min_present_value, stages)
}

fn validate_min_present_value(
    min_present_value: f32,
    stages: &[BACnetStageLimitValue],
) -> Result<(), Error> {
    if !min_present_value.is_finite() || min_present_value >= stages[0].limit - stages[0].deadband {
        return Err(common::value_out_of_range_error());
    }
    Ok(())
}

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::protocol(class, code)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod write_validation_tests;
