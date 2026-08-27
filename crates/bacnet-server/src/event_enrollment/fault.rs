use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::support::classify_required_property_read_error;
use super::LocalConfigurationReadError;

/// Fault algorithms intentionally implemented by the Event Enrollment core.
///
/// `FaultOutOfRange` operates on the repository's normalized numeric `f64`
/// representation; it does not claim the full datatype-dependent model in
/// ASHRAE 135-2020 Clause 13.4.7.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SupportedFaultAlgorithm {
    None,
    StatusFlags { object_identifier: ObjectIdentifier },
    OutOfRange { min_normal: f64, max_normal: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonitoredReliability {
    Absent,
    Value(Reliability),
    ConfigurationError,
    ObservationUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FaultAlgorithmEvaluation {
    Healthy,
    Fault(Reliability),
    ConfigurationError,
    ObservationUnavailable,
}

pub(super) fn read_event_parameters(
    enrollment: &dyn BACnetObject,
) -> Result<BACnetEventParameter, LocalConfigurationReadError> {
    match enrollment.read_property(PropertyIdentifier::EVENT_PARAMETERS, None) {
        Ok(PropertyValue::ApplicationData(bytes)) => {
            match bacnet_encoding::constructed::decode_event_parameter(&bytes, 0) {
                Ok((parameters, consumed)) if consumed == bytes.len() => Ok(parameters),
                _ => Err(LocalConfigurationReadError::Malformed),
            }
        }
        Ok(value) => {
            BACnetEventParameter::decode(&value).map_err(|_| LocalConfigurationReadError::Malformed)
        }
        Err(error) => Err(classify_required_property_read_error(&error)),
    }
}

pub(super) fn read_fault_algorithm(
    enrollment: &dyn BACnetObject,
    local_device_oid: Option<ObjectIdentifier>,
) -> Result<SupportedFaultAlgorithm, LocalConfigurationReadError> {
    let parameters = match enrollment.read_property(PropertyIdentifier::FAULT_PARAMETERS, None) {
        Ok(PropertyValue::ApplicationData(bytes)) => {
            match bacnet_encoding::constructed::decode_fault_parameters(&bytes, 0) {
                Ok((parameters, consumed)) if consumed == bytes.len() => parameters,
                _ => return Err(LocalConfigurationReadError::Malformed),
            }
        }
        Ok(value) => FaultParameters::decode_property_value(&value)
            .map_err(|_| LocalConfigurationReadError::Malformed)?,
        Err(error) if optional_property_absent(&error) => {
            return Ok(SupportedFaultAlgorithm::None);
        }
        Err(error) => return Err(classify_required_property_read_error(&error)),
    };

    match parameters {
        FaultParameters::FaultNone => Ok(SupportedFaultAlgorithm::None),
        FaultParameters::FaultStatusFlags { reference } => {
            let local = reference
                .device_identifier
                .is_none_or(|device| Some(device) == local_device_oid);
            if !local
                || reference.property_identifier != PropertyIdentifier::STATUS_FLAGS.to_raw()
                || reference.property_array_index.is_some()
            {
                return Err(LocalConfigurationReadError::Malformed);
            }
            Ok(SupportedFaultAlgorithm::StatusFlags {
                object_identifier: reference.object_identifier,
            })
        }
        FaultParameters::FaultOutOfRange {
            min_normal,
            max_normal,
        } if min_normal.is_finite() && max_normal.is_finite() && min_normal <= max_normal => {
            Ok(SupportedFaultAlgorithm::OutOfRange {
                min_normal,
                max_normal,
            })
        }
        // These alternatives round-trip at the object boundary, but their
        // state models are deliberately deferred. Treating them as healthy
        // would silently disable a configured fault algorithm.
        _ => Err(LocalConfigurationReadError::Malformed),
    }
}

pub(super) fn read_monitored_reliability(object: &dyn BACnetObject) -> MonitoredReliability {
    match object.read_property(PropertyIdentifier::RELIABILITY, None) {
        Ok(PropertyValue::Enumerated(value)) => {
            MonitoredReliability::Value(Reliability::from_raw(value))
        }
        Ok(_) => MonitoredReliability::ConfigurationError,
        Err(error) if optional_property_absent(&error) => MonitoredReliability::Absent,
        Err(_) => MonitoredReliability::ObservationUnavailable,
    }
}

pub(super) fn evaluate_fault_algorithm(
    db: &ObjectDatabase,
    algorithm: &SupportedFaultAlgorithm,
    monitored_value: Option<&PropertyValue>,
) -> FaultAlgorithmEvaluation {
    match algorithm {
        SupportedFaultAlgorithm::None => FaultAlgorithmEvaluation::Healthy,
        SupportedFaultAlgorithm::StatusFlags { object_identifier } => {
            let Some(object) = db.get(object_identifier) else {
                return FaultAlgorithmEvaluation::ObservationUnavailable;
            };
            match object.read_property(PropertyIdentifier::STATUS_FLAGS, None) {
                Ok(PropertyValue::BitString {
                    unused_bits: 4,
                    data,
                }) if data.len() == 1 => {
                    if data[0] & 0x40 != 0 {
                        FaultAlgorithmEvaluation::Fault(Reliability::MEMBER_FAULT)
                    } else {
                        FaultAlgorithmEvaluation::Healthy
                    }
                }
                Ok(_) => FaultAlgorithmEvaluation::ConfigurationError,
                Err(_) => FaultAlgorithmEvaluation::ObservationUnavailable,
            }
        }
        SupportedFaultAlgorithm::OutOfRange {
            min_normal,
            max_normal,
        } => {
            let Some(value) = monitored_value.and_then(numeric_f64) else {
                return FaultAlgorithmEvaluation::ConfigurationError;
            };
            if !value.is_finite() {
                FaultAlgorithmEvaluation::ConfigurationError
            } else if value < *min_normal {
                FaultAlgorithmEvaluation::Fault(Reliability::UNDER_RANGE)
            } else if value > *max_normal {
                FaultAlgorithmEvaluation::Fault(Reliability::OVER_RANGE)
            } else {
                FaultAlgorithmEvaluation::Healthy
            }
        }
    }
}

fn numeric_f64(value: &PropertyValue) -> Option<f64> {
    match value {
        PropertyValue::Real(value) => Some(f64::from(*value)),
        PropertyValue::Double(value) => Some(*value),
        PropertyValue::Unsigned(value) => Some(*value as f64),
        PropertyValue::Signed(value) => Some(*value as f64),
        _ => None,
    }
}

fn optional_property_absent(error: &Error) -> bool {
    matches!(
        error,
        Error::Protocol { class, code }
            if *class == ErrorClass::PROPERTY.to_raw() as u32
                && *code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
    )
}
