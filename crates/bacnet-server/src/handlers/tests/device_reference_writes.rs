//! The writable device reference properties over the wire (#1313, #1308):
//! the same WriteProperty octets get the same answer from every one of them,
//! now that each reaches its object as raw reference octets and is decoded
//! by the shared helpers. A Schedule's list also answers a non-Device member
//! in AddListElement with VALUE_OUT_OF_RANGE, naming the element.

use super::*;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetStageLimitValue,
};
use bacnet_types::enums::EngineeringUnits;

type P = PropertyIdentifier;

const EMPTY: u32 = ObjectIdentifier::WILDCARD_INSTANCE;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Device 7, and one object of each writable user, each holding one valid
/// reference where it needs one.
fn database() -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(7, 0).expect("valid Device identity"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        bacnet_objects::averaging::AveragingObject::new(1, "AVG-1").unwrap(),
    ))
    .unwrap();
    db.add(Box::new(TrendLogObject::new(1, "TL-1", 8).unwrap()))
        .unwrap();
    let local = |object_type| {
        BACnetDeviceObjectPropertyReference::new_local(
            oid(object_type, 1),
            P::PRESENT_VALUE.to_raw(),
        )
    };
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    tlm.add_property_reference(local(ObjectType::ANALOG_INPUT))
        .unwrap();
    db.add(Box::new(tlm)).unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 1).unwrap();
    channel
        .set_members(vec![local(ObjectType::ANALOG_OUTPUT)])
        .unwrap();
    db.add(Box::new(channel)).unwrap();
    db.add(Box::new(
        ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap(),
    ))
    .unwrap();
    let stage = |limit, active| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    let staging = StagingObject::new(
        1,
        "STG-1",
        StagingConfig {
            present_value: 5.0,
            min_present_value: -1.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![stage(10.0, false), stage(20.0, true)],
            target_references: vec![oid(ObjectType::BINARY_OUTPUT, 1).into()],
            stage_names: None,
        },
    )
    .unwrap();
    db.add(Box::new(staging)).unwrap();
    db
}

/// One writable property: the object, the property and index, whether the
/// value holds a single reference, and how to encode a reference to an
/// object of the type it takes.
struct User {
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    single: bool,
    reference: fn(u32, Option<ObjectIdentifier>) -> Vec<u8>,
}

fn property_reference(
    object: ObjectType,
    instance: u32,
    device: Option<ObjectIdentifier>,
) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_device_object_property_reference(
        &mut bytes,
        &BACnetDeviceObjectPropertyReference {
            object_identifier: oid(object, instance),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: device,
        },
    );
    bytes.to_vec()
}

fn input(instance: u32, device: Option<ObjectIdentifier>) -> Vec<u8> {
    property_reference(ObjectType::ANALOG_INPUT, instance, device)
}

fn output(instance: u32, device: Option<ObjectIdentifier>) -> Vec<u8> {
    property_reference(ObjectType::ANALOG_OUTPUT, instance, device)
}

fn value(instance: u32, device: Option<ObjectIdentifier>) -> Vec<u8> {
    property_reference(ObjectType::ANALOG_VALUE, instance, device)
}

fn target(instance: u32, device: Option<ObjectIdentifier>) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_device_object_reference(
        &mut bytes,
        &BACnetDeviceObjectReference {
            device_identifier: device,
            object_identifier: oid(ObjectType::BINARY_OUTPUT, instance),
        },
    );
    bytes.to_vec()
}

fn user(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    single: bool,
    reference: fn(u32, Option<ObjectIdentifier>) -> Vec<u8>,
) -> User {
    User {
        oid: oid(object_type, 1),
        property,
        index,
        single,
        reference,
    }
}

fn users() -> [User; 9] {
    use ObjectType as T;
    [
        user(
            T::AVERAGING,
            P::OBJECT_PROPERTY_REFERENCE,
            None,
            true,
            input,
        ),
        user(
            T::TREND_LOG,
            P::LOG_DEVICE_OBJECT_PROPERTY,
            None,
            true,
            input,
        ),
        user(
            T::TREND_LOG_MULTIPLE,
            P::LOG_DEVICE_OBJECT_PROPERTY,
            Some(1),
            true,
            input,
        ),
        user(
            T::TREND_LOG_MULTIPLE,
            P::LOG_DEVICE_OBJECT_PROPERTY,
            None,
            false,
            input,
        ),
        user(
            T::CHANNEL,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            Some(1),
            true,
            output,
        ),
        user(
            T::CHANNEL,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            false,
            output,
        ),
        user(
            T::SCHEDULE,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            false,
            value,
        ),
        user(T::STAGING, P::TARGET_REFERENCES, Some(1), true, target),
        user(T::STAGING, P::TARGET_REFERENCES, None, false, target),
    ]
}

fn wp(db: &mut ObjectDatabase, user: &User, value: &[u8]) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: user.oid,
        property_identifier: user.property,
        property_array_index: user.index,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

type Octets = fn(&User) -> Vec<u8>;

/// One row of the wire table: the octets written for a user, and what every
/// user answers, for a single-reference value and for a whole list or array.
struct Row {
    what: &'static str,
    single_only: bool,
    octets: Octets,
    single: Answer,
    list: Answer,
}

#[derive(Clone, Copy)]
enum Answer {
    /// Taken, and read back as these octets.
    Stored(Octets),
    Refused(ErrorCode),
}

/// The octets a read carries: raw octets, or the chunks of a list joined.
fn read_octets(value: &PropertyValue) -> Vec<u8> {
    match value {
        PropertyValue::ApplicationData(bytes) => bytes.clone(),
        PropertyValue::List(items) => items.iter().flat_map(read_octets).collect(),
        other => panic!("not reference octets: {other:?}"),
    }
}

fn rows() -> Vec<Row> {
    use Answer::{Refused, Stored};
    use ErrorCode as E;
    let local: Octets = |u| (u.reference)(2, None);
    let row = |what, single_only, octets, single, list| Row {
        what,
        single_only,
        octets,
        single,
        list,
    };
    let both = |what, octets, answer| row(what, false, octets, answer, answer);
    vec![
        both("a local reference", local, Stored(local)),
        both(
            "the empty object instance",
            |u| (u.reference)(EMPTY, None),
            Stored(|u| (u.reference)(EMPTY, None)),
        ),
        // Stored in its local form, without the Device member.
        both(
            "a reference naming this device",
            |u| (u.reference)(2, Some(oid(ObjectType::DEVICE, 7))),
            Stored(local),
        ),
        both(
            "a non-Device device identifier",
            |u| (u.reference)(2, Some(oid(ObjectType::ANALOG_VALUE, 9))),
            Refused(E::VALUE_OUT_OF_RANGE),
        ),
        both(
            "a non-Device device identifier at the empty instance",
            |u| (u.reference)(EMPTY, Some(oid(ObjectType::ANALOG_VALUE, EMPTY))),
            Refused(E::VALUE_OUT_OF_RANGE),
        ),
        row(
            "a second reference",
            true,
            |u| [(u.reference)(2, None), (u.reference)(3, None)].concat(),
            Refused(E::INVALID_DATA_ENCODING),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        // The first member alone: well-formed tags, but no whole reference.
        both(
            "an incomplete reference",
            |u| (u.reference)(2, Some(oid(ObjectType::DEVICE, 9)))[..5].to_vec(),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        // After one whole reference, anything is an encoding fault in a
        // single-reference value; in a list it is the next element, which
        // can't open a reference or doesn't decode.
        row(
            "a context tag [4] after the reference",
            false,
            |u| [(u.reference)(2, None), vec![0x49, 0x01]].concat(),
            Refused(E::INVALID_DATA_ENCODING),
            Refused(E::INVALID_DATA_TYPE),
        ),
        row(
            "an application Unsigned after the reference",
            false,
            |u| [(u.reference)(2, None), vec![0x21, 0x01]].concat(),
            Refused(E::INVALID_DATA_ENCODING),
            Refused(E::INVALID_DATA_TYPE),
        ),
        both(
            "a context tag [0] after the reference",
            |u| [(u.reference)(2, None), vec![0x09, 0x01]].concat(),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        both(
            "an application-tagged object identifier",
            |_| vec![0xC4, 0x00, 0x00, 0x00, 0x02],
            Refused(E::INVALID_DATA_TYPE),
        ),
    ]
}

#[test]
fn every_writable_device_reference_answers_the_same_octets_the_same_way() {
    for user in users() {
        for row in rows() {
            if row.single_only && !user.single {
                continue;
            }
            let mut db = database();
            let read = |db: &ObjectDatabase| {
                db.get(&user.oid)
                    .unwrap()
                    .read_property(user.property, user.index)
                    .unwrap()
            };
            let before = read(&db);
            let result = wp(&mut db, &user, &(row.octets)(&user));
            let context = format!(
                "{:?} {:?}[{:?}]: {}",
                user.oid, user.property, user.index, row.what
            );
            match if user.single { row.single } else { row.list } {
                Answer::Stored(stored) => {
                    result.unwrap_or_else(|error| panic!("{context}: {error:?}"));
                    assert_eq!(read_octets(&read(&db)), stored(&user), "{context}: stored");
                }
                Answer::Refused(code) => {
                    let (class, actual, _) = list_refusal(result);
                    assert_eq!((class, actual), (ErrorClass::PROPERTY, code), "{context}");
                    assert_eq!(read(&db), before, "{context}: a refusal changes nothing");
                }
            }
        }
    }
}

/// An AddListElement request for the Schedule's reference list.
fn add_list_element(elements: &[u8]) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut encoded,
        0,
        &oid(ObjectType::SCHEDULE, 1),
    );
    bacnet_encoding::primitives::encode_ctx_enumerated(
        &mut encoded,
        1,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES.to_raw(),
    );
    encoded.extend_from_slice(&[0x3e]);
    encoded.extend_from_slice(elements);
    encoded.extend_from_slice(&[0x3f]);
    encoded.to_vec()
}

#[test]
fn schedule_add_list_element_refuses_a_non_device_member_by_position() {
    let mut db = database();
    let elements = [
        value(1, None),
        value(2, Some(oid(ObjectType::ANALOG_VALUE, 9))),
    ]
    .concat();
    assert_eq!(
        list_refusal(handle_add_list_element(
            &mut db,
            &add_list_element(&elements)
        )),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2)
    );
    // Another Device keeps the refusal it had, ahead of which the non-Device
    // check now runs.
    let remote = [value(1, None), value(2, Some(oid(ObjectType::DEVICE, 9)))].concat();
    assert_eq!(
        list_refusal(handle_add_list_element(&mut db, &add_list_element(&remote))),
        (
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            2
        )
    );
    assert_eq!(
        db.get(&oid(ObjectType::SCHEDULE, 1))
            .unwrap()
            .read_property(P::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
            .unwrap(),
        PropertyValue::ApplicationData(Vec::new()),
        "nothing was added"
    );
}
