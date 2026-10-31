use super::*;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use pyo3::exceptions::PyOverflowError;
use pyo3::types::{PyDict, PyList};

use crate::types::PyObjectIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn py_oid(py: Python<'_>, object_type: ObjectType, instance: u32) -> Bound<'_, PyAny> {
    Bound::new(
        py,
        PyObjectIdentifier::from_rust(oid(object_type, instance)),
    )
    .unwrap()
    .into_any()
}

fn present_value(py: Python<'_>) -> Bound<'_, PyAny> {
    Bound::new(
        py,
        PyPropertyIdentifier {
            inner: PropertyIdentifier::PRESENT_VALUE,
        },
    )
    .unwrap()
    .into_any()
}

fn tuple<'py>(py: Python<'py>, items: Vec<Bound<'py, PyAny>>) -> Bound<'py, PyAny> {
    PyTuple::new(py, items).unwrap().into_any()
}

fn int(py: Python<'_>, value: i64) -> Bound<'_, PyAny> {
    value.into_pyobject(py).unwrap().into_any()
}

/// AV-1's Present_Value as a reference mapping.
fn av1_mapping(py: Python<'_>) -> Bound<'_, PyDict> {
    let reference = PyDict::new(py);
    reference
        .set_item("object_identifier", py_oid(py, ObjectType::ANALOG_VALUE, 1))
        .unwrap();
    reference
        .set_item("property_identifier", present_value(py))
        .unwrap();
    reference
}

fn parse<'py>(
    py: Python<'py>,
    references: &[Bound<'py, PyAny>],
) -> PyResult<Vec<BACnetDeviceObjectPropertyReference>> {
    property_references_from_py(PyList::new(py, references).unwrap().as_any(), "members")
}

#[test]
fn property_reference_tuples_and_mappings_convert() {
    Python::initialize();
    Python::attach(|py| {
        let ao1 = || py_oid(py, ObjectType::ANALOG_OUTPUT, 1);
        let remote = av1_mapping(py);
        remote.set_item("property_array_index", 4).unwrap();
        remote
            .set_item("device_identifier", py_oid(py, ObjectType::DEVICE, 99))
            .unwrap();
        let nones = av1_mapping(py);
        nones.set_item("property_array_index", py.None()).unwrap();
        nones.set_item("device_identifier", py.None()).unwrap();

        let parsed = parse(
            py,
            &[
                tuple(py, vec![ao1(), present_value(py)]),
                tuple(py, vec![ao1(), present_value(py), int(py, 8)]),
                tuple(py, vec![ao1(), present_value(py), py.None().into_bound(py)]),
                av1_mapping(py).into_any(),
                remote.into_any(),
                nones.into_any(),
            ],
        )
        .unwrap();
        let pv = PropertyIdentifier::PRESENT_VALUE.to_raw();
        let ao1 =
            BACnetDeviceObjectPropertyReference::new_local(oid(ObjectType::ANALOG_OUTPUT, 1), pv);
        let av1 =
            BACnetDeviceObjectPropertyReference::new_local(oid(ObjectType::ANALOG_VALUE, 1), pv);
        assert_eq!(
            parsed,
            vec![
                ao1.clone(),
                ao1.clone().with_index(8),
                ao1,
                av1.clone(),
                BACnetDeviceObjectPropertyReference::new_remote(
                    oid(ObjectType::ANALOG_VALUE, 1),
                    pv,
                    oid(ObjectType::DEVICE, 99),
                )
                .with_index(4),
                av1,
            ]
        );
        assert_eq!(parse(py, &[]).unwrap(), vec![]);
    });
}

#[test]
fn property_reference_errors_follow_the_neighbouring_conversions() {
    Python::initialize();
    Python::attach(|py| {
        let ao1 = || py_oid(py, ObjectType::ANALOG_OUTPUT, 1);
        let pv = || present_value(py);

        // Not a list at all.
        let error = property_references_from_py(&int(py, 3), "members").unwrap_err();
        assert!(error.is_instance_of::<PyTypeError>(py), "{error}");

        let type_errors: Vec<Bound<'_, PyAny>> = vec![
            // A bare object, without its property.
            ao1(),
            // Too few and too many items.
            tuple(py, vec![ao1()]),
            tuple(py, vec![ao1(), pv(), int(py, 1), int(py, 2)]),
            // The items out of order, and a property given as a number.
            tuple(py, vec![pv(), ao1()]),
            tuple(py, vec![ao1(), int(py, 85)]),
            // An index that isn't an int.
            tuple(
                py,
                vec![ao1(), pv(), "1".into_pyobject(py).unwrap().into_any()],
            ),
            // A list where the tuple goes.
            PyList::new(py, [ao1(), pv()]).unwrap().into_any(),
            // A mapping whose property is a number.
            {
                let reference = av1_mapping(py);
                reference.set_item("property_identifier", 85).unwrap();
                reference.into_any()
            },
        ];
        for reference in type_errors {
            let error = parse(py, std::slice::from_ref(&reference)).unwrap_err();
            assert!(
                error.is_instance_of::<PyTypeError>(py),
                "{reference}: {error}"
            );
        }

        // An index outside unsigned32 overflows, as `add_group`'s do, in a
        // tuple or a mapping alike (#1360).
        for index in [-1, 1 << 32] {
            let mapping = av1_mapping(py);
            mapping.set_item("property_array_index", index).unwrap();
            for reference in [
                tuple(py, vec![ao1(), pv(), int(py, index)]),
                mapping.into_any(),
            ] {
                let error = parse(py, &[reference]).unwrap_err();
                assert!(error.is_instance_of::<PyOverflowError>(py), "{error}");
            }
        }

        let value_errors: Vec<Bound<'_, PyAny>> = vec![
            // An unknown key.
            {
                let reference = av1_mapping(py);
                reference.set_item("priority", 8).unwrap();
                reference.into_any()
            },
            // A missing key.
            {
                let reference = av1_mapping(py);
                reference.del_item("object_identifier").unwrap();
                reference.into_any()
            },
            // A device member that isn't a Device.
            {
                let reference = av1_mapping(py);
                reference.set_item("device_identifier", ao1()).unwrap();
                reference.into_any()
            },
        ];
        for reference in value_errors {
            let error = parse(py, std::slice::from_ref(&reference)).unwrap_err();
            assert!(
                error.is_instance_of::<PyValueError>(py),
                "{reference}: {error}"
            );
        }

        // The error names the element at fault.
        let good = tuple(py, vec![ao1(), pv()]);
        let error = parse(py, &[good, tuple(py, vec![ao1(), int(py, 85)])]).unwrap_err();
        assert!(error.to_string().contains("members[1] property"), "{error}");
    });
}

fn minimal(py: Python<'_>) -> Bound<'_, PyDict> {
    let reference = PyDict::new(py);
    reference
        .set_item(
            "object_identifier",
            PyObjectIdentifier::from_rust(oid(ObjectType::ANALOG_VALUE, 1)),
        )
        .unwrap();
    reference
        .set_item(
            "property_identifier",
            PyPropertyIdentifier {
                inner: PropertyIdentifier::PRESENT_VALUE,
            },
        )
        .unwrap();
    reference
}

#[test]
fn reference_mappings_fill_every_field_and_default_the_optional_ones() {
    Python::initialize();
    Python::attach(|py| {
        let full = minimal(py);
        full.set_item("property_array_index", 3).unwrap();
        full.set_item(
            "device_identifier",
            PyObjectIdentifier::from_rust(oid(ObjectType::DEVICE, 9)),
        )
        .unwrap();
        let nones = minimal(py);
        for key in OPTIONAL {
            nones.set_item(*key, py.None()).unwrap();
        }
        let list = PyList::new(py, [minimal(py), full, nones]).unwrap();
        let plain = BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        );
        assert_eq!(
            property_references_from_py(list.as_any(), "members").unwrap(),
            [
                plain.clone(),
                BACnetDeviceObjectPropertyReference {
                    property_array_index: Some(3),
                    device_identifier: Some(oid(ObjectType::DEVICE, 9)),
                    ..plain.clone()
                },
                plain,
            ]
        );
    });
}

fn listed<'py>(py: Python<'py>, reference: Bound<'py, PyDict>) -> Bound<'py, PyList> {
    PyList::new(py, [reference]).unwrap()
}

#[test]
fn reference_mappings_refuse_bad_shapes_and_types() {
    Python::initialize();
    Python::attach(|py| {
        let error =
            |value: &Bound<'_, PyAny>| property_references_from_py(value, "members").unwrap_err();
        let type_error = |value: &Bound<'_, PyAny>, needle: &str| {
            let error = error(value);
            assert!(error.is_instance_of::<PyTypeError>(py), "{error}");
            assert!(error.to_string().contains(needle), "{error}");
        };
        let value_error = |value: &Bound<'_, PyAny>, needle: &str| {
            let error = error(value);
            assert!(error.is_instance_of::<PyValueError>(py), "{error}");
            assert!(error.to_string().contains(needle), "{error}");
        };
        type_error(
            pyo3::types::PyString::new(py, "x").as_any(),
            "members must be a list",
        );
        type_error(
            PyList::new(py, [1]).unwrap().as_any(),
            "members[0] must be an (object, property)",
        );
        let missing = minimal(py);
        missing.del_item("property_identifier").unwrap();
        value_error(
            listed(py, missing).as_any(),
            "missing required key 'property_identifier'",
        );
        let unknown = minimal(py);
        unknown.set_item("array_index", 1).unwrap();
        value_error(listed(py, unknown).as_any(), "unknown key 'array_index'");
        let wide = minimal(py);
        wide.set_item("property_array_index", 1_u64 << 32).unwrap();
        let overflow = error(listed(py, wide).as_any());
        assert!(overflow.is_instance_of::<PyOverflowError>(py), "{overflow}");
        assert!(
            overflow
                .to_string()
                .contains("members[0].property_array_index"),
            "{overflow}"
        );
        let not_a_device = minimal(py);
        not_a_device
            .set_item(
                "device_identifier",
                PyObjectIdentifier::from_rust(oid(ObjectType::ANALOG_INPUT, 9)),
            )
            .unwrap();
        value_error(
            listed(py, not_a_device).as_any(),
            "members[0]: the device must be a Device object identifier",
        );
        for (key, bad) in [
            (
                "property_identifier",
                85_i64.into_pyobject(py).unwrap().into_any(),
            ),
            (
                "object_identifier",
                1_i64.into_pyobject(py).unwrap().into_any(),
            ),
            (
                "device_identifier",
                9_i64.into_pyobject(py).unwrap().into_any(),
            ),
            (
                "property_array_index",
                true.into_pyobject(py).unwrap().to_owned().into_any(),
            ),
        ] {
            let reference = minimal(py);
            reference.set_item(key, bad).unwrap();
            type_error(listed(py, reference).as_any(), key);
        }
    });
}

#[test]
fn own_device_names_the_server_device() {
    assert_eq!(local_device(1262), oid(ObjectType::DEVICE, 1262));
    assert_eq!(local_device(0), oid(ObjectType::DEVICE, 0));
}

#[test]
fn localize_drops_only_the_own_device() {
    let pv = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let local =
        BACnetDeviceObjectPropertyReference::new_local(oid(ObjectType::ANALOG_VALUE, 1), pv);
    let naming = |device: ObjectIdentifier| BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device),
        ..local.clone()
    };
    let own = oid(ObjectType::DEVICE, 1262);
    let other = oid(ObjectType::DEVICE, 99);
    let wildcard = oid(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE);
    let given = vec![naming(own), naming(other), local.clone(), naming(wildcard)];

    let mut references = given.clone();
    localize(&mut references, local_device(1262));
    assert_eq!(
        references,
        vec![
            local.clone(),
            naming(other),
            local.clone(),
            naming(wildcard)
        ]
    );
}
