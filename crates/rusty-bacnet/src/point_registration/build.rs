//! Build a [`Registration`], the same way for `BACnetServer` and the endpoints.
//!
//! Inside the build that [`keep_reporting`] runs, so before storage
//! attaches: the object, its Value keywords, its initial value and its
//! parameter keywords; each refusal there leaves the file untouched. A
//! value a peer wrote and the file kept is then served over a keyword's,
//! and a clearing start builds again on the cleared file. The profile
//! rows come last, as they always have; [`Registration::check`] has
//! checked them before the build, so they can't be refused after a
//! clearing start has cleared the file (#2711).

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::{BACnetObject, LocalInput, LocalInputValue};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use pyo3::PyResult;

use super::{Registration, RegistrationKind, Storage};
use crate::object_options::{apply_profile, keep_reporting};

/// Present_Value set as a local input: refused as the object refuses a
/// write of it (an Analog Input's NaN or infinity, a Binary Input's 2, a
/// state outside a Multi-state Input's count).
fn local_input(object: &mut impl LocalInput, value: PropertyValue) -> Result<(), Error> {
    object.apply_local_input(LocalInputValue::present_value(value))
}

impl Registration {
    /// Build the object, with `storage` attached when it names a file.
    /// Every check the object makes raises here: BacnetError or
    /// BacnetProtocolError, as the object refuses.
    pub(crate) fn build(&self, storage: Storage<'_>) -> PyResult<Box<dyn BACnetObject + Send>> {
        let Self {
            instance,
            name,
            kind,
            profile,
            parameters,
        } = self;
        let instance = *instance;
        let name = name.as_str();
        match kind {
            RegistrationKind::AnalogInput {
                units,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = AnalogInputObject::new(
                            instance,
                            name,
                            EngineeringUnits::from_raw(*units),
                        )?;
                        local_input(&mut object, PropertyValue::Real(*present_value))?;
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    AnalogInputObject::with_reporting_persistence,
                    AnalogInputObject::clear_written_reporting,
                )?;
                finish(object, profile, AnalogInputObject::set_profile)
            }
            RegistrationKind::AnalogOutput {
                units,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = AnalogOutputObject::new(
                            instance,
                            name,
                            EngineeringUnits::from_raw(*units),
                        )?;
                        if let Some(initial) = present_value {
                            object.set_relinquish_default(*initial)?;
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    AnalogOutputObject::with_reporting_persistence,
                    AnalogOutputObject::clear_written_reporting,
                )?;
                finish(object, profile, AnalogOutputObject::set_profile)
            }
            RegistrationKind::AnalogValue {
                units,
                value,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = AnalogValueObject::with_access(
                            instance,
                            name,
                            EngineeringUnits::from_raw(*units),
                            value.access,
                        )?;
                        object.set_audit_policy(value.audit_policy);
                        object.set_value_source_tracking(value.value_source_tracking);
                        match present_value {
                            Some(initial) if value.access == PresentValueAccess::Commandable => {
                                object.set_relinquish_default(*initial)?
                            }
                            Some(initial) => {
                                local_input(&mut object, PropertyValue::Real(*initial))?
                            }
                            None => {}
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    AnalogValueObject::with_reporting_persistence,
                    AnalogValueObject::clear_written_reporting,
                )?;
                finish(object, profile, AnalogValueObject::set_profile)
            }
            RegistrationKind::BinaryInput {
                polarity,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = BinaryInputObject::new(instance, name)?;
                        object.set_polarity(*polarity)?;
                        if let Some(initial) = present_value {
                            local_input(&mut object, PropertyValue::Enumerated(*initial))?;
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    BinaryInputObject::with_reporting_persistence,
                    BinaryInputObject::clear_written_reporting,
                )?;
                finish(object, profile, BinaryInputObject::set_profile)
            }
            RegistrationKind::BinaryOutput {
                polarity,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = BinaryOutputObject::new(instance, name)?;
                        object.set_polarity(*polarity)?;
                        if let Some(initial) = present_value {
                            object.set_relinquish_default(*initial)?;
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    BinaryOutputObject::with_reporting_persistence,
                    BinaryOutputObject::clear_written_reporting,
                )?;
                finish(object, profile, BinaryOutputObject::set_profile)
            }
            RegistrationKind::BinaryValue {
                value,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object =
                            BinaryValueObject::with_access(instance, name, value.access)?;
                        object.set_audit_policy(value.audit_policy);
                        object.set_value_source_tracking(value.value_source_tracking);
                        match present_value {
                            Some(initial) if value.access == PresentValueAccess::Commandable => {
                                object.set_relinquish_default(*initial)?
                            }
                            Some(initial) => {
                                local_input(&mut object, PropertyValue::Enumerated(*initial))?
                            }
                            None => {}
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    BinaryValueObject::with_reporting_persistence,
                    BinaryValueObject::clear_written_reporting,
                )?;
                finish(object, profile, BinaryValueObject::set_profile)
            }
            RegistrationKind::MultiStateInput {
                number_of_states,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object =
                            MultiStateInputObject::new(instance, name, *number_of_states)?;
                        if let Some(initial) = present_value {
                            local_input(&mut object, PropertyValue::Unsigned((*initial).into()))?;
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    MultiStateInputObject::with_reporting_persistence,
                    MultiStateInputObject::clear_written_reporting,
                )?;
                finish(object, profile, MultiStateInputObject::set_profile)
            }
            RegistrationKind::MultiStateOutput {
                number_of_states,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object =
                            MultiStateOutputObject::new(instance, name, *number_of_states)?;
                        if let Some(initial) = present_value {
                            object.set_relinquish_default(*initial)?;
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    MultiStateOutputObject::with_reporting_persistence,
                    MultiStateOutputObject::clear_written_reporting,
                )?;
                finish(object, profile, MultiStateOutputObject::set_profile)
            }
            RegistrationKind::MultiStateValue {
                number_of_states,
                access,
                value_source_tracking,
                present_value,
            } => {
                let object = keep_reporting(
                    || {
                        let mut object = MultiStateValueObject::with_access(
                            instance,
                            name,
                            *number_of_states,
                            *access,
                        )?;
                        object.set_value_source_tracking(*value_source_tracking);
                        match present_value {
                            Some(initial) if *access == PresentValueAccess::Commandable => {
                                object.set_relinquish_default(*initial)?
                            }
                            Some(initial) => local_input(
                                &mut object,
                                PropertyValue::Unsigned((*initial).into()),
                            )?,
                            None => {}
                        }
                        parameters.apply(&mut object)?;
                        Ok(object)
                    },
                    storage.path,
                    storage.clear,
                    MultiStateValueObject::with_reporting_persistence,
                    MultiStateValueObject::clear_written_reporting,
                )?;
                finish(object, profile, MultiStateValueObject::set_profile)
            }
        }
    }
}

/// Provision the profile rows, then box the object.
fn finish<T: BACnetObject + 'static>(
    mut object: T,
    profile: &Option<ObjectProfile>,
    set_profile: fn(&mut T, ObjectProfile) -> Result<(), Error>,
) -> PyResult<Box<dyn BACnetObject + Send>> {
    apply_profile(profile.clone(), |profile| set_profile(&mut object, profile))?;
    Ok(Box::new(object))
}
