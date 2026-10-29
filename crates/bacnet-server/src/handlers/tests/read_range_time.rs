use super::read_range::*;
use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{Apdu, ComplexAck, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::log_buffer::LogRecordIdentity;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::read_range::RangeSpec;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, StatusFlags, Time};
use tokio::time::{Duration, timeout};

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(matches!(
        error,
        Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected
    ));
}

#[test]
fn by_time_excludes_equal_duplicates_and_reports_exact_endpoints() {
    let other_weekday = Date {
        day_of_week: 2,
        ..DATE
    };
    let identities = vec![
        identity(1, 1),
        identity(2, 2),
        LogRecordIdentity::new(3, other_weekday, time(2)).unwrap(),
        identity(4, 3),
        identity(5, 4),
    ];
    let (db, oid) = list_db(
        PropertyIdentifier::LOG_BUFFER,
        unsigned_items(&[10, 20, 30, 40, 50]),
        Some(identities),
    );
    let reference = (
        Date {
            day_of_week: 7,
            ..DATE
        },
        time(2),
    );

    let positive = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByTime {
            reference_time: reference,
            count: 2,
        }),
    )
    .unwrap();
    assert_ack(
        &positive,
        &unsigned_items(&[40, 50]),
        (false, true, false),
        Some(4),
    );

    let negative = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByTime {
            reference_time: reference,
            count: -2,
        }),
    )
    .unwrap();
    assert_ack(
        &negative,
        &unsigned_items(&[10]),
        (true, false, false),
        Some(1),
    );
}

#[test]
fn by_time_scans_resident_endpoints_without_sorting_clock_rollback() {
    let identities = vec![
        identity(1, 5),
        identity(2, 1),
        identity(3, 4),
        identity(4, 2),
    ];
    let (db, oid) = list_db(
        PropertyIdentifier::LOG_BUFFER,
        unsigned_items(&[10, 20, 30, 40]),
        Some(identities),
    );
    for (count, expected, flags, sequence) in [
        (2, vec![10, 20], (true, false, false), 1),
        (-2, vec![30, 40], (false, true, false), 3),
    ] {
        let ack = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(3)),
                count,
            }),
        )
        .unwrap();
        assert_ack(&ack, &unsigned_items(&expected), flags, Some(sequence));
    }
}

#[test]
fn by_time_empty_and_no_match_are_success_without_metadata() {
    let (empty_db, empty_oid) =
        list_db(PropertyIdentifier::LOG_BUFFER, Vec::new(), Some(Vec::new()));
    let empty = call(
        &empty_db,
        empty_oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByTime {
            reference_time: (DATE, time(1)),
            count: 1,
        }),
    )
    .unwrap();
    assert_ack(&empty, &[], (false, false, false), None);

    let (db, oid) = list_db(
        PropertyIdentifier::LOG_BUFFER,
        unsigned_items(&[10, 20]),
        Some(vec![identity(1, 1), identity(2, 2)]),
    );
    for (reference_time, count) in [(time(2), 1), (time(1), -1)] {
        let miss = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, reference_time),
                count,
            }),
        )
        .unwrap();
        assert_ack(&miss, &[], (false, false, false), None);
    }
}

#[test]
fn by_time_rejects_invalid_absent_or_misaligned_timestamp_identities() {
    let invalid = [
        (
            Date {
                year: Date::UNSPECIFIED,
                ..DATE
            },
            time(1),
        ),
        (Date { month: 0, ..DATE }, time(1)),
        (Date { month: 13, ..DATE }, time(1)),
        (Date { day: 0, ..DATE }, time(1)),
        (Date { day: 32, ..DATE }, time(1)),
        (
            Date {
                day_of_week: 0,
                ..DATE
            },
            time(1),
        ),
        (
            DATE,
            Time {
                hour: 24,
                ..time(1)
            },
        ),
        (
            DATE,
            Time {
                minute: 60,
                ..time(1)
            },
        ),
        (
            DATE,
            Time {
                second: 60,
                ..time(1)
            },
        ),
        (
            DATE,
            Time {
                hundredths: 100,
                ..time(1)
            },
        ),
    ];
    for (date, invalid_time) in invalid {
        let identities = Some(vec![LogRecordIdentity::new(1, date, invalid_time).unwrap()]);
        let (db, oid) = list_db(
            PropertyIdentifier::LOG_BUFFER,
            unsigned_items(&[1]),
            identities,
        );
        let error = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(0)),
                count: 1,
            }),
        )
        .unwrap_err();
        assert_property_error(error, ErrorCode::LIST_ITEM_NOT_TIMESTAMPED);
    }

    for (property, identities) in [
        (PropertyIdentifier::LOG_BUFFER, None),
        (PropertyIdentifier::LOG_BUFFER, Some(vec![identity(1, 1)])),
        // A BACnetLIST other than LOG_BUFFER carries no timestamps.
        (
            PropertyIdentifier::DATE_LIST,
            Some(vec![identity(1, 1), identity(2, 2)]),
        ),
    ] {
        let (db, oid) = list_db(property, unsigned_items(&[1, 2]), identities);
        let error = call(
            &db,
            oid,
            property,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(0)),
                count: 1,
            }),
        )
        .unwrap_err();
        assert_property_error(error, ErrorCode::LIST_ITEM_NOT_TIMESTAMPED);
    }
}

#[test]
fn every_log_family_reads_by_time_after_fifo_eviction() {
    for family in [LogFamily::Event, LogFamily::Trend, LogFamily::TrendMultiple] {
        let object = fifo_log(family);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        for (reference, count, expected, flags, sequence) in [
            (2, 2, vec![3, 4], (false, true, false), 3),
            (4, -2, vec![2, 3], (true, false, false), 2),
        ] {
            let ack = call(
                &db,
                oid,
                PropertyIdentifier::LOG_BUFFER,
                Some(RangeSpec::ByTime {
                    reference_time: (DATE, time(reference)),
                    count,
                }),
            )
            .unwrap();
            let expected = expected
                .into_iter()
                .map(|value| projected(family, value))
                .collect::<Vec<_>>();
            assert_ack(&ack, &expected, flags, Some(sequence));
        }
    }
}

fn record_with_status(value: u64) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: time(value as u8),
        log_datum: LogDatum::UnsignedValue(value),
        status_flags: Some(StatusFlags::FAULT),
    }
}

fn log_with_record(family: LogFamily, record: BACnetLogRecord) -> Box<dyn BACnetObject> {
    match family {
        LogFamily::Event => {
            // An Event Log record has no status flags to carry.
            let LogDatum::UnsignedValue(value) = record.log_datum else {
                panic!("an unsigned sample");
            };
            let mut object = EventLogObject::new(1, "EL-1", 1).unwrap();
            object.add_record(event_record(value)).unwrap();
            Box::new(object)
        }
        LogFamily::Trend => {
            let mut object = TrendLogObject::new(1, "TL-1", 1).unwrap();
            object.add_record(record).unwrap();
            Box::new(object)
        }
        LogFamily::TrendMultiple => {
            // A Trend Log Multiple record has no status flags to carry.
            let LogDatum::UnsignedValue(value) = record.log_datum else {
                panic!("an unsigned sample");
            };
            let mut object = TrendLogMultipleObject::new(1, "TLM-1", 1).unwrap();
            object.add_record(multiple_record(value)).unwrap();
            Box::new(object)
        }
    }
}

#[test]
fn read_range_preserves_family_specific_status_projection_bytes() {
    for family in [LogFamily::Event, LogFamily::Trend, LogFamily::TrendMultiple] {
        let object = log_with_record(family, record_with_status(1));
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let ack = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(0)),
                count: 1,
            }),
        )
        .unwrap();
        // Only a Trend Log record carries status-flags [2], after its datum.
        let expected = match family {
            LogFamily::Trend => {
                let mut framed = projected(family, 1);
                let PropertyValue::ApplicationData(bytes) = &mut framed else {
                    unreachable!()
                };
                bytes.extend([0x2A, 0x04, 0x40]);
                framed
            }
            _ => projected(family, 1),
        };
        assert_ack(&ack, &[expected], (true, true, false), Some(1));
    }
}

#[test]
fn indexed_log_buffer_is_rejected_for_every_log_family() {
    for family in [LogFamily::Event, LogFamily::Trend, LogFamily::TrendMultiple] {
        let object = fifo_log(family);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let error =
            call_with_index(&db, oid, PropertyIdentifier::LOG_BUFFER, Some(1), None).unwrap_err();
        assert_property_error(error, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY);
    }
}

#[test]
fn zero_array_index_is_rejected_by_request_decoder() {
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, Vec::new(), Some(Vec::new()));
    let mut service_data = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(&mut service_data, 0, &oid);
    service_data.extend_from_slice(&[0x19, 131, 0x29, 0]);
    let error = handle_read_range(&db, &service_data, &mut BytesMut::new()).unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Reject { .. })),
        "{error:?}"
    );
}

#[tokio::test]
async fn client_accepts_by_time_ack_and_continues_by_returned_sequence() {
    let client_mac = vec![0x31];
    let server_mac = vec![0x32];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let object = fifo_log(LogFamily::Trend);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();

    let server_task = tokio::spawn(async move {
        for _ in 0..2 {
            let received = timeout(Duration::from_secs(2), server_rx.recv())
                .await
                .expect("server timed out waiting for ReadRange")
                .expect("server transport closed");
            assert_eq!(&received.source_mac[..], &client_mac);
            let npdu = decode_npdu(received.npdu).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed ReadRange request");
            };
            assert_eq!(request.service_choice, ConfirmedServiceChoice::READ_RANGE);

            let mut service_ack = BytesMut::new();
            handle_read_range(&db, &request.service_request, &mut service_ack).unwrap();
            let response = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: service_ack.freeze(),
            });
            let mut encoded_apdu = BytesMut::new();
            encode_apdu(&mut encoded_apdu, &response).unwrap();
            let mut encoded_npdu = BytesMut::new();
            encode_npdu(
                &mut encoded_npdu,
                &Npdu {
                    payload: encoded_apdu.freeze(),
                    ..Npdu::default()
                },
            )
            .unwrap();
            server_transport
                .send_unicast(&encoded_npdu, &client_mac)
                .await
                .unwrap();
        }
        server_transport.stop().await.unwrap();
    });

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2_000)
        .build()
        .await
        .unwrap();
    let first = client
        .read_range(
            &server_mac,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(1)),
                count: 3,
            }),
        )
        .await
        .unwrap();
    assert_ack(
        &first,
        &[
            projected(LogFamily::Trend, 2),
            projected(LogFamily::Trend, 3),
            projected(LogFamily::Trend, 4),
        ],
        (true, true, false),
        Some(2),
    );
    // The client's items decode as BACnetLogRecords, back to back (#1233).
    let mut offset = 0;
    let mut decoded = Vec::new();
    while offset < first.item_data.len() {
        let (record, next) =
            bacnet_encoding::constructed::decode_log_record(&first.item_data, offset).unwrap();
        decoded.push(record);
        offset = next;
    }
    assert_eq!(decoded, vec![record(2), record(3), record(4)]);

    let returned_sequence = first.first_sequence_number.unwrap();
    let continuation = client
        .read_range(
            &server_mac,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: returned_sequence,
                count: 2,
            }),
        )
        .await
        .unwrap();
    assert_ack(
        &continuation,
        &[
            projected(LogFamily::Trend, 2),
            projected(LogFamily::Trend, 3),
        ],
        (true, false, false),
        Some(returned_sequence),
    );

    client.stop().await.unwrap();
    server_task.await.unwrap();
}
