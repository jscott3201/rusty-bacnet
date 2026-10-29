//! An Accumulator's Scale and Prescale read as the values `add_accumulator`
//! takes (#1487), from their context-tagged octets.

use super::*;
use bacnet_encoding::constructed::{encode_prescale, encode_scale};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::constructed::{BACnetPrescale, BACnetScale};
use bytes::BytesMut;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};

use crate::types::read_value::decode_read_value;
use crate::types::scale_from_py;

type O = ObjectType;
type P = PropertyIdentifier;

/// Read `octets` from `property` of an Accumulator, check the typed read
/// and that it encodes back to `octets`, and return its tag and value.
fn read_single(py: Python<'_>, property: PropertyIdentifier, octets: &[u8]) -> (String, Py<PyAny>) {
    let value = decode_read_value(O::ACCUMULATOR, property, None, octets).unwrap();
    assert!(value.element.is_some(), "{octets:02X?}");
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value.inner).unwrap();
    assert_eq!(encoded.to_vec(), octets);
    let value = Bound::new(py, value).unwrap();
    (
        value.getattr("tag").unwrap().extract().unwrap(),
        value.getattr("value").unwrap().unbind(),
    )
}

#[test]
fn scale_reads_as_a_float_or_an_int_by_its_choice() {
    Python::initialize();
    Python::attach(|py| {
        // float-scale [0] REAL 2.5, then integer-scale [1] INTEGER -3.
        let (tag, value) = read_single(py, P::SCALE, &[0x0C, 0x40, 0x20, 0x00, 0x00]);
        assert_eq!(tag, "scale");
        assert!(
            value
                .bind(py)
                .is_exact_instance_of::<pyo3::types::PyFloat>()
        );
        assert_eq!(value.extract::<f64>(py).unwrap(), 2.5);
        let (tag, value) = read_single(py, P::SCALE, &[0x19, 0xFD]);
        assert_eq!(tag, "scale");
        assert!(value.bind(py).is_exact_instance_of::<pyo3::types::PyInt>());
        assert_eq!(value.extract::<i64>(py).unwrap(), -3);
    });
}

#[test]
fn prescale_reads_as_a_multiplier_and_modulo_divide_pair() {
    Python::initialize();
    Python::attach(|py| {
        let (tag, value) = read_single(py, P::PRESCALE, &[0x09, 0x05, 0x19, 0x64]);
        assert_eq!(tag, "prescale");
        assert_eq!(value.extract::<(u32, u32)>(py).unwrap(), (5, 100));
    });
}

#[test]
fn the_values_add_accumulator_takes_encode_to_what_reads_back() {
    Python::initialize();
    Python::attach(|py| {
        for (source, expected) in [
            (c"2.5", BACnetScale::FloatScale(2.5)),
            (c"-3", BACnetScale::IntegerScale(-3)),
            (c"2147483647", BACnetScale::IntegerScale(i32::MAX)),
        ] {
            let python = py.eval(source, None, None).unwrap();
            let scale = scale_from_py(&python).unwrap();
            assert_eq!(scale, expected);
            let mut octets = BytesMut::new();
            encode_scale(&mut octets, &scale);
            let (_, value) = read_single(py, P::SCALE, &octets);
            assert!(value.bind(py).eq(&python).unwrap(), "{source:?}");
        }
        let mut octets = BytesMut::new();
        encode_prescale(
            &mut octets,
            &BACnetPrescale {
                multiplier: 9000,
                modulo_divide: 1200,
            },
        );
        let (_, value) = read_single(py, P::PRESCALE, &octets);
        assert_eq!(value.extract::<(u32, u32)>(py).unwrap(), (9000, 1200));
        // A bool or a str is no scale, an int past INTEGER overflows, and a
        // float past a REAL's range isn't finite once rounded.
        for (source, error) in [
            (c"True", "TypeError"),
            (c"'2'", "TypeError"),
            (c"2 ** 31", "OverflowError"),
            (c"1e39", "ValueError"),
            (c"float('nan')", "ValueError"),
        ] {
            let error_raised = scale_from_py(&py.eval(source, None, None).unwrap()).unwrap_err();
            let matches = match error {
                "TypeError" => error_raised.is_instance_of::<PyTypeError>(py),
                "OverflowError" => error_raised.is_instance_of::<PyOverflowError>(py),
                _ => error_raised.is_instance_of::<PyValueError>(py),
            };
            assert!(matches, "{source:?}: {error_raised}");
        }
    });
}

#[test]
fn the_old_application_tagged_forms_are_not_typed() {
    // An application REAL, or two application Unsigneds, are no Clause 21
    // production: the generic decoder takes them.
    for (property, octets) in [
        (P::SCALE, &[0x44, 0x40, 0x20, 0x00, 0x00][..]),
        (P::PRESCALE, &[0x21, 0x05, 0x21, 0x64][..]),
    ] {
        assert_eq!(
            decode_read_value(O::ACCUMULATOR, property, None, octets)
                .unwrap()
                .element,
            None
        );
    }
    // Another object type's Scale isn't an Accumulator's.
    assert_eq!(
        decode_read_value(O::ANALOG_INPUT, P::SCALE, None, &[0x19, 0x02])
            .unwrap()
            .element,
        None
    );
}
