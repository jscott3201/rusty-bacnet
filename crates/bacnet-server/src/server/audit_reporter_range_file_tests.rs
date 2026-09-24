use super::*;
use bacnet_services::{
    file::{AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod},
    read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest},
};

#[path = "audit_reporter_range_file_support.rs"]
mod support;
use support::*;
#[path = "audit_reporter_range_file_boundary_tests.rs"]
mod boundary;

#[tokio::test]
async fn audit_reporter_read_range_pages_identity_value_free_and_response_parity() {
    for case in ["success", "empty", "item cap", "byte cap", "array"] {
        let mut fixture = server(read_reporter()).await;
        let mut plain = plain_server(read_reporter()).await;
        let reads = add_target(&fixture, Kind::Range, None, false).await;
        let plain_reads = add_target(&plain, Kind::Range, None, false).await;
        let (property, index) = if case == "array" {
            (PropertyIdentifier::WEEKLY_SCHEDULE, Some(3))
        } else {
            (PropertyIdentifier::LOG_BUFFER, None)
        };
        let request = range_request(
            Kind::Range.target(),
            property,
            index,
            Some(RangeSpec::ByPosition {
                reference_index: if case == "empty" { 99 } else { 1 },
                count: 2,
            }),
        );
        let mut data = BytesMut::new();
        request.encode(&mut data).unwrap();
        if case == "item cap" {
            fixture.server.config.read_range_budget.max_returned_items = 1;
        } else if case == "byte cap" {
            let mut one = BytesMut::new();
            ReadRangeAck {
                object_identifier: request.object_identifier,
                property_identifier: property,
                property_array_index: index,
                result_flags: (true, false, true),
                item_count: 1,
                item_data: vec![0x21, 11],
                first_sequence_number: None,
            }
            .encode(&mut one);
            fixture
                .server
                .config
                .read_range_budget
                .max_service_ack_bytes = one.len();
        }
        plain.server.config.read_range_budget = fixture.server.config.read_range_budget;
        let data = data.freeze();
        let response = dispatch(&fixture.server, Kind::Range.service(), data.clone()).await;
        let baseline = dispatch(&plain.server, Kind::Range.service(), data).await;
        assert_eq!(wire(&response), wire(&baseline), "{case}");
        let Apdu::ComplexAck(ack) = response else {
            panic!("{case}: {response:?}")
        };
        let ack = ReadRangeAck::decode(&ack.service_ack).unwrap();
        let truncated = case == "item cap" || case == "byte cap";
        assert_eq!(
            ack.item_count,
            if case == "empty" {
                0
            } else if truncated {
                1
            } else {
                2
            }
        );
        assert_eq!(
            ack.item_data,
            if case == "empty" {
                vec![]
            } else if truncated {
                vec![0x21, 11]
            } else {
                vec![0x21, 11, 0x21, 22]
            }
        );
        assert_eq!(
            ack.result_flags,
            if case == "empty" {
                (false, false, false)
            } else {
                (true, !truncated, truncated)
            }
        );
        settle().await;
        assert_eq!(
            records(&fixture),
            vec![expected(Kind::Range.target(), property, index, 77, 0, None)]
        );
        assert!(records(&plain).is_empty());
        assert_eq!(reads.load(Ordering::Acquire), 1);
        assert_eq!(plain_reads.load(Ordering::Acquire), 1);
        assert_idle(&fixture);
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_atomic_read_file_stream_record_empty_eof_no_reread_and_parity() {
    for kind in [Kind::Stream, Kind::Record] {
        for case in ["window", "empty", "eof"] {
            let empty = case == "empty";
            let mut fixture = server(read_reporter()).await;
            let mut plain = plain_server(read_reporter()).await;
            let reads = add_target(&fixture, kind, None, false).await;
            let plain_reads = add_target(&plain, kind, None, false).await;
            let start = if !empty {
                0
            } else if kind == Kind::Stream {
                5
            } else {
                2
            };
            let data = kind.request(start, if case == "eof" { 9 } else { 2 });
            let response = dispatch(&fixture.server, kind.service(), data.clone()).await;
            let baseline = dispatch(&plain.server, kind.service(), data).await;
            assert_eq!(wire(&response), wire(&baseline));
            let Apdu::ComplexAck(ack) = response else {
                panic!("{response:?}")
            };
            let ack = AtomicReadFileAck::decode(&ack.service_ack).unwrap();
            // Preserve storage's existing EOF flag: an empty window at the end
            // of a nonempty file has not consumed its final byte/record.
            assert_eq!(
                ack.end_of_file,
                !empty && (kind == Kind::Record || case == "eof")
            );
            assert_eq!(
                ack.access,
                if kind == Kind::Stream {
                    FileReadAckMethod::Stream {
                        file_start_position: start,
                        file_data: if empty {
                            vec![]
                        } else if case == "eof" {
                            vec![1, 2, 3, 4, 5]
                        } else {
                            vec![1, 2]
                        },
                    }
                } else {
                    FileReadAckMethod::Record {
                        file_start_record: start,
                        returned_record_count: if empty { 0 } else { 2 },
                        file_record_data: if empty {
                            vec![]
                        } else {
                            vec![vec![], vec![1, 2, 3, 4, 5]]
                        },
                    }
                }
            );
            settle().await;
            assert_eq!(records(&fixture), vec![kind.expected(77, None)]);
            assert!(records(&plain).is_empty());
            assert_eq!(reads.load(Ordering::Acquire), 1, "no second storage read");
            assert_eq!(plain_reads.load(Ordering::Acquire), 1);
            assert_idle(&fixture);
            fixture.server.stop().await.unwrap();
            plain.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_range_file_execution_errors_exact_result_and_silent_unknown_outcomes() {
    type Case = (fn() -> Error, Option<(ErrorClass, ErrorCode)>);
    let cases: [Case; 5] = [
        (
            || Error::Protocol { class: 5, code: 5 },
            Some((ErrorClass::SERVICES, ErrorCode::FILE_ACCESS_DENIED)),
        ),
        (
            || Error::Encoding("execution failure".into()),
            Some((ErrorClass::SERVICES, ErrorCode::OTHER)),
        ),
        (|| Error::Timeout(Duration::from_secs(1)), None),
        (
            || Error::Reject {
                reason: RejectReason::OTHER.to_raw(),
            },
            None,
        ),
        (
            || Error::Abort {
                reason: AbortReason::OTHER.to_raw(),
            },
            None,
        ),
    ];
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
        for (failure, result) in cases {
            let mut fixture = server(read_reporter()).await;
            let mut plain = plain_server(read_reporter()).await;
            let reads = add_target(&fixture, kind, Some(failure), false).await;
            add_target(&plain, kind, Some(failure), false).await;
            let data = kind.request(1, 1);
            let response = dispatch(&fixture.server, kind.service(), data.clone()).await;
            let baseline = dispatch(&plain.server, kind.service(), data).await;
            assert_eq!(wire(&response), wire(&baseline));
            assert!(matches!(response, Apdu::Error(_) | Apdu::Reject(_)));
            if let Some(result) = result {
                let Apdu::Error(error) = &response else {
                    panic!("{response:?}")
                };
                assert_eq!((error.error_class, error.error_code), result);
            }
            settle().await;
            assert_eq!(
                records(&fixture),
                result
                    .map(|result| kind.expected(77, Some(result)))
                    .into_iter()
                    .collect::<Vec<_>>()
            );
            assert_eq!(reads.load(Ordering::Acquire), 1);
            assert_idle(&fixture);
            fixture.server.stop().await.unwrap();
            plain.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_range_file_service_validation_errors_preserve_identity() {
    for (kind, case, class, code) in [
        (
            Kind::Range,
            "missing",
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            Kind::Range,
            "wildcard",
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            Kind::Range,
            "property",
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            Kind::Range,
            "array",
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            Kind::Range,
            "list",
            ErrorClass::SERVICES,
            ErrorCode::PROPERTY_IS_NOT_A_LIST,
        ),
        (
            Kind::Range,
            "identity",
            ErrorClass::PROPERTY,
            ErrorCode::LIST_ITEM_NOT_NUMBERED,
        ),
        (
            Kind::Stream,
            "missing",
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            Kind::Stream,
            "type",
            ErrorClass::SERVICES,
            ErrorCode::INCONSISTENT_OBJECT_TYPE,
        ),
        (
            Kind::Stream,
            "method",
            ErrorClass::SERVICES,
            ErrorCode::INVALID_FILE_ACCESS_METHOD,
        ),
        (
            Kind::Stream,
            "start",
            ErrorClass::SERVICES,
            ErrorCode::INVALID_FILE_START_POSITION,
        ),
    ] {
        let mut fixture = server(read_reporter()).await;
        let mut plain = plain_server(read_reporter()).await;
        add_target(&fixture, kind, None, false).await;
        add_target(&plain, kind, None, false).await;
        let mut target = kind.target();
        if case == "missing" {
            target = oid(target.object_type(), 99);
        }
        if case == "wildcard" {
            target = oid(ObjectType::DEVICE, 4194303);
        }
        if case == "type" {
            target = oid(ObjectType::ANALOG_INPUT, 1);
        }
        let property = match case {
            "property" => PropertyIdentifier::DESCRIPTION,
            "list" => PropertyIdentifier::OBJECT_NAME,
            _ => PropertyIdentifier::LOG_BUFFER,
        };
        let index = (case == "array").then_some(7);
        let mut data = BytesMut::new();
        if kind == Kind::Range {
            range_request(
                target,
                property,
                index,
                (case == "identity").then_some(RangeSpec::BySequenceNumber {
                    reference_seq: 1,
                    count: 1,
                }),
            )
            .encode(&mut data)
            .unwrap();
        } else {
            AtomicReadFileRequest {
                file_identifier: target,
                access: if case == "method" {
                    FileAccessMethod::Record {
                        file_start_record: 0,
                        requested_record_count: 1,
                    }
                } else {
                    FileAccessMethod::Stream {
                        file_start_position: if case == "start" { -1 } else { 0 },
                        requested_octet_count: 1,
                    }
                },
            }
            .encode(&mut data);
        }
        let data = data.freeze();
        let response = dispatch(&fixture.server, kind.service(), data.clone()).await;
        let baseline = dispatch(&plain.server, kind.service(), data).await;
        assert_eq!(wire(&response), wire(&baseline));
        let Apdu::Error(error) = response else {
            panic!("{case}: {response:?}")
        };
        assert_eq!((error.error_class, error.error_code), (class, code));
        settle().await;
        let mut record = expected(target, property, index, 77, 0, Some((class, code)));
        if kind != Kind::Range {
            record.target_property = None;
        }
        assert_eq!(records(&fixture), vec![record], "{kind:?} {case}");
        assert_idle(&fixture);
        fixture.server.stop().await.unwrap();
        plain.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_range_file_read_bit_level_and_monitored_objects() {
    for kind in [Kind::Range, Kind::Stream, Kind::Record] {
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
                Some(vec![Selector::Object(kind.target()); 2]),
                true,
            ),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::ObjectType(kind.target().object_type())]),
                true,
            ),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::Object(oid(ObjectType::DEVICE, 10))]),
                false,
            ),
        ] {
            let mut reporter = read_reporter();
            reporter.set_audit_level(level).unwrap();
            if !bit {
                reporter
                    .set_auditable_operations(AuditOperationFlags::empty())
                    .unwrap();
            }
            reporter.set_monitored_objects(selection).unwrap();
            reporter
                .set_audit_priority_filter(BACnetPriorityFilter::empty())
                .unwrap();
            let mut fixture = server(reporter).await;
            add_target(&fixture, kind, None, false).await;
            assert!(matches!(
                dispatch(&fixture.server, kind.service(), kind.request(1, 1)).await,
                Apdu::ComplexAck(_)
            ));
            settle().await;
            assert_eq!(
                records(&fixture),
                if selected {
                    vec![kind.expected(77, None)]
                } else {
                    vec![]
                },
                "{kind:?} {level:?} {bit}"
            );
            if kind == Kind::Range {
                let mut data = BytesMut::new();
                range_request(kind.target(), PropertyIdentifier::PRESENT_VALUE, None, None)
                    .encode(&mut data)
                    .unwrap();
                assert!(matches!(
                    dispatch(&fixture.server, kind.service(), data.freeze()).await,
                    Apdu::ComplexAck(_)
                ));
                settle().await;
                let pv = selected && level != AuditLevel::AUDIT_CONFIG;
                assert_eq!(
                    records(&fixture).len(),
                    usize::from(selected) + usize::from(pv)
                );
            }
            assert_idle(&fixture);
            fixture.server.stop().await.unwrap();
        }
    }
}
