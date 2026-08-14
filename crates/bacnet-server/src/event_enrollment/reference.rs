use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

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

/// Read the object-property reference from an Event Enrollment object.
///
/// `Err` means the property could not be read; `Ok(None)` means it was read but
/// does not contain a usable local reference shape.
pub(super) fn read_object_property_ref(
    enrollment: &dyn BACnetObject,
) -> Result<Option<MonitoredReference>, ()> {
    match enrollment.read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None) {
        Ok(PropertyValue::List(ref items)) if (2..=4).contains(&items.len()) => {
            let object_identifier = match &items[0] {
                PropertyValue::ObjectIdentifier(oid) => *oid,
                _ => return Ok(None),
            };
            let PropertyValue::Unsigned(property_identifier) = &items[1] else {
                return Ok(None);
            };
            let Ok(property_identifier) = u32::try_from(*property_identifier) else {
                return Ok(None);
            };
            if property_identifier > 0x3F_FFFF {
                return Ok(None);
            }
            let property_identifier = PropertyIdentifier::from_raw(property_identifier);
            let array_index = match items.get(2) {
                None | Some(PropertyValue::Null) => None,
                Some(PropertyValue::Unsigned(index)) => match u32::try_from(*index) {
                    Ok(index) => Some(index),
                    Err(_) => return Ok(None),
                },
                Some(_) => return Ok(None),
            };
            let device_identifier = match items.get(3) {
                None | Some(PropertyValue::Null) => None,
                Some(PropertyValue::ObjectIdentifier(oid)) => Some(*oid),
                Some(_) => return Ok(None),
            };
            Ok(Some(MonitoredReference {
                object_identifier,
                property_identifier,
                array_index,
                device_identifier,
            }))
        }
        Ok(_) => Ok(None),
        Err(_) => Err(()),
    }
}

/// Hash the inputs that determine whether an in-flight countdown is still
/// valid. The device qualifier is excluded because accepted qualified and
/// unqualified references resolve to the same local target.
pub(super) fn params_fingerprint(
    params: &BACnetEventParameter,
    normal_delay: u64,
    event_type_raw: u32,
    monitored: &MonitoredReference,
) -> u64 {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, params);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in buf
        .iter()
        .copied()
        .chain(normal_delay.to_le_bytes())
        .chain(event_type_raw.to_le_bytes())
        .chain(monitored.object_identifier.encode())
        .chain(monitored.property_identifier.to_raw().to_le_bytes())
        // BACnet assigns different meanings to an omitted index and index 0.
        .chain([u8::from(monitored.array_index.is_some())])
        .chain(monitored.array_index.unwrap_or_default().to_le_bytes())
    {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
