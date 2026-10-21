//! The Access Credential's Table 12-40 required rows (#1073): the derived
//! Credential_Status and Reason_For_Disable, Credential_Disable, the
//! validity window against the wall clock, Global_Identifier, and the two
//! constructed arrays.

use std::sync::{Arc, Mutex};

use bacnet_encoding::constructed::{
    decode_assigned_access_rights, decode_credential_authentication_factor,
};
use bacnet_types::constructed::{
    BACnetAssignedAccessRights, BACnetAuthenticationFactor, BACnetCredentialAuthenticationFactor,
    BACnetDeviceObjectReference,
};
use bacnet_types::enums::{
    AccessAuthenticationFactorDisable as FactorDisable, AccessCredentialDisable,
    AccessCredentialDisableReason as Reason, AuthenticationFactorType, ErrorClass, ErrorCode,
};

use super::*;
use crate::clock::{ClockFrame, ClockReader};

const CS: PropertyIdentifier = PropertyIdentifier::CREDENTIAL_STATUS;
const RFD: PropertyIdentifier = PropertyIdentifier::REASON_FOR_DISABLE;
const CD: PropertyIdentifier = PropertyIdentifier::CREDENTIAL_DISABLE;
const AT: PropertyIdentifier = PropertyIdentifier::ACTIVATION_TIME;
const ET: PropertyIdentifier = PropertyIdentifier::EXPIRATION_TIME;
const AF: PropertyIdentifier = PropertyIdentifier::AUTHENTICATION_FACTORS;
const AAR: PropertyIdentifier = PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS;

/// A wall clock the test moves by hand; `None` reads as no clock frame.
struct TestClock(Mutex<Option<ClockFrame>>);

impl ClockReader for TestClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
    }
}

fn date(year: u16, month: u8, day: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week: Date::UNSPECIFIED,
    }
}

fn time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn frame(date: Date, time: Time) -> Option<ClockFrame> {
    Some(ClockFrame {
        local_date: date,
        local_time: time,
        utc_offset: 0,
        daylight_savings_status: false,
    })
}

fn date_time(date: Date, time: Time) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

fn reasons(raw: &[u32]) -> PropertyValue {
    PropertyValue::List(raw.iter().map(|&r| PropertyValue::Enumerated(r)).collect())
}

fn assert_property_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY / {expected:?}, got {other:?}"),
    }
}

fn read(object: &AccessCredentialObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn status_and_reasons(object: &AccessCredentialObject) -> (PropertyValue, PropertyValue) {
    (read(object, CS), read(object, RFD))
}

#[test]
fn access_credential_status_follows_credential_disable() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(
        status_and_reasons(&credential),
        (PropertyValue::Enumerated(1), reasons(&[]))
    );
    // Each named value adds its reason, a vendor value adds DISABLED, and a
    // change drops the reason the previous value added.
    for (disable, added) in [(1, 0), (2, 9), (3, 5), (64, 0), (65535, 0)] {
        credential
            .write_property(CD, None, PropertyValue::Enumerated(disable), None)
            .unwrap();
        assert_eq!(read(&credential, CD), PropertyValue::Enumerated(disable));
        assert_eq!(
            status_and_reasons(&credential),
            (PropertyValue::Enumerated(0), reasons(&[added])),
            "Credential_Disable {disable}"
        );
    }
    credential
        .write_property(CD, None, PropertyValue::Enumerated(0), None)
        .unwrap();
    assert_eq!(
        status_and_reasons(&credential),
        (PropertyValue::Enumerated(1), reasons(&[]))
    );
}

#[test]
fn access_credential_disable_refuses_reserved_and_mistyped_values() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .set_credential_disable(AccessCredentialDisable::DISABLE_MANUAL)
        .unwrap();
    for raw in [4, 63, 65536, u32::MAX] {
        assert_property_error(
            credential.write_property(CD, None, PropertyValue::Enumerated(raw), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_property_error(
            credential.set_credential_disable(AccessCredentialDisable::from_raw(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for value in [PropertyValue::Unsigned(1), PropertyValue::Null] {
        assert_property_error(
            credential.write_property(CD, None, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(read(&credential, CD), PropertyValue::Enumerated(2));
    assert_eq!(read(&credential, RFD), reasons(&[9]));
}

#[test]
fn access_credential_local_reasons_merge_with_credential_disable() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .add_disable_reason(Reason::DISABLED_LOCKOUT)
        .unwrap();
    credential
        .add_disable_reason(Reason::from_raw(700))
        .unwrap();
    credential
        .add_disable_reason(Reason::DISABLED_LOCKOUT)
        .unwrap();
    credential
        .add_disable_reason(Reason::DISABLED_NEEDS_PROVISIONING)
        .unwrap();
    assert_eq!(read(&credential, RFD), reasons(&[1, 5, 700]));
    // Credential_Disable's lockout reason and the local one coincide; the
    // list names it once, and dropping one source keeps the other's.
    credential
        .set_credential_disable(AccessCredentialDisable::DISABLE_LOCKOUT)
        .unwrap();
    assert_eq!(read(&credential, RFD), reasons(&[1, 5, 700]));
    assert!(credential.remove_disable_reason(Reason::DISABLED_LOCKOUT));
    assert!(!credential.remove_disable_reason(Reason::DISABLED_LOCKOUT));
    assert_eq!(read(&credential, RFD), reasons(&[1, 5, 700]));
    credential
        .set_credential_disable(AccessCredentialDisable::NONE)
        .unwrap();
    assert_eq!(read(&credential, RFD), reasons(&[1, 700]));
    assert!(credential.remove_disable_reason(Reason::DISABLED_NEEDS_PROVISIONING));
    assert!(credential.remove_disable_reason(Reason::from_raw(700)));
    assert_eq!(credential.credential_status(), BinaryPV::ACTIVE);
    assert!(credential.reason_for_disable().is_empty());

    // The window's two reasons, reserved values and anything past 16 bits
    // are not the application's to raise.
    for raw in [3, 4, 10, 63, 65536] {
        assert_property_error(
            credential.add_disable_reason(Reason::from_raw(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read(&credential, RFD), reasons(&[]));
}

#[test]
fn access_credential_validity_window_follows_the_clock() {
    let clock = Arc::new(TestClock(Mutex::new(None)));
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential.bind_clock_internal(Some(clock.clone()));
    credential
        .write_property(AT, None, date_time(date(2026, 3, 1), time(8, 0)), None)
        .unwrap();
    credential
        .set_expiration_time(date(2026, 3, 31), time(17, 30))
        .unwrap();
    assert_eq!(
        read(&credential, AT),
        date_time(date(2026, 3, 1), time(8, 0))
    );
    assert_eq!(
        read(&credential, ET),
        date_time(date(2026, 3, 31), time(17, 30))
    );

    // Without a clock frame the window is not judged.
    assert_eq!(read(&credential, RFD), reasons(&[]));
    for (now, expected) in [
        (frame(date(2026, 2, 28), time(23, 59)), reasons(&[3])),
        (frame(date(2026, 3, 1), time(7, 59)), reasons(&[3])),
        // The bounds themselves are inside the window.
        (frame(date(2026, 3, 1), time(8, 0)), reasons(&[])),
        (frame(date(2026, 3, 31), time(17, 30)), reasons(&[])),
        (frame(date(2026, 3, 31), time(17, 31)), reasons(&[4])),
        (frame(date(2027, 1, 1), time(0, 0)), reasons(&[4])),
        // A frame that names no real moment counts as no clock.
        (frame(date(2026, 2, 30), time(12, 0)), reasons(&[])),
        (
            frame(
                date(2026, 3, 15),
                Time {
                    hour: 0xFF,
                    ..time(0, 0)
                },
            ),
            reasons(&[]),
        ),
    ] {
        *clock.0.lock().unwrap() = now;
        let status = if expected == reasons(&[]) { 1 } else { 0 };
        assert_eq!(
            status_and_reasons(&credential),
            (PropertyValue::Enumerated(status), expected),
            "{now:?}"
        );
    }

    // All-X'FF' limits leave both ends open.
    let open = (
        Date {
            year: 0xFF,
            ..date(2000, 0xFF, 0xFF)
        },
        Time {
            hour: 0xFF,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        },
    );
    credential.set_activation_time(open.0, open.1).unwrap();
    credential
        .write_property(ET, None, date_time(open.0, open.1), None)
        .unwrap();
    *clock.0.lock().unwrap() = frame(date(2154, 12, 31), time(23, 59));
    assert_eq!(read(&credential, RFD), reasons(&[]));
    assert_eq!(credential.expiration_time(), open);
}

#[test]
fn access_credential_validity_window_refuses_partial_and_mistyped_limits() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .set_activation_time(date(2026, 3, 1), time(8, 0))
        .unwrap();
    let partial = [
        // A wildcard year, a special month and an impossible day.
        date_time(
            Date {
                year: 0xFF,
                ..date(2026, 3, 1)
            },
            time(8, 0),
        ),
        date_time(date(2026, 13, 1), time(8, 0)),
        date_time(date(2026, 2, 29), time(8, 0)),
        // A specific date with an unspecified hour, and the reverse.
        date_time(
            date(2026, 3, 1),
            Time {
                hour: 0xFF,
                ..time(8, 0)
            },
        ),
        date_time(
            Date {
                year: 0xFF,
                month: 0xFF,
                day: 0xFF,
                day_of_week: 0xFF,
            },
            time(8, 0),
        ),
    ];
    for value in partial {
        for property in [AT, ET] {
            assert_property_error(
                credential.write_property(property, None, value.clone(), None),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
    }
    for value in [
        PropertyValue::Date(date(2026, 3, 1)),
        PropertyValue::List(vec![PropertyValue::Date(date(2026, 3, 1))]),
        PropertyValue::List(vec![
            PropertyValue::Time(time(8, 0)),
            PropertyValue::Date(date(2026, 3, 1)),
        ]),
        PropertyValue::Null,
    ] {
        assert_property_error(
            credential.write_property(AT, None, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(credential.activation_time(), (date(2026, 3, 1), time(8, 0)));
}

#[test]
fn access_credential_global_identifier_is_unsigned32() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    let gi = PropertyIdentifier::GLOBAL_IDENTIFIER;
    assert_eq!(read(&credential, gi), PropertyValue::Unsigned(0));
    credential
        .write_property(gi, None, PropertyValue::Unsigned(u32::MAX.into()), None)
        .unwrap();
    assert_eq!(
        read(&credential, gi),
        PropertyValue::Unsigned(u32::MAX.into())
    );
    assert_property_error(
        credential.write_property(gi, None, PropertyValue::Unsigned(1 << 32), None),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        credential.write_property(gi, None, PropertyValue::Signed(1), None),
        ErrorCode::INVALID_DATA_TYPE,
    );
    credential.set_global_identifier(42);
    assert_eq!(read(&credential, gi), PropertyValue::Unsigned(42));
}

fn factor(disable: u32, format: u32, value: &[u8]) -> BACnetCredentialAuthenticationFactor {
    BACnetCredentialAuthenticationFactor {
        disable: FactorDisable::from_raw(disable),
        authentication_factor: BACnetAuthenticationFactor {
            format_type: AuthenticationFactorType::from_raw(format),
            format_class: 1,
            value: value.to_vec(),
        },
    }
}

fn rights(
    object_type: ObjectType,
    instance: u32,
    device: Option<ObjectType>,
) -> BACnetAssignedAccessRights {
    BACnetAssignedAccessRights {
        assigned_access_rights: BACnetDeviceObjectReference {
            device_identifier: device.map(|t| ObjectIdentifier::new(t, 9).unwrap()),
            object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
        },
        enable: instance.is_multiple_of(2),
    }
}

/// A codec's decoder for one element at an offset.
type Decoder<T> = fn(&[u8], usize) -> Result<(T, usize), Error>;

/// Decode every framed element of a whole-array read.
fn elements<T>(value: PropertyValue, decode: Decoder<T>) -> Vec<T> {
    let PropertyValue::List(items) = value else {
        panic!("a whole array reads as a list");
    };
    items
        .into_iter()
        .map(|item| {
            let PropertyValue::ApplicationData(bytes) = item else {
                panic!("each element is framed");
            };
            let (element, end) = decode(&bytes, 0).unwrap();
            assert_eq!(end, bytes.len());
            element
        })
        .collect()
}

#[test]
fn access_credential_arrays_read_whole_by_index_and_by_size() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    let factors = vec![
        factor(0, 8, &[1, 2, 3]),
        factor(2, 24, b"pw"),
        factor(64, 0, &[]),
    ];
    credential
        .set_authentication_factors(factors.clone())
        .unwrap();
    let assigned = vec![
        rights(ObjectType::ACCESS_RIGHTS, 2, None),
        rights(ObjectType::ACCESS_RIGHTS, 3, Some(ObjectType::DEVICE)),
        // The unused marker, whatever its type.
        rights(
            ObjectType::ANALOG_INPUT,
            ObjectIdentifier::MAX_INSTANCE,
            None,
        ),
    ];
    credential
        .set_assigned_access_rights(assigned.clone())
        .unwrap();

    assert_eq!(
        elements(
            read(&credential, AF),
            decode_credential_authentication_factor
        ),
        factors
    );
    assert_eq!(
        elements(read(&credential, AAR), decode_assigned_access_rights),
        assigned
    );
    for property in [AF, AAR] {
        assert_eq!(
            credential.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_property_error(
            credential.read_property(property, Some(4)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
    let PropertyValue::ApplicationData(second) = credential.read_property(AF, Some(2)).unwrap()
    else {
        panic!("an element reads framed");
    };
    assert_eq!(
        decode_credential_authentication_factor(&second, 0)
            .unwrap()
            .0,
        factors[1]
    );
    assert_eq!(credential.authentication_factors(), factors.as_slice());
    assert_eq!(credential.assigned_access_rights(), assigned.as_slice());
}

#[test]
fn access_credential_arrays_refuse_bad_elements_and_network_writes() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    credential
        .set_authentication_factors(vec![factor(0, 8, &[1])])
        .unwrap();
    credential
        .set_assigned_access_rights(vec![rights(ObjectType::ACCESS_RIGHTS, 2, None)])
        .unwrap();
    let before = (read(&credential, AF), read(&credential, AAR));
    // A reserved disable value, one past 16 bits, and a format past the
    // closed set; one bad element refuses the whole replacement.
    for bad in [factor(6, 8, &[]), factor(65536, 8, &[]), factor(0, 25, &[])] {
        assert_property_error(
            credential.set_authentication_factors(vec![factor(0, 8, &[9]), bad]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for bad in [
        rights(ObjectType::ACCESS_USER, 2, None),
        rights(ObjectType::ACCESS_RIGHTS, 2, Some(ObjectType::ANALOG_VALUE)),
    ] {
        assert_property_error(
            credential.set_assigned_access_rights(vec![bad]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    // Table 12-40 makes both R; the network can't write them, whole or by
    // element.
    for property in [AF, AAR] {
        assert!(!credential.is_writable_property(property));
        for index in [None, Some(1)] {
            let value = credential.read_property(property, index).unwrap();
            assert_property_error(
                credential.write_property(property, index, value, None),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
    }
    assert_property_error(
        credential.write_property(RFD, None, reasons(&[]), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!((read(&credential, AF), read(&credential, AAR)), before);
}
