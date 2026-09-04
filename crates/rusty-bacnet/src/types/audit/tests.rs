use super::*;

use bacnet_services::audit::{
    AuditLogQueryAck, BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::types::{PyDict, PyList};
use pyo3::PyTypeInfo;

use crate::types::audit_projection::audit_log_query_ack_to_py;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn py_oid(object_type: ObjectType, instance: u32) -> PyObjectIdentifier {
    PyObjectIdentifier::from_rust(oid(object_type, instance))
}

fn device_recipient(py: Python<'_>, instance: u32) -> Bound<'_, PyDict> {
    let value = PyDict::new(py);
    value.set_item("kind", "device").unwrap();
    value
        .set_item("object_identifier", py_oid(ObjectType::DEVICE, instance))
        .unwrap();
    value
}

fn address_recipient(py: Python<'_>) -> Bound<'_, PyDict> {
    let value = PyDict::new(py);
    value.set_item("kind", "address").unwrap();
    value.set_item("network_number", u16::MAX).unwrap();
    value
        .set_item("mac_address", PyBytes::new(py, b"\x01\x02"))
        .unwrap();
    value
}

fn minimal_notification(py: Python<'_>, raw_operation: u32) -> Bound<'_, PyDict> {
    let value = PyDict::new(py);
    value
        .set_item("source_device", device_recipient(py, 1))
        .unwrap();
    value
        .set_item(
            "operation",
            PyAuditOperation {
                inner: AuditOperation::from_raw(raw_operation),
            },
        )
        .unwrap();
    value
        .set_item("target_device", device_recipient(py, 2))
        .unwrap();
    value
}

fn notification_request<'py>(
    py: Python<'py>,
    notification: &Bound<'py, PyDict>,
) -> Bound<'py, PyDict> {
    let request = PyDict::new(py);
    let notifications = PyList::new(py, [notification]).unwrap();
    request.set_item("notifications", notifications).unwrap();
    request
}

fn assert_error_type<T: PyTypeInfo>(py: Python<'_>, error: PyErr) {
    assert!(error.is_instance_of::<T>(py), "unexpected error: {error}");
}

#[test]
fn notification_mapping_preserves_every_field_and_native_unsigned_bounds() {
    Python::initialize();
    Python::attach(|py| {
        let notification = minimal_notification(py, 63);
        notification
            .set_item(
                "source_timestamp",
                PyBACnetTimeStamp::from_rust(BACnetTimeStamp::SequenceNumber(u16::MAX)),
            )
            .unwrap();
        notification
            .set_item(
                "target_timestamp",
                PyBACnetTimeStamp::from_rust(BACnetTimeStamp::Time(Time {
                    hour: 1,
                    minute: 2,
                    second: 3,
                    hundredths: 4,
                })),
            )
            .unwrap();
        notification
            .set_item("source_object", py_oid(ObjectType::ANALOG_INPUT, 3))
            .unwrap();
        notification.set_item("source_comment", "source").unwrap();
        notification.set_item("target_comment", "target").unwrap();
        notification.set_item("invoke_id", u8::MAX).unwrap();
        notification.set_item("source_user_id", u16::MAX).unwrap();
        notification.set_item("source_user_role", u8::MAX).unwrap();
        notification
            .set_item("target_device", address_recipient(py))
            .unwrap();
        notification
            .set_item("target_object", py_oid(ObjectType::ANALOG_OUTPUT, 4))
            .unwrap();
        let property = PyDict::new(py);
        property
            .set_item(
                "property_identifier",
                PyPropertyIdentifier {
                    inner: PropertyIdentifier::PRESENT_VALUE,
                },
            )
            .unwrap();
        property.set_item("property_array_index", u64::MAX).unwrap();
        notification.set_item("target_property", property).unwrap();
        notification.set_item("target_priority", 16).unwrap();
        notification
            .set_item("target_value", PyBytes::new(py, &[0x00]))
            .unwrap();
        notification
            .set_item("current_value", PyBytes::new(py, &[0x11]))
            .unwrap();
        notification
            .set_item(
                "result",
                (
                    PyErrorClass {
                        inner: ErrorClass::PROPERTY,
                    },
                    PyErrorCode {
                        inner: ErrorCode::WRITE_ACCESS_DENIED,
                    },
                ),
            )
            .unwrap();

        let parsed =
            audit_notification_request_from_py(notification_request(py, &notification).as_any())
                .unwrap();
        let parsed = &parsed.notifications[0];
        assert_eq!(parsed.operation, AuditOperation::from_raw(63));
        assert_eq!(parsed.invoke_id, Some(u8::MAX));
        assert_eq!(parsed.source_user_id, Some(u16::MAX));
        assert_eq!(parsed.source_user_role, Some(u8::MAX));
        assert_eq!(parsed.target_priority, Some(16));
        assert_eq!(parsed.target_value.as_deref(), Some([0x00].as_slice()));
        assert_eq!(parsed.current_value.as_deref(), Some([0x11].as_slice()));
        assert_eq!(
            parsed.result,
            Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
        );
        assert_eq!(
            parsed
                .target_property
                .as_ref()
                .unwrap()
                .property_array_index,
            Some(u64::MAX)
        );
        let BACnetRecipient::Address(address) = &parsed.target_device else {
            panic!("expected address recipient");
        };
        assert_eq!(address.network_number, u16::MAX);
        assert_eq!(address.mac_address.as_slice(), b"\x01\x02");
    });
}

#[test]
fn notification_mapping_rejects_schema_type_range_and_operation_errors() {
    Python::initialize();
    Python::attach(|py| {
        assert_error_type::<PyTypeError>(
            py,
            audit_notification_request_from_py(PyList::empty(py).as_any()).unwrap_err(),
        );

        let empty = PyDict::new(py);
        empty.set_item("notifications", PyList::empty(py)).unwrap();
        assert_error_type::<PyValueError>(
            py,
            audit_notification_request_from_py(empty.as_any()).unwrap_err(),
        );

        for raw in [16, 31, 64] {
            let item = minimal_notification(py, raw);
            assert_error_type::<PyValueError>(
                py,
                audit_notification_request_from_py(notification_request(py, &item).as_any())
                    .unwrap_err(),
            );
        }

        let wrong_type = minimal_notification(py, 0);
        wrong_type.set_item("invoke_id", true).unwrap();
        assert_error_type::<PyTypeError>(
            py,
            audit_notification_request_from_py(notification_request(py, &wrong_type).as_any())
                .unwrap_err(),
        );

        let out_of_range = minimal_notification(py, 0);
        out_of_range.set_item("target_priority", 0).unwrap();
        assert_error_type::<PyValueError>(
            py,
            audit_notification_request_from_py(notification_request(py, &out_of_range).as_any())
                .unwrap_err(),
        );

        let unknown = minimal_notification(py, 0);
        unknown.set_item("unknown", 1).unwrap();
        assert_error_type::<PyValueError>(
            py,
            audit_notification_request_from_py(notification_request(py, &unknown).as_any())
                .unwrap_err(),
        );

        let bad_discriminator = minimal_notification(py, 0);
        let source = PyDict::new(py);
        source.set_item("kind", "DEVICE").unwrap();
        source
            .set_item("object_identifier", py_oid(ObjectType::DEVICE, 1))
            .unwrap();
        bad_discriminator.set_item("source_device", source).unwrap();
        assert_error_type::<PyValueError>(
            py,
            audit_notification_request_from_py(
                notification_request(py, &bad_discriminator).as_any(),
            )
            .unwrap_err(),
        );
    });
}

fn base_query<'py>(py: Python<'py>, parameters: &Bound<'py, PyDict>) -> Bound<'py, PyDict> {
    let query = PyDict::new(py);
    query
        .set_item("audit_log", py_oid(ObjectType::AUDIT_LOG, 10))
        .unwrap();
    query.set_item("query_parameters", parameters).unwrap();
    query.set_item("requested_count", u16::MAX).unwrap();
    query
        .set_item("start_at_sequence_number", u32::MAX)
        .unwrap();
    query
}

#[test]
fn query_mapping_preserves_both_choices_and_rejects_invalid_flags() {
    Python::initialize();
    Python::attach(|py| {
        let by_target = PyDict::new(py);
        by_target.set_item("kind", "by_target").unwrap();
        by_target
            .set_item("target_device_identifier", py_oid(ObjectType::DEVICE, 11))
            .unwrap();
        by_target
            .set_item("target_device_address", address_recipient(py))
            .unwrap();
        by_target
            .set_item(
                "target_object_identifier",
                py_oid(ObjectType::ANALOG_VALUE, 12),
            )
            .unwrap();
        by_target
            .set_item(
                "target_property_identifier",
                PyPropertyIdentifier {
                    inner: PropertyIdentifier::PRESENT_VALUE,
                },
            )
            .unwrap();
        by_target.set_item("target_array_index", u64::MAX).unwrap();
        by_target.set_item("target_priority", 16).unwrap();
        by_target
            .set_item("operations", (1u64 << 15) | (1u64 << 63))
            .unwrap();
        by_target.set_item("successful_actions_only", true).unwrap();
        let parsed = audit_log_query_request_from_py(base_query(py, &by_target).as_any()).unwrap();
        assert_eq!(parsed.start_at_sequence_number, Some(u32::MAX));
        assert_eq!(parsed.requested_count, u16::MAX);
        let BACnetAuditLogQueryParameters::ByTarget {
            target_array_index,
            target_priority,
            operations,
            ..
        } = parsed.query_parameters
        else {
            panic!("expected by-target query");
        };
        assert_eq!(target_array_index, Some(u64::MAX));
        assert_eq!(target_priority, Some(16));
        assert_eq!(operations.unwrap().bits(), (1u64 << 15) | (1u64 << 63));

        let by_source = PyDict::new(py);
        by_source.set_item("kind", "by_source").unwrap();
        by_source
            .set_item("source_device_identifier", py_oid(ObjectType::DEVICE, 13))
            .unwrap();
        by_source
            .set_item("successful_actions_only", false)
            .unwrap();
        let parsed = audit_log_query_request_from_py(base_query(py, &by_source).as_any()).unwrap();
        assert!(matches!(
            parsed.query_parameters,
            BACnetAuditLogQueryParameters::BySource { .. }
        ));

        for invalid in [1u64 << 16, 1u64 << 31] {
            by_source.set_item("operations", invalid).unwrap();
            assert_error_type::<PyValueError>(
                py,
                audit_log_query_request_from_py(base_query(py, &by_source).as_any()).unwrap_err(),
            );
        }
        by_source.set_item("operations", -1).unwrap();
        assert_error_type::<PyValueError>(
            py,
            audit_log_query_request_from_py(base_query(py, &by_source).as_any()).unwrap_err(),
        );
        by_source.set_item("operations", true).unwrap();
        assert_error_type::<PyTypeError>(
            py,
            audit_log_query_request_from_py(base_query(py, &by_source).as_any()).unwrap_err(),
        );
    });
}

fn audit_notification(all_optional: bool) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: all_optional.then_some(BACnetTimeStamp::SequenceNumber(1)),
        target_timestamp: all_optional.then_some(BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        })),
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
        source_object: all_optional.then_some(oid(ObjectType::ANALOG_INPUT, 2)),
        operation: AuditOperation::WRITE,
        source_comment: all_optional.then(|| "source".into()),
        target_comment: all_optional.then(|| "target".into()),
        invoke_id: all_optional.then_some(3),
        source_user_id: all_optional.then_some(4),
        source_user_role: all_optional.then_some(5),
        target_device: BACnetRecipient::Address(BACnetAddress {
            network_number: 6,
            mac_address: MacAddr::from_slice(b"\x07"),
        }),
        target_object: all_optional.then_some(oid(ObjectType::ANALOG_OUTPUT, 8)),
        target_property: all_optional.then_some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(u64::MAX),
        }),
        target_priority: all_optional.then_some(16),
        target_value: all_optional.then(|| vec![0x00]),
        current_value: all_optional.then(|| vec![0x11]),
        result: all_optional.then_some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    }
}

#[test]
fn ack_projection_covers_every_datum_and_nested_optional_field() {
    let timestamp = || {
        (
            Date {
                year: 126,
                month: 9,
                day: 4,
                day_of_week: 5,
            },
            Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
        )
    };
    let datums = [
        BACnetAuditLogDatum::LogStatus(0b010),
        BACnetAuditLogDatum::AuditNotification(audit_notification(true)),
        BACnetAuditLogDatum::AuditNotification(audit_notification(false)),
        BACnetAuditLogDatum::TimeChange(-1.5),
    ];
    let ack = AuditLogQueryAck {
        audit_log: oid(ObjectType::AUDIT_LOG, 9),
        records: datums
            .into_iter()
            .enumerate()
            .map(|(index, datum)| BACnetAuditLogRecordResult {
                sequence_number: index as u64 + 1,
                record: BACnetAuditLogRecord {
                    timestamp: timestamp(),
                    datum,
                },
            })
            .collect(),
        no_more_items: true,
    };

    Python::initialize();
    Python::attach(|py| {
        let projected = audit_log_query_ack_to_py(py, &ack).unwrap();
        let projected = projected.bind(py).cast::<PyDict>().unwrap();
        assert_eq!(projected.len(), 3);
        assert_eq!(
            projected
                .get_item("audit_log")
                .unwrap()
                .unwrap()
                .extract::<PyObjectIdentifier>()
                .unwrap()
                .to_rust(),
            oid(ObjectType::AUDIT_LOG, 9)
        );
        assert!(projected
            .get_item("no_more_items")
            .unwrap()
            .unwrap()
            .is_truthy()
            .unwrap());
        let records = projected
            .get_item("records")
            .unwrap()
            .unwrap()
            .cast_into::<PyList>()
            .unwrap();
        assert_eq!(records.len(), 4);

        let mut kinds = Vec::new();
        for (index, item) in records.iter().enumerate() {
            let item = item.cast_into::<PyDict>().unwrap();
            assert_eq!(item.len(), 2);
            assert_eq!(
                item.get_item("sequence_number")
                    .unwrap()
                    .unwrap()
                    .extract::<u64>()
                    .unwrap(),
                index as u64 + 1
            );
            let record = item
                .get_item("record")
                .unwrap()
                .unwrap()
                .cast_into::<PyDict>()
                .unwrap();
            assert_eq!(
                record
                    .get_item("timestamp")
                    .unwrap()
                    .unwrap()
                    .extract::<((u16, u8, u8, u8), (u8, u8, u8, u8))>()
                    .unwrap(),
                ((2026, 9, 4, 5), (12, 34, 56, 78))
            );
            let datum = record
                .get_item("datum")
                .unwrap()
                .unwrap()
                .cast_into::<PyDict>()
                .unwrap();
            kinds.push(
                datum
                    .get_item("kind")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
            );
            match index {
                0 => assert_eq!(
                    datum
                        .get_item("log_status")
                        .unwrap()
                        .unwrap()
                        .extract::<u8>()
                        .unwrap(),
                    0b010
                ),
                3 => assert_eq!(
                    datum
                        .get_item("time_change")
                        .unwrap()
                        .unwrap()
                        .extract::<f32>()
                        .unwrap(),
                    -1.5
                ),
                _ => {}
            }
            if index == 1 || index == 2 {
                let notification = datum
                    .get_item("audit_notification")
                    .unwrap()
                    .unwrap()
                    .cast_into::<PyDict>()
                    .unwrap();
                assert_eq!(notification.len(), 17);
                for key in NOTIFICATION_OPTIONAL {
                    assert!(notification.contains(*key).unwrap());
                }
                if index == 1 {
                    for key in NOTIFICATION_OPTIONAL {
                        assert!(!notification.get_item(*key).unwrap().unwrap().is_none());
                    }
                } else {
                    for key in NOTIFICATION_OPTIONAL {
                        assert!(notification.get_item(*key).unwrap().unwrap().is_none());
                    }
                }
            }
        }
        assert_eq!(
            kinds,
            [
                "log_status",
                "audit_notification",
                "audit_notification",
                "time_change"
            ]
        );
    });
}
