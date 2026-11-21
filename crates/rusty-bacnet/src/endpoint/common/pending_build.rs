//! Rebuild a validated endpoint registration from its saved parameters.

use super::*;
use crate::object_options::apply_profile;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::traits::{LocalInput, LocalInputValue};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::PropertyValue;

impl PendingObject {
    pub(super) fn identity(&self) -> (ObjectType, u32, &str) {
        match self {
            Self::AnalogInput { instance, name, .. } => (ObjectType::ANALOG_INPUT, *instance, name),
            Self::AnalogValue { instance, name, .. } => (ObjectType::ANALOG_VALUE, *instance, name),
            Self::AnalogOutput { instance, name, .. } => {
                (ObjectType::ANALOG_OUTPUT, *instance, name)
            }
            Self::BinaryInput { instance, name, .. } => (ObjectType::BINARY_INPUT, *instance, name),
            Self::BinaryOutput { instance, name, .. } => {
                (ObjectType::BINARY_OUTPUT, *instance, name)
            }
            Self::BinaryValue { instance, name, .. } => (ObjectType::BINARY_VALUE, *instance, name),
            Self::MultiStateInput { instance, name, .. } => {
                (ObjectType::MULTI_STATE_INPUT, *instance, name)
            }
            Self::MultiStateOutput { instance, name, .. } => {
                (ObjectType::MULTI_STATE_OUTPUT, *instance, name)
            }
            Self::MultiStateValue { instance, name, .. } => {
                (ObjectType::MULTI_STATE_VALUE, *instance, name)
            }
            Self::Group { instance, name, .. } => (ObjectType::GROUP, *instance, name),
            Self::AuditReporter { instance, name, .. } => {
                (ObjectType::AUDIT_REPORTER, *instance, name)
            }
        }
    }

    /// Builds the object, the same way at registration and at each start.
    pub(crate) fn build(&self) -> PyResult<Box<dyn BACnetObject>> {
        match self {
            Self::AnalogInput {
                instance,
                name,
                units,
                present_value,
                profile,
                parameters,
            } => {
                let mut object =
                    AnalogInputObject::new(*instance, name, EngineeringUnits::from_raw(*units))
                        .map_err(to_py_err)?;
                object
                    .apply_local_input(LocalInputValue::present_value(PropertyValue::Real(
                        *present_value,
                    )))
                    .map_err(to_py_err)?;
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::AnalogValue {
                instance,
                name,
                units,
                value,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = AnalogValueObject::with_access(
                    *instance,
                    name,
                    EngineeringUnits::from_raw(*units),
                    value.access,
                )
                .map_err(to_py_err)?;
                object.set_audit_policy(value.audit_policy);
                object.set_value_source_tracking(value.value_source_tracking);
                if let Some(initial) = present_value {
                    if value.access == PresentValueAccess::Commandable {
                        object.set_relinquish_default(*initial).map_err(to_py_err)?;
                    } else {
                        object
                            .apply_local_input(LocalInputValue::present_value(PropertyValue::Real(
                                *initial,
                            )))
                            .map_err(to_py_err)?;
                    }
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::AnalogOutput {
                instance,
                name,
                units,
                present_value,
                profile,
                parameters,
            } => {
                let mut object =
                    AnalogOutputObject::new(*instance, name, EngineeringUnits::from_raw(*units))
                        .map_err(to_py_err)?;
                if let Some(initial) = present_value {
                    object.set_relinquish_default(*initial).map_err(to_py_err)?;
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::BinaryInput {
                instance,
                name,
                polarity,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = BinaryInputObject::new(*instance, name).map_err(to_py_err)?;
                object.set_polarity(*polarity).map_err(to_py_err)?;
                if let Some(initial) = present_value {
                    object
                        .apply_local_input(LocalInputValue::present_value(
                            PropertyValue::Enumerated(*initial),
                        ))
                        .map_err(to_py_err)?;
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::BinaryOutput {
                instance,
                name,
                polarity,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = BinaryOutputObject::new(*instance, name).map_err(to_py_err)?;
                object.set_polarity(*polarity).map_err(to_py_err)?;
                if let Some(initial) = present_value {
                    object.set_relinquish_default(*initial).map_err(to_py_err)?;
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::BinaryValue {
                instance,
                name,
                value,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = BinaryValueObject::with_access(*instance, name, value.access)
                    .map_err(to_py_err)?;
                object.set_audit_policy(value.audit_policy);
                object.set_value_source_tracking(value.value_source_tracking);
                if let Some(initial) = present_value {
                    if value.access == PresentValueAccess::Commandable {
                        object.set_relinquish_default(*initial).map_err(to_py_err)?;
                    } else {
                        object
                            .apply_local_input(LocalInputValue::present_value(
                                PropertyValue::Enumerated(*initial),
                            ))
                            .map_err(to_py_err)?;
                    }
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::MultiStateInput {
                instance,
                name,
                number_of_states,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = MultiStateInputObject::new(*instance, name, *number_of_states)
                    .map_err(to_py_err)?;
                if let Some(initial) = present_value {
                    object
                        .apply_local_input(LocalInputValue::present_value(PropertyValue::Unsigned(
                            u64::from(*initial),
                        )))
                        .map_err(to_py_err)?;
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::MultiStateOutput {
                instance,
                name,
                number_of_states,
                present_value,
                profile,
                parameters,
            } => {
                let mut object = MultiStateOutputObject::new(*instance, name, *number_of_states)
                    .map_err(to_py_err)?;
                if let Some(initial) = present_value {
                    object.set_relinquish_default(*initial).map_err(to_py_err)?;
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::MultiStateValue {
                instance,
                name,
                number_of_states,
                access,
                value_source_tracking,
                present_value,
                profile,
                parameters,
            } => {
                let mut object =
                    MultiStateValueObject::with_access(*instance, name, *number_of_states, *access)
                        .map_err(to_py_err)?;
                object.set_value_source_tracking(*value_source_tracking);
                if let Some(initial) = present_value {
                    if *access == PresentValueAccess::Commandable {
                        object.set_relinquish_default(*initial).map_err(to_py_err)?;
                    } else {
                        object
                            .apply_local_input(LocalInputValue::present_value(
                                PropertyValue::Unsigned((*initial).into()),
                            ))
                            .map_err(to_py_err)?;
                    }
                }
                parameters.apply(&mut object).map_err(to_py_err)?;
                apply_profile(profile.clone(), |p| object.set_profile(p))?;
                Ok(Box::new(object))
            }
            Self::Group {
                instance,
                name,
                members,
            } => Ok(Box::new(group_members::group(*instance, name, members)?)),
            Self::AuditReporter {
                instance,
                name,
                level,
                operations,
                confirmed,
            } => {
                let mut object = AuditReporterObject::new(*instance, name).map_err(to_py_err)?;
                object
                    .audit_reporter_mut()
                    .ok_or_else(|| PyValueError::new_err("Audit Reporter lacks its capability"))?
                    .configure(
                        *level,
                        *operations,
                        *confirmed,
                        None,
                        BACnetPriorityFilter::all(),
                        None,
                    )
                    .map_err(to_py_err)?;
                Ok(Box::new(object))
            }
        }
    }
}
