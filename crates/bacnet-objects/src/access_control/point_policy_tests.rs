//! Access Point Authentication_Policy_List and Authentication_Policy_Names
//! (Clauses 12.31.10 to 12.31.13, Table 12-36; #1325): the application sets
//! them, the count follows, an invalid policy in effect drops the active
//! policy to zero and Reliability to CONFIGURATION_ERROR, and Reliability
//! takes simulated writes while out of service (Clause 12.31.8).

use bacnet_encoding::constructed::encode_authentication_policy;
use bacnet_types::constructed::{BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};

use super::*;

const LIST: P = P::AUTHENTICATION_POLICY_LIST;
const NAMES: P = P::AUTHENTICATION_POLICY_NAMES;
const POLICY: P = P::ACTIVE_AUTHENTICATION_POLICY;
const POLICIES: P = P::NUMBER_OF_AUTHENTICATION_POLICIES;

fn read(point: &AccessPointObject, property: P) -> PropertyValue {
    point.read_property(property, None).unwrap()
}

fn write(point: &mut AccessPointObject, property: P, value: PropertyValue) -> Result<(), Error> {
    point.write_property(property, None, value, None)
}

fn assert_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == expected.to_raw() as u32),
        "expected {expected:?}, got {result:?}"
    );
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// An entry reading Credential Data Input `instance` here, at step `index`.
fn entry(instance: u32, index: u32) -> BACnetAuthenticationPolicyEntry {
    BACnetAuthenticationPolicyEntry {
        credential_data_input: oid(ObjectType::CREDENTIAL_DATA_INPUT, instance).into(),
        index,
    }
}

fn policy(entries: Vec<BACnetAuthenticationPolicyEntry>) -> BACnetAuthenticationPolicy {
    BACnetAuthenticationPolicy {
        policy: entries,
        order_enforced: true,
        timeout: 30,
    }
}

/// A card alone, then a card and a PIN.
fn card_and_pin() -> Vec<(&'static str, BACnetAuthenticationPolicy)> {
    vec![
        ("card", policy(vec![entry(1, 1)])),
        ("card and PIN", policy(vec![entry(1, 1), entry(2, 2)])),
    ]
}

fn encoded(policy: &BACnetAuthenticationPolicy) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_authentication_policy(&mut buf, policy);
    PropertyValue::ApplicationData(buf.to_vec())
}

fn reliability(point: &AccessPointObject) -> Reliability {
    let PropertyValue::Enumerated(raw) = read(point, P::RELIABILITY) else {
        panic!("Reliability is an Enumerated");
    };
    Reliability::from_raw(raw)
}

/// Whether Status_Flags carries FAULT.
fn fault(point: &AccessPointObject) -> bool {
    let PropertyValue::BitString { data, .. } = read(point, P::STATUS_FLAGS) else {
        panic!("Status_Flags is a BIT STRING");
    };
    data[0] & 0x40 != 0
}

#[test]
fn access_point_serves_the_policy_arrays_once_the_application_sets_them() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    for property in [LIST, NAMES] {
        assert!(!point.property_list().contains(&property));
        assert!(point.read_property(property, None).is_err());
        assert!(point.is_array_property(property));
    }
    let policies = card_and_pin();
    point.set_authentication_policies(policies.clone()).unwrap();
    // Both arrays are as long as the count, which follows them (footnote 1).
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(2));
    assert_eq!(
        read(&point, LIST),
        PropertyValue::List(policies.iter().map(|(_, p)| encoded(p)).collect())
    );
    assert_eq!(
        read(&point, NAMES),
        PropertyValue::List(vec![
            PropertyValue::CharacterString("card".into()),
            PropertyValue::CharacterString("card and PIN".into()),
        ])
    );
    for property in [LIST, NAMES] {
        assert_eq!(
            point.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert!(matches!(
            point.read_property(property, Some(3)),
            Err(Error::Protocol { code, .. }) if code == ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32
        ));
    }
    assert_eq!(
        point.read_property(LIST, Some(2)).unwrap(),
        encoded(&policies[1].1)
    );
    assert_eq!(
        point.read_property(NAMES, Some(1)).unwrap(),
        PropertyValue::CharacterString("card".into())
    );
    // Optional rows before Property_List, read-only over the network.
    let list = point.property_list();
    assert_eq!(&list[list.len() - 2..], &[LIST, NAMES]);
    for property in [LIST, NAMES] {
        assert!(!point.required_properties().contains(&property));
        assert!(!point.is_writable_property(property));
    }
    // The first policy is valid and in effect.
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(1));
    assert_eq!(reliability(&point), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn access_point_policy_arrays_and_count_refuse_network_writes() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point.set_authentication_policies(card_and_pin()).unwrap();
    let before = [
        read(&point, LIST),
        read(&point, NAMES),
        read(&point, POLICIES),
    ];
    for (property, value) in [
        (LIST, read(&point, LIST)),
        (NAMES, read(&point, NAMES)),
        (POLICIES, PropertyValue::Unsigned(1)),
    ] {
        assert_error(
            write(&mut point, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_error(
        point.write_property(
            NAMES,
            Some(1),
            PropertyValue::CharacterString("x".into()),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        [
            read(&point, LIST),
            read(&point, NAMES),
            read(&point, POLICIES)
        ],
        before
    );
    // No policies at all is no count (Clause 12.31.11).
    let none: Vec<(&str, BACnetAuthenticationPolicy)> = Vec::new();
    assert_error(
        point.set_authentication_policies(none),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        [
            read(&point, LIST),
            read(&point, NAMES),
            read(&point, POLICIES)
        ],
        before
    );
}

#[test]
fn access_point_invalid_policy_in_effect_drops_to_zero_and_faults() {
    let remote_device = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(ObjectType::CREDENTIAL_DATA_INPUT, 3),
    };
    let not_a_device = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::ANALOG_VALUE, 9)),
        object_identifier: oid(ObjectType::CREDENTIAL_DATA_INPUT, 3),
    };
    let malformed = [
        // No entries (Clause 12.31.12).
        vec![],
        // An entry naming another object type.
        vec![BACnetAuthenticationPolicyEntry {
            credential_data_input: oid(ObjectType::ACCESS_POINT, 1).into(),
            index: 1,
        }],
        // A device identifier that isn't a Device.
        vec![BACnetAuthenticationPolicyEntry {
            credential_data_input: not_a_device,
            index: 1,
        }],
        // Indexes that don't start at 1, skip a step or go back.
        vec![entry(1, 0)],
        vec![entry(1, 2)],
        vec![entry(1, 1), entry(2, 3)],
        vec![entry(1, 1), entry(2, 2), entry(3, 1)],
    ];
    for entries in malformed {
        let mut point = AccessPointObject::new(1, "AP-1").unwrap();
        point
            .set_authentication_policies([
                ("bad", policy(entries.clone())),
                card_and_pin()[0].clone(),
            ])
            .unwrap();
        assert_eq!(
            read(&point, POLICY),
            PropertyValue::Unsigned(0),
            "{entries:?}"
        );
        assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
        assert!(fault(&point));
        // Naming the invalid policy is refused; naming the valid one puts
        // it in effect (Clause 12.31.10), but the invalid policy in the list
        // keeps the configuration error (Clause 12.31.12).
        assert_error(
            write(&mut point, POLICY, PropertyValue::Unsigned(1)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(0));
        write(&mut point, POLICY, PropertyValue::Unsigned(2)).unwrap();
        assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(2));
        assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
        // A list with every policy valid clears it.
        point.set_authentication_policies(card_and_pin()).unwrap();
        assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(2));
        assert_eq!(reliability(&point), Reliability::NO_FAULT_DETECTED);
        assert!(!fault(&point));
    }
    // Choices at one step, a remote reader and a policy with no timeout or
    // ordering are all well formed.
    for entries in [
        vec![
            entry(1, 1),
            entry(2, 1),
            entry(3, 2),
            entry(4, 2),
            entry(5, 3),
        ],
        vec![BACnetAuthenticationPolicyEntry {
            credential_data_input: remote_device,
            index: 1,
        }],
    ] {
        let mut point = AccessPointObject::new(1, "AP-1").unwrap();
        let unordered = BACnetAuthenticationPolicy {
            policy: entries.clone(),
            order_enforced: false,
            timeout: 0,
        };
        point
            .set_authentication_policies([("ok", unordered)])
            .unwrap();
        assert_eq!(
            read(&point, POLICY),
            PropertyValue::Unsigned(1),
            "{entries:?}"
        );
        assert_eq!(reliability(&point), Reliability::NO_FAULT_DETECTED);
    }
}

#[test]
fn access_point_policy_arrays_follow_the_count() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point.set_authentication_policies(card_and_pin()).unwrap();
    write(&mut point, POLICY, PropertyValue::Unsigned(2)).unwrap();
    // Growing adds an empty policy, unordered with no timeout (Clause
    // 12.31.12.2), and an empty name. An empty policy can't be put in
    // effect, and while the list holds one the point reports the
    // configuration error (Clause 12.31.12).
    point.set_number_of_authentication_policies(3).unwrap();
    assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
    assert_eq!(
        point.read_property(LIST, Some(3)).unwrap(),
        encoded(&BACnetAuthenticationPolicy::default())
    );
    assert_eq!(
        point.read_property(NAMES, Some(3)).unwrap(),
        PropertyValue::CharacterString(String::new())
    );
    for property in [LIST, NAMES] {
        assert_eq!(
            point.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(3)
        );
    }
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(2));
    assert_error(
        write(&mut point, POLICY, PropertyValue::Unsigned(3)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    // Shrinking below the policy in effect drops it and its elements.
    point.set_number_of_authentication_policies(1).unwrap();
    assert_eq!(
        read(&point, NAMES),
        PropertyValue::List(vec![PropertyValue::CharacterString("card".into())])
    );
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(0));
    assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
    // A new list replaces the old; a usable policy 1 doesn't put itself in
    // effect, so a client picks it.
    point.set_authentication_policies(card_and_pin()).unwrap();
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(0));
    write(&mut point, POLICY, PropertyValue::Unsigned(1)).unwrap();
    assert_eq!(reliability(&point), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn access_point_reliability_takes_simulated_writes_only_out_of_service() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    let simulated = PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw());
    assert_error(
        write(&mut point, P::RELIABILITY, simulated.clone()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(reliability(&point), Reliability::NO_FAULT_DETECTED);

    write(&mut point, P::OUT_OF_SERVICE, PropertyValue::Boolean(true)).unwrap();
    write(&mut point, P::RELIABILITY, simulated).unwrap();
    assert_eq!(reliability(&point), Reliability::UNRELIABLE_OTHER);
    assert!(fault(&point));
    for (value, code) in [
        (PropertyValue::Unsigned(1), ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::Enumerated(70_000),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_error(write(&mut point, P::RELIABILITY, value), code);
    }
    assert_eq!(reliability(&point), Reliability::UNRELIABLE_OTHER);

    // Out of service Reliability is decoupled from the policies: an invalid
    // policy in effect still drops the active policy, but the simulation
    // stays served.
    point
        .set_authentication_policies([("empty", BACnetAuthenticationPolicy::default())])
        .unwrap();
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(0));
    assert_eq!(reliability(&point), Reliability::UNRELIABLE_OTHER);
    write(
        &mut point,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    )
    .unwrap();
    assert!(!fault(&point));

    // The return to service serves the derived value again.
    write(&mut point, P::OUT_OF_SERVICE, PropertyValue::Boolean(false)).unwrap();
    assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
    assert!(fault(&point));
    // Entering out of service again starts the simulation from it.
    write(&mut point, P::OUT_OF_SERVICE, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(reliability(&point), Reliability::CONFIGURATION_ERROR);
}

#[test]
fn access_point_caps_the_policy_count_while_it_serves_the_arrays() {
    let names = |count: u32| (0..count).map(|n| (format!("p{n}"), policy(vec![entry(1, 1)])));
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // Without the arrays nothing is allocated, so any nonzero count goes.
    point
        .set_number_of_authentication_policies(u32::MAX)
        .unwrap();
    // A list past the cap is refused, and the point keeps what it had.
    assert_error(
        point.set_authentication_policies(names(MAX_AUTHENTICATION_POLICIES + 1)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert!(!point.property_list().contains(&LIST));
    point
        .set_authentication_policies(names(MAX_AUTHENTICATION_POLICIES))
        .unwrap();
    assert_eq!(
        read(&point, POLICIES),
        PropertyValue::Unsigned(MAX_AUTHENTICATION_POLICIES.into())
    );
    // Neither can a count grow the arrays past it.
    for count in [MAX_AUTHENTICATION_POLICIES + 1, u32::MAX] {
        assert_error(
            point.set_number_of_authentication_policies(count),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(
        point.read_property(NAMES, Some(0)).unwrap(),
        PropertyValue::Unsigned(MAX_AUTHENTICATION_POLICIES.into())
    );
    point.set_number_of_authentication_policies(2).unwrap();
}
