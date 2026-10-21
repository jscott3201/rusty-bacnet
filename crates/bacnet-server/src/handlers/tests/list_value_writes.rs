//! Whole-list writes over WriteProperty and WritePropertyMultiple (#1328).
//! A property the target object holds as a BACnetLIST reaches it as a list
//! at every length: no octets is the empty list (Clause 20.2.17), and one
//! element is a list of one. So Alarm_Values clears over the wire on the
//! multi-state objects and the Access Zone, and the multi-state objects take
//! a one-element write. The object judges a list write it can't take, so an
//! empty value on a read-only list or one the object lacks gets the object's
//! refusal. A scalar property still needs a value: no octets stays PROPERTY
//! / INVALID_DATA_ENCODING (Clause 15.9.1.3).

use super::*;
use bacnet_objects::access_control::AccessZoneObject;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::life_safety::LifeSafetyZoneObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::multistate::{MultiStateInputObject, MultiStateValueObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::EngineeringUnits;

const ALARM_VALUES: PropertyIdentifier = PropertyIdentifier::ALARM_VALUES;

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_property_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: property,
                property_array_index: None,
                value: value.to_vec(),
                priority: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

type Write =
    fn(&mut ObjectDatabase, ObjectIdentifier, PropertyIdentifier, &[u8]) -> Result<(), Error>;

const SERVICES: [(&str, Write); 2] = [
    ("WriteProperty", write_property),
    ("WritePropertyMultiple", write_property_multiple),
];

/// The value as the object serves it, before the service encodes it.
fn served(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

/// One object serving Alarm_Values, with how its elements go on the wire.
struct Target {
    oid: ObjectIdentifier,
    /// The application tag octet of a one-octet element of its datatype.
    tag: u8,
    /// Two of its alarm values.
    values: [u32; 2],
}

impl Target {
    fn encode(&self, values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|&value| [self.tag, value as u8])
            .collect()
    }

    fn served_as(&self, values: &[u32]) -> PropertyValue {
        PropertyValue::List(
            values
                .iter()
                .map(|&value| match self.tag {
                    0x21 => PropertyValue::Unsigned(value.into()),
                    _ => PropertyValue::Enumerated(value),
                })
                .collect(),
        )
    }
}

/// A database holding a Multi-state Input and a Multi-state Value, whose
/// alarm values are Unsigned states, and an Access Zone, whose alarm values
/// are BACnetAccessZoneOccupancyState (ABOVE_UPPER_LIMIT and
/// BELOW_LOWER_LIMIT here).
fn alarm_value_objects() -> (ObjectDatabase, [Target; 3]) {
    let mut db = ObjectDatabase::new();
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let targets = [
        Target {
            oid: msi.object_identifier(),
            tag: 0x21,
            values: [2, 3],
        },
        Target {
            oid: msv.object_identifier(),
            tag: 0x21,
            values: [2, 3],
        },
        Target {
            oid: zone.object_identifier(),
            tag: 0x91,
            values: [4, 1],
        },
    ];
    db.add(Box::new(msi)).unwrap();
    db.add(Box::new(msv)).unwrap();
    db.add(Box::new(zone)).unwrap();
    (db, targets)
}

#[test]
fn empty_alarm_values_write_clears_the_list() {
    let (mut db, targets) = alarm_value_objects();
    for (service, write) in SERVICES {
        for target in &targets {
            let oid = target.oid;
            write(&mut db, oid, ALARM_VALUES, &target.encode(&target.values)).unwrap();
            assert_eq!(
                served(&db, oid, ALARM_VALUES),
                target.served_as(&target.values),
                "{service} {oid:?}"
            );
            write(&mut db, oid, ALARM_VALUES, &[]).unwrap_or_else(|error| {
                panic!("{service} of an empty Alarm_Values on {oid:?}: {error:?}")
            });
            assert_eq!(
                served(&db, oid, ALARM_VALUES),
                PropertyValue::List(vec![]),
                "{service} {oid:?}"
            );
        }
    }
}

#[test]
fn one_element_alarm_values_write_is_a_list_of_one() {
    let (mut db, targets) = alarm_value_objects();
    for (service, write) in SERVICES {
        for target in &targets {
            let oid = target.oid;
            let one = &target.values[..1];
            write(&mut db, oid, ALARM_VALUES, &[]).unwrap();
            write(&mut db, oid, ALARM_VALUES, &target.encode(one)).unwrap_or_else(|error| {
                panic!("{service} of a one-element Alarm_Values on {oid:?}: {error:?}")
            });
            assert_eq!(
                served(&db, oid, ALARM_VALUES),
                target.served_as(one),
                "{service} {oid:?}"
            );
        }
    }
}

#[test]
fn one_element_of_another_datatype_is_refused_as_element_one() {
    let (mut db, targets) = alarm_value_objects();
    for (service, write) in SERVICES {
        for target in &targets {
            let oid = target.oid;
            write(&mut db, oid, ALARM_VALUES, &target.encode(&target.values)).unwrap();
            // A Boolean is no alarm value of any of these types.
            assert_eq!(
                list_refusal(write(&mut db, oid, ALARM_VALUES, &[0x11])),
                (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 1),
                "{service} {oid:?}"
            );
            assert_eq!(
                served(&db, oid, ALARM_VALUES),
                target.served_as(&target.values),
                "{service} {oid:?}"
            );
        }
    }
}

#[test]
fn empty_value_on_a_scalar_property_is_still_invalid_data_encoding() {
    let mut db = ObjectDatabase::new();
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let scalars = [
        (msv.object_identifier(), PropertyIdentifier::PRESENT_VALUE),
        (msv.object_identifier(), PropertyIdentifier::DESCRIPTION),
        (lo.object_identifier(), PropertyIdentifier::SETPOINT),
    ];
    db.add(Box::new(msv)).unwrap();
    db.add(Box::new(lo)).unwrap();
    for (service, write) in SERVICES {
        for (oid, property) in scalars {
            let before = served(&db, oid, property);
            assert_eq!(
                list_refusal(write(&mut db, oid, property, &[])),
                (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING, 0),
                "{service} {oid:?} {property:?}"
            );
            assert_eq!(served(&db, oid, property), before, "{service} {property:?}");
        }
    }
}

#[test]
fn empty_value_on_a_read_only_list_is_write_access_denied() {
    // The value is a valid list, so the object judges the write and refuses
    // it as it refuses any write of Zone_Members.
    let mut db = ObjectDatabase::new();
    let zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
    let oid = zone.object_identifier();
    db.add(Box::new(zone)).unwrap();
    for (service, write) in SERVICES {
        assert_eq!(
            list_refusal(write(&mut db, oid, PropertyIdentifier::ZONE_MEMBERS, &[])),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED, 0),
            "{service}"
        );
    }
}

#[test]
fn empty_value_on_a_list_the_object_lacks_is_unknown_property() {
    // Date_List and Log_Buffer are lists on every object type, and
    // Alarm_Values on all but the CharacterString and BitString Values, so
    // the empty value reaches an Analog Value, which serves none of them.
    let mut db = ObjectDatabase::new();
    let av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = av.object_identifier();
    db.add(Box::new(av)).unwrap();
    for (service, write) in SERVICES {
        for property in [
            PropertyIdentifier::DATE_LIST,
            ALARM_VALUES,
            PropertyIdentifier::LOG_BUFFER,
        ] {
            assert_eq!(
                list_refusal(write(&mut db, oid, property, &[])),
                (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY, 0),
                "{service} {property:?}"
            );
        }
    }
}
