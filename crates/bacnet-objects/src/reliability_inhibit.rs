//! Shared state machine for the elected Reliability_Evaluation_Inhibit property.

use bacnet_types::enums::{PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common::{
    invalid_data_type_error, is_reliability_value_valid, value_out_of_range_error,
    write_access_denied_error,
};

/// State shared by the nine intrinsic-reporting object types that elect the
/// optional Reliability_Evaluation_Inhibit property.
///
/// The client-override bit records ownership, not value inequality: an
/// accepted same-value Reliability write while Out_Of_Service is still the
/// alternate write recognized by the inhibit exception.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReliabilityInhibitState {
    enabled: bool,
    oos_client_reliability_override: bool,
}

impl ReliabilityInhibitState {
    #[inline]
    pub(crate) fn enabled(self) -> bool {
        self.enabled
    }

    #[inline]
    pub(crate) fn read(self, property: PropertyIdentifier) -> Option<PropertyValue> {
        (property == PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT)
            .then_some(PropertyValue::Boolean(self.enabled))
    }

    /// Apply a Reliability_Evaluation_Inhibit write atomically.
    ///
    /// TRUE immediately normalizes Reliability unless the current OOS period
    /// owns a successful client Reliability write. FALSE only re-enables
    /// evaluation; it never restores a pre-inhibit value.
    #[inline]
    pub(crate) fn write_inhibit(
        &mut self,
        reliability: &mut u32,
        out_of_service: bool,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT {
            return None;
        }
        let PropertyValue::Boolean(enabled) = value else {
            return Some(Err(invalid_data_type_error()));
        };

        self.enabled = *enabled;
        if self.enabled && !(out_of_service && self.oos_client_reliability_override) {
            *reliability = Reliability::NO_FAULT_DETECTED.to_raw();
        }
        Some(Ok(()))
    }

    /// Apply target-object OOS sequencing without changing the generic helper
    /// retained by Loop, Schedule, and unrelated object types.
    #[inline]
    pub(crate) fn write_out_of_service(
        &mut self,
        out_of_service: &mut bool,
        reliability: &mut u32,
        saved_reliability: &mut Option<u32>,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != PropertyIdentifier::OUT_OF_SERVICE {
            return None;
        }
        let PropertyValue::Boolean(enabled) = value else {
            return Some(Err(invalid_data_type_error()));
        };

        if !*out_of_service && *enabled {
            *saved_reliability = Some(*reliability);
            self.oos_client_reliability_override = false;
            if self.enabled {
                *reliability = Reliability::NO_FAULT_DETECTED.to_raw();
            }
        } else if *out_of_service && !*enabled {
            self.oos_client_reliability_override = false;
            let saved = saved_reliability.take();
            *reliability = if self.enabled {
                Reliability::NO_FAULT_DETECTED.to_raw()
            } else {
                saved.unwrap_or(Reliability::NO_FAULT_DETECTED.to_raw())
            };
        }
        *out_of_service = *enabled;
        Some(Ok(()))
    }

    /// Apply the OOS-only client Reliability route and record successful
    /// ownership only after type and value validation have completed.
    #[inline]
    pub(crate) fn write_client_reliability(
        &mut self,
        out_of_service: bool,
        reliability: &mut u32,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if property != PropertyIdentifier::RELIABILITY {
            return None;
        }
        if !out_of_service {
            return Some(Err(write_access_denied_error()));
        }
        let PropertyValue::Enumerated(new_reliability) = value else {
            return Some(Err(invalid_data_type_error()));
        };
        if !is_reliability_value_valid(*new_reliability) {
            return Some(Err(value_out_of_range_error()));
        }

        *reliability = *new_reliability;
        self.oos_client_reliability_override = true;
        Some(Ok(()))
    }
}
