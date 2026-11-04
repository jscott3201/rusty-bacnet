//! Table 13-1 values an ordinary (SubscribeCOV) notification reports after
//! its leading value and Status_Flags, captured under the same object borrow.
use alloc::boxed::Box;
use alloc::vec::Vec;

use super::CovSample;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::property_metadata::has_property;
use bacnet_objects::traits::{BACnetObject, CovReportedProperty};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bytes::BytesMut;

#[cfg(test)]
#[path = "reported_tests.rs"]
mod tests;

/// The value a whole-object notification reports first, ahead of
/// Status_Flags, which Active_COV_Subscriptions also names as the monitored
/// property. Present_Value, a trigger filtered by the object's COV_Increment,
/// for every type but Access Point: its Table 13-1 row starts with
/// Access_Event, and footnote 1 has the subscription list name it, but only
/// Access_Event_Time and Status_Flags changes send its notification.
pub fn lead(object_type: ObjectType) -> CovReportedProperty {
    if object_type == ObjectType::ACCESS_POINT {
        CovReportedProperty::Value(PropertyIdentifier::ACCESS_EVENT)
    } else {
        CovReportedProperty::Trigger(PropertyIdentifier::PRESENT_VALUE)
    }
}

/// The encoded extra values in report order, and the bounded samples of those
/// whose changes trigger a notification. A declared property the object's
/// Property_List lacks is left out; a listed one that fails to read fails the
/// whole capture, as an unreadable leading value does.
pub struct PreparedReported {
    /// The values a notification carries after the leading value and flags.
    pub values: Vec<BACnetPropertyValue>,
    /// The bounded samples of the values whose changes send a notification.
    pub triggers: Box<[CovSample]>,
}
impl PreparedReported {
    /// Capture `object`'s Table 13-1 extras.
    pub fn read(object: &dyn BACnetObject) -> Result<Self, Error> {
        // An object that names no list reports its type's Table 13-1 row.
        let declared = match object.cov() {
            None => &[][..],
            Some(cov) => cov.cov_reported_properties().unwrap_or_else(|| {
                CovReportedProperty::standard_for(object.object_identifier().object_type())
            }),
        };
        if declared.is_empty() {
            return Ok(Self {
                values: Vec::new(),
                triggers: Box::default(),
            });
        }
        let rows = object.property_metadata();
        let mut values = Vec::with_capacity(declared.len());
        let mut triggers = Vec::new();
        for reported in declared {
            let property = reported.property();
            if !has_property(&rows, property) {
                continue;
            }
            let value = object.read_property(property, None)?;
            // Bound before encoding or retaining a copy.
            let sample = CovSample::new(&value)?;
            let mut encoded = BytesMut::new();
            encode_property_value(&mut encoded, sample.value())?;
            values.push(BACnetPropertyValue {
                property_identifier: property,
                property_array_index: None,
                value: encoded.to_vec(),
                priority: None,
            });
            if reported.triggers() {
                triggers.push(sample);
            }
        }
        Ok(Self {
            values,
            triggers: triggers.into_boxed_slice(),
        })
    }
}
