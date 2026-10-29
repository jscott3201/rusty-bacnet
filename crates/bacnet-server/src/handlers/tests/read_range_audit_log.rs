//! ReadRange over an Audit Log's Log_Buffer (#1092; Clauses 12.64.10 and
//! 15.8). The buffer is the object's retained record ring, the same one
//! AuditLogQuery scans; ReadProperty refuses it, and each ReadRange item is
//! one bare BACnetAuditLogRecord identified by its Unsigned64 sequence number.
use super::read_range::*;
use super::*;
use bacnet_types::error::RemoteError;
use std::sync::{Arc, Mutex};

use bacnet_encoding::constructed::encode_audit_log_record;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::audit::{AuditLogQueryRequest, BACnetAuditLogQueryParameters};
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
    BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, BACnetSuccessFilter};

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

struct FixedClock(u8);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: time(self.0),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

const INSTANCE: u32 = 5;

fn audit_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, INSTANCE).unwrap()
}

fn notification(operation: AuditOperation) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        ),
        source_object: None,
        operation,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap(),
        ),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result: None,
    }
}

/// One record per hour; the log-status and time-change hours show that the
/// buffer lists every datum kind, not only notifications.
fn record(hour: u8) -> BACnetAuditLogRecord {
    let datum = match hour {
        3 => BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED),
        5 => BACnetAuditLogDatum::TimeChange(1.5),
        _ => BACnetAuditLogDatum::AuditNotification(notification(AuditOperation::WRITE)),
    };
    BACnetAuditLogRecord {
        timestamp: (DATE, time(hour)),
        datum,
    }
}

fn log_db(capacity: u32, hours: &[u8]) -> ObjectDatabase {
    let mut log = AuditLogObject::new(
        INSTANCE,
        "AL-5",
        capacity,
        Arc::new(MemoryPersistence::default()),
    )
    .unwrap();
    for &hour in hours {
        log.add_record(record(hour)).unwrap();
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(log)).unwrap();
    db
}

fn encoded(records: &[BACnetAuditLogRecord]) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for record in records {
        encode_audit_log_record(record, &mut bytes).unwrap();
    }
    bytes.to_vec()
}

fn read(db: &ObjectDatabase, range: Option<RangeSpec>) -> ReadRangeAck {
    call(db, audit_oid(), PropertyIdentifier::LOG_BUFFER, range).unwrap()
}

fn assert_page(
    ack: &ReadRangeAck,
    hours: &[u8],
    flags: (bool, bool, bool),
    first_sequence_number: Option<u64>,
) {
    let records: Vec<_> = hours.iter().map(|&hour| record(hour)).collect();
    assert_eq!(ack.object_identifier, audit_oid());
    assert_eq!(ack.property_identifier, PropertyIdentifier::LOG_BUFFER);
    assert_eq!(ack.item_count, hours.len() as u32);
    assert_eq!(ack.item_data, encoded(&records));
    assert_eq!(ack.result_flags, flags);
    assert_eq!(ack.first_sequence_number, first_sequence_number);
}

fn position(reference_index: u64, count: i32) -> Option<RangeSpec> {
    Some(RangeSpec::ByPosition {
        reference_index,
        count,
    })
}

fn sequence(reference_seq: u64, count: i32) -> Option<RangeSpec> {
    Some(RangeSpec::BySequenceNumber {
        reference_seq,
        count,
    })
}

fn at(hour: u8, count: i32) -> Option<RangeSpec> {
    Some(RangeSpec::ByTime {
        reference_time: (DATE, time(hour)),
        count,
    })
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

#[test]
fn audit_log_buffer_pages_retained_records_by_position_sequence_and_time() {
    // Capacity 4 after six appends retains sequences 3..=6 (hours 3..=6).
    let db = log_db(4, &[1, 2, 3, 4, 5, 6]);
    assert_page(&read(&db, None), &[3, 4, 5, 6], (true, true, false), None);

    assert_page(
        &read(&db, position(1, 2)),
        &[3, 4],
        (true, false, false),
        None,
    );
    assert_page(
        &read(&db, position(4, -2)),
        &[5, 6],
        (false, true, false),
        None,
    );
    assert_page(
        &read(&db, position(2, 100)),
        &[4, 5, 6],
        (false, true, false),
        None,
    );

    assert_page(
        &read(&db, sequence(5, 2)),
        &[5, 6],
        (false, true, false),
        Some(5),
    );
    assert_page(
        &read(&db, sequence(4, -10)),
        &[3, 4],
        (true, false, false),
        Some(3),
    );
    assert_page(
        &read(&db, sequence(3, 4)),
        &[3, 4, 5, 6],
        (true, true, false),
        Some(3),
    );

    // By Time excludes the reference instant itself (Clause 15.8.1.1.4.3).
    assert_page(&read(&db, at(4, 2)), &[5, 6], (false, true, false), Some(5));
    assert_page(
        &read(&db, at(5, -2)),
        &[3, 4],
        (true, false, false),
        Some(3),
    );
    assert_page(&read(&db, at(0, 1)), &[3], (true, false, false), Some(3));
}

#[test]
fn audit_log_buffer_empty_and_out_of_range_windows_succeed_without_items() {
    let empty = log_db(4, &[]);
    for range in [None, position(1, 1), sequence(1, 1), at(1, 1), at(1, -1)] {
        assert_page(&read(&empty, range), &[], (false, false, false), None);
    }

    let full = log_db(4, &[1, 2, 3, 4, 5, 6]);
    for range in [
        position(0, 1),
        position(5, 1),
        position(1 << 32, -1),
        sequence(2, 1),       // evicted
        sequence(7, -1),      // not yet written
        sequence(1 << 40, 1), // beyond any Unsigned32 identity
        at(6, 1),             // nothing newer than the newest record
        at(3, -1),            // nothing older than the oldest record
    ] {
        assert_page(&read(&full, range), &[], (false, false, false), None);
    }
}

#[test]
fn audit_log_buffer_reports_unsigned64_sequence_numbers_across_wrap() {
    // A restored ring whose identities run past 2^32 - 1 and wrap at
    // u64::MAX back to 1 (Clause 12.64.12).
    let sequences = [u64::MAX - 1, u64::MAX, 1, 2];
    let hours = [1, 2, 4, 6];
    let snapshot = AuditLogSnapshot {
        object_identifier: audit_oid(),
        generation: 1,
        capacity: 4,
        log_enable: true,
        total_record_count: 2,
        records: sequences
            .iter()
            .zip(hours)
            .map(|(&sequence_number, hour)| BACnetAuditLogRecordResult {
                sequence_number,
                record: record(hour),
            })
            .collect(),
        completed_receipts: Vec::new(),
    };
    let persistence = Arc::new(MemoryPersistence(Mutex::new(Some(snapshot))));
    let log = AuditLogObject::new(INSTANCE, "AL-5", 4, persistence).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(log)).unwrap();

    assert_page(
        &read(&db, sequence(1, -2)),
        &[2, 4],
        (false, false, false),
        Some(u64::MAX),
    );
    assert_page(
        &read(&db, sequence(u64::MAX - 1, 1)),
        &[1],
        (true, false, false),
        Some(u64::MAX - 1),
    );
    // Insertion order, not numeric order: 2 follows u64::MAX.
    assert_page(
        &read(&db, sequence(u64::MAX, 3)),
        &[2, 4, 6],
        (false, true, false),
        Some(u64::MAX),
    );
    assert_page(
        &read(&db, at(1, 2)),
        &[2, 4],
        (false, false, false),
        Some(u64::MAX),
    );
    assert_page(
        &read(&db, position(4, -1)),
        &[6],
        (false, true, false),
        None,
    );
}

#[test]
fn audit_log_buffer_refuses_read_property_and_indexes_but_read_range_serves_it() {
    let db = log_db(4, &[1, 2]);
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: audit_oid(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    assert_property_error(
        handle_read_property(&db, &request, &mut response).unwrap_err(),
        ErrorCode::READ_ACCESS_DENIED,
    );
    assert!(response.is_empty());

    // An index fails as on any BACnetLIST (Clause 15.8.1.3.1).
    assert_property_error(
        call_with_index(
            &db,
            audit_oid(),
            PropertyIdentifier::LOG_BUFFER,
            Some(1),
            None,
        )
        .unwrap_err(),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_page(&read(&db, None), &[1, 2], (true, true, false), None);
}

#[test]
fn audit_log_buffer_shows_records_appended_after_a_read_and_matches_audit_log_query() {
    let mut db = log_db(8, &[1, 2]);
    assert_page(&read(&db, None), &[1, 2], (true, true, false), None);

    let object = db.get_mut(&audit_oid()).unwrap();
    object.bind_clock_internal(Some(Arc::new(FixedClock(7))));
    object
        .audit_log_notification_sink_internal()
        .unwrap()
        .store_notifications(&[notification(AuditOperation::CREATE)], 3_000)
        .unwrap();
    // Disabling logging appends its own log-status record (Clause 12.64.10).
    object
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

    let appended = [
        BACnetAuditLogRecord {
            timestamp: (DATE, time(7)),
            datum: BACnetAuditLogDatum::AuditNotification(notification(AuditOperation::CREATE)),
        },
        BACnetAuditLogRecord {
            timestamp: (DATE, time(7)),
            datum: BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED),
        },
    ];
    let next = read(&db, sequence(3, 5));
    assert_eq!(next.item_count, 2);
    assert_eq!(next.item_data, encoded(&appended));
    assert_eq!(next.result_flags, (false, true, false));
    assert_eq!(next.first_sequence_number, Some(3));
    assert_eq!(read(&db, None).item_count, 4);

    // AuditLogQuery reads the same ring: its newest notification carries the
    // sequence number and record the buffer lists at that position.
    let mut query = BytesMut::new();
    AuditLogQueryRequest {
        audit_log: audit_oid(),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
            source_device_address: None,
            source_object_identifier: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::ALL,
        },
        start_at_sequence_number: None,
        requested_count: 1,
    }
    .try_encode(&mut query)
    .unwrap();
    let (_, page) = handle_audit_log_query(&db, &query).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].sequence_number, 3);
    assert_eq!(page.records[0].record, appended[0]);
}

#[test]
fn audit_log_buffer_pages_are_bounded_by_the_request_budget() {
    use crate::server::ReadRangeBudget;
    let db = log_db(6, &[1, 2, 3, 4, 5, 6]);
    let page = |range: Option<RangeSpec>, max_returned_items, max_service_ack_bytes| {
        let mut request = BytesMut::new();
        ReadRangeRequest {
            object_identifier: audit_oid(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            range,
        }
        .encode(&mut request)
        .unwrap();
        let mut response = BytesMut::new();
        handle_read_range_budgeted(
            &db,
            &request,
            &mut response,
            ReadRangeBudget {
                max_returned_items,
                max_service_ack_bytes,
            },
        )
        .map(|()| ReadRangeAck::decode(&response).unwrap())
    };

    let forward = page(sequence(2, 5), 2, 16_384).unwrap();
    assert_page(&forward, &[2, 3], (false, false, true), Some(2));
    let backward = page(position(6, -6), 2, 16_384).unwrap();
    assert_page(&backward, &[5, 6], (false, true, true), None);
    let all = page(None, 256, 16_384).unwrap();
    assert_page(&all, &[1, 2, 3, 4, 5, 6], (true, true, false), None);

    // A byte budget that fits the envelope and one record returns one item;
    // one too small for the first record aborts rather than paging nothing.
    let one = encoded(&[record(1)]).len();
    let envelope = page(None, 1, 16_384).unwrap();
    let mut header = BytesMut::new();
    ReadRangeAck {
        item_data: Vec::new(),
        ..envelope
    }
    .encode(&mut header);
    let fits = header.len() + one;
    assert_page(
        &page(None, 256, fits).unwrap(),
        &[1],
        (true, false, true),
        None,
    );
    assert!(matches!(
        page(None, 256, fits - 1),
        Err(ReadRangeFailure::Bytes)
    ));
}

#[tokio::test]
async fn running_server_serves_audit_log_buffer_and_later_records() {
    use crate::server::BACnetServer;
    use bacnet_client::client::BACnetClient;
    use std::net::Ipv4Addr;

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(log_db(8, &[1, 2, 3]))
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac().to_vec();

    let refused = client
        .read_property(&mac, audit_oid(), PropertyIdentifier::LOG_BUFFER, None)
        .await
        .unwrap_err();
    assert_property_error(refused, ErrorCode::READ_ACCESS_DENIED);

    let first = client
        .read_range(
            &mac,
            audit_oid(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            at(1, 5),
        )
        .await
        .unwrap();
    assert_page(&first, &[2, 3], (false, true, false), Some(2));

    {
        let mut db = server.database().write().await;
        let log = db.get_mut(&audit_oid()).unwrap();
        log.bind_clock_internal(Some(Arc::new(FixedClock(9))));
        log.audit_log_notification_sink_internal()
            .unwrap()
            .store_notifications(&[notification(AuditOperation::DELETE)], 3_000)
            .unwrap();
    }
    let next = client
        .read_range(
            &mac,
            audit_oid(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            sequence(3, 2),
        )
        .await
        .unwrap();
    assert_eq!(next.item_count, 2);
    assert_eq!(
        next.item_data,
        encoded(&[
            record(3),
            BACnetAuditLogRecord {
                timestamp: (DATE, time(9)),
                datum: BACnetAuditLogDatum::AuditNotification(notification(AuditOperation::DELETE)),
            },
        ])
    );
    assert_eq!(next.result_flags, (false, true, false));
    assert_eq!(next.first_sequence_number, Some(3));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
