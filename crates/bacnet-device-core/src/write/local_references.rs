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
//!
//! So are an Event Enrollment's Object_Property_Reference and a Structured
//! View's Subordinate_List, which take their value at creation (#2208): a
//! reference naming this device reads back in the local form, and one naming
//! another device is kept as given.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use bacnet_encoding::constructed::{
    decode_device_object_property_reference, decode_device_object_reference,
    encode_device_object_property_reference, encode_device_object_reference,
};
use bacnet_objects::database::{LocalDevice, ObjectDatabase};
use bacnet_objects::property_metadata::{Datatype, ValueShape, ValueType};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// A reference whose optional Device member the server may drop, with the
/// shared codec for its wire form.
///
/// Sealed: it is public only so [`localize_member`] can name it in a bound,
/// and only the two reference types here implement it.
pub trait DeviceQualified: sealed::Sealed + Sized {
    /// Decode one reference at `offset`: the reference and the offset after it.
    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error>;
    /// Append the reference's encoding to `buf`.
    fn encode(&self, buf: &mut BytesMut);
    /// The optional Device member.
    fn device_mut(&mut self) -> &mut Option<ObjectIdentifier>;
}

mod sealed {
    /// The private supertrait that keeps [`DeviceQualified`](super::DeviceQualified)
    /// closed to the types in this module.
    pub trait Sealed {}

    impl Sealed for super::BACnetDeviceObjectPropertyReference {}
    impl Sealed for super::BACnetDeviceObjectReference {}
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

/// The rewrite for references of this value type, which the write policy
/// keeps inside the device ([`super::policy::WritePolicy`]). A list or an
/// array takes [`localize_members`], a property holding one reference
/// [`localize_single`], each with the codec of the row's reference datatype.
fn rewrite(value_type: ValueType) -> Option<Rewrite> {
    let single = matches!(value_type.shape, ValueShape::Single);
    match value_type.datatype {
        Datatype::DeviceObjectPropertyReference if single => {
            Some(localize_single::<BACnetDeviceObjectPropertyReference>)
        }
        Datatype::DeviceObjectPropertyReference => {
            Some(localize_members::<BACnetDeviceObjectPropertyReference>)
        }
        Datatype::DeviceObjectReference if single => {
            Some(localize_single::<BACnetDeviceObjectReference>)
        }
        Datatype::DeviceObjectReference => Some(localize_members::<BACnetDeviceObjectReference>),
        _ => None,
    }
}

/// Drop `member`'s Device identifier if it names this device.
pub fn localize_member<R: DeviceQualified>(member: &mut R, local: LocalDevice) {
    let device = member.device_mut();
    if local.is_local(*device) {
        *device = None;
    }
}

/// A value written to `property` of `oid`, with every member that names the
/// local Device in its local form. Other properties, and values that are
/// not raw member bytes, pass through unchanged for the object to judge.
pub fn localize(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> PropertyValue {
    if !super::policy::write_policy(oid.object_type(), property).device_local_references {
        return value;
    }
    let Some(rewrite) = db.value_type(&oid, property).and_then(rewrite) else {
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
    use crate::write::policy::write_policy;
    use bacnet_objects::property_metadata::standard_value_type;
    use bacnet_types::enums::ObjectType;

    /// The rewrite's match before #2177, frozen, with the two pairs #2208
    /// adds.
    fn old_rewrite(object_type: ObjectType, property: PropertyIdentifier) -> Option<Rewrite> {
        match (object_type, property) {
            (
                ObjectType::SCHEDULE | ObjectType::CHANNEL,
                PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            ) => Some(localize_members::<BACnetDeviceObjectPropertyReference>),
            (ObjectType::STAGING, PropertyIdentifier::TARGET_REFERENCES)
            | (ObjectType::STRUCTURED_VIEW, PropertyIdentifier::SUBORDINATE_LIST) => {
                Some(localize_members::<BACnetDeviceObjectReference>)
            }
            (
                ObjectType::AVERAGING | ObjectType::EVENT_ENROLLMENT,
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            )
            | (ObjectType::TREND_LOG, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY) => {
                Some(localize_single::<BACnetDeviceObjectPropertyReference>)
            }
            (ObjectType::TREND_LOG_MULTIPLE, PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY) => {
                Some(localize_members::<BACnetDeviceObjectPropertyReference>)
            }
            _ => None,
        }
    }

    /// Before/after table test for #2177: the old match against the
    /// standard value types every built-in row resolves to, over every
    /// (object type, property) pair, comparing the rewrite chosen.
    #[test]
    fn standard_value_types_localize_where_the_old_match_did() {
        let mut differences = Vec::new();
        for object_type in (0..=1023).map(ObjectType::from_raw) {
            for property in (0..=1023).map(PropertyIdentifier::from_raw).chain(
                PropertyIdentifier::ALL_NAMED
                    .iter()
                    .map(|&(_, property)| property)
                    .filter(|property| property.to_raw() > 1023),
            ) {
                let new = if write_policy(object_type, property).device_local_references {
                    standard_value_type(object_type, property).and_then(rewrite)
                } else {
                    None
                };
                let old = old_rewrite(object_type, property);
                if new.map(|f| f as usize) != old.map(|f| f as usize) {
                    differences.push(format!("{object_type:?} {property:?}"));
                }
            }
        }
        assert!(differences.is_empty(), "{}", differences.join("\n"));
    }
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

    /// Bound to Device 7, as a server binds its identity's Device, the
    /// database localizes members naming 7, not a lower Device 3 added
    /// later (#2227).
    #[test]
    fn members_naming_the_bound_device_lose_it_beside_a_lower_one() {
        let mut db = database(7);
        db.bind_local_device_internal(Some(device(7)));
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(3, 0)
                    .unwrap()
                    .with_name("Device-3"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let written = encoded(&[member(1, Some(device(7))), member(2, Some(device(3)))]);
        let expected = encoded(&[member(1, None), member(2, Some(device(3)))]);
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

    /// A Structured View's Subordinate_List, which CreateObject sets, is
    /// localized as Staging targets are (#2208).
    #[test]
    fn subordinates_naming_the_local_device_lose_their_device() {
        let db = database(7);
        let view = ObjectIdentifier::new(ObjectType::STRUCTURED_VIEW, 1).unwrap();
        let written = encoded(&[target(1, Some(device(7))), target(2, Some(device(9)))]);
        let expected = encoded(&[target(1, None), target(2, Some(device(9)))]);
        assert_eq!(
            localize(
                &db,
                view,
                PropertyIdentifier::SUBORDINATE_LIST,
                PropertyValue::ApplicationData(written)
            ),
            PropertyValue::ApplicationData(expected)
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
        // An Event Enrollment's reference, which it takes at creation, is
        // localized too (#2208); the same property of another object type is
        // not.
        let named = split(member(1, Some(device(7))));
        let enrollment = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
        assert_eq!(
            localize(&db, enrollment, REFERENCE, named.clone()),
            PropertyValue::ApplicationData(encoded(&[member(1, None)]))
        );
        let other = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap();
        assert_eq!(localize(&db, other, REFERENCE, named.clone()), named);
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
        // The wildcard Device names no device in particular, and no
        // database holds a Device there (#2171).
        let named = encoded(&[member(1, Some(device(ObjectIdentifier::WILDCARD_INSTANCE)))]);
        assert_eq!(
            localize(
                &empty,
                schedule(),
                LIST,
                PropertyValue::ApplicationData(named.clone())
            ),
            PropertyValue::ApplicationData(named)
        );
    }
}
