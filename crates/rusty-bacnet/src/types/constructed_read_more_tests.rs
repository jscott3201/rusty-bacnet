//! Typed reads of the access-control collections (#1344) and the other
//! constructed properties (#1345): each value, built from the shape its
//! typed write takes where there is one, reads back as that shape, encodes
//! back to the same octets, and falls back to the generic decoder when it
//! isn't the expected production.

use super::*;
use bacnet_encoding::constructed::{
    encode_access_rule, encode_calendar_entry, encode_cov_subscription, encode_daily_schedule,
    encode_date_range, encode_device_object_property_reference, encode_device_object_reference,
    encode_property_access_result, encode_recipient, encode_special_event, encode_value_source,
};
use bacnet_encoding::primitives::{encode_property_value, encode_timestamp_choice};
use bacnet_types::MacAddr;
use bacnet_types::constructed::{
    AccessResult, BACnetAddress, BACnetCOVSubscription, BACnetCalendarEntry, BACnetDateRange,
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, BACnetPropertyAccessResult, BACnetRecipient,
    BACnetRecipientProcess, BACnetSpecialEvent, BACnetTimeValue, BACnetValueSource, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;
use pyo3::types::PyDict;
use std::ffi::CStr;

use crate::types::read_value::decode_read_value;
use crate::types::{
    IntEnumValue, PyBACnetTimeStamp, PyObjectIdentifier, PyPropertyIdentifier,
    access_rules_from_py, audit_recipient_from_py, device_object_reference,
};

type O = ObjectType;
type P = PropertyIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Evaluate `source` with `ai1`, `ai2`, `sched` (Schedule 1), `cal` (Calendar
/// 3) and `device` (Device 99) bound, the property identifiers `PV` and
/// `NAME`, the classes `PropertyValue` and `BACnetTimeStamp`, and
/// `NOT_INITIALIZED`, the error a Global Group holds for an unread member.
fn eval<'py>(py: Python<'py>, source: &CStr) -> Bound<'py, PyAny> {
    let locals = PyDict::new(py);
    for (name, id) in [
        ("ai1", oid(O::ANALOG_INPUT, 1)),
        ("ai2", oid(O::ANALOG_INPUT, 2)),
        ("sched", oid(O::SCHEDULE, 1)),
        ("cal", oid(O::CALENDAR, 3)),
        ("device", oid(O::DEVICE, 99)),
    ] {
        locals
            .set_item(name, PyObjectIdentifier::from_rust(id))
            .unwrap();
    }
    for (name, property) in [("PV", P::PRESENT_VALUE), ("NAME", P::OBJECT_NAME)] {
        locals
            .set_item(name, PyPropertyIdentifier { inner: property })
            .unwrap();
    }
    locals
        .set_item("PropertyValue", py.get_type::<PyPropertyValue>())
        .unwrap();
    locals
        .set_item("BACnetTimeStamp", py.get_type::<PyBACnetTimeStamp>())
        .unwrap();
    locals
        .set_item(
            "NOT_INITIALIZED",
            (
                IntEnumValue(ErrorClass::PROPERTY),
                IntEnumValue(ErrorCode::VALUE_NOT_INITIALIZED),
            ),
        )
        .unwrap();
    py.eval(source, None, Some(&locals)).unwrap()
}

fn read(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> PyPropertyValue {
    decode_read_value(object_type, property, array_index, octets).unwrap()
}

/// Check that a read of `octets` is a typed read of `element`, tagged `tag`
/// (`"list"` for a whole collection), whose `.value` equals `expected`, and
/// that it encodes back to `octets`.
fn assert_read(
    py: Python<'_>,
    (object_type, property, array_index): (ObjectType, PropertyIdentifier, Option<u32>),
    octets: &[u8],
    (element, tag): (Element, &str),
    expected: &Bound<'_, PyAny>,
) {
    let value = read(object_type, property, array_index, octets);
    assert_eq!(value.element, Some(element), "{object_type} {property}");
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value.inner).unwrap();
    assert_eq!(encoded.to_vec(), octets);
    let value = Bound::new(py, value).unwrap();
    assert_eq!(
        value.getattr("tag").unwrap().extract::<String>().unwrap(),
        tag
    );
    let read = value.getattr("value").unwrap();
    assert!(read.eq(expected).unwrap(), "{read} != {expected}");
}

/// Whole and indexed reads of `octets`, a collection of `element` whose
/// elements end at `ends`.
fn assert_collection(
    py: Python<'_>,
    properties: &[(ObjectType, PropertyIdentifier)],
    (octets, ends): (&[u8], &[usize]),
    element: Element,
    expected: &Bound<'_, PyAny>,
) {
    for &(object_type, property) in properties {
        assert_read(
            py,
            (object_type, property, None),
            octets,
            (element, "list"),
            expected,
        );
        let mut start = 0;
        for (index, &end) in ends.iter().enumerate() {
            assert_read(
                py,
                (object_type, property, Some(index as u32 + 1)),
                &octets[start..end],
                (element, element.tag()),
                &expected.get_item(index).unwrap(),
            );
            start = end;
        }
    }
}

/// `items` encoded back to back by `encode`, and where each one ends.
fn concat<T>(items: &[T], encode: impl Fn(&mut BytesMut, &T)) -> (Vec<u8>, Vec<usize>) {
    let mut buf = BytesMut::new();
    let mut ends = Vec::new();
    for item in items {
        encode(&mut buf, item);
        ends.push(buf.len());
    }
    (buf.to_vec(), ends)
}

fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week,
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

#[test]
fn access_rules_read_as_the_rules_written() {
    Python::initialize();
    Python::attach(|py| {
        let rules = eval(
            py,
            c"[
                {'enable': True, 'location': None,
                 'time_range': {'object_identifier': sched, 'property_identifier': PV,
                                'property_array_index': None, 'device_identifier': None}},
                {'enable': False, 'time_range': None, 'location': None},
                {'enable': True, 'location': (device, ai2),
                 'time_range': {'object_identifier': sched, 'property_identifier': PV,
                                'property_array_index': 3, 'device_identifier': device}},
            ]",
        );
        let parsed = access_rules_from_py(&rules, "rules").unwrap();
        let (octets, ends) = concat(&parsed, encode_access_rule);
        assert_collection(
            py,
            &[
                (O::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES),
                (O::ACCESS_RIGHTS, P::NEGATIVE_ACCESS_RULES),
            ],
            (&octets, &ends),
            Element::AccessRule,
            &rules,
        );
    });
}

#[test]
fn object_reference_lists_and_accompaniment_read_as_written() {
    Python::initialize();
    Python::attach(|py| {
        let references = eval(py, c"[ai1, (device, ai2)]");
        let parsed: Vec<BACnetDeviceObjectReference> = references
            .try_iter()
            .unwrap()
            .map(|item| device_object_reference(&item.unwrap(), "reference").unwrap())
            .collect();
        let (octets, ends) = concat(&parsed, encode_device_object_reference);
        assert_collection(
            py,
            &[
                (O::ACCESS_ZONE, P::ENTRY_POINTS),
                (O::ACCESS_ZONE, P::EXIT_POINTS),
                (O::ACCESS_USER, P::CREDENTIALS),
                (O::ACCESS_USER, P::MEMBERS),
                (O::ACCESS_USER, P::MEMBER_OF),
                (O::LIFE_SAFETY_POINT, P::MEMBER_OF),
                (O::LIFE_SAFETY_ZONE, P::MEMBER_OF),
                (O::LIFE_SAFETY_ZONE, P::ZONE_MEMBERS),
            ],
            (&octets, &ends),
            Element::DeviceObjectReference,
            &references,
        );
        // Accompaniment holds one reference: a whole read is that element.
        let accompaniment = (O::ACCESS_RIGHTS, P::ACCOMPANIMENT, None);
        for (index, start) in [(0, 0), (1, ends[0])] {
            assert_read(
                py,
                accompaniment,
                &octets[start..ends[index]],
                (Element::DeviceObjectReference, "device_object_reference"),
                &references.get_item(index).unwrap(),
            );
        }
        // Two references aren't one: the generic octets.
        assert_eq!(
            read(O::ACCESS_RIGHTS, P::ACCOMPANIMENT, None, &octets),
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(octets.clone()))
        );
    });
}

#[test]
fn property_reference_lists_read_as_reference_mappings() {
    Python::initialize();
    Python::attach(|py| {
        let mut remote = BACnetDeviceObjectPropertyReference::new_local(
            oid(O::ANALOG_INPUT, 2),
            P::OBJECT_NAME.to_raw(),
        )
        .with_index(4);
        remote.device_identifier = Some(oid(O::DEVICE, 99));
        let references = [
            BACnetDeviceObjectPropertyReference::new_local(
                oid(O::ANALOG_INPUT, 1),
                P::PRESENT_VALUE.to_raw(),
            ),
            remote,
        ];
        let (octets, ends) = concat(&references, encode_device_object_property_reference);
        let expected = eval(
            py,
            c"[
                {'object_identifier': ai1, 'property_identifier': PV,
                 'property_array_index': None, 'device_identifier': None},
                {'object_identifier': ai2, 'property_identifier': NAME,
                 'property_array_index': 4, 'device_identifier': device},
            ]",
        );
        assert_collection(
            py,
            &[
                (O::GLOBAL_GROUP, P::GROUP_MEMBERS),
                (O::SCHEDULE, P::LIST_OF_OBJECT_PROPERTY_REFERENCES),
                (O::CHANNEL, P::LIST_OF_OBJECT_PROPERTY_REFERENCES),
                (O::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY),
            ],
            (&octets, &ends),
            Element::DeviceObjectPropertyReference,
            &expected,
        );
    });
}

#[test]
fn global_group_present_value_reads_values_and_errors() {
    Python::initialize();
    Python::attach(|py| {
        let results = [
            BACnetPropertyAccessResult {
                reference: BACnetDeviceObjectPropertyReference::new_local(
                    oid(O::ANALOG_INPUT, 1),
                    P::PRESENT_VALUE.to_raw(),
                ),
                access_result: AccessResult::Value(PropertyValue::Real(21.5)),
            },
            BACnetPropertyAccessResult {
                reference: BACnetDeviceObjectPropertyReference::new_local(
                    oid(O::ANALOG_INPUT, 2),
                    P::OBJECT_NAME.to_raw(),
                ),
                access_result: AccessResult::Error {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::VALUE_NOT_INITIALIZED,
                },
            },
        ];
        let (octets, ends) = concat(&results, |buf, result| {
            encode_property_access_result(buf, result).unwrap()
        });
        let expected = eval(
            py,
            c"[
                {'object_identifier': ai1, 'property_identifier': PV,
                 'property_array_index': None, 'device_identifier': None,
                 'value': PropertyValue.real(21.5), 'error': None},
                {'object_identifier': ai2, 'property_identifier': NAME,
                 'property_array_index': None, 'device_identifier': None,
                 'value': None,
                 'error': NOT_INITIALIZED},
            ]",
        );
        assert_collection(
            py,
            &[(O::GLOBAL_GROUP, P::PRESENT_VALUE)],
            (&octets, &ends),
            Element::PropertyAccessResult,
            &expected,
        );
    });
}

#[test]
fn audit_notification_recipient_reads_as_the_recipient_configured() {
    Python::initialize();
    Python::attach(|py| {
        for source in [
            c"{'kind': 'device', 'object_identifier': device}",
            cr"{'kind': 'address', 'network_number': 7, 'mac_address': b'\x0a\x00\x00\x01\xba\xc0'}",
        ] {
            let recipient = eval(py, source);
            let mut octets = BytesMut::new();
            encode_recipient(
                &mut octets,
                &audit_recipient_from_py(&recipient, "recipient").unwrap(),
            )
            .unwrap();
            assert_read(
                py,
                (O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT, None),
                &octets,
                (Element::Recipient, "recipient"),
                &recipient,
            );
        }
    });
}

#[test]
fn schedule_and_calendar_properties_read_typed() {
    Python::initialize();
    Python::attach(|py| {
        let monday = vec![
            BACnetTimeValue {
                time: time(8, 0),
                value: PropertyValue::Real(21.0),
            },
            BACnetTimeValue {
                time: time(17, 30),
                value: PropertyValue::Null,
            },
        ];
        let (weekly, ends) = concat(&[monday.clone(), vec![]], |buf, day| {
            encode_daily_schedule(buf, day).unwrap()
        });
        let expected = eval(
            py,
            c"[[((8, 0, 0, 0), PropertyValue.real(21.0)), ((17, 30, 0, 0), PropertyValue.null())],
               []]",
        );
        assert_collection(
            py,
            &[(O::SCHEDULE, P::WEEKLY_SCHEDULE)],
            (&weekly, &ends),
            Element::DailySchedule,
            &expected,
        );

        let range = BACnetDateRange {
            start_date: date(2026, 1, 1, 4),
            end_date: Date {
                year: Date::UNSPECIFIED,
                month: Date::UNSPECIFIED,
                day: Date::UNSPECIFIED,
                day_of_week: Date::UNSPECIFIED,
            },
        };
        let entries = [
            BACnetCalendarEntry::Date(date(2026, 12, 25, 5)),
            BACnetCalendarEntry::DateRange(range.clone()),
            BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: 11,
                week_of_month: 4,
                day_of_week: BACnetWeekNDay::ANY,
            }),
        ];
        let (date_list, ends) = concat(&entries, encode_calendar_entry);
        let expected = eval(
            py,
            c"[
                {'kind': 'date', 'date': (2026, 12, 25, 5)},
                {'kind': 'date_range', 'start_date': (2026, 1, 1, 4),
                 'end_date': (255, 255, 255, 255)},
                {'kind': 'week_n_day', 'month': 11, 'week_of_month': 4, 'day_of_week': 255},
            ]",
        );
        assert_collection(
            py,
            &[(O::CALENDAR, P::DATE_LIST)],
            (&date_list, &ends),
            Element::CalendarEntry,
            &expected,
        );

        let events = [
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(entries[0].clone()),
                list_of_time_values: monday,
                event_priority: 3,
            },
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarReference(oid(O::CALENDAR, 3)),
                list_of_time_values: vec![],
                event_priority: 16,
            },
        ];
        let (exceptions, ends) = concat(&events, |buf, event| {
            encode_special_event(buf, event).unwrap()
        });
        let expected = eval(
            py,
            c"[
                {'period': {'kind': 'date', 'date': (2026, 12, 25, 5)},
                 'time_values': [((8, 0, 0, 0), PropertyValue.real(21.0)),
                                 ((17, 30, 0, 0), PropertyValue.null())],
                 'priority': 3},
                {'period': cal, 'time_values': [], 'priority': 16},
            ]",
        );
        assert_collection(
            py,
            &[(O::SCHEDULE, P::EXCEPTION_SCHEDULE)],
            (&exceptions, &ends),
            Element::SpecialEvent,
            &expected,
        );

        let mut period = BytesMut::new();
        encode_date_range(&mut period, &range);
        assert_read(
            py,
            (O::SCHEDULE, P::EFFECTIVE_PERIOD, None),
            &period,
            (Element::DateRange, "date_range"),
            &eval(py, c"((2026, 1, 1, 4), (255, 255, 255, 255))"),
        );
    });
}

#[test]
fn timestamps_read_as_bacnet_timestamps() {
    Python::initialize();
    Python::attach(|py| {
        let stamps = [
            BACnetTimeStamp::SequenceNumber(7),
            BACnetTimeStamp::Time(time(10, 15)),
            BACnetTimeStamp::DateTime {
                date: date(2026, 10, 5, 1),
                time: time(9, 30),
            },
        ];
        let (octets, ends) = concat(&stamps, |buf, stamp| {
            encode_timestamp_choice(buf, stamp).unwrap()
        });
        let expected = eval(
            py,
            c"[BACnetTimeStamp.sequence_number(7), BACnetTimeStamp.time(10, 15, 0, 0),
               BACnetTimeStamp.date_time((2026, 10, 5, 1), (9, 30, 0, 0))]",
        );
        assert_collection(
            py,
            &[
                (O::ANALOG_INPUT, P::EVENT_TIME_STAMPS),
                (O::EVENT_ENROLLMENT, P::EVENT_TIME_STAMPS),
                (O::ANALOG_OUTPUT, P::COMMAND_TIME_ARRAY),
            ],
            (&octets, &ends),
            Element::TimeStamp,
            &expected,
        );
        assert_read(
            py,
            (O::BINARY_OUTPUT, P::LAST_COMMAND_TIME, None),
            &octets[..ends[0]],
            (Element::TimeStamp, "timestamp"),
            &expected.get_item(0).unwrap(),
        );
    });
}

#[test]
fn active_cov_subscriptions_read_as_mappings() {
    Python::initialize();
    Python::attach(|py| {
        let subscriptions = [
            BACnetCOVSubscription {
                recipient: BACnetRecipientProcess {
                    recipient: BACnetRecipient::Device(oid(O::DEVICE, 99)),
                    process_identifier: 5,
                },
                monitored_property_reference: BACnetObjectPropertyReference::new(
                    oid(O::ANALOG_INPUT, 1),
                    P::PRESENT_VALUE.to_raw(),
                ),
                issue_confirmed_notifications: true,
                time_remaining: 300,
                cov_increment: Some(0.5),
            },
            BACnetCOVSubscription {
                recipient: BACnetRecipientProcess {
                    recipient: BACnetRecipient::Address(BACnetAddress {
                        network_number: 0,
                        mac_address: MacAddr::from_slice(&[10, 0, 0, 2, 0xBA, 0xC0]),
                    }),
                    process_identifier: 0,
                },
                monitored_property_reference: BACnetObjectPropertyReference {
                    object_identifier: oid(O::ANALOG_INPUT, 2),
                    property_identifier: P::OBJECT_NAME.to_raw(),
                    property_array_index: Some(1),
                },
                issue_confirmed_notifications: false,
                time_remaining: 0,
                cov_increment: None,
            },
        ];
        let (octets, ends) = concat(&subscriptions, |buf, subscription| {
            encode_cov_subscription(buf, subscription).unwrap()
        });
        let expected = eval(
            py,
            cr"[
                {'recipient': {'kind': 'device', 'object_identifier': device},
                 'process_identifier': 5, 'object_identifier': ai1, 'property_identifier': PV,
                 'property_array_index': None, 'issue_confirmed_notifications': True,
                 'time_remaining': 300, 'cov_increment': 0.5},
                {'recipient': {'kind': 'address', 'network_number': 0,
                               'mac_address': b'\x0a\x00\x00\x02\xba\xc0'},
                 'process_identifier': 0, 'object_identifier': ai2,
                 'property_identifier': NAME, 'property_array_index': 1,
                 'issue_confirmed_notifications': False, 'time_remaining': 0,
                 'cov_increment': None},
            ]",
        );
        assert_collection(
            py,
            &[(O::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS)],
            (&octets, &ends),
            Element::CovSubscription,
            &expected,
        );
    });
}

#[test]
fn value_sources_read_as_none_references_or_addresses() {
    Python::initialize();
    Python::attach(|py| {
        let sources = [
            BACnetValueSource::None,
            BACnetValueSource::Object(oid(O::ANALOG_INPUT, 1).into()),
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: Some(oid(O::DEVICE, 99)),
                object_identifier: oid(O::ANALOG_INPUT, 2),
            }),
            BACnetValueSource::Address(BACnetAddress {
                network_number: 5,
                mac_address: MacAddr::from_slice(&[0x0A]),
            }),
        ];
        let (octets, ends) = concat(&sources, |buf, source| {
            encode_value_source(buf, source).unwrap()
        });
        let expected = eval(
            py,
            cr"[None, ai1, (device, ai2),
                {'kind': 'address', 'network_number': 5, 'mac_address': b'\x0a'}]",
        );
        assert_collection(
            py,
            &[(O::ANALOG_OUTPUT, P::VALUE_SOURCE_ARRAY)],
            (&octets, &ends),
            Element::ValueSource,
            &expected,
        );
        let mut start = 0;
        for (index, &end) in ends.iter().enumerate() {
            assert_read(
                py,
                (O::BINARY_VALUE, P::VALUE_SOURCE, None),
                &octets[start..end],
                (Element::ValueSource, "value_source"),
                &expected.get_item(index).unwrap(),
            );
            start = end;
        }
    });
}

#[test]
fn a_single_value_that_is_not_one_element_falls_back() {
    // A Value_Source of two sources, or one read by index, isn't a single
    // value: the generic octets.
    let two = [0x08, 0x08];
    assert_eq!(
        read(O::ANALOG_VALUE, P::VALUE_SOURCE, None, &two),
        PyPropertyValue::from_rust(PropertyValue::ApplicationData(two.to_vec()))
    );
    assert_eq!(
        read(O::ANALOG_VALUE, P::VALUE_SOURCE, Some(1), &[0x08]).element,
        None
    );
    // An access rule whose specifiers disagree with its references, either
    // way, and a date list entry of the wrong length, keep their octets.
    for (object_type, property, octets) in [
        // SPECIFIED with no time range.
        (
            O::ACCESS_RIGHTS,
            P::POSITIVE_ACCESS_RULES,
            &[0x09, 0x00, 0x29, 0x01, 0x49, 0x01][..],
        ),
        // ALWAYS with a time range (Schedule 1's Present_Value).
        (
            O::ACCESS_RIGHTS,
            P::NEGATIVE_ACCESS_RULES,
            &[
                0x09, 0x01, 0x1E, 0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x55, 0x1F, 0x29, 0x01, 0x49,
                0x01,
            ][..],
        ),
        // ALL with a location (Access Point 2).
        (
            O::ACCESS_RIGHTS,
            P::POSITIVE_ACCESS_RULES,
            &[
                0x09, 0x01, 0x29, 0x01, 0x3E, 0x1C, 0x08, 0x40, 0x00, 0x02, 0x3F, 0x49, 0x01,
            ][..],
        ),
        (O::CALENDAR, P::DATE_LIST, &[0x0B, 0x7E, 0x0C, 0x19][..]),
    ] {
        assert_eq!(
            read(object_type, property, None, octets),
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(octets.to_vec())),
            "{object_type} {property}"
        );
    }
    // An Effective_Period of one date stays the generic date.
    let one_date = [0xA4, 0x7E, 0x01, 0x01, 0x04];
    assert_eq!(
        read(O::SCHEDULE, P::EFFECTIVE_PERIOD, None, &one_date).element,
        None
    );
}

#[test]
fn address_bindings_read_as_mappings() {
    Python::initialize();
    Python::attach(|py| {
        let bindings = [
            (oid(O::DEVICE, 99), 0u64, vec![0x0A, 0, 0, 9, 0xBA, 0xC0]),
            (oid(O::DEVICE, 99), 77, vec![0x07]),
        ];
        let (octets, ends) = concat(&bindings, |buf, (device, network, mac)| {
            bacnet_encoding::primitives::encode_app_object_id(buf, device);
            bacnet_encoding::primitives::encode_app_unsigned(buf, *network);
            bacnet_encoding::primitives::encode_app_octet_string(buf, mac);
        });
        let expected = eval(
            py,
            cr"[{'device_identifier': device, 'network_number': 0,
                 'mac_address': b'\x0a\x00\x00\x09\xba\xc0'},
                {'device_identifier': device,
                 'network_number': 77, 'mac_address': b'\x07'}]",
        );
        assert_collection(
            py,
            &[(O::DEVICE, P::DEVICE_ADDRESS_BINDING)],
            (&octets, &ends),
            Element::AddressBinding,
            &expected,
        );
    });
}
