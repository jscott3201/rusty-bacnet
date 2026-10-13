//! Access Point Active_Authentication_Policy,
//! Number_Of_Authentication_Policies, Authorization_Mode and
//! Priority_For_Writing, the Table 12-36 required rows #1307 adds (Clauses
//! 12.31.10, 12.31.11, 12.31.14 and 12.31.33).

use bacnet_types::enums::{AuthorizationMode, ErrorClass, ErrorCode, PropertyIdentifier as P};

use super::*;

const POLICY: P = P::ACTIVE_AUTHENTICATION_POLICY;
const POLICIES: P = P::NUMBER_OF_AUTHENTICATION_POLICIES;
const MODE: P = P::AUTHORIZATION_MODE;
const PRIORITY: P = P::PRIORITY_FOR_WRITING;

fn read(point: &AccessPointObject, property: P) -> PropertyValue {
    point.read_property(property, None).unwrap()
}

fn write(point: &mut AccessPointObject, property: P, value: PropertyValue) -> Result<(), Error> {
    point.write_property(property, None, value, None)
}

fn mode(mode: AuthorizationMode) -> PropertyValue {
    PropertyValue::Enumerated(mode.to_raw())
}

/// Every standard BACnetAuthorizationMode, AUTHORIZE (0) to NONE (5).
const STANDARD_MODES: [AuthorizationMode; 6] = [
    AuthorizationMode::AUTHORIZE,
    AuthorizationMode::GRANT_ACTIVE,
    AuthorizationMode::DENY_ALL,
    AuthorizationMode::VERIFICATION_REQUIRED,
    AuthorizationMode::AUTHORIZATION_DELAYED,
    AuthorizationMode::NONE,
];

fn assert_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == expected.to_raw() as u32),
        "expected {expected:?}, got {result:?}"
    );
}

#[test]
fn access_point_serves_policy_mode_and_priority_defaults() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    // One policy, in effect; AUTHORIZE; the lowest command priority.
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(1));
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(1));
    assert_eq!(read(&point, MODE), mode(AuthorizationMode::AUTHORIZE));
    assert_eq!(read(&point, PRIORITY), PropertyValue::Unsigned(16));
    for property in [POLICY, POLICIES, MODE, PRIORITY] {
        assert!(point.property_list().contains(&property), "{property:?}");
        assert!(
            point.required_properties().contains(&property),
            "{property:?}"
        );
        assert!(!point.is_array_property(property), "{property:?}");
    }
    // A client picks the policy and the mode; the count and the priority
    // are the application's.
    assert!(point.is_writable_property(POLICY));
    assert!(point.is_writable_property(MODE));
    assert!(!point.is_writable_property(POLICIES));
    assert!(!point.is_writable_property(PRIORITY));
}

#[test]
fn access_point_active_policy_write_names_one_of_the_policies() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point.set_number_of_authentication_policies(3).unwrap();
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(3));
    for policy in [3, 1, 2] {
        write(&mut point, POLICY, PropertyValue::Unsigned(policy)).unwrap();
        assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(policy));
    }
    // Out of service the policy in effect can still be switched.
    write(&mut point, P::OUT_OF_SERVICE, PropertyValue::Boolean(true)).unwrap();
    write(&mut point, POLICY, PropertyValue::Unsigned(3)).unwrap();
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(3));
}

#[test]
fn access_point_active_policy_refuses_writes_outside_the_policies() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // A new point defines one policy, so only 1 names one; zero names none.
    write(&mut point, POLICY, PropertyValue::Unsigned(1)).unwrap();
    for policy in [0, 2, u64::from(u32::MAX) + 1, u64::MAX] {
        assert_error(
            write(&mut point, POLICY, PropertyValue::Unsigned(policy)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for value in [
        PropertyValue::Enumerated(1),
        PropertyValue::Signed(1),
        PropertyValue::Null,
    ] {
        assert_error(
            write(&mut point, POLICY, value),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_error(
        point.write_property(POLICY, Some(1), PropertyValue::Unsigned(1), None),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(1));
}

#[test]
fn access_point_authorization_mode_write_takes_each_declared_standard_mode() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_supported_authorization_modes(STANDARD_MODES)
        .unwrap();
    // GRANT_ACTIVE (1) to NONE (5), then back to AUTHORIZE (0).
    for raw in [1, 2, 3, 4, 5, 0] {
        write(&mut point, MODE, PropertyValue::Enumerated(raw)).unwrap();
        assert_eq!(read(&point, MODE), PropertyValue::Enumerated(raw));
    }
}

#[test]
fn access_point_authorization_mode_refuses_other_values() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // A new point supports AUTHORIZE alone: it enforces no mode, so the
    // other standard modes wait for the application to declare them.
    write(&mut point, MODE, mode(AuthorizationMode::AUTHORIZE)).unwrap();
    // Undeclared standard modes, reserved values, an undeclared proprietary
    // mode and values past the Unsigned16 range.
    for raw in [1, 2, 3, 4, 5, 6, 63, 64, 65_535, 65_536, u32::MAX] {
        assert_error(
            write(&mut point, MODE, PropertyValue::Enumerated(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for value in [PropertyValue::Unsigned(0), PropertyValue::Null] {
        assert_error(write(&mut point, MODE, value), ErrorCode::INVALID_DATA_TYPE);
    }
    assert_error(
        point.write_property(MODE, Some(0), PropertyValue::Enumerated(0), None),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_eq!(read(&point, MODE), mode(AuthorizationMode::AUTHORIZE));
}

#[test]
fn access_point_authorization_mode_follows_the_supported_modes() {
    let proprietary = AuthorizationMode::from_raw(300);
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_supported_authorization_modes([
            AuthorizationMode::AUTHORIZE,
            AuthorizationMode::DENY_ALL,
            proprietary,
            AuthorizationMode::DENY_ALL,
        ])
        .unwrap();
    for accepted in [AuthorizationMode::DENY_ALL, proprietary] {
        write(&mut point, MODE, mode(accepted)).unwrap();
        assert_eq!(read(&point, MODE), mode(accepted));
    }
    // A standard mode the application left out is refused like any other.
    assert_error(
        write(&mut point, MODE, mode(AuthorizationMode::GRANT_ACTIVE)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&point, MODE), mode(proprietary));
}

#[test]
fn access_point_supported_modes_refuse_sets_without_authorize_or_the_mode_in_effect() {
    use AuthorizationMode as M;
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_supported_authorization_modes([M::AUTHORIZE, M::GRANT_ACTIVE, M::DENY_ALL])
        .unwrap();
    write(&mut point, MODE, mode(M::DENY_ALL)).unwrap();
    for modes in [
        // AUTHORIZE is the mode every point carries out (Clause 12.31.14).
        vec![M::GRANT_ACTIVE, M::DENY_ALL],
        // The set would drop DENY_ALL, the mode in effect.
        vec![M::AUTHORIZE, M::GRANT_ACTIVE],
        // Reserved, and past the Unsigned16 range.
        vec![M::AUTHORIZE, M::DENY_ALL, M::from_raw(6)],
        vec![M::AUTHORIZE, M::DENY_ALL, M::from_raw(63)],
        vec![M::AUTHORIZE, M::DENY_ALL, M::from_raw(65_536)],
    ] {
        assert_error(
            point.set_supported_authorization_modes(modes),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    // The set declared before is kept: GRANT_ACTIVE is still taken, NONE
    // still refused.
    write(&mut point, MODE, mode(M::GRANT_ACTIVE)).unwrap();
    assert_eq!(read(&point, MODE), mode(M::GRANT_ACTIVE));
    assert_error(
        write(&mut point, MODE, mode(M::NONE)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn access_point_policy_count_below_the_active_policy_drops_it_to_zero() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point.set_number_of_authentication_policies(3).unwrap();
    write(&mut point, POLICY, PropertyValue::Unsigned(3)).unwrap();
    assert_error(
        point.set_number_of_authentication_policies(0),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(3));
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(3));
    // A count below the policy in effect leaves none in effect (Clause
    // 12.31.10), and the point reports the configuration fault (#1325).
    point.set_number_of_authentication_policies(2).unwrap();
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(2));
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(0));
    assert_eq!(
        read(&point, P::RELIABILITY),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    // Picking one of the remaining policies clears both.
    write(&mut point, POLICY, PropertyValue::Unsigned(2)).unwrap();
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(2));
    assert_eq!(
        read(&point, P::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    // A count at or above the policy in effect leaves it alone.
    point.set_number_of_authentication_policies(2).unwrap();
    point.set_number_of_authentication_policies(5).unwrap();
    assert_eq!(read(&point, POLICY), PropertyValue::Unsigned(2));
}

#[test]
fn access_point_count_and_priority_come_from_the_application() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point.set_priority_for_writing(8).unwrap();
    assert_eq!(read(&point, PRIORITY), PropertyValue::Unsigned(8));
    for priority in [0, 17, u8::MAX] {
        assert_error(
            point.set_priority_for_writing(priority),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read(&point, PRIORITY), PropertyValue::Unsigned(8));
    // Neither takes a network write, even of a value the setter accepts.
    for property in [PRIORITY, POLICIES] {
        assert_error(
            write(&mut point, property, PropertyValue::Unsigned(4)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(read(&point, PRIORITY), PropertyValue::Unsigned(8));
    assert_eq!(read(&point, POLICIES), PropertyValue::Unsigned(1));
}
