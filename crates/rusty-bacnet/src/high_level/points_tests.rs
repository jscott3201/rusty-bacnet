//! A point record as Python reads it, and the inventory calls' arguments
//! (#2561). The calls themselves run over the wire in the Python suite
//! (`tests/test_inventory.py`).

use std::ffi::CString;

use bacnet_ops::DeviceAddress;
use bacnet_ops::format::{Floats, PropertyError, StateTexts, ValueFormat};
use bacnet_ops::inventory::{PointRecord, Problem};
use bacnet_ops::target::AddressStyle;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use super::{PyPoint, once_each, options};
use crate::errors::{BacnetError, BacnetProtocolError};

fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn eval<'py>(py: Python<'py>, code: &str) -> Bound<'py, PyAny> {
    py.eval(&CString::new(code).unwrap(), None, None).unwrap()
}

/// An analog input's record: its fields decoded, its Present_Value a
/// `Value` naming the units, its problems the exceptions a typed read
/// would raise, each found again by `problem`.
#[test]
fn a_record_is_decoded_and_its_problems_are_exceptions() {
    Python::initialize();
    Python::attach(|py| {
        let device = DeviceAddress::local(&[127, 0, 0, 1, 0xBA, 0xC0]).unwrap();
        let ai = object(ObjectType::ANALOG_INPUT, 1);
        let mut record = PointRecord::new(device, 1234, ai);
        record.name = Some("Zone\u{7}Temp".into());
        record.units = Some(EngineeringUnits::DEGREES_CELSIUS);
        let texts = StateTexts::default();
        let format = ValueFormat::new(ObjectType::ANALOG_INPUT, &texts, Floats::Exact);
        record.present_value = Some(bacnet_ops::Value::new(
            &format,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x44, 0x41, 0xAC, 0x00, 0x00],
            record.units,
        ));
        let unknown = RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_PROPERTY,
        };
        record.problems = vec![
            Problem::new(
                PropertyIdentifier::DESCRIPTION,
                PropertyError::Remote(unknown),
            ),
            Problem::new(
                PropertyIdentifier::STATUS_FLAGS,
                PropertyError::Other("not a bit string".into()),
            ),
        ];
        let point = Bound::new(py, PyPoint::new(py, &record, AddressStyle::Bip).unwrap()).unwrap();
        let get = |name: &str| point.getattr(name).unwrap();

        assert_eq!(
            get("address").extract::<String>().unwrap(),
            "127.0.0.1:47808"
        );
        assert_eq!(get("device_instance").extract::<u32>().unwrap(), 1234);
        assert_eq!(
            get("units_text").extract::<String>().unwrap(),
            "degrees-celsius"
        );
        assert_eq!(get("units").extract::<u32>().unwrap(), 62);
        assert!(get("state_texts").cast::<PyTuple>().unwrap().is_empty());
        assert!(get("relinquish_default").is_none() && !get("commandable").is_truthy().unwrap());
        let present = get("present_value");
        assert_eq!(present.getattr("text").unwrap().to_string(), "21.5 °C");
        // Device text is kept as sent; the repr escapes it.
        assert_eq!(get("name").extract::<String>().unwrap(), "Zone\u{7}Temp");
        assert_eq!(
            point.repr().unwrap().to_string(),
            "Point(analog-input 1, name='Zone\\x07Temp')"
        );

        let problems = get("problems");
        let problems = problems.cast::<PyTuple>().unwrap();
        assert_eq!(problems.len(), 2);
        let (property, refused): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
            problems.get_item(0).unwrap().extract().unwrap();
        assert_eq!(property.getattr("name").unwrap().to_string(), "DESCRIPTION");
        assert!(refused.is_instance_of::<BacnetProtocolError>());
        let code: u16 = refused.getattr("error_code").unwrap().extract().unwrap();
        assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY.to_raw());
        let (_, wrong): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
            problems.get_item(1).unwrap().extract().unwrap();
        assert!(wrong.get_type().is(py.get_type::<BacnetError>()));
        assert_eq!(wrong.to_string(), "not a bit string");

        let found = point.call_method1("problem", ("description",)).unwrap();
        assert!(found.is(&refused));
        let found = point.call_method1("problem", (property,)).unwrap();
        assert!(found.is(&refused));
        assert!(point.call_method1("problem", ("units",)).unwrap().is_none());
        let element = point
            .call_method1("problem", ("priority-array[8]",))
            .unwrap_err();
        assert!(element.is_instance_of::<PyValueError>(py));
        let wrong_type = point.call_method1("problem", (5,)).unwrap_err();
        assert!(wrong_type.is_instance_of::<PyTypeError>(py));
    });
}

/// A multi-state object's state texts are a tuple, state 1 first; a unit
/// without a name is its number as text; a long name is cut in the repr.
#[test]
fn state_texts_units_and_a_long_name() {
    Python::initialize();
    Python::attach(|py| {
        let device = DeviceAddress::new(2, &[5]).unwrap();
        let msv = object(ObjectType::MULTI_STATE_VALUE, 4);
        let mut record = PointRecord::new(device, 7, msv);
        record.name = Some("x".repeat(80));
        record.state_texts.states = vec!["Off".into(), "Low".into(), "High".into()];
        record.number_of_states = Some(3);
        record.commandable = true;
        let point = Bound::new(py, PyPoint::new(py, &record, AddressStyle::Bip).unwrap()).unwrap();
        let texts: Vec<String> = point.getattr("state_texts").unwrap().extract().unwrap();
        assert_eq!(texts, ["Off", "Low", "High"]);
        assert!(
            point
                .getattr("state_texts")
                .unwrap()
                .is_instance_of::<PyTuple>()
        );
        assert_eq!(point.getattr("address").unwrap().to_string(), "2:0x05");
        assert!(point.getattr("commandable").unwrap().is_truthy().unwrap());
        let repr = point.repr().unwrap().to_string();
        assert_eq!(
            repr,
            format!("Point(multi-state-value 4, name='{}…')", "x".repeat(59))
        );

        let ai = object(ObjectType::ANALOG_INPUT, 9);
        let mut record = PointRecord::new(DeviceAddress::new(2, &[5]).unwrap(), 7, ai);
        record.units = Some(EngineeringUnits::from_raw(1000));
        let point = Bound::new(py, PyPoint::new(py, &record, AddressStyle::Bip).unwrap()).unwrap();
        assert_eq!(point.getattr("units_text").unwrap().to_string(), "1000");
        assert_eq!(
            point.getattr("units").unwrap().extract::<u32>().unwrap(),
            1000
        );
        assert!(point.getattr("name").unwrap().is_none());
        assert_eq!(
            point.repr().unwrap().to_string(),
            "Point(analog-input 9, name=None)"
        );
    });
}

/// `types` takes ObjectTypes or their text in any iterable, but not one
/// str and not none at all; the flags set what is read.
#[test]
fn the_options_are_checked_when_the_call_is_made() {
    Python::initialize();
    Python::attach(|py| {
        let read = options(None, true, false, true).unwrap();
        assert!(read.types.is_none() && read.values && !read.settings && read.details);
        let types = eval(py, "['ai', 'analog-output', 'msv']");
        let read = options(Some(&types), false, false, false).unwrap();
        assert_eq!(
            read.types.unwrap(),
            [
                ObjectType::ANALOG_INPUT,
                ObjectType::ANALOG_OUTPUT,
                ObjectType::MULTI_STATE_VALUE
            ]
        );
        let none = eval(py, "None");
        assert!(
            options(Some(&none), false, false, false)
                .unwrap()
                .types
                .is_none()
        );
        // Text is refused whole, never read a character or an octet at a time.
        for bad in ["'ai'", "b'ai'", "bytearray(b'ai')"] {
            let error = options(Some(&eval(py, bad)), false, false, false).unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py), "{bad}: {error}");
            assert!(
                error.to_string().ends_with(": put one in a list"),
                "{bad}: {error}"
            );
        }
        for bad in ["5", "[5]", "[None]"] {
            let error = options(Some(&eval(py, bad)), false, false, false).unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py), "{bad}: {error}");
        }
        for bad in ["[]", "['nonsense']", "()"] {
            let error = options(Some(&eval(py, bad)), false, false, false).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{bad}: {error}");
        }
    });
}

/// Each object once, in order, those kept.
#[test]
fn each_object_once_in_order() {
    let ai = |n| object(ObjectType::ANALOG_INPUT, n);
    let ao = object(ObjectType::ANALOG_OUTPUT, 1);
    let listed = vec![ai(3), ao, ai(1), ai(3), ao];
    assert_eq!(once_each(listed.clone(), |_| true), [ai(3), ao, ai(1)]);
    let inputs = once_each(listed, |o| o.object_type() == ObjectType::ANALOG_INPUT);
    assert_eq!(inputs, [ai(3), ai(1)]);
}
