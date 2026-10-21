//! References that name this device (#1122, #1136, #1151, #1153, #1234).
//!
//! These properties hold references this server keeps inside its own device:
//! the List_Of_Object_Property_References of a Schedule (Clause 12.24.10), a
//! Staging object's Target_References (Clause 12.62.14), an Averaging
//! object's Object_Property_Reference (Clause 12.5.13) and the
//! Log_DeviceObjectProperty of a Trend Log or Trend Log Multiple (Clauses
//! 12.25.8 and 12.30.11). Each clause lets the object stay within its own
//! device, which permits refusing a reference to an object in some other
//! device, and nothing more. None of these objects can tell which Device
//! holds it, so each refuses every member that carries a Device identifier
//! (a Trend Log Multiple keeps an empty element, one naming instance
//! 4194303). A member whose Device identifier is this device's points inside
//! the device, so refusing it would be stricter than the clauses allow. The
//! server knows the local Device ([`ObjectDatabase::local_device`], under the
//! same database guard as the write), so it rewrites such a member as the
//! local reference it denotes before the object sees it: WriteProperty,
//! WritePropertyMultiple, `write_local`, and, for the Schedule's list, the
//! elements of AddListElement and RemoveListElement. A member naming any
//! other device keeps its Device identifier, and the object refuses it. The
//! object stores, and a read returns, the local form, so a member written
//! with and without the identifier is one member.
//!
//! A Channel's List_Of_Object_Property_References (Clause 12.53.11) is
//! localized the same way, so its members naming this device read back in
//! the local form; a member naming another device is kept, and the Channel
//! writes it there (#1264).

use std::borrow::Cow;

use bacnet_encoding::constructed::{
    decode_device_object_property_reference, decode_device_object_reference,
    encode_device_object_property_reference, encode_device_object_reference,
};
use bacnet_objects::database::{LocalDevice, ObjectDatabase};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// A reference whose optional Device member the server may drop, with the
/// shared codec for its wire form.
pub(crate) trait DeviceQualified: Sized {
    /// Decode one reference at `offset`: the reference and the offset after it.
    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error>;
    /// Append the reference's encoding to `buf`.
    fn encode(&self, buf: &mut BytesMut);
    /// The optional Device member.
    fn device_mut(&mut self) -> &mut Option<ObjectIdentifier>;
}

impl DeviceQualified for BACnetDeviceObjectPropertyReference {
    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_device_object_property_reference(bytes, offset)
    }

    fn encode(&self, buf: &mut BytesMut) {
        encode_device_object_property_reference(buf, self);
    }

    fn device_mut(&mut self) -> &mut Option<ObjectIdentifier> {
        &mut self.device_identifier
    }
}

impl DeviceQualified for BACnetDeviceObjectReference {
    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_device_object_reference(bytes, offset)
    }

    fn encode(&self, buf: &mut BytesMut) {
        encode_device_object_reference(buf, self);
    }

    fn device_mut(&mut self) -> &mut Option<ObjectIdentifier> {
        &mut self.device_identifier
    }
}

/// Rewrites a written value so no reference in it names the given Device.
type Rewrite = fn(PropertyValue, ObjectIdentifier) -> PropertyValue;

/// The rewrite for `property` of an object of `object_type`, if the server
/// localizes its references. A list or an array takes
/// [`localize_members`], a property holding one reference
/// [`localize_single`].
fn rewrite(object_type: ObjectType, property: PropertyIdentifier) -> Option<Rewrite> {
    match (object_type, property) {
        (
            ObjectType::SCHEDULE | ObjectType::CHANNEL,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        ) => Some(localize_members::<BACnetDeviceObjectPropertyReference>),
        (ObjectType::STAGING, PropertyIdentifier::TARGET_REFERENCES) => {
            Some(localize_members::<BACnetDeviceObjectReference>)
        }
        (ObjectType::AVERAGING, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE)
        | (ObjectType::TREND_LOG, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY) => {
            Some(localize_single::<BACnetDeviceObjectPropertyReference>)
        }
        (ObjectType::TREND_LOG_MULTIPLE, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY) => {
            Some(localize_members::<BACnetDeviceObjectPropertyReference>)
        }
        _ => None,
    }
}

/// Drop `member`'s Device identifier if it names this device.
pub(crate) fn localize_member<R: DeviceQualified>(member: &mut R, local: LocalDevice) {
    let device = member.device_mut();
    if local.is_local(*device) {
        *device = None;
    }
}

/// A value written to `property` of `oid`, with every member that names the
/// local Device in its local form. Other properties, and values that are
/// not raw member bytes, pass through unchanged for the object to judge.
pub(crate) fn localize(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> PropertyValue {
    let Some(rewrite) = rewrite(oid.object_type(), property) else {
        return value;
    };
    let Some(local) = db.local_device().identifier() else {
        return value;
    };
    rewrite(value, local)
}

/// A list or array value with each member naming `local` in its local form.
/// Each chunk holds whole members and is rewritten on its own: a whole value
/// comes as one chunk of members back to back (what the handler passes on),
/// or as one chunk per element (the shape a read returns, which a
/// `write_local` caller may hand back).
fn localize_members<R: DeviceQualified>(
    value: PropertyValue,
    local: ObjectIdentifier,
) -> PropertyValue {
    let chunk = |value| match value {
        PropertyValue::ApplicationData(bytes) => {
            PropertyValue::ApplicationData(localize_bytes::<R>(&bytes, local))
        }
        other => other,
    };
    match value {
        PropertyValue::List(elements) => {
            PropertyValue::List(elements.into_iter().map(chunk).collect())
        }
        value => chunk(value),
    }
}

/// A value holding one reference, as a single chunk in its local form if the
/// reference names `local`. The service decode hands the reference over as
/// one chunk; a caller of `write_local` may split it into one chunk per
/// context-tagged member, so the chunks are joined before the decode. Unless
/// the joined bytes are exactly one reference naming `local`, the value
/// passes on as written, for the object to judge.
fn localize_single<R: DeviceQualified>(
    value: PropertyValue,
    local: ObjectIdentifier,
) -> PropertyValue {
    let Some(bytes) = joined_chunks(&value) else {
        return value;
    };
    let Ok((mut reference, end)) = R::decode(&bytes, 0) else {
        return value;
    };
    if end != bytes.len() || *reference.device_mut() != Some(local) {
        return value;
    }
    *reference.device_mut() = None;
    let mut localized = BytesMut::new();
    reference.encode(&mut localized);
    PropertyValue::ApplicationData(localized.to_vec())
}

/// The raw bytes of a chunk, or of a list made only of chunks, joined in
/// order. Any other value has none.
fn joined_chunks(value: &PropertyValue) -> Option<Cow<'_, [u8]>> {
    match value {
        PropertyValue::ApplicationData(bytes) => Some(Cow::Borrowed(bytes)),
        PropertyValue::List(chunks) => chunks
            .iter()
            .map(|chunk| match chunk {
                PropertyValue::ApplicationData(bytes) => Some(bytes.as_slice()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(|parts| Cow::Owned(parts.concat())),
        _ => None,
    }
}

/// Re-encode each member of `bytes` that names `local` without its Device
/// identifier. Decoding stops at the first member that doesn't decode; it and
/// everything after it pass on as written, so the object refuses that member
/// with its own error and position.
fn localize_bytes<R: DeviceQualified>(bytes: &[u8], local: ObjectIdentifier) -> Vec<u8> {
    let mut localized = BytesMut::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        let (mut member, end) = match R::decode(bytes, offset) {
            Ok((member, end)) if end > offset => (member, end),
            _ => {
                localized.extend_from_slice(&bytes[offset..]);
                break;
            }
        };
        if *member.device_mut() == Some(local) {
            *member.device_mut() = None;
            member.encode(&mut localized);
        } else {
            localized.extend_from_slice(&bytes[offset..end]);
        }
        offset = end;
    }
    localized.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::schedule::ScheduleObject;

    fn device(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
    }

    fn member(
        instance: u32,
        device: Option<ObjectIdentifier>,
    ) -> BACnetDeviceObjectPropertyReference {
        BACnetDeviceObjectPropertyReference {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: device,
        }
    }

    fn target(instance: u32, device: Option<ObjectIdentifier>) -> BACnetDeviceObjectReference {
        BACnetDeviceObjectReference {
            device_identifier: device,
            object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance).unwrap(),
        }
    }

    fn encoded<R: DeviceQualified>(members: &[R]) -> Vec<u8> {
        let mut bytes = BytesMut::new();
        for member in members {
            member.encode(&mut bytes);
        }
        bytes.to_vec()
    }

    fn database(instance: u32) -> ObjectDatabase {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(instance, 0)
                    .expect("valid Device identity"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        db.add(Box::new(
            ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap(),
        ))
        .unwrap();
        db
    }

    const LIST: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    const TARGETS: PropertyIdentifier = PropertyIdentifier::TARGET_REFERENCES;

    fn schedule() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap()
    }

    fn staging() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
    }

    #[test]
    fn only_members_naming_the_local_device_lose_their_device() {
        let db = database(7);
        let written = encoded(&[
            member(1, Some(device(7))),
            member(2, None),
            member(3, Some(device(9))),
        ]);
        let expected = encoded(&[member(1, None), member(2, None), member(3, Some(device(9)))]);
        assert_eq!(
            localize(
                &db,
                schedule(),
                LIST,
                PropertyValue::ApplicationData(written)
            ),
            PropertyValue::ApplicationData(expected)
        );
        // The shape a read returns, one chunk per element.
        assert_eq!(
            localize(
                &db,
                schedule(),
                LIST,
                PropertyValue::List(vec![
                    PropertyValue::ApplicationData(encoded(&[member(1, Some(device(7)))])),
                    PropertyValue::ApplicationData(encoded(&[member(3, Some(device(9)))])),
                ])
            ),
            PropertyValue::List(vec![
                PropertyValue::ApplicationData(encoded(&[member(1, None)])),
                PropertyValue::ApplicationData(encoded(&[member(3, Some(device(9)))])),
            ])
        );
    }

    #[test]
    fn staging_targets_naming_the_local_device_lose_their_device() {
        // Device 7 has no Staging object; the rewrite keys on the identifier.
        let db = database(7);
        // A whole array, one element per chunk, as a read returns it.
        assert_eq!(
            localize(
                &db,
                staging(),
                TARGETS,
                PropertyValue::List(vec![
                    PropertyValue::ApplicationData(encoded(&[target(1, Some(device(7)))])),
                    PropertyValue::ApplicationData(encoded(&[target(2, None)])),
                    PropertyValue::ApplicationData(encoded(&[target(3, Some(device(9)))])),
                ])
            ),
            PropertyValue::List(vec![
                PropertyValue::ApplicationData(encoded(&[target(1, None)])),
                PropertyValue::ApplicationData(encoded(&[target(2, None)])),
                PropertyValue::ApplicationData(encoded(&[target(3, Some(device(9)))])),
            ])
        );
        // One element by index, and the array size at index 0.
        assert_eq!(
            localize(
                &db,
                staging(),
                TARGETS,
                PropertyValue::ApplicationData(encoded(&[target(4, Some(device(7)))]))
            ),
            PropertyValue::ApplicationData(encoded(&[target(4, None)]))
        );
        assert_eq!(
            localize(&db, staging(), TARGETS, PropertyValue::Unsigned(3)),
            PropertyValue::Unsigned(3)
        );
    }

    const REFERENCE: PropertyIdentifier = PropertyIdentifier::OBJECT_PROPERTY_REFERENCE;

    fn averaging() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::AVERAGING, 1).unwrap()
    }

    /// One reference split into a chunk per context-tagged member, as a
    /// `write_local` caller may hand it over.
    fn split(reference: BACnetDeviceObjectPropertyReference) -> PropertyValue {
        let bytes = encoded(&[reference]);
        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let (chunk, end) =
                bacnet_encoding::primitives::decode_application_value(&bytes, offset).unwrap();
            chunks.push(chunk);
            offset = end;
        }
        PropertyValue::List(chunks)
    }

    #[test]
    fn an_averaging_reference_naming_the_local_device_loses_its_device() {
        // Device 7 has no Averaging object; the rewrite keys on the identifier.
        let db = database(7);
        let local = PropertyValue::ApplicationData(encoded(&[member(1, None)]));
        // Whole, as the handler passes it, and split, as write_local may.
        assert_eq!(
            localize(
                &db,
                averaging(),
                REFERENCE,
                split(member(1, Some(device(7))))
            ),
            local
        );
        assert_eq!(
            localize(
                &db,
                averaging(),
                REFERENCE,
                PropertyValue::ApplicationData(encoded(&[member(1, Some(device(7)))]))
            ),
            local
        );
        // An array index stays.
        let indexed = |device| BACnetDeviceObjectPropertyReference {
            property_array_index: Some(3),
            ..member(1, device)
        };
        assert_eq!(
            localize(&db, averaging(), REFERENCE, split(indexed(Some(device(7))))),
            PropertyValue::ApplicationData(encoded(&[indexed(None)]))
        );
        // Anything but exactly one reference naming device 7 passes on as
        // written: another device, no device, a second reference after it, a
        // flat list, an empty list and Null.
        let analog_input = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        for value in [
            split(member(1, Some(device(9)))),
            split(member(1, None)),
            PropertyValue::ApplicationData(encoded(&[member(1, Some(device(7))), member(2, None)])),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(analog_input),
                PropertyValue::Unsigned(85),
            ]),
            PropertyValue::List(Vec::new()),
            PropertyValue::Null,
        ] {
            assert_eq!(localize(&db, averaging(), REFERENCE, value.clone()), value);
        }
        // The same property of another object type is not localized.
        let enrollment = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
        let named = split(member(1, Some(device(7))));
        assert_eq!(localize(&db, enrollment, REFERENCE, named.clone()), named);
    }

    #[test]
    fn channel_members_naming_the_local_device_lose_their_device() {
        // The rewrite keys on the identifier; Device 7 holds no Channel.
        let db = database(7);
        let channel = ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap();
        let written = encoded(&[member(1, Some(device(7))), member(2, Some(device(9)))]);
        let expected = encoded(&[member(1, None), member(2, Some(device(9)))]);
        assert_eq!(
            localize(&db, channel, LIST, PropertyValue::ApplicationData(written)),
            PropertyValue::ApplicationData(expected)
        );
        // One element by index, and the array size at index 0.
        assert_eq!(
            localize(
                &db,
                channel,
                LIST,
                PropertyValue::ApplicationData(encoded(&[member(3, Some(device(7)))]))
            ),
            PropertyValue::ApplicationData(encoded(&[member(3, None)]))
        );
        assert_eq!(
            localize(&db, channel, LIST, PropertyValue::Unsigned(2)),
            PropertyValue::Unsigned(2)
        );
    }

    #[test]
    fn a_malformed_member_and_what_follows_pass_on_as_written() {
        let db = database(7);
        let local = encoded(&[member(2, Some(device(7)))]);
        // An object identifier without its property, then a local member.
        let rest = [local[..5].to_vec(), local].concat();
        let written = [encoded(&[member(1, Some(device(7)))]), rest.clone()].concat();
        let expected = [encoded(&[member(1, None)]), rest].concat();
        assert_eq!(
            localize(
                &db,
                schedule(),
                LIST,
                PropertyValue::ApplicationData(written)
            ),
            PropertyValue::ApplicationData(expected)
        );
    }

    #[test]
    fn other_targets_and_devices_are_left_alone() {
        let local = encoded(&[member(1, Some(device(7)))]);
        let value = || PropertyValue::ApplicationData(local.clone());
        // Another property, another object type, and no Device object.
        let db = database(7);
        let av = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
        assert_eq!(
            localize(&db, schedule(), PropertyIdentifier::DESCRIPTION, value()),
            value()
        );
        assert_eq!(localize(&db, av, LIST, value()), value());
        let staged = encoded(&[target(1, Some(device(7)))]);
        let staged_value = || PropertyValue::ApplicationData(staged.clone());
        assert_eq!(localize(&db, av, TARGETS, staged_value()), staged_value());
        assert_eq!(
            localize(&db, schedule(), TARGETS, staged_value()),
            staged_value()
        );
        assert_eq!(localize(&db, staging(), LIST, value()), value());
        let mut empty = ObjectDatabase::new();
        empty
            .add(Box::new(
                ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap(),
            ))
            .unwrap();
        assert_eq!(localize(&empty, schedule(), LIST, value()), value());
        assert_eq!(
            localize(&empty, staging(), TARGETS, staged_value()),
            staged_value()
        );
        // A Device at the wildcard instance names no device in particular.
        let wildcard = database(ObjectIdentifier::WILDCARD_INSTANCE);
        let named = encoded(&[member(1, Some(device(ObjectIdentifier::WILDCARD_INSTANCE)))]);
        assert_eq!(
            localize(
                &wildcard,
                schedule(),
                LIST,
                PropertyValue::ApplicationData(named.clone())
            ),
            PropertyValue::ApplicationData(named)
        );
    }
}
