//! Independent reference octets and the distinction between read fallback layers.
use super::*;
use crate::types::read_value::decode_read_value;
use crate::types::rpm_wpm::read_access_result_to_py;
use bacnet_services::rpm::ReadResultElement;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use pyo3::types::{PyDict, PyList};

const PROPERTY: &[u8] = &[0x0c, 0, 0, 0, 7, 0x19, 85];
const OBJECT: &[u8] = &[0x1c, 5, 0xc0, 0, 7];
const CASES: [(ObjectType, PropertyIdentifier, Element, &[u8]); 6] = [
    (
        ObjectType::TREND_LOG,
        PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
        Element::DeviceObjectPropertyReference,
        PROPERTY,
    ),
    (
        ObjectType::AVERAGING,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        Element::DeviceObjectPropertyReference,
        PROPERTY,
    ),
    (
        ObjectType::EVENT_ENROLLMENT,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        Element::DeviceObjectPropertyReference,
        PROPERTY,
    ),
    (
        ObjectType::ACCESS_POINT,
        PropertyIdentifier::ACCESS_EVENT_CREDENTIAL,
        Element::DeviceObjectReference,
        OBJECT,
    ),
    (
        ObjectType::LIFT,
        PropertyIdentifier::ENERGY_METER_REF,
        Element::DeviceObjectReference,
        OBJECT,
    ),
    (
        ObjectType::ESCALATOR,
        PropertyIdentifier::ENERGY_METER_REF,
        Element::DeviceObjectReference,
        OBJECT,
    ),
];

#[test]
fn six_single_reference_mappings_keep_exact_octets_and_outer_index_boundaries() {
    for (object, property, element, octets) in CASES {
        let value = decode_read_value(object, property, None, octets).unwrap();
        assert_eq!(value.element, Some(element));
        assert_eq!(value.inner, PropertyValue::ApplicationData(octets.to_vec()));
        for index in [Some(0), Some(1)] {
            let value = decode_read_value(object, property, index, octets).unwrap();
            assert_eq!(value.element, None);
            assert_eq!(value.inner, PropertyValue::ApplicationData(octets.to_vec()));
        }
        let twice = [octets, octets].concat();
        let value = decode_read_value(object, property, None, &twice).unwrap();
        assert_eq!(value.element, None);
        assert_eq!(value.inner, PropertyValue::ApplicationData(twice));
    }
    // Same property on an object outside the six pairings is not reclassified.
    assert_eq!(
        element(
            ObjectType::ANALOG_INPUT,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE
        ),
        None
    );
    assert_eq!(
        element(
            ObjectType::TREND_LOG_MULTIPLE,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY
        ),
        Some((Element::DeviceObjectPropertyReference, Shape::Collection))
    );
}

#[test]
fn malformed_references_preserve_context_primitive_and_framing_fallbacks() {
    for (object, property, _, _) in CASES {
        for bytes in [&[0x2c, 0, 0, 0, 7][..], &[0x72, 0, 0xff][..]] {
            let value = decode_read_value(object, property, None, bytes).unwrap();
            assert_eq!(value.element, None);
            assert_eq!(value.inner, PropertyValue::ApplicationData(bytes.to_vec()));
        }
        let primitive = decode_read_value(object, property, None, &[0x21, 7]).unwrap();
        assert_eq!(primitive.element, None);
        assert_eq!(primitive.inner, PropertyValue::Unsigned(7));
        for broken in [&[0x0c, 0, 0][..], &[0x0e, 0x1f][..]] {
            assert!(decode_read_value(object, property, None, broken).is_err());
        }
    }
}

#[test]
fn rpm_converter_keeps_typed_raw_value_raw_bytes_and_embedded_error_distinct() {
    let property = PropertyIdentifier::OBJECT_PROPERTY_REFERENCE;
    let wrong = vec![0x2c, 0, 0, 0, 7];
    let broken = vec![0x0c, 0, 0];
    let row = |value| ReadResultElement {
        property_identifier: property,
        property_array_index: None,
        property_value: Some(value),
        error: None,
    };
    // The service parser may reject broken framing before producing this model.
    // This deliberately exercises the converter's retained per-value fallback.
    let result = ReadAccessResult {
        object_identifier: ObjectIdentifier::new(ObjectType::AVERAGING, 1).unwrap(),
        list_of_results: vec![
            row(PROPERTY.to_vec()),
            row(wrong.clone()),
            row(broken.clone()),
            ReadResultElement {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                property_value: None,
                error: Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
            },
        ],
    };
    Python::initialize();
    Python::attach(|py| {
        let result = read_access_result_to_py(py, result).unwrap();
        let rows = result.get_item("results").unwrap().unwrap();
        let rows = rows.cast::<PyList>().unwrap();
        assert_eq!(rows.len(), 4);
        for (index, expected) in [(0, Some(Element::DeviceObjectPropertyReference)), (1, None)] {
            let row = rows.get_item(index).unwrap();
            let row = row.cast::<PyDict>().unwrap();
            let value = row.get_item("value").unwrap().unwrap();
            let value = value.extract::<PyRef<'_, PyPropertyValue>>().unwrap();
            assert_eq!(value.element, expected);
            assert_eq!(
                value.inner,
                PropertyValue::ApplicationData(if index == 0 {
                    PROPERTY.to_vec()
                } else {
                    wrong.clone()
                })
            );
            assert!(row.get_item("error").unwrap().unwrap().is_none());
        }
        let raw = rows.get_item(2).unwrap();
        let raw = raw.cast::<PyDict>().unwrap();
        assert_eq!(
            raw.get_item("value")
                .unwrap()
                .unwrap()
                .cast::<PyBytes>()
                .unwrap()
                .as_bytes(),
            broken
        );
        assert!(raw.get_item("error").unwrap().unwrap().is_none());
        let error = rows.get_item(3).unwrap();
        let error = error.cast::<PyDict>().unwrap();
        assert!(error.get_item("value").unwrap().unwrap().is_none());
        let pair = error.get_item("error").unwrap().unwrap();
        let (class, code): (crate::types::PyErrorClass, crate::types::PyErrorCode) =
            pair.extract().unwrap();
        assert_eq!(class.inner, ErrorClass::PROPERTY);
        assert_eq!(code.inner, ErrorCode::UNKNOWN_PROPERTY);
    });
}
