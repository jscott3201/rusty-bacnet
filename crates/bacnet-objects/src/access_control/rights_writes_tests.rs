//! Network writes of Access Rights' Positive_Access_Rules and
//! Negative_Access_Rules (#1330): whole and indexed writes, the index-0
//! resize with the Clause 12.34.9.3 element, and every refusal leaving both
//! arrays as they were.

use bacnet_encoding::constructed::encode_access_rule;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::*;
use crate::access_control::{AccessRightsObject, MAX_ACCESS_RULES};
use crate::traits::BACnetObject;
use PropertyIdentifier as P;

const ARRAYS: [P; 2] = [P::POSITIVE_ACCESS_RULES, P::NEGATIVE_ACCESS_RULES];

/// The rule an index-0 write appends, in its exact octets: SPECIFIED
/// Schedule 4194303 Present_Value, SPECIFIED Access Point 4194303, disabled.
const GROWN: &[u8] = &[
    0x09, 0x00, 0x1E, 0x0C, 0x04, 0x7F, 0xFF, 0xFF, 0x19, 0x55, 0x1F, //
    0x29, 0x00, 0x3E, 0x1C, 0x08, 0x7F, 0xFF, 0xFF, 0x3F, 0x49, 0x00,
];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn octets(rule: &BACnetAccessRule) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_access_rule(&mut buf, rule);
    buf.to_vec()
}

/// One rule as an indexed write carries it.
fn element(rule: &BACnetAccessRule) -> PropertyValue {
    PropertyValue::ApplicationData(octets(rule))
}

/// A whole-array value as the server passes it on: every rule's octets back
/// to back.
fn wire(rules: &[BACnetAccessRule]) -> PropertyValue {
    PropertyValue::ApplicationData(rules.iter().flat_map(octets).collect())
}

/// SPECIFIED Schedule 1 Present_Value, SPECIFIED Access Point 2, enabled.
fn business_hours() -> BACnetAccessRule {
    BACnetAccessRule::new(
        Some(BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::SCHEDULE, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )),
        Some(oid(ObjectType::ACCESS_POINT, 2).into()),
        true,
    )
}

/// ALWAYS, ALL, disabled.
fn anywhere_off() -> BACnetAccessRule {
    BACnetAccessRule::new(None, None, false)
}

/// ALWAYS, SPECIFIED Access Zone 3 in Device 99, enabled.
fn remote_zone() -> BACnetAccessRule {
    BACnetAccessRule::new(
        None,
        Some(BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 99)),
            object_identifier: oid(ObjectType::ACCESS_ZONE, 3),
        }),
        true,
    )
}

fn rules_of(rights: &AccessRightsObject, property: P) -> &[BACnetAccessRule] {
    if property == P::POSITIVE_ACCESS_RULES {
        rights.positive_access_rules()
    } else {
        rights.negative_access_rules()
    }
}

fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

#[test]
fn access_rights_rule_arrays_take_whole_and_indexed_writes() {
    for property in ARRAYS {
        let other = ARRAYS.into_iter().find(|p| *p != property).unwrap();
        let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
        assert!(rights.is_writable_property(property));

        // A whole write in the server's form, the rules' octets back to back.
        rights
            .write_property(
                property,
                None,
                wire(&[business_hours(), anywhere_off()]),
                None,
            )
            .unwrap();
        assert_eq!(
            rules_of(&rights, property),
            [business_hours(), anywhere_off()]
        );

        // An indexed write replaces one rule, an unspecified reference
        // included.
        rights
            .write_property(property, Some(2), element(&remote_zone()), None)
            .unwrap();
        rights
            .write_property(property, Some(1), element(&grown_rule()), None)
            .unwrap();
        assert_eq!(rules_of(&rights, property), [grown_rule(), remote_zone()]);
        assert_eq!(
            rights.read_property(property, Some(2)).unwrap(),
            element(&remote_zone())
        );

        // A whole read writes back unchanged, in the List form a read returns.
        let read = rights.read_property(property, None).unwrap();
        assert!(matches!(&read, PropertyValue::List(elements) if elements.len() == 2));
        rights
            .write_property(property, None, read.clone(), None)
            .unwrap();
        assert_eq!(rights.read_property(property, None).unwrap(), read);
        assert!(rules_of(&rights, other).is_empty(), "{other:?}");

        // A whole write with no rules empties the array.
        rights
            .write_property(property, None, PropertyValue::ApplicationData(vec![]), None)
            .unwrap();
        assert_eq!(
            rights.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(0)
        );
    }
}

#[test]
fn access_rights_index_zero_grows_with_the_clause_default_and_shrinks_from_the_end() {
    assert_eq!(octets(&grown_rule()), GROWN);
    // The added rule passes the setters' checks.
    let mut scratch = AccessRightsObject::new(1, "AR-1").unwrap();
    scratch.set_positive_access_rules([grown_rule()]).unwrap();

    for property in ARRAYS {
        let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
        rights
            .write_property(property, None, wire(&[business_hours()]), None)
            .unwrap();
        rights
            .write_property(property, Some(0), PropertyValue::Unsigned(3), None)
            .unwrap();
        assert_eq!(
            rights.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            rights.read_property(property, Some(1)).unwrap(),
            element(&business_hours())
        );
        for index in [2, 3] {
            assert_eq!(
                rights.read_property(property, Some(index)).unwrap(),
                PropertyValue::ApplicationData(GROWN.to_vec()),
                "{property:?} {index}"
            );
        }
        // The grown array writes back whole.
        let read = rights.read_property(property, None).unwrap();
        rights.write_property(property, None, read, None).unwrap();
        assert_eq!(
            rules_of(&rights, property),
            [business_hours(), grown_rule(), grown_rule()]
        );

        // The same size changes nothing; a smaller one drops the tail.
        rights
            .write_property(property, Some(0), PropertyValue::Unsigned(3), None)
            .unwrap();
        assert_eq!(rules_of(&rights, property).len(), 3);
        rights
            .write_property(property, Some(1), element(&remote_zone()), None)
            .unwrap();
        rights
            .write_property(property, Some(0), PropertyValue::Unsigned(1), None)
            .unwrap();
        assert_eq!(rules_of(&rights, property), [remote_zone()]);
        rights
            .write_property(property, Some(0), PropertyValue::Unsigned(0), None)
            .unwrap();
        assert!(rules_of(&rights, property).is_empty());

        // Growing from empty reaches the cap.
        rights
            .write_property(
                property,
                Some(0),
                PropertyValue::Unsigned(MAX_ACCESS_RULES as u64),
                None,
            )
            .unwrap();
        let rules = rules_of(&rights, property);
        assert_eq!(rules.len(), MAX_ACCESS_RULES);
        assert!(rules.iter().all(|rule| *rule == grown_rule()));
    }
}

#[test]
fn access_rights_refused_rule_writes_leave_both_arrays_unchanged() {
    use AccessRuleLocationSpecifier as L;
    use AccessRuleTimeRangeSpecifier as T;
    let with = |time_range_specifier, time_range, location_specifier, location| BACnetAccessRule {
        time_range_specifier,
        time_range,
        location_specifier,
        location,
        enable: true,
    };
    let schedule = || {
        Some(BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::SCHEDULE, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
    };
    let point = || {
        Some(BACnetDeviceObjectReference::from(oid(
            ObjectType::ACCESS_POINT,
            2,
        )))
    };
    let not_a_device = oid(ObjectType::ANALOG_VALUE, 99);

    // Each is VALUE_OUT_OF_RANGE, as from the setters.
    let refused_rules = [
        // A device member that isn't a Device (#1285), in either reference.
        with(
            T::SPECIFIED,
            Some(BACnetDeviceObjectPropertyReference::new_remote(
                oid(ObjectType::SCHEDULE, 1),
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
                not_a_device,
            )),
            L::ALL,
            None,
        ),
        with(
            T::ALWAYS,
            None,
            L::SPECIFIED,
            Some(BACnetDeviceObjectReference {
                device_identifier: Some(not_a_device),
                object_identifier: oid(ObjectType::ACCESS_POINT, 2),
            }),
        ),
        // A specifier outside its two values.
        with(T::from_raw(2), None, L::ALL, None),
        with(T::ALWAYS, None, L::from_raw(5), None),
        // SPECIFIED without its reference.
        with(T::SPECIFIED, None, L::ALL, None),
        with(T::ALWAYS, None, L::SPECIFIED, None),
        // ALWAYS or ALL with a reference that isn't unspecified.
        with(T::ALWAYS, schedule(), L::ALL, None),
        with(T::ALWAYS, None, L::ALL, point()),
        // A location that is neither an Access Point nor an Access Zone.
        with(
            T::ALWAYS,
            None,
            L::SPECIFIED,
            Some(oid(ObjectType::ACCESS_DOOR, 2).into()),
        ),
    ];
    let good = business_hours();
    let mut cases: Vec<(Option<u32>, PropertyValue, ErrorClass, ErrorCode)> = Vec::new();
    for bad in &refused_rules {
        cases.push((
            None,
            wire(&[good.clone(), bad.clone()]),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ));
        cases.push((
            Some(1),
            element(bad),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ));
    }
    let truncated = octets(&good)[..5].to_vec();
    let property_error = |index, value, code| (index, value, ErrorClass::PROPERTY, code);
    let no_space = |index, value| {
        (
            index,
            value,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        )
    };
    cases.extend([
        // A rule cut short, after a whole one and alone.
        property_error(
            None,
            PropertyValue::ApplicationData([octets(&good), truncated.clone()].concat()),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        property_error(
            Some(1),
            PropertyValue::ApplicationData(truncated),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        // An element that opens with an application tag isn't a rule.
        property_error(
            None,
            PropertyValue::ApplicationData(vec![0x21, 0x01]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        property_error(
            Some(2),
            PropertyValue::ApplicationData(vec![0x91, 0x00]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // A value of another datatype.
        property_error(
            None,
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        property_error(
            Some(1),
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // An indexed write of two rules, or of none.
        property_error(
            Some(1),
            wire(&[good.clone(), good.clone()]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        property_error(
            Some(1),
            PropertyValue::ApplicationData(vec![]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        // An index past the end.
        property_error(Some(3), element(&good), ErrorCode::INVALID_ARRAY_INDEX),
        property_error(
            Some(u32::MAX),
            element(&good),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        // Index 0 takes an Unsigned size, up to the cap.
        property_error(
            Some(0),
            PropertyValue::Enumerated(3),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        property_error(Some(0), element(&good), ErrorCode::INVALID_DATA_TYPE),
        no_space(
            Some(0),
            PropertyValue::Unsigned(MAX_ACCESS_RULES as u64 + 1),
        ),
        no_space(Some(0), PropertyValue::Unsigned(u64::MAX)),
        // A whole write past the cap.
        no_space(None, wire(&vec![anywhere_off(); MAX_ACCESS_RULES + 1])),
    ]);

    let initial = [business_hours(), anywhere_off()];
    for property in ARRAYS {
        for (index, value, class, code) in cases.clone() {
            let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
            rights.set_positive_access_rules(initial.clone()).unwrap();
            rights.set_negative_access_rules(initial.clone()).unwrap();
            let shown = format!("{property:?} {index:?} {value:?}");
            assert_error(
                rights.write_property(property, index, value, None),
                class,
                code,
            );
            assert_eq!(rights.positive_access_rules(), initial, "{shown}");
            assert_eq!(rights.negative_access_rules(), initial, "{shown}");
        }
    }
}
