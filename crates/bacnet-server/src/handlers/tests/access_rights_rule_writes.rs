//! Access Rights Enable, Positive_Access_Rules and Negative_Access_Rules
//! written over WriteProperty and WritePropertyMultiple (#1330, #1332).
//!
//! The arrays take a whole value, one element and a resize at index 0, which
//! fills new elements with the Clause 12.34.9.3 rule; each refused write
//! leaves both arrays as ReadProperty and ReadPropertyMultiple read them
//! before. Enable reads as an application BOOLEAN and takes BOOLEAN writes.
//! ReadPropertyMultiple ALL carries what the writes left.

use super::access_control_arrays::{array_cases, assert_reads, db_with, Expected};
use super::access_rights_rules::{configured, ANYWHERE_OFF, BUSINESS_HOURS, REMOTE_ZONE};
use super::*;
use bacnet_encoding::constructed::encode_access_rule;
use bacnet_objects::access_control::AccessRightsObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::{AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier};
use PropertyIdentifier as P;

const ARRAYS: [P; 2] = [P::POSITIVE_ACCESS_RULES, P::NEGATIVE_ACCESS_RULES];

/// The rule an index-0 write appends: SPECIFIED Schedule 4194303
/// Present_Value, SPECIFIED Access Point 4194303, disabled.
const GROWN: &[u8] = &[
    0x09, 0x00, 0x1E, 0x0C, 0x04, 0x7F, 0xFF, 0xFF, 0x19, 0x55, 0x1F, //
    0x29, 0x00, 0x3E, 0x1C, 0x08, 0x7F, 0xFF, 0xFF, 0x3F, 0x49, 0x00,
];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// An index-0 value: application Unsigned `size`.
fn size(size: u8) -> Vec<u8> {
    vec![0x21, size]
}

fn octets(rule: &BACnetAccessRule) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_access_rule(&mut buf, rule);
    buf.to_vec()
}

/// A rule in force everywhere, enabled, with the given specifiers and
/// references.
fn rule(
    time_range_specifier: AccessRuleTimeRangeSpecifier,
    time_range: Option<BACnetDeviceObjectPropertyReference>,
    location_specifier: AccessRuleLocationSpecifier,
    location: Option<BACnetDeviceObjectReference>,
) -> Vec<u8> {
    octets(&BACnetAccessRule {
        time_range_specifier,
        time_range,
        location_specifier,
        location,
        enable: true,
    })
}

fn write(
    db: &mut ObjectDatabase,
    rights: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    property_value: Vec<u8>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: rights,
        property_identifier: property,
        property_array_index: index,
        property_value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_multiple(
    db: &mut ObjectDatabase,
    rights: ObjectIdentifier,
    writes: Vec<(P, Option<u32>, Vec<u8>)>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: rights,
            list_of_properties: writes
                .into_iter()
                .map(|(property, index, value)| BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: index,
                    value,
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

fn assert_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

/// Both arrays as `configured()` sets them, through ReadProperty and
/// ReadPropertyMultiple.
fn configured_cases() -> Vec<(P, Option<u32>, Expected)> {
    let mut cases = array_cases(P::POSITIVE_ACCESS_RULES, &[BUSINESS_HOURS, ANYWHERE_OFF]);
    cases.extend(array_cases(P::NEGATIVE_ACCESS_RULES, &[REMOTE_ZONE]));
    cases
}

#[test]
fn access_rights_rule_arrays_take_whole_indexed_and_resize_writes() {
    for property in ARRAYS {
        let (mut db, rights) = db_with(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()));
        write(
            &mut db,
            rights,
            property,
            None,
            [BUSINESS_HOURS, ANYWHERE_OFF].concat(),
        )
        .unwrap();
        assert_reads(
            &db,
            rights,
            &array_cases(property, &[BUSINESS_HOURS, ANYWHERE_OFF]),
        );

        write(&mut db, rights, property, Some(2), REMOTE_ZONE.to_vec()).unwrap();
        assert_reads(
            &db,
            rights,
            &array_cases(property, &[BUSINESS_HOURS, REMOTE_ZONE]),
        );

        // Growing adds the Clause 12.34.9.3 rule after the rules kept.
        write(&mut db, rights, property, Some(0), size(4)).unwrap();
        let mut cases = array_cases(property, &[BUSINESS_HOURS, REMOTE_ZONE, GROWN, GROWN]);
        cases.push((property, Some(2), Ok(REMOTE_ZONE.to_vec())));
        cases.push((property, Some(3), Ok(GROWN.to_vec())));
        assert_reads(&db, rights, &cases);

        // Shrinking drops the tail, and an empty whole write empties it.
        write(&mut db, rights, property, Some(0), size(1)).unwrap();
        assert_reads(&db, rights, &array_cases(property, &[BUSINESS_HOURS]));
        write(&mut db, rights, property, None, Vec::new()).unwrap();
        assert_reads(&db, rights, &array_cases(property, &[]));
    }
}

#[test]
fn access_rights_wpm_writes_the_arrays_and_enable() {
    let (mut db, rights) = db_with(Box::new(configured()));
    write_multiple(
        &mut db,
        rights,
        vec![
            (P::POSITIVE_ACCESS_RULES, None, REMOTE_ZONE.to_vec()),
            (P::NEGATIVE_ACCESS_RULES, Some(1), ANYWHERE_OFF.to_vec()),
            (P::NEGATIVE_ACCESS_RULES, Some(0), size(2)),
            (P::LOG_ENABLE, None, vec![0x10]),
        ],
    )
    .unwrap();
    let mut cases = array_cases(P::POSITIVE_ACCESS_RULES, &[REMOTE_ZONE]);
    cases.extend(array_cases(
        P::NEGATIVE_ACCESS_RULES,
        &[ANYWHERE_OFF, GROWN],
    ));
    cases.push((P::LOG_ENABLE, None, Ok(vec![0x10])));
    assert_reads(&db, rights, &cases);

    // A refused element ends the request there: the write before it stays,
    // and neither array changes.
    let door = rule(
        AccessRuleTimeRangeSpecifier::ALWAYS,
        None,
        AccessRuleLocationSpecifier::SPECIFIED,
        Some(oid(ObjectType::ACCESS_DOOR, 2).into()),
    );
    assert_error(
        write_multiple(
            &mut db,
            rights,
            vec![
                (P::LOG_ENABLE, None, vec![0x11]),
                (P::POSITIVE_ACCESS_RULES, Some(1), door),
                (P::NEGATIVE_ACCESS_RULES, None, Vec::new()),
            ],
        ),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    cases.pop();
    cases.push((P::LOG_ENABLE, None, Ok(vec![0x11])));
    assert_reads(&db, rights, &cases);
}

#[test]
fn access_rights_refused_rule_writes_leave_the_arrays_unchanged() {
    use AccessRuleLocationSpecifier as L;
    use AccessRuleTimeRangeSpecifier as T;
    let schedule = BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::SCHEDULE, 1),
        P::PRESENT_VALUE.to_raw(),
    );
    let not_a_device = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(oid(ObjectType::ANALOG_VALUE, 99)),
        ..schedule.clone()
    };
    let value_out_of_range = |bytes| (bytes, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    let whole = vec![
        // A time range in a "device" that isn't a Device (#1285).
        value_out_of_range(
            [
                BUSINESS_HOURS.to_vec(),
                rule(T::SPECIFIED, Some(not_a_device), L::ALL, None),
            ]
            .concat(),
        ),
        // A rule that stops before its enable flag, after a whole one.
        (
            [BUSINESS_HOURS, &[0x09, 0x01, 0x29, 0x01]].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        // An application-tagged value isn't a rule.
        (
            vec![0x21, 0x01],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ];
    let indexed = vec![
        // A location that is neither an Access Point nor an Access Zone.
        value_out_of_range(rule(
            T::ALWAYS,
            None,
            L::SPECIFIED,
            Some(oid(ObjectType::ACCESS_DOOR, 2).into()),
        )),
        // SPECIFIED without its reference; ALWAYS with a reference to
        // something; a specifier outside its two values.
        value_out_of_range(rule(T::SPECIFIED, None, L::ALL, None)),
        value_out_of_range(rule(T::ALWAYS, Some(schedule), L::ALL, None)),
        value_out_of_range(rule(T::from_raw(2), None, L::ALL, None)),
        // Two rules where one belongs.
        (
            [ANYWHERE_OFF, ANYWHERE_OFF].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            vec![0x91, 0x00],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ];
    let mut cases: Vec<(Option<u32>, Vec<u8>, ErrorClass, ErrorCode)> = Vec::new();
    cases.extend(
        whole
            .into_iter()
            .map(|(bytes, class, code)| (None, bytes, class, code)),
    );
    cases.extend(
        indexed
            .into_iter()
            .map(|(bytes, class, code)| (Some(1), bytes, class, code)),
    );
    cases.extend([
        // Past the end of both arrays.
        (
            Some(3),
            ANYWHERE_OFF.to_vec(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        // A size that isn't an Unsigned, and one past the cap (1025).
        (
            Some(0),
            vec![0x91, 0x02],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            Some(0),
            vec![0x22, 0x04, 0x01],
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ),
    ]);
    for property in ARRAYS {
        for (index, bytes, class, code) in cases.clone() {
            let (mut db, rights) = db_with(Box::new(configured()));
            assert_error(write(&mut db, rights, property, index, bytes), class, code);
            assert_reads(&db, rights, &configured_cases());
        }
    }
}

#[test]
fn access_rights_enable_reads_and_writes_over_the_wire() {
    let (mut db, rights) = db_with(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()));
    assert_reads(&db, rights, &[(P::LOG_ENABLE, None, Ok(vec![0x11]))]);
    write(&mut db, rights, P::LOG_ENABLE, None, vec![0x10]).unwrap();
    assert_reads(&db, rights, &[(P::LOG_ENABLE, None, Ok(vec![0x10]))]);
    assert_error(
        write(&mut db, rights, P::LOG_ENABLE, None, size(1)),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_error(
        write(&mut db, rights, P::LOG_ENABLE, Some(1), vec![0x11]),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_reads(&db, rights, &[(P::LOG_ENABLE, None, Ok(vec![0x10]))]);
}

#[test]
fn rpm_all_after_network_writes_carries_the_written_rules_and_enable() {
    let (mut db, rights) = db_with(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()));
    write(
        &mut db,
        rights,
        P::POSITIVE_ACCESS_RULES,
        None,
        [BUSINESS_HOURS, ANYWHERE_OFF].concat(),
    )
    .unwrap();
    write(&mut db, rights, P::NEGATIVE_ACCESS_RULES, Some(0), size(1)).unwrap();
    write(&mut db, rights, P::LOG_ENABLE, None, vec![0x10]).unwrap();

    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: rights,
            list_of_property_references: vec![PropertyReference {
                property_identifier: P::ALL,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut response = BytesMut::new();
    handle_read_property_multiple(&db, &request, &mut response).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    for (property, octets) in [
        (
            P::POSITIVE_ACCESS_RULES,
            [BUSINESS_HOURS, ANYWHERE_OFF].concat(),
        ),
        (P::NEGATIVE_ACCESS_RULES, GROWN.to_vec()),
        (P::LOG_ENABLE, vec![0x10]),
    ] {
        let found: Vec<_> = results
            .iter()
            .filter(|result| result.property_identifier == property)
            .collect();
        assert_eq!(found.len(), 1, "{property:?}");
        assert_eq!(found[0].error, None, "{property:?}");
        assert_eq!(
            found[0].property_value.as_deref(),
            Some(octets.as_slice()),
            "{property:?}"
        );
    }
}
