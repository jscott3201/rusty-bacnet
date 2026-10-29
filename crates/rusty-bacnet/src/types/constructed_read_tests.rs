//! Typed reads of constructed collections (#1310): each property's value,
//! built from the shape its typed write takes, encodes and reads back as that
//! same shape, re-encodes to the same octets, and falls back to the generic
//! decoder when it isn't the expected production.

use super::*;
use bacnet_encoding::constructed::{
    encode_action_list, encode_authentication_factor_format, encode_destination,
    encode_device_object_reference, encode_port_permission, encode_read_access_specification,
    encode_stage_limit_value,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::rpm::ReadResultElement;
use bacnet_types::constructed::{
    BACnetAuthenticationFactorFormat, BACnetDeviceObjectReference, BACnetPortPermission,
    BACnetStageLimitValue,
};
use bacnet_types::enums::AuthenticationFactorType;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use pyo3::types::{PyDict, PyList};
use std::ffi::CStr;

use crate::types::destination::destination as destination_from_py;
use crate::types::read_value::decode_read_value;
use crate::types::{
    PyObjectIdentifier, PyPropertyIdentifier, PyReadAccessSpec, action_lists_from_py,
    py_to_rpm_specs, rpm_ack_to_py,
};

const NF: ObjectType = ObjectType::NOTIFICATION_FORWARDER;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Run `test` with `source` evaluated. `ai1`, `ai2` and `device` are Analog
/// Inputs 1 and 2 and Device 99, `PV` and `NAME` the Present_Value and
/// Object_Name identifiers, and `REAL` the value 50.0.
fn with_value(source: &CStr, test: impl FnOnce(Python<'_>, &Bound<'_, PyAny>)) {
    Python::initialize();
    Python::attach(|py| {
        let locals = PyDict::new(py);
        for (name, id) in [
            ("ai1", oid(ObjectType::ANALOG_INPUT, 1)),
            ("ai2", oid(ObjectType::ANALOG_INPUT, 2)),
            ("device", oid(ObjectType::DEVICE, 99)),
        ] {
            locals
                .set_item(name, PyObjectIdentifier::from_rust(id))
                .unwrap();
        }
        for (name, property) in [
            ("PV", PropertyIdentifier::PRESENT_VALUE),
            ("NAME", PropertyIdentifier::OBJECT_NAME),
        ] {
            locals
                .set_item(name, PyPropertyIdentifier { inner: property })
                .unwrap();
        }
        locals
            .set_item(
                "REAL",
                PyPropertyValue::from_rust(PropertyValue::Real(50.0)),
            )
            .unwrap();
        let value = py.eval(source, None, Some(&locals)).unwrap();
        test(py, &value);
    });
}

/// The elements of `value`, a Python list, encoded back to back by `encode`.
fn octets<T>(
    value: &Bound<'_, PyAny>,
    convert: impl Fn(&Bound<'_, PyAny>) -> T,
    encode: impl Fn(&mut BytesMut, &T),
) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for item in value.try_iter().unwrap() {
        encode(&mut buf, &convert(&item.unwrap()));
    }
    buf.to_vec()
}

fn read(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> PyPropertyValue {
    decode_read_value(object_type, property, array_index, octets).unwrap()
}

/// Check that a whole read of `octets` is the typed list of `element`, that
/// its `.value` equals `expected` and its tag is "list", and that it encodes
/// back to `octets`.
fn assert_typed(
    py: Python<'_>,
    (object_type, property): (ObjectType, PropertyIdentifier),
    octets: &[u8],
    element: Element,
    expected: &Bound<'_, PyAny>,
) {
    let value = read(object_type, property, None, octets);
    assert_eq!(value.element, Some(element));
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value.inner).unwrap();
    assert_eq!(encoded.to_vec(), octets);
    let value = Bound::new(py, value).unwrap();
    assert_eq!(
        value.getattr("tag").unwrap().extract::<String>().unwrap(),
        "list"
    );
    let read = value.getattr("value").unwrap();
    assert!(read.eq(expected).unwrap(), "{read} != {expected}");
}

/// `PropertyValue._typed_element`, which pickles call (#1500), finds every
/// element by its tag.
#[test]
fn every_element_is_found_by_its_tag() {
    let mut tags = std::collections::HashSet::new();
    for element in Element::ALL {
        assert!(tags.insert(element.tag()), "{element:?} listed twice");
        assert_eq!(Element::from_tag(element.tag()), Some(element));
    }
    assert_eq!(Element::from_tag("list"), None);
}

#[test]
fn recipient_lists_read_as_the_destinations_written() {
    with_value(
        cr"[
            {'recipient': {'kind': 'device', 'object_identifier': device},
             'process_identifier': 7, 'valid_days': 0x7F, 'from_time': (0, 0, 0, 0),
             'to_time': (23, 59, 59, 99), 'issue_confirmed_notifications': False,
             'transitions': 0x07},
            {'recipient': {'kind': 'address', 'network_number': 5, 'mac_address': b'\x0a\x0b'},
             'process_identifier': 4294967295, 'valid_days': 0b0011111,
             'from_time': (8, 0, 0, 0), 'to_time': (17, 30, 15, 50),
             'issue_confirmed_notifications': True, 'transitions': 0b101},
        ]",
        |py, destinations| {
            let octets = octets(
                destinations,
                |item| destination_from_py(item, "destination").unwrap(),
                |buf, destination| encode_destination(buf, destination).unwrap(),
            );
            for object_type in [ObjectType::NOTIFICATION_CLASS, NF] {
                assert_typed(
                    py,
                    (object_type, PropertyIdentifier::RECIPIENT_LIST),
                    &octets,
                    Element::Destination,
                    destinations,
                );
            }
        },
    );
}

#[test]
fn port_filters_read_as_the_pairs_written_whole_and_by_index() {
    with_value(c"[(0, True), (1, False)]", |py, pairs| {
        let port_filter = (NF, PropertyIdentifier::PORT_FILTER);
        let octets = octets(
            pairs,
            |item| {
                let (port_id, enabled) = item.extract::<(u8, bool)>().unwrap();
                BACnetPortPermission { port_id, enabled }
            },
            encode_port_permission,
        );
        assert_typed(py, port_filter, &octets, Element::PortPermission, pairs);
        // Index 2 is the second element alone, index 0 the array's size.
        let second = read(NF, port_filter.1, Some(2), &octets[4..]);
        assert_eq!(
            second,
            PyPropertyValue::constructed(
                PropertyValue::ApplicationData(octets[4..].to_vec()),
                Element::PortPermission
            )
        );
        let second = Bound::new(py, second).unwrap();
        assert_eq!(
            second.getattr("tag").unwrap().extract::<String>().unwrap(),
            "port_permission"
        );
        assert!(
            second
                .getattr("value")
                .unwrap()
                .eq(pairs.get_item(1).unwrap())
                .unwrap()
        );
        assert_eq!(
            read(NF, port_filter.1, Some(0), &[0x21, 0x02]),
            PyPropertyValue::from_rust(PropertyValue::Unsigned(2))
        );
    });
}

#[test]
fn group_members_read_as_the_specs_written() {
    with_value(
        c"[(ai1, [(PV, None), (NAME, None)]), (ai2, [(PV, 3)])]",
        |py, members| {
            let specs = py_to_rpm_specs(members.extract::<Vec<PyReadAccessSpec>>().unwrap());
            let mut octets = BytesMut::new();
            for spec in &specs {
                encode_read_access_specification(&mut octets, spec);
            }
            assert_typed(
                py,
                (ObjectType::GROUP, PropertyIdentifier::LIST_OF_GROUP_MEMBERS),
                &octets,
                Element::ReadAccessSpecification,
                members,
            );
        },
    );
}

#[test]
fn group_present_value_reads_as_read_property_multiple_results() {
    Python::initialize();
    Python::attach(|py| {
        let results = vec![
            ReadAccessResult {
                object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
                list_of_results: vec![
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        property_value: Some(vec![0x44, 0x41, 0xAC, 0x00, 0x00]),
                        error: None,
                    },
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::DESCRIPTION,
                        property_array_index: None,
                        property_value: None,
                        error: Some((
                            bacnet_types::enums::ErrorClass::PROPERTY,
                            bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
                        )),
                    },
                ],
            },
            ReadAccessResult {
                object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x44, 0x41, 0xB4, 0x00, 0x00]),
                    error: None,
                }],
            },
        ];
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: results,
        };
        let mut octets = BytesMut::new();
        ack.encode(&mut octets);
        let expected = rpm_ack_to_py(py, ack).unwrap().into_bound(py);
        assert_typed(
            py,
            (ObjectType::GROUP, PropertyIdentifier::PRESENT_VALUE),
            &octets,
            Element::ReadAccessResult,
            &expected,
        );
    });
}

#[test]
fn command_actions_read_as_the_action_commands_written() {
    // Every key present, as the read gives them.
    with_value(
        c"[
            [
                {'device_identifier': None, 'object_identifier': ai1,
                 'property_identifier': PV, 'property_array_index': None,
                 'property_value': REAL, 'priority': 8, 'post_delay': 5,
                 'quit_on_failure': True, 'write_successful': False},
                {'device_identifier': device, 'object_identifier': ai2,
                 'property_identifier': PV, 'property_array_index': 3,
                 'property_value': REAL, 'priority': None, 'post_delay': None,
                 'quit_on_failure': False, 'write_successful': True},
            ],
            [],
        ]",
        |py, action| {
            let mut lists = action_lists_from_py(action).unwrap();
            // The write ignores the flag; a run sets it, and a read carries it.
            assert!(!lists[0].commands[1].write_successful);
            lists[0].commands[1].write_successful = true;
            let mut octets = BytesMut::new();
            for list in &lists {
                encode_action_list(&mut octets, list).unwrap();
            }
            assert_typed(
                py,
                (ObjectType::COMMAND, PropertyIdentifier::ACTION),
                &octets,
                Element::ActionList,
                action,
            );
        },
    );
}

#[test]
fn object_references_read_as_identifiers_or_device_pairs() {
    with_value(c"[ai1, (device, ai2)]", |py, references| {
        let octets = octets(
            references,
            |item| match item.extract::<(PyObjectIdentifier, PyObjectIdentifier)>() {
                Ok((device, object)) => BACnetDeviceObjectReference {
                    device_identifier: Some(device.to_rust()),
                    object_identifier: object.to_rust(),
                },
                Err(_) => item
                    .extract::<PyObjectIdentifier>()
                    .unwrap()
                    .to_rust()
                    .into(),
            },
            encode_device_object_reference,
        );
        for property in [
            (ObjectType::ACCESS_DOOR, PropertyIdentifier::DOOR_MEMBERS),
            (ObjectType::ACCESS_POINT, PropertyIdentifier::ACCESS_DOORS),
            (ObjectType::STAGING, PropertyIdentifier::TARGET_REFERENCES),
        ] {
            assert_typed(
                py,
                property,
                &octets,
                Element::DeviceObjectReference,
                references,
            );
        }
    });
}

#[test]
fn supported_formats_read_as_numbers_or_vendor_triples() {
    with_value(c"[0, (2, 7, 9), (3, 4, None)]", |py, formats| {
        let formats_rust = [
            BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::from_raw(0)),
            BACnetAuthenticationFactorFormat {
                format_type: AuthenticationFactorType::from_raw(2),
                vendor_id: Some(7),
                vendor_format: Some(9),
            },
            // Only one vendor member: the missing one reads as None.
            BACnetAuthenticationFactorFormat {
                format_type: AuthenticationFactorType::from_raw(3),
                vendor_id: Some(4),
                vendor_format: None,
            },
        ];
        let mut octets = BytesMut::new();
        for format in &formats_rust {
            encode_authentication_factor_format(&mut octets, format);
        }
        assert_typed(
            py,
            (
                ObjectType::CREDENTIAL_DATA_INPUT,
                PropertyIdentifier::SUPPORTED_FORMATS,
            ),
            &octets,
            Element::AuthenticationFactorFormat,
            formats,
        );
    });
}

#[test]
fn stages_read_as_the_triples_written() {
    with_value(
        c"[(10.0, [True, False, True], 0.5), (20.0, [], 1.25)]",
        |py, stages| {
            let octets = octets(
                stages,
                |item| {
                    let (limit, values, deadband) =
                        item.extract::<(f32, Vec<bool>, f32)>().unwrap();
                    BACnetStageLimitValue {
                        limit,
                        values,
                        deadband,
                    }
                },
                encode_stage_limit_value,
            );
            assert_typed(
                py,
                (ObjectType::STAGING, PropertyIdentifier::STAGES),
                &octets,
                Element::StageLimitValue,
                stages,
            );
        },
    );
}

/// One BACnetDestination naming Device 9, every day and transition.
const DESTINATION: [u8; 24] = [
    0x82, 0x01, 0xFE, 0xB4, 0, 0, 0, 0, 0xB4, 23, 59, 59, 99, 0x0C, 0x02, 0x00, 0x00, 0x09, 0x21,
    0x01, 0x10, 0x82, 0x05, 0xE0,
];

#[test]
fn a_value_that_is_not_the_expected_production_falls_back() {
    let generic = |object_type, property, octets: &[u8]| {
        let value = read(object_type, property, None, octets);
        assert_eq!(value.element, None, "{octets:02X?}");
        value.inner
    };
    let recipient_list = PropertyIdentifier::RECIPIENT_LIST;
    // A context element where a destination starts, and a destination with
    // a trailing value.
    assert_eq!(
        generic(NF, recipient_list, &[0x09, 0x01]),
        PropertyValue::ApplicationData(vec![0x09, 0x01])
    );
    let trailing = [&DESTINATION[..], &[0x21, 0x01]].concat();
    assert_eq!(
        generic(NF, recipient_list, &trailing),
        PropertyValue::ApplicationData(trailing.clone())
    );
    // A port permission missing its enable flag.
    let port_filter = [0x09, 0x01, 0x19, 0x01, 0x09, 0x02];
    assert_eq!(
        generic(NF, PropertyIdentifier::PORT_FILTER, &port_filter),
        PropertyValue::ApplicationData(port_filter.to_vec())
    );
    // A Group result whose value has no property identifier before it.
    let group = [0x0C, 0, 0, 0, 1, 0x1E, 0x4E, 0x10, 0x4F, 0x1F];
    assert_eq!(
        generic(ObjectType::GROUP, PropertyIdentifier::PRESENT_VALUE, &group),
        PropertyValue::ApplicationData(group.to_vec())
    );
    // A command with priority 0, which no action list carries.
    let action = [
        0x0E, 0x1C, 0x00, 0x40, 0x00, 0x01, 0x29, 0x55, 0x4E, 0x10, 0x4F, 0x59, 0x00, 0x79, 0x00,
        0x89, 0x00, 0x0F,
    ];
    assert_eq!(
        generic(ObjectType::COMMAND, PropertyIdentifier::ACTION, &action),
        PropertyValue::ApplicationData(action.to_vec())
    );
    // Stages whose limit is an Unsigned stay the generic flat list.
    let stages = [0x21, 0x01, 0x82, 0x07, 0x80, 0x44, 0x3F, 0x00, 0x00, 0x00];
    assert_eq!(
        generic(ObjectType::STAGING, PropertyIdentifier::STAGES, &stages),
        PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::BitString {
                unused_bits: 7,
                data: vec![0x80]
            },
            PropertyValue::Real(0.5),
        ])
    );
    // Broken framing still fails, as for any other read.
    assert!(decode_read_value(NF, recipient_list, None, &DESTINATION[..16]).is_err());
}

#[test]
fn empty_values_and_other_properties_keep_the_generic_shape() {
    // An empty collection is the generic empty list.
    assert_eq!(
        read(NF, PropertyIdentifier::RECIPIENT_LIST, None, &[]),
        PyPropertyValue::from_rust(PropertyValue::List(vec![]))
    );
    // The same octets on an object type without the property, or on a
    // property with no typed form, keep the octets.
    for (object_type, property) in [
        (
            ObjectType::from_raw(200),
            PropertyIdentifier::RECIPIENT_LIST,
        ),
        (ObjectType::GROUP, PropertyIdentifier::DESCRIPTION),
        (
            ObjectType::DEVICE,
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        ),
    ] {
        assert_eq!(
            read(object_type, property, None, &DESTINATION),
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(DESTINATION.to_vec()))
        );
    }
    // An indexed read of a list property is one element when it decodes as
    // one, and the octets otherwise.
    assert_eq!(
        read(
            NF,
            PropertyIdentifier::RECIPIENT_LIST,
            Some(1),
            &DESTINATION
        )
        .element,
        Some(Element::Destination)
    );
    let two = [DESTINATION, DESTINATION].concat();
    assert_eq!(
        read(NF, PropertyIdentifier::RECIPIENT_LIST, Some(1), &two),
        PyPropertyValue::from_rust(PropertyValue::ApplicationData(two.clone()))
    );
}

#[test]
fn equality_and_repr_follow_the_element_production() {
    Python::initialize();
    Python::attach(|py| {
        let typed = read(NF, PropertyIdentifier::RECIPIENT_LIST, None, &DESTINATION);
        let octets =
            PyPropertyValue::from_rust(PropertyValue::List(vec![PropertyValue::ApplicationData(
                DESTINATION.to_vec(),
            )]));
        // Same octets, but only the typed read is a list of destinations.
        assert_ne!(typed, octets);
        let typed = Bound::new(py, typed).unwrap();
        assert!(!typed.eq(Bound::new(py, octets).unwrap()).unwrap());
        assert_eq!(
            typed.repr().unwrap().to_string(),
            "PropertyValue.list(<1 destination elements>)"
        );
        let element = Bound::new(
            py,
            read(
                NF,
                PropertyIdentifier::RECIPIENT_LIST,
                Some(1),
                &DESTINATION,
            ),
        )
        .unwrap();
        assert_eq!(
            element.repr().unwrap().to_string(),
            "PropertyValue.destination(<24 bytes>)"
        );
        assert_eq!(
            element.hash().unwrap(),
            Bound::new(
                py,
                read(
                    NF,
                    PropertyIdentifier::RECIPIENT_LIST,
                    Some(1),
                    &DESTINATION
                )
            )
            .unwrap()
            .hash()
            .unwrap()
        );
    });
}

#[test]
fn a_list_of_one_productions_elements_is_that_typed_collection() {
    Python::initialize();
    Python::attach(|py| {
        let port_filter = [0x09, 0x00, 0x19, 0x01, 0x09, 0x01, 0x19, 0x00];
        let property = PropertyIdentifier::PORT_FILTER;
        let element = |range: std::ops::Range<usize>| {
            Bound::new(py, read(NF, property, Some(1), &port_filter[range])).unwrap()
        };
        let list = |items: Vec<Bound<'_, PyAny>>| {
            py.get_type::<PyPropertyValue>()
                .call_method1("list", (PyList::new(py, items).unwrap(),))
                .unwrap()
                .extract::<PyPropertyValue>()
                .unwrap()
        };
        // Both elements: the whole read.
        assert_eq!(
            list(vec![element(0..4).into_any(), element(4..8).into_any()]),
            read(NF, property, None, &port_filter)
        );
        // Mixed with an untyped value, or empty: a plain list.
        let octets = Bound::new(
            py,
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(port_filter[4..].to_vec())),
        )
        .unwrap();
        assert_eq!(
            list(vec![element(0..4).into_any(), octets.into_any()]).element,
            None
        );
        assert_eq!(list(vec![]).element, None);
    });
}
