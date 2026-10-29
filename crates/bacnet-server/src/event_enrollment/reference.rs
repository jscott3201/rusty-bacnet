use bacnet_encoding::constructed::decode_device_object_property_reference;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EventType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::LocalConfigurationReadError;
use super::support::classify_required_property_read_error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MonitoredReference {
    pub(super) object_identifier: ObjectIdentifier,
    pub(super) property_identifier: PropertyIdentifier,
    pub(super) array_index: Option<u32>,
    pub(super) device_identifier: Option<ObjectIdentifier>,
}

impl MonitoredReference {
    #[cfg(test)]
    pub(super) fn local(
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Self {
        Self {
            object_identifier,
            property_identifier,
            array_index,
            device_identifier: None,
        }
    }
}

/// The reference an Event Enrollment's Object_Property_Reference read holds:
/// exactly one BACnetDeviceObjectPropertyReference in its Clause 21 encoding
/// (#1182). Any property identifier counts, ASHRAE's above 4194303 included
/// (#887). Anything else is `None`, and so is the unset form an enrollment
/// without a reference reads as, its object or Device at the reserved
/// instance 4194303 (#1417).
pub(crate) fn decode_reference_value(
    value: &PropertyValue,
) -> Option<BACnetDeviceObjectPropertyReference> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return None;
    };
    let (reference, end) = decode_device_object_property_reference(bytes, 0).ok()?;
    (end == bytes.len() && !reference.is_unset()).then_some(reference)
}

/// Read the object-property reference from an Event Enrollment object.
///
/// `Malformed` means the required property is absent or does not contain a
/// usable reference shape. Other read failures remain temporarily unavailable.
pub(super) fn read_object_property_ref(
    enrollment: &dyn BACnetObject,
) -> Result<MonitoredReference, LocalConfigurationReadError> {
    match enrollment.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None) {
        Ok(value) => {
            let reference =
                decode_reference_value(&value).ok_or(LocalConfigurationReadError::Malformed)?;
            Ok(MonitoredReference {
                object_identifier: reference.object_identifier,
                property_identifier: PropertyIdentifier::from_raw(reference.property_identifier),
                array_index: reference.property_array_index,
                device_identifier: reference.device_identifier,
            })
        }
        Err(error) => Err(classify_required_property_read_error(&error)),
    }
}

/// Hash the inputs that determine whether an in-flight countdown is still
/// valid. The device qualifier is excluded because accepted qualified and
/// unqualified references resolve to the same local target.
pub(super) fn params_fingerprint(
    params: &BACnetEventParameter,
    normal_delay: u64,
    event_type: EventType,
    monitored: &MonitoredReference,
) -> Result<u64, bacnet_types::error::Error> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, params)?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in buf
        .iter()
        .copied()
        .chain(normal_delay.to_le_bytes())
        .chain(event_type.to_raw().to_le_bytes())
        .chain(monitored.object_identifier.encode())
        .chain(monitored.property_identifier.to_raw().to_le_bytes())
        // BACnet assigns different meanings to an omitted index and index 0.
        .chain([u8::from(monitored.array_index.is_some())])
        .chain(monitored.array_index.unwrap_or_default().to_le_bytes())
    {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(h)
}
