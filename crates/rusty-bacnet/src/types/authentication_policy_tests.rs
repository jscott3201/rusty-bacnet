//! The Python form of an Access Point's authentication policies (#1325):
//! the keyword's shape checks, and a typed read of Authentication_Policy_List
//! that gives back the form the keyword takes.

use super::*;
use crate::types::read_value::decode_read_value;
use crate::types::PyObjectIdentifier;
use bacnet_encoding::constructed::encode_authentication_policy;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::types::{PyDict, PyList};
use std::ffi::CStr;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Evaluate `source` with `cdi1`, `cdi2` (Credential Data Inputs 1 and 2),
/// `device` (Device 99) and `av` (Analog Value 9) bound.
fn eval<'py>(py: Python<'py>, source: &CStr) -> Bound<'py, PyAny> {
    let locals = PyDict::new(py);
    for (name, id) in [
        ("cdi1", oid(ObjectType::CREDENTIAL_DATA_INPUT, 1)),
        ("cdi2", oid(ObjectType::CREDENTIAL_DATA_INPUT, 2)),
        ("device", oid(ObjectType::DEVICE, 99)),
        ("av", oid(ObjectType::ANALOG_VALUE, 9)),
    ] {
        locals
            .set_item(name, PyObjectIdentifier::from_rust(id))
            .unwrap();
    }
    py.eval(source, None, Some(&locals)).unwrap()
}

#[test]
fn policies_read_from_python_as_named_pairs() {
    Python::initialize();
    Python::attach(|py| {
        let policies = eval(
            py,
            c"[('card', ([(cdi1, 1)], True, 30)),
               ('card and PIN', ([(cdi1, 1), ((device, cdi2), 2)], False, 0)),
               ('empty', ([], False, 0))]",
        );
        let parsed = authentication_policies_from_py(&policies, "policies").unwrap();
        let names: Vec<_> = parsed.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["card", "card and PIN", "empty"]);
        let pin = &parsed[1].1;
        assert!(!pin.order_enforced);
        assert_eq!(pin.timeout, 0);
        assert_eq!(pin.policy[0].index, 1);
        assert_eq!(
            pin.policy[1].credential_data_input,
            BACnetDeviceObjectReference {
                device_identifier: Some(oid(ObjectType::DEVICE, 99)),
                object_identifier: oid(ObjectType::CREDENTIAL_DATA_INPUT, 2),
            }
        );
        assert!(parsed[2].1.policy.is_empty());
        // The point judges what a policy names; this layer only its shape.
        let kept = eval(py, c"[('other', ([(av, 7)], True, 1))]");
        assert_eq!(
            authentication_policies_from_py(&kept, "policies").unwrap()[0]
                .1
                .policy[0]
                .index,
            7
        );
    });
}

#[test]
fn malformed_policies_raise_naming_the_element() {
    Python::initialize();
    Python::attach(|py| {
        for (source, kind, needle) in [
            (c"{}", "TypeError", "policies must be a list"),
            (
                c"[([(cdi1, 1)], True, 30)]",
                "TypeError",
                "policies[0] must be a (name, policy) pair",
            ),
            (
                c"[('x', ([(cdi1, 1)], True))]",
                "TypeError",
                "policies[0] must be an (entries",
            ),
            (
                c"[('x', ([(cdi1, 1)], 1, 30))]",
                "TypeError",
                "order_enforced must be a bool",
            ),
            (
                c"[('x', (5, True, 30))]",
                "TypeError",
                "entries must be a list",
            ),
            (
                c"[('x', ([cdi1], True, 30))]",
                "TypeError",
                "entries[0] must be a (reference, index)",
            ),
            (
                c"[('x', ([(1, 1)], True, 30))]",
                "TypeError",
                "must be an ObjectIdentifier",
            ),
            (
                c"[('x', ([((av, cdi1), 1)], True, 30))]",
                "ValueError",
                "entries[0]",
            ),
            (c"[('x', ([(cdi1, -1)], True, 30))]", "OverflowError", ""),
            (c"[('x', ([(cdi1, 1)], True, 2**32))]", "OverflowError", ""),
        ] {
            let error = authentication_policies_from_py(&eval(py, source), "policies").unwrap_err();
            let matches = match kind {
                "TypeError" => error.is_instance_of::<PyTypeError>(py),
                "ValueError" => error.is_instance_of::<PyValueError>(py),
                _ => error.is_instance_of::<PyOverflowError>(py),
            };
            assert!(matches, "{source:?}: {error}");
            assert!(error.to_string().contains(needle), "{source:?}: {error}");
        }
    });
}

#[test]
fn a_policy_list_reads_back_as_the_keyword_takes_it() {
    Python::initialize();
    Python::attach(|py| {
        let policies = eval(
            py,
            c"[([(cdi1, 1)], True, 30), ([(cdi1, 1), ((device, cdi2), 2)], False, 0)]",
        );
        let pairs = PyList::empty(py);
        for policy in policies.try_iter().unwrap() {
            pairs.append(("name", policy.unwrap())).unwrap();
        }
        let parsed = authentication_policies_from_py(&pairs, "policies").unwrap();
        let mut octets = BytesMut::new();
        let mut ends = Vec::new();
        for (_, policy) in &parsed {
            encode_authentication_policy(&mut octets, policy);
            ends.push(octets.len());
        }
        let read = |index: Option<u32>, octets: &[u8]| {
            let value = decode_read_value(
                ObjectType::ACCESS_POINT,
                PropertyIdentifier::AUTHENTICATION_POLICY_LIST,
                index,
                octets,
            )
            .unwrap();
            let mut encoded = BytesMut::new();
            encode_property_value(&mut encoded, &value.inner).unwrap();
            assert_eq!(encoded.to_vec(), octets);
            Bound::new(py, value).unwrap()
        };
        let whole = read(None, &octets);
        assert_eq!(
            whole.getattr("tag").unwrap().extract::<String>().unwrap(),
            "list"
        );
        assert!(whole.getattr("value").unwrap().eq(&policies).unwrap());
        let mut start = 0;
        for (index, &end) in ends.iter().enumerate() {
            let one = read(Some(index as u32 + 1), &octets[start..end]);
            assert_eq!(
                one.getattr("tag").unwrap().extract::<String>().unwrap(),
                "authentication_policy"
            );
            assert!(one
                .getattr("value")
                .unwrap()
                .eq(policies.get_item(index).unwrap())
                .unwrap());
            start = end;
        }
        // Another property's octets, or a policy cut short, stay generic.
        let other = decode_read_value(
            ObjectType::ACCESS_ZONE,
            PropertyIdentifier::AUTHENTICATION_POLICY_LIST,
            None,
            &octets,
        )
        .unwrap();
        assert_eq!(other.element, None);
        let cut = decode_read_value(
            ObjectType::ACCESS_POINT,
            PropertyIdentifier::AUTHENTICATION_POLICY_LIST,
            None,
            &octets[..octets.len() - 1],
        );
        assert!(cut.is_err() || cut.unwrap().element.is_none());
    });
}
