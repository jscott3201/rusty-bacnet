use crate::event::decode_reference_value;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{EventType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::LocalConfigurationReadError;
use super::classify_required_property_read_error;

/// The property an enrollment monitors (Object_Property_Reference).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitoredReference {
    /// The object.
    pub object_identifier: ObjectIdentifier,
    /// Its property.
    pub property_identifier: PropertyIdentifier,
    /// The array index, if any.
    pub array_index: Option<u32>,
    /// The device named, if the reference names one.
    pub device_identifier: Option<ObjectIdentifier>,
}

impl MonitoredReference {
    /// A reference naming no device, for tests.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn local(
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

/// Read the object-property reference from an Event Enrollment object.
///
/// `Malformed` means the required property is absent or does not contain a
/// usable reference shape. Other read failures remain temporarily unavailable.
pub fn read_object_property_ref(
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
pub fn params_fingerprint(
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
