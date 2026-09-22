use super::*;
use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest};
use bacnet_types::{constructed::BACnetAuditLogQueryParameters, enums::BACnetSuccessFilter};

#[path = "audit_reporter_query_support.rs"]
mod support;
use support::*;
#[path = "audit_reporter_query_boundary_tests.rs"]
mod boundary;

#[tokio::test]
async fn audit_reporter_query_empty_nonempty_zero_and_pages_are_value_free_and_unchanged() {
    for count in [0, 3] {
        let mut fixture = server(read_reporter()).await;
        let mut plain = server(read_reporter()).await;
        plain.server.config.audit_reporter = None;
        let (reads, persistence) = add_log(&fixture, count, "real").await;
        let (plain_reads, _) = add_log(&plain, count, "real").await;
        let snapshot = persistence.0.lock().unwrap().clone();
        let mut expected_records = Vec::new();
        for (i, (start, requested, sequences, exhausted)) in [
            (None, 10, vec![3, 2, 1], true),
            (None, 0, vec![], false),
            (Some(u64::from(u32::MAX) + 1), 1, vec![3], false),
            (Some(3), 1, vec![2], false),
            (Some(2), 1, vec![1], true),
            (Some(1), 1, vec![], true),
        ]
        .into_iter()
        .enumerate()
        {
            let data = encode(&query(start, requested));
            let response = dispatch(&fixture.server, SERVICE, data.clone()).await;
            let baseline = dispatch(&plain.server, SERVICE, data).await;
            assert_eq!(wire(&response), wire(&baseline));
            let Apdu::ComplexAck(ack) = response else {
                panic!("{response:?}")
            };
            assert_eq!(
                AuditLogQueryAck::decode(&ack.service_ack).unwrap(),
                AuditLogQueryAck {
                    audit_log: target(),
                    records: if count == 0 {
                        vec![]
                    } else {
                        sequences
                            .into_iter()
                            .map(|s| stored_record(s, false))
                            .collect()
                    },
                    no_more_items: count == 0 || exhausted,
                }
            );
            settle().await;
            expected_records.push(expected_query(target(), 77 + i as u8, i as u16, None));
            assert_eq!(records(&fixture), expected_records);
            assert!(records(&plain).is_empty());
            assert_eq!(
                reads.load(Ordering::Acquire),
                i + 1,
                "no reread or synchronous recursion"
            );
            assert_eq!(plain_reads.load(Ordering::Acquire), i + 1);
            assert_eq!(*persistence.0.lock().unwrap(), snapshot, "no log mutation");
            assert_idle(&fixture);
        }
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_query_filters_remain_query_only_not_notification_fields() {
    let mut fixture = server(read_reporter()).await;
    let mut plain = server(read_reporter()).await;
    plain.server.config.audit_reporter = None;
    add_log(&fixture, 3, "real").await;
    add_log(&plain, 3, "real").await;
    for (i, (filter, sequences)) in [
        (BACnetSuccessFilter::ALL, vec![3, 2, 1]),
        (BACnetSuccessFilter::SUCCESSES_ONLY, vec![3, 1]),
        (BACnetSuccessFilter::FAILURES_ONLY, vec![2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut query = query(Some(u64::MAX), 10);
        query.query_parameters = BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 10),
            target_device_address: None,
            target_object_identifier: Some(target()),
            target_property_identifier: Some(PropertyIdentifier::LOG_BUFFER),
            target_array_index: Some(7),
            target_priority: Some(5),
            operations: None,
            successful_actions_only: filter,
        };
        let data = encode(&query);
        let response = dispatch(&fixture.server, SERVICE, data.clone()).await;
        assert_eq!(
            wire(&response),
            wire(&dispatch(&plain.server, SERVICE, data).await)
        );
        let Apdu::ComplexAck(ack) = response else {
            panic!("{response:?}")
        };
        let ack = AuditLogQueryAck::decode(&ack.service_ack).unwrap();
        assert_eq!(
            ack.records,
            sequences
                .into_iter()
                .map(|s| stored_record(s, false))
                .collect::<Vec<_>>()
        );
        assert!(ack.no_more_items);
        settle().await;
        assert_eq!(records(&fixture).len(), i + 1);
        assert_eq!(
            records(&fixture)[i],
            expected_query(target(), 77 + i as u8, i as u16, None)
        );
    }
    fixture.server.stop().await.unwrap();
    plain.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_query_execution_errors_have_exact_result_and_response_parity() {
    for (target, mode, result) in [
        (
            oid(ObjectType::AUDIT_LOG, 99),
            "real",
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            "real",
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            target(),
            "no capability",
            (
                ErrorClass::SERVICES,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
    ] {
        let mut fixture = server(read_reporter()).await;
        let mut plain = server(read_reporter()).await;
        plain.server.config.audit_reporter = None;
        let (reads, _) = add_log(&fixture, 0, mode).await;
        add_log(&plain, 0, mode).await;
        let mut query = query(None, 1);
        query.audit_log = target;
        let data = encode(&query);
        let response = dispatch(&fixture.server, SERVICE, data.clone()).await;
        assert_eq!(
            wire(&response),
            wire(&dispatch(&plain.server, SERVICE, data).await)
        );
        let Apdu::Error(error) = response else {
            panic!("{response:?}")
        };
        assert_eq!((error.error_class, error.error_code), result);
        settle().await;
        assert_eq!(
            records(&fixture),
            vec![expected_query(target, 77, 0, Some(result))]
        );
        assert_eq!(reads.load(Ordering::Acquire), 0);
        assert_idle(&fixture);
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_query_read_bit_levels_and_monitored_objects_gate_success_and_error() {
    for (level, bit, selection, selected) in [
        (AuditLevel::NONE, true, None, false),
        (AuditLevel::AUDIT_ALL, false, None, false),
        (AuditLevel::AUDIT_CONFIG, true, None, true),
        (AuditLevel::AUDIT_ALL, true, None, true),
        (AuditLevel::AUDIT_ALL, true, Some(vec![]), false),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::None]),
            false,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::Object(target()); 2]),
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::ObjectType(ObjectType::AUDIT_LOG)]),
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::Object(oid(ObjectType::AUDIT_LOG, 99))]),
            false,
        ),
        (
            AuditLevel::AUDIT_ALL,
            true,
            Some(vec![Selector::ObjectType(ObjectType::FILE)]),
            false,
        ),
    ] {
        for mode in ["real", "no capability"] {
            let mut reporter = read_reporter();
            reporter.set_audit_level(level).unwrap();
            if !bit {
                reporter.set_auditable_operations(AuditOperationFlags::empty());
            }
            reporter.set_monitored_objects(selection.clone());
            reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
            let mut fixture = server(reporter).await;
            add_log(&fixture, 0, mode).await;
            let response = dispatch(&fixture.server, SERVICE, encode(&query(None, 1))).await;
            let result = if mode == "real" {
                assert!(matches!(response, Apdu::ComplexAck(_)));
                None
            } else {
                assert!(matches!(response, Apdu::Error(_)));
                Some((
                    ErrorClass::SERVICES,
                    ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                ))
            };
            settle().await;
            assert_eq!(
                records(&fixture),
                if selected {
                    vec![expected_query(target(), 77, 0, result)]
                } else {
                    vec![]
                }
            );
            assert_idle(&fixture);
            fixture.server.stop().await.unwrap();
        }
    }
}
