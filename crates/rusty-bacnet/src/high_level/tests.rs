//! The priority calls' arguments, policy, values and errors (#2561). The
//! calls themselves, and the errors only a call can make (a refused
//! priority, a failed read back), run over the wire in the Python suite
//! (`tests/test_relinquish.py`).

use bacnet_ops::format::{Floats, StateTexts, ValueFormat};
use bacnet_ops::{DeviceAddress, OpsError};
use bacnet_types::enums::{
    AbortReason, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, RejectReason,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::errors::BacnetNoPriorityArrayError;
use super::ops_err;
use super::priorities::PyWritePolicy;
use super::priorities::priority_arg;
use super::value::{PyValue, data_to_py};
use crate::errors::{
    BacnetAbortError, BacnetError, BacnetProtocolError, BacnetRejectError, BacnetTimeoutError,
    ambiguous_error_type, lookup_error_type,
};
use bacnet_ops::target::AddressStyle;

/// [`ops_err`] on a B/IP client.
fn ops_err_bip(error: OpsError) -> PyErr {
    ops_err(error, AddressStyle::Bip)
}

fn station() -> DeviceAddress {
    DeviceAddress::local(&[127, 0, 0, 1, 0xBA, 0xC0]).unwrap()
}

/// 1 to 16 is a priority; another int raises ValueError, however large
/// or negative; a bool, a float, a str or None raises TypeError.
#[test]
fn a_priority_is_an_int_from_one_to_sixteen() {
    Python::initialize();
    Python::attach(|py| {
        for level in 1..=16u8 {
            let value = level.into_pyobject(py).unwrap();
            assert_eq!(priority_arg(value.as_any()).unwrap().get(), level);
        }
        for bad in ["0", "17", "-1", "2 ** 70", "256"] {
            let value = py
                .eval(&std::ffi::CString::new(bad).unwrap(), None, None)
                .unwrap();
            let error = priority_arg(&value).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{bad}: {error}");
        }
        for bad in ["True", "False", "8.0", "'8'", "None"] {
            let value = py
                .eval(&std::ffi::CString::new(bad).unwrap(), None, None)
                .unwrap();
            let error = priority_arg(&value).unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py), "{bad}: {error}");
        }
    });
}

/// The default refuses 1, 2, 5 and 6; allowing some opens those alone;
/// two policies refusing the same are equal and hash alike.
#[test]
fn the_default_policy_guards_four_priorities() {
    Python::initialize();
    Python::attach(|py| {
        let policy = PyWritePolicy::new();
        assert_eq!(policy.__repr__(), "WritePolicy(refused=(1, 2, 5, 6))");
        let allowed = PyList::new(py, [1u8, 5]).unwrap();
        let opened = policy.allow_priorities(allowed.as_any()).unwrap();
        assert_eq!(opened.__repr__(), "WritePolicy(refused=(2, 6))");
        let one = 1u8.into_pyobject(py).unwrap();
        assert!(!policy.allows(one.as_any()).unwrap());
        assert!(opened.allows(one.as_any()).unwrap());
        let all = PyList::new(py, [1u8, 2, 5, 6]).unwrap();
        let open = policy.allow_priorities(all.as_any()).unwrap();
        assert_eq!(open.__repr__(), "WritePolicy(refused=())");
        let two = PyList::new(py, [1u8, 5, 6]).unwrap();
        let only_two = policy.allow_priorities(two.as_any()).unwrap();
        assert_eq!(only_two.__repr__(), "WritePolicy(refused=(2,))");
        let again = PyWritePolicy::new()
            .allow_priorities(allowed.as_any())
            .unwrap();
        assert_eq!(again.__hash__(), opened.__hash__());
        assert_ne!(policy.__hash__(), opened.__hash__());
        // A priority outside 1 to 16 among them refuses the lot.
        let bad = PyList::new(py, [1u8, 0]).unwrap();
        let error = policy.allow_priorities(bad.as_any()).err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

/// A value's typed data is the CLI's JSON as plain Python: a REAL as the
/// shortest decimal that reads back as it, an enumeration with its name.
#[test]
fn a_value_holds_its_text_its_data_and_its_typed_form() {
    Python::initialize();
    Python::attach(|py| {
        let texts = StateTexts::default();
        let format = ValueFormat::new(ObjectType::BINARY_OUTPUT, &texts, Floats::Exact);
        // A BinaryPV active: application ENUMERATED 1.
        let active = bacnet_ops::Value::new(
            &format,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x91, 0x01],
            None,
        );
        let value = PyValue::new(
            ObjectType::BINARY_OUTPUT,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &active,
        );
        assert_eq!(value.text(), "active");
        let data = data_to_py(py, &active.data).unwrap();
        let data = data.bind(py).cast::<PyDict>().unwrap();
        let number: u32 = data.get_item("value").unwrap().unwrap().extract().unwrap();
        let name: String = data.get_item("name").unwrap().unwrap().extract().unwrap();
        assert_eq!((number, name.as_str()), (1, "active"));

        let format = ValueFormat::new(ObjectType::ANALOG_OUTPUT, &texts, Floats::Exact);
        let tenth = bacnet_ops::Value::new(
            &format,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x44, 0x3D, 0xCC, 0xCC, 0xCD],
            None,
        );
        let data: f64 = data_to_py(py, &tenth.data).unwrap().extract(py).unwrap();
        assert_eq!(data, 0.1);
        let value = PyValue::new(
            ObjectType::ANALOG_OUTPUT,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &tenth,
        );
        assert_eq!(value.text(), "0.1");
        assert_eq!(value.value().to_rust(), &PropertyValue::Real(0.1));
    });
}

/// Each whole-call error a priority call can meet before or after sending
/// raises the class the typed calls raise for it: a timeout reported as
/// the TSM's abort as BacnetTimeoutError, never BacnetAbortError.
#[test]
fn ops_errors_raise_the_typed_calls_classes() {
    Python::initialize();
    Python::attach(|py| {
        let parse = ops_err_bip(OpsError::Parse {
            what: "object",
            input: "analog-input 1".into(),
            reason: "analog-input 1 keeps no Priority_Array".into(),
        });
        assert!(parse.is_instance_of::<PyValueError>(py));
        assert_eq!(
            parse.value(py).to_string(),
            "analog-input 1 keeps no Priority_Array"
        );

        for source in [
            Error::Timeout(std::time::Duration::from_secs(1)),
            Error::abort(AbortReason::TSM_TIMEOUT),
        ] {
            let timeout = ops_err_bip(OpsError::Unreachable {
                device: station(),
                source,
            });
            assert!(
                timeout.is_instance_of::<BacnetTimeoutError>(py),
                "{timeout}"
            );
            assert!(!timeout.is_instance_of::<BacnetAbortError>(py));
        }
        let link = ops_err_bip(OpsError::Unreachable {
            device: station(),
            source: Error::Transport(std::io::Error::other("closed")),
        });
        assert!(link.is_instance_of::<PyOSError>(py) && link.is_instance_of::<BacnetError>(py));

        let refused = ops_err_bip(OpsError::Refused {
            device: station(),
            error: RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::WRITE_ACCESS_DENIED,
            },
            detail: None,
        });
        assert!(refused.is_instance_of::<BacnetProtocolError>(py));
        assert!(!refused.is_instance_of::<BacnetNoPriorityArrayError>(py));
        let code: u16 = refused
            .value(py)
            .getattr("error_code")
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED.to_raw());
        let rejected = ops_err_bip(OpsError::Refused {
            device: station(),
            error: RemoteError::Reject {
                reason: RejectReason::UNRECOGNIZED_SERVICE,
            },
            detail: None,
        });
        assert!(rejected.is_instance_of::<BacnetRejectError>(py));

        let ambiguous = ops_err_bip(OpsError::Ambiguous {
            instance: 7,
            candidates: vec![station(), DeviceAddress::new(2, &[5]).unwrap()],
        });
        assert!(ambiguous.is_instance(py, ambiguous_error_type(py).unwrap().bind(py)));
        let client = ops_err_bip(OpsError::Client(Error::Encoding("bad".into())));
        assert!(client.value(py).get_type().is(py.get_type::<BacnetError>()));

        // SC TLS material, as the binding's own SC startup reports it.
        let refused = bacnet_ops::ScNodeTlsConfig::from_pem(b"", b"", b"").unwrap_err();
        let tls = ops_err_bip(OpsError::from(refused));
        assert!(tls.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
        assert_eq!(
            tls.value(py).to_string(),
            "TLS config error: the CA bundle holds no certificate, so nothing would be trusted"
        );
    });
}

/// A device saying the object has no Priority_Array raises a protocol
/// error of its own class, with the device's class and code, the object,
/// and no write when none went first.
#[test]
fn no_priority_array_is_a_protocol_error_of_its_own() {
    Python::initialize();
    Python::attach(|py| {
        let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3).unwrap();
        let error = ops_err_bip(OpsError::NoPriorityArray {
            device: station(),
            object,
            relinquished: None,
        });
        assert!(error.is_instance_of::<BacnetNoPriorityArrayError>(py));
        assert!(error.is_instance_of::<BacnetProtocolError>(py));
        let value = error.value(py);
        let code: u16 = value.getattr("error_code").unwrap().extract().unwrap();
        let class: u16 = value.getattr("error_class").unwrap().extract().unwrap();
        assert_eq!(
            (class, code),
            (
                ErrorClass::PROPERTY.to_raw(),
                ErrorCode::UNKNOWN_PROPERTY.to_raw()
            )
        );
        assert!(value.getattr("written").unwrap().is_none());
        assert!(
            value
                .getattr("first_failed_element_number")
                .unwrap()
                .is_none()
        );
        let named: crate::types::PyObjectIdentifier =
            value.getattr("object_id").unwrap().extract().unwrap();
        assert_eq!(named.to_rust(), object);
        assert!(value.to_string().contains("analog-value 3"), "{value}");
    });
}

/// A device that can't be placed raises BacnetLookupError, which is also a
/// LookupError and a BacnetError; two addresses claiming it raise its
/// subclass, with every candidate as a (network, MAC) pair. The low-level
/// client's own lookup errors, which the typed calls raise, map the same.
#[test]
fn lookup_errors_are_one_tree() {
    use pyo3::exceptions::PyLookupError;
    Python::initialize();
    Python::attach(|py| {
        let lookup = lookup_error_type(py).unwrap().bind(py);
        let ambiguous_type = ambiguous_error_type(py).unwrap().bind(py);
        assert!(ambiguous_type.is_subclass(lookup).unwrap());
        let unknown = [
            ops_err_bip(OpsError::UnknownDevice {
                target: "2:1234".into(),
                asked: Some(bacnet_ops::Scope::Network(2)),
            }),
            crate::errors::to_py_err(Error::UnknownDevice {
                device: "2:1234".into(),
            }),
        ];
        for error in unknown {
            assert!(error.value(py).get_type().is(lookup), "{error}");
            assert!(error.is_instance_of::<PyLookupError>(py));
            assert!(error.is_instance_of::<BacnetError>(py));
            let value = error.value(py);
            let target: String = value.getattr("target").unwrap().extract().unwrap();
            assert_eq!(target, "2:1234");
            // The instance the name holds, though no address claims it.
            let instance: u32 = value.getattr("instance").unwrap().extract().unwrap();
            assert_eq!(instance, 1234);
            let candidates: Vec<(u16, Vec<u8>)> =
                value.getattr("candidates").unwrap().extract().unwrap();
            assert!(candidates.is_empty());
        }
        // An address names no instance.
        let by_address = ops_err_bip(OpsError::UnknownDevice {
            target: "9:0x05".into(),
            asked: None,
        });
        assert!(by_address.value(py).get_type().is(lookup));
        assert!(by_address.value(py).getattr("instance").unwrap().is_none());
        let asked = ops_err_bip(OpsError::UnknownDevice {
            target: "1234".into(),
            asked: Some(bacnet_ops::Scope::Local),
        });
        assert!(asked.to_string().contains("this network"), "{asked}");

        let routed = DeviceAddress::new(2, &[5]).unwrap();
        let claimed = [
            ops_err_bip(OpsError::Ambiguous {
                instance: 7,
                candidates: vec![station(), routed.clone()],
            }),
            crate::errors::to_py_err(Error::AmbiguousDevice {
                instance: 7,
                candidates: [(&station()).into(), (&routed).into()].into(),
            }),
        ];
        for error in claimed {
            assert!(error.value(py).get_type().is(ambiguous_type), "{error}");
            assert!(error.is_instance_of::<PyLookupError>(py));
            let value = error.value(py);
            let instance: u32 = value.getattr("instance").unwrap().extract().unwrap();
            assert_eq!(instance, 7);
            let candidates: Vec<(u16, Vec<u8>)> =
                value.getattr("candidates").unwrap().extract().unwrap();
            assert_eq!(
                candidates,
                [(0, station().mac().to_vec()), (2, vec![5])],
                "{error}"
            );
        }
    });
}

/// Every `OpsError` variant raises a class chosen for it: none is left to
/// the fallback for variants added later, which raises a bare BacnetError.
#[test]
fn every_variant_is_mapped() {
    use bacnet_ops::error::PagingFault;
    use bacnet_ops::watch::WatchError;
    use pyo3::exceptions::PyRuntimeError;

    use super::errors::BacnetPagingError;
    Python::initialize();
    Python::attach(|py| {
        let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
        let repeated = ops_err_bip(OpsError::Paging(PagingFault::Repeated {
            service: "GetEventInformation",
            object,
        }));
        assert!(
            repeated.is_instance_of::<BacnetPagingError>(py),
            "{repeated}"
        );
        let value = repeated.value(py);
        let service: String = value.getattr("service").unwrap().extract().unwrap();
        assert_eq!(service, "GetEventInformation");
        let named: crate::types::PyObjectIdentifier =
            value.getattr("object_id").unwrap().extract().unwrap();
        assert_eq!(named.to_rust(), object);
        assert!(
            value.to_string().contains("analog-input:3 again"),
            "{value}"
        );
        let stalled = ops_err_bip(OpsError::Paging(PagingFault::Stalled {
            service: "GetEventInformation",
        }));
        assert!(stalled.is_instance_of::<BacnetPagingError>(py));
        assert!(stalled.value(py).getattr("object_id").unwrap().is_none());

        let full = ops_err_bip(OpsError::Watch(WatchError::Full { max: 4096 }));
        assert!(full.is_instance_of::<PyRuntimeError>(py), "{full}");
        assert!(full.to_string().contains("4096"), "{full}");
        // An Object_List element's failure raises what the element's own
        // error raises, saying which element: a refusal with its class and
        // code, a lost link as the transport error, a timeout as one.
        let element = |source: OpsError| {
            ops_err_bip(OpsError::ObjectList {
                index: 7,
                total: 40,
                source: Box::new(source),
            })
        };
        let refused = element(OpsError::Refused {
            device: station(),
            error: RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::READ_ACCESS_DENIED,
            },
            detail: None,
        });
        assert!(
            refused.is_instance_of::<BacnetProtocolError>(py),
            "{refused}"
        );
        assert!(refused.to_string().contains("element 7 of 40"), "{refused}");
        let code: u16 = refused
            .value(py)
            .getattr("error_code")
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(code, ErrorCode::READ_ACCESS_DENIED.to_raw());
        let silent = element(OpsError::Unreachable {
            device: station(),
            source: Error::Timeout(std::time::Duration::from_secs(1)),
        });
        assert!(silent.is_instance_of::<BacnetTimeoutError>(py), "{silent}");
        let link = element(OpsError::Unreachable {
            device: station(),
            source: Error::Transport(std::io::Error::other("closed")),
        });
        assert!(link.is_instance_of::<PyOSError>(py), "{link}");
        assert!(link.to_string().contains("element 7 of 40"), "{link}");
        // The variants the other tests cover: Parse, UnknownDevice,
        // Ambiguous, Unreachable, Refused, Client, Denied (over the wire),
        // ReadBack (over the wire) and NoPriorityArray. With Paging, Watch
        // and ObjectList that is all twelve.
    });
}

/// A `Value` rebuilt from the parts its pickle holds is equal to it and
/// decodes alike, and the parts it compares are the octets, the text, the
/// data and what it is a value of.
#[test]
fn a_value_is_its_parts() {
    Python::initialize();
    Python::attach(|py| {
        let texts = StateTexts::default();
        let format = ValueFormat::new(ObjectType::ANALOG_OUTPUT, &texts, Floats::Exact);
        let real = |octets: [u8; 5], property| {
            let value = bacnet_ops::Value::new(&format, property, None, octets.to_vec(), None);
            Bound::new(
                py,
                PyValue::new(ObjectType::ANALOG_OUTPUT, property, None, &value),
            )
            .unwrap()
        };
        let pv = PropertyIdentifier::PRESENT_VALUE;
        let fifty = real([0x44, 0x42, 0x48, 0, 0], pv);
        let same = real([0x44, 0x42, 0x48, 0, 0], pv);
        let other = real([0x44, 0x42, 0x49, 0, 0], pv);
        let elsewhere = real(
            [0x44, 0x42, 0x48, 0, 0],
            PropertyIdentifier::RELINQUISH_DEFAULT,
        );
        assert!(fifty.eq(&same).unwrap());
        assert_eq!(fifty.hash().unwrap(), same.hash().unwrap());
        assert!(!fifty.eq(&other).unwrap());
        // What it is a value of isn't compared, as Rust's `Value` doesn't:
        // the same octets, text and data are the same value.
        assert!(fifty.eq(&elsewhere).unwrap());
        assert_eq!(fifty.hash().unwrap(), elsewhere.hash().unwrap());
        assert!(!fifty.eq(50.0f64).unwrap());
        let not_implemented = fifty.call_method1("__eq__", (50.0f64,)).unwrap();
        assert!(not_implemented.is(py.NotImplemented()));
        // The pickle still says what it is a value of, which decodes it.
        let (_, parts) = elsewhere
            .call_method0("__reduce__")
            .unwrap()
            .extract::<(Bound<'_, PyAny>, Bound<'_, pyo3::types::PyTuple>)>()
            .unwrap();
        let property: u32 = parts.get_item(1).unwrap().extract().unwrap();
        assert_eq!(property, PropertyIdentifier::RELINQUISH_DEFAULT.to_raw());
        let (rebuild, args) = fifty
            .call_method0("__reduce__")
            .unwrap()
            .extract::<(Bound<'_, PyAny>, Bound<'_, pyo3::types::PyTuple>)>()
            .unwrap();
        let rebuilt = rebuild.call1(args).unwrap();
        assert!(rebuilt.eq(&fifty).unwrap());
        let raw: Vec<u8> = rebuilt.getattr("raw").unwrap().extract().unwrap();
        assert_eq!(raw, [0x44, 0x42, 0x48, 0, 0]);
        let typed: f64 = rebuilt
            .getattr("value")
            .unwrap()
            .getattr("value")
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(typed, 50.0);
        // Data that isn't JSON is refused; octets that don't decode read
        // back as raw application data, as a read of them would.
        let class = fifty.get_type();
        let bad = class.call_method1(
            "_rebuild",
            (
                1u32,
                85u32,
                None::<u32>,
                pyo3::types::PyBytes::new(py, &[0x44]),
                "x",
                "{",
            ),
        );
        assert!(bad.unwrap_err().is_instance_of::<PyValueError>(py));
        let odd = class
            .call_method1(
                "_rebuild",
                (
                    1u32,
                    85u32,
                    None::<u32>,
                    pyo3::types::PyBytes::new(py, &[0x44]),
                    "x",
                    "null",
                ),
            )
            .unwrap();
        let tag: String = odd
            .getattr("value")
            .unwrap()
            .getattr("tag")
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(tag, "application_data");
    });
}

/// The operations core's link refusals name no keyword; the binding adds
/// its own, and leaves every other refusal as it came.
#[test]
fn a_link_refusal_names_the_transport_keyword() {
    use bacnet_ops::target::{NEEDS_BIP6, NEEDS_MSTP};
    Python::initialize();
    Python::attach(|py| {
        for (needs, transport) in [(NEEDS_BIP6, "ipv6"), (NEEDS_MSTP, "mstp")] {
            let error = ops_err_bip(OpsError::Parse {
                what: "target",
                input: "x".into(),
                reason: format!("'x' is an address: {needs}"),
            });
            assert!(error.is_instance_of::<PyValueError>(py));
            let text = error.value(py).to_string();
            let hint = format!("(BACnetClient(transport='{transport}'))");
            assert!(text.ends_with(&hint), "{text}");
        }
        assert_eq!(super::link_hint("plain".into()), "plain");
    });
}
