//! Access Point Authentication_Status and Access_Event_Credential, the
//! Table 12-36 required rows #1284 adds (Clauses 12.31.9, 12.31.27.1 and
//! 12.31.30).

use bacnet_types::enums::{ErrorCode, PropertyIdentifier as P};

use super::credential_data_input_out_of_service_tests::stamp;
use super::point_out_of_service_tests::{credential, CREDENTIAL_3, NO_CREDENTIAL};
use super::*;

fn status(point: &AccessPointObject) -> PropertyValue {
    point.read_property(P::AUTHENTICATION_STATUS, None).unwrap()
}

fn status_value(status: AuthenticationStatus) -> PropertyValue {
    PropertyValue::Enumerated(status.to_raw())
}

fn set_out_of_service(point: &mut AccessPointObject, out_of_service: bool) {
    point
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn assert_value_out_of_range(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { code, .. })
            if code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32),
        "expected VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

fn event_credential(point: &AccessPointObject) -> PropertyValue {
    point
        .read_property(P::ACCESS_EVENT_CREDENTIAL, None)
        .unwrap()
}

#[test]
fn access_point_authentication_status_reads_disabled_out_of_service() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_eq!(status(&point), status_value(AuthenticationStatus::READY));
    point
        .set_authentication_status(AuthenticationStatus::IN_PROGRESS)
        .unwrap();
    assert_eq!(
        status(&point),
        status_value(AuthenticationStatus::IN_PROGRESS)
    );

    // No authentication runs out of service, so DISABLED is served whatever
    // the application reports meanwhile.
    set_out_of_service(&mut point, true);
    assert_eq!(status(&point), status_value(AuthenticationStatus::DISABLED));
    point
        .set_authentication_status(AuthenticationStatus::WAITING_FOR_VERIFICATION)
        .unwrap();
    assert_eq!(status(&point), status_value(AuthenticationStatus::DISABLED));
    assert_eq!(
        point.authentication_status(),
        AuthenticationStatus::DISABLED
    );

    // The return to service serves the status last reported.
    set_out_of_service(&mut point, false);
    assert_eq!(
        status(&point),
        status_value(AuthenticationStatus::WAITING_FOR_VERIFICATION)
    );
}

#[test]
fn access_point_authentication_status_refuses_values_past_the_production() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_authentication_status(AuthenticationStatus::NOT_READY)
        .unwrap();
    // BACnetAuthenticationStatus is closed at IN_PROGRESS (6).
    for raw in [7, 64, u32::MAX] {
        assert_value_out_of_range(
            point.set_authentication_status(AuthenticationStatus::from_raw(raw)),
        );
        assert_eq!(
            status(&point),
            status_value(AuthenticationStatus::NOT_READY)
        );
    }
}

#[test]
fn access_point_access_event_stores_its_credential() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // Before any event: the no-credential reference.
    assert_eq!(
        event_credential(&point),
        PropertyValue::ApplicationData(NO_CREDENTIAL.to_vec())
    );
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(9)),
            credential: Some(credential()),
            ..AccessEventReport::new(AccessEvent::GRANTED, 1)
        })
        .unwrap();
    assert_eq!(
        event_credential(&point),
        PropertyValue::ApplicationData(CREDENTIAL_3.to_vec())
    );
    // An event without a credential stores the no-credential reference
    // again (Clause 12.31.30).
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(10)),
            credential: None,
            ..AccessEventReport::new(AccessEvent::DENIED_UNKNOWN_CREDENTIAL, 2)
        })
        .unwrap();
    assert_eq!(
        event_credential(&point),
        PropertyValue::ApplicationData(NO_CREDENTIAL.to_vec())
    );
}

#[test]
fn access_point_access_event_refuses_a_credential_of_another_type() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(9)),
            credential: Some(credential()),
            ..AccessEventReport::new(AccessEvent::GRANTED, 1)
        })
        .unwrap();
    let rows = [P::ACCESS_EVENT, P::ACCESS_EVENT_TAG, P::ACCESS_EVENT_TIME]
        .map(|property| point.read_property(property, None).unwrap());
    for object_type in [ObjectType::ACCESS_USER, ObjectType::CREDENTIAL_DATA_INPUT] {
        let other = ObjectIdentifier::new(object_type, 3).unwrap();
        assert_value_out_of_range(point.set_access_event(AccessEventReport {
            time: Some(stamp(10)),
            credential: Some(other.into()),
            ..AccessEventReport::new(AccessEvent::DENIED_OTHER, 2)
        }));
        assert_eq!(
            [P::ACCESS_EVENT, P::ACCESS_EVENT_TAG, P::ACCESS_EVENT_TIME]
                .map(|property| point.read_property(property, None).unwrap()),
            rows
        );
        assert_eq!(
            event_credential(&point),
            PropertyValue::ApplicationData(CREDENTIAL_3.to_vec())
        );
    }
}

#[test]
fn access_point_access_event_refuses_a_half_empty_credential() {
    let empty = ObjectIdentifier::MAX_INSTANCE;
    let reference = |object: u32, device: Option<u32>| BACnetDeviceObjectReference {
        device_identifier: device
            .map(|device| ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, object).unwrap(),
    };
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(9)),
            credential: Some(credential()),
            ..AccessEventReport::new(AccessEvent::GRANTED, 1)
        })
        .unwrap();
    // 4194303 in only one of the two instances is neither a credential nor
    // the no-credential reference (Clause 12.31.30), and nothing changes.
    for half_empty in [reference(empty, Some(9)), reference(3, Some(empty))] {
        assert_value_out_of_range(point.set_access_event(AccessEventReport {
            time: Some(stamp(10)),
            credential: Some(half_empty),
            ..AccessEventReport::new(AccessEvent::DENIED_OTHER, 2)
        }));
        assert_eq!(
            event_credential(&point),
            PropertyValue::ApplicationData(CREDENTIAL_3.to_vec())
        );
        assert_eq!(
            point.read_property(P::ACCESS_EVENT_TAG, None).unwrap(),
            PropertyValue::Unsigned(1)
        );
    }
    // Both empty with a device, or the object alone without one, is the
    // no-credential reference; a credential in another device is accepted.
    for accepted in [
        reference(empty, Some(empty)),
        reference(empty, None),
        reference(3, Some(9)),
    ] {
        point
            .set_access_event(AccessEventReport {
                time: Some(stamp(10)),
                credential: Some(accepted),
                ..AccessEventReport::new(AccessEvent::DENIED_OTHER, 2)
            })
            .unwrap();
    }
}
