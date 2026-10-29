use super::*;
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::types::PyDict;
use std::ffi::CStr;

use crate::types::PyObjectIdentifier;

/// Evaluate `source` with `device` bound to Device 99, and read the result as
/// one destination.
fn read(source: &CStr) -> PyResult<BACnetDestination> {
    Python::initialize();
    Python::attach(|py| {
        let locals = PyDict::new(py);
        let device = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
        locals.set_item("device", PyObjectIdentifier::from_rust(device))?;
        let value = py.eval(source, None, Some(&locals))?;
        destination(&value, "recipients[0]")
    })
}

fn device_99() -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap())
}

/// The two required keys alone give a destination that is active every day,
/// all day, for every transition, with unconfirmed notifications.
#[test]
fn required_keys_alone_give_an_always_active_destination() {
    let destination = read(
        c"{'recipient': {'kind': 'device', 'object_identifier': device}, 'process_identifier': 7}",
    )
    .unwrap();
    assert_eq!(
        destination,
        BACnetDestination {
            valid_days: DaysOfWeek::all(),
            from_time: Time {
                hour: 0,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            to_time: Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            },
            recipient: device_99(),
            process_identifier: 7,
            issue_confirmed_notifications: false,
            transitions: EventTransitionBits::all(),
        }
    );
}

#[test]
fn every_key_reaches_its_member() {
    let destination = read(
        cr"{
            'recipient': {'kind': 'address', 'network_number': 5, 'mac_address': b'\x0a\x0b'},
            'process_identifier': 4294967295,
            'valid_days': 0b0011111,
            'from_time': (8, 0, 0, 0),
            'to_time': (17, 30, 15, 50),
            'issue_confirmed_notifications': True,
            'transitions': 0b101,
        }",
    )
    .unwrap();
    assert_eq!(
        destination,
        BACnetDestination {
            valid_days: DaysOfWeek::MONDAY
                | DaysOfWeek::TUESDAY
                | DaysOfWeek::WEDNESDAY
                | DaysOfWeek::THURSDAY
                | DaysOfWeek::FRIDAY,
            from_time: Time {
                hour: 8,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            to_time: Time {
                hour: 17,
                minute: 30,
                second: 15,
                hundredths: 50,
            },
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: 5,
                mac_address: MacAddr::from_slice(&[0x0a, 0x0b]),
            }),
            process_identifier: u32::MAX,
            issue_confirmed_notifications: true,
            transitions: EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
        }
    );
}

/// Shapes and types are checked here, each error naming the member; what the
/// list accepts is left to the object.
#[test]
fn malformed_destinations_raise_value_or_type_errors() {
    let device = "'recipient': {'kind': 'device', 'object_identifier': device}";
    for (members, expected) in [
        (
            "[]".to_owned(),
            "TypeError: recipients[0] must be a mapping",
        ),
        (
            format!("{{{device}}}"),
            "ValueError: recipients[0] is missing required key 'process_identifier'",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'days': 1}}"),
            "ValueError: recipients[0] contains unknown key 'days'",
        ),
        (
            format!("{{{device}, 'process_identifier': 2**32}}"),
            "OverflowError: recipients[0].process_identifier must be 0..=4294967295, got 4294967296",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'valid_days': 256}}"),
            "OverflowError: recipients[0].valid_days must be 0..=255, got 256",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'valid_days': 128}}"),
            "ValueError: recipients[0].valid_days must be 0..=127",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'valid_days': True}}"),
            "TypeError: recipients[0].valid_days must be an integer",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'transitions': 8}}"),
            "ValueError: recipients[0].transitions must be 0..=7",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'issue_confirmed_notifications': 1}}"),
            "TypeError: recipients[0].issue_confirmed_notifications must be a bool",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'from_time': (24, 0, 0, 0)}}"),
            "ValueError: hour must be 0..=23 or 255 (unspecified)",
        ),
        (
            format!("{{{device}, 'process_identifier': 1, 'to_time': (23, 59)}}"),
            "ValueError: recipients[0].to_time must be a tuple of exactly 4 integers",
        ),
        (
            "{'recipient': {'kind': 'broadcast'}, 'process_identifier': 1}".to_owned(),
            "ValueError: recipients[0].recipient.kind must be 'device' or 'address'",
        ),
    ] {
        let source = std::ffi::CString::new(members.clone()).unwrap();
        let error = read(&source).unwrap_err();
        let message = Python::attach(|py| {
            let kind = if error.is_instance_of::<PyValueError>(py) {
                "ValueError"
            } else if error.is_instance_of::<pyo3::exceptions::PyOverflowError>(py) {
                "OverflowError"
            } else if error.is_instance_of::<PyTypeError>(py) {
                "TypeError"
            } else {
                "other"
            };
            format!("{kind}: {}", error.value(py))
        });
        assert!(message.starts_with(expected), "{members}: {message}");
    }
}
