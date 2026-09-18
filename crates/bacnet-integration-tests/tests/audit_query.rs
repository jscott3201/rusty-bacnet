//! End-to-end AuditLogQuery tests: real BACnetClient ↔ real BACnetServer over
//! loopback UDP, exercising the corrected RB-02/RB-20 contract (three-state
//! `BACnetSuccessFilter`, Unsigned64 cursor) through wire encode, server
//! dispatch, retained-storage filtering, ACK encode, and client decode —
//! including the segmented-ACK path under a small max-APDU.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use bacnet_client::client::BACnetClient;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::audit::{AuditLogQueryRequest, BACnetAuditLogQueryParameters};
use bacnet_types::constructed::{
    BACnetAddress, BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
    BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{
    AuditOperation, BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType, Segmentation,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bacnet_types::MacAddr;

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl MemoryPersistence {
    fn with_snapshot(snapshot: AuditLogSnapshot) -> Self {
        Self(Mutex::new(Some(snapshot)))
    }
}

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn timestamp() -> (Date, Time) {
    (
        Date {
            year: 126,
            month: 9,
            day: 4,
            day_of_week: 5,
        },
        Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
    )
}

fn notification(
    source: BACnetRecipient,
    target: BACnetRecipient,
    operation: AuditOperation,
    result: Option<(ErrorClass, ErrorCode)>,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: source,
        source_object: None,
        operation,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: target,
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result,
    }
}

fn wrap(sequence_number: u64, notification: BACnetAuditNotification) -> BACnetAuditLogRecordResult {
    BACnetAuditLogRecordResult {
        sequence_number,
        record: BACnetAuditLogRecord {
            timestamp: timestamp(),
            datum: BACnetAuditLogDatum::AuditNotification(notification),
        },
    }
}

fn log_object(
    instance: u32,
    capacity: u32,
    records: Vec<BACnetAuditLogRecordResult>,
) -> AuditLogObject {
    let total_record_count = records.last().map_or(0, |entry| entry.sequence_number);
    let persistence = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: oid(ObjectType::AUDIT_LOG, instance),
        generation: 1,
        capacity,
        log_enable: true,
        total_record_count,
        records,
        completed_receipts: Vec::new(),
    }));
    AuditLogObject::new(instance, format!("Audit-{instance}"), capacity, persistence).unwrap()
}

fn by_target(
    target: ObjectIdentifier,
    filter: BACnetSuccessFilter,
) -> BACnetAuditLogQueryParameters {
    BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: target,
        target_device_address: None,
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: filter,
    }
}

fn by_source(
    source: ObjectIdentifier,
    filter: BACnetSuccessFilter,
) -> BACnetAuditLogQueryParameters {
    BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: source,
        source_device_address: None,
        source_object_identifier: None,
        operations: None,
        successful_actions_only: filter,
    }
}

fn query(
    audit_log: ObjectIdentifier,
    parameters: BACnetAuditLogQueryParameters,
    start: Option<u64>,
    count: u16,
) -> AuditLogQueryRequest {
    AuditLogQueryRequest {
        audit_log,
        query_parameters: parameters,
        start_at_sequence_number: start,
        requested_count: count,
    }
}

fn sequences(results: &[BACnetAuditLogRecordResult]) -> Vec<u64> {
    results.iter().map(|entry| entry.sequence_number).collect()
}

/// Build the server database: log 7 holds a mixed success/failure log, log 8
/// holds 40 padded records that force a segmented ACK under a small max-APDU,
/// and log 9 holds identities above u32::MAX for the Unsigned64 cursor.
fn database() -> ObjectDatabase {
    let failure = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let address = BACnetRecipient::Address(BACnetAddress {
        network_number: 9,
        mac_address: MacAddr::from_slice(&[0xAA]),
    });

    let mixed = vec![
        wrap(
            1,
            notification(
                BACnetRecipient::Device(device(1)),
                BACnetRecipient::Device(device(2)),
                AuditOperation::WRITE,
                None,
            ),
        ),
        wrap(
            2,
            notification(
                BACnetRecipient::Device(device(1)),
                BACnetRecipient::Device(device(2)),
                AuditOperation::WRITE,
                failure,
            ),
        ),
        wrap(
            3,
            notification(
                BACnetRecipient::Device(device(1)),
                address,
                AuditOperation::READ,
                None,
            ),
        ),
        wrap(
            4,
            notification(
                BACnetRecipient::Device(device(9)),
                BACnetRecipient::Device(device(2)),
                AuditOperation::WRITE,
                failure,
            ),
        ),
    ];

    let base = u64::from(u32::MAX) + 1;
    let high = vec![
        wrap(
            base,
            notification(
                BACnetRecipient::Device(device(1)),
                BACnetRecipient::Device(device(2)),
                AuditOperation::WRITE,
                None,
            ),
        ),
        wrap(
            base + 1,
            notification(
                BACnetRecipient::Device(device(1)),
                BACnetRecipient::Device(device(2)),
                AuditOperation::WRITE,
                failure,
            ),
        ),
    ];

    let mut bulky = Vec::with_capacity(40);
    for sequence_number in 1..=40u64 {
        let mut padded = notification(
            BACnetRecipient::Device(device(1)),
            BACnetRecipient::Device(device(2)),
            AuditOperation::WRITE,
            None,
        );
        padded.source_comment = Some("padding for segmentation coverage ".repeat(4));
        padded.target_comment = Some("padding for segmentation coverage ".repeat(4));
        bulky.push(wrap(sequence_number, padded));
    }

    let mut db = ObjectDatabase::new();
    let device_object = DeviceObject::new(DeviceConfig {
        instance: 4321,
        name: "Audit E2E Device".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device_object)).unwrap();
    db.add(Box::new(log_object(7, 8, mixed))).unwrap();
    db.add(Box::new(log_object(8, 40, bulky))).unwrap();
    db.add(Box::new(log_object(9, 4, high))).unwrap();
    db
}

async fn started() -> (
    BACnetServer<bacnet_transport::bip::BipTransport>,
    BACnetClient<bacnet_transport::bip::BipTransport>,
    Vec<u8>,
) {
    let server = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database())
        .segmentation_supported(Segmentation::BOTH)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();
    // A small max-APDU forces the server's segmented ComplexACK path for the
    // bulky log while single-APDU queries still complete in one exchange.
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(5_000)
        .max_apdu_length(480)
        .build()
        .await
        .unwrap();
    (server, client, server_mac)
}

#[tokio::test]
async fn audit_query_both_choices_times_all_three_filters() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 7);

    // By-target over device(2): records 1 (success), 2 (failure), 4 (failure).
    for (filter, expected) in [
        (BACnetSuccessFilter::ALL, vec![4, 2, 1]),
        (BACnetSuccessFilter::SUCCESSES_ONLY, vec![1]),
        (BACnetSuccessFilter::FAILURES_ONLY, vec![4, 2]),
    ] {
        let ack = client
            .audit_log_query(
                &server_mac,
                &query(log, by_target(device(2), filter), None, 10),
            )
            .await
            .unwrap();
        assert_eq!(ack.audit_log, log);
        assert_eq!(
            sequences(&ack.records),
            expected,
            "by-target filter {}",
            filter.to_raw()
        );
        assert!(ack.no_more_items);
    }

    // By-source over device(1): records 1 (success), 2 (failure), 3 (success).
    for (filter, expected) in [
        (BACnetSuccessFilter::ALL, vec![3, 2, 1]),
        (BACnetSuccessFilter::SUCCESSES_ONLY, vec![3, 1]),
        (BACnetSuccessFilter::FAILURES_ONLY, vec![2]),
    ] {
        let ack = client
            .audit_log_query(
                &server_mac,
                &query(log, by_source(device(1), filter), None, 10),
            )
            .await
            .unwrap();
        assert_eq!(
            sequences(&ack.records),
            expected,
            "by-source filter {}",
            filter.to_raw()
        );
        assert!(ack.no_more_items);
    }

    client.stop().await.unwrap();
    drop(server);
}

#[tokio::test]
async fn audit_query_empty_results_and_requested_count_boundaries() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 7);

    // No retained record matches: honest empty page.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(
                log,
                by_source(device(100), BACnetSuccessFilter::ALL),
                None,
                10,
            ),
        )
        .await
        .unwrap();
    assert!(ack.records.is_empty());
    assert!(ack.no_more_items);

    // Unknown target object is a filter value, not a database lookup: empty
    // with honest NoMoreItems, not an error.
    let unknown_target = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(2),
        target_device_address: None,
        target_object_identifier: Some(oid(ObjectType::ANALOG_VALUE, 999_999)),
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: BACnetSuccessFilter::ALL,
    };
    let ack = client
        .audit_log_query(&server_mac, &query(log, unknown_target, None, 10))
        .await
        .unwrap();
    assert!(ack.records.is_empty());
    assert!(ack.no_more_items);

    // requested_count 0 with later matches is not exhaustion.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(log, by_target(device(2), BACnetSuccessFilter::ALL), None, 0),
        )
        .await
        .unwrap();
    assert!(ack.records.is_empty());
    assert!(!ack.no_more_items);

    // requested_count 0 with no match is exhaustion.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(
                log,
                by_source(device(100), BACnetSuccessFilter::ALL),
                None,
                0,
            ),
        )
        .await
        .unwrap();
    assert!(ack.records.is_empty());
    assert!(ack.no_more_items);

    // requested_count 1 returns only the newest match without exhaustion.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(log, by_target(device(2), BACnetSuccessFilter::ALL), None, 1),
        )
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![4]);
    assert!(!ack.no_more_items);

    // u16::MAX admits the whole retained log; the 10,000-record
    // persistence/ACK cap is covered by the storage unit test on a full ring.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(
                log,
                by_target(device(2), BACnetSuccessFilter::ALL),
                None,
                u16::MAX,
            ),
        )
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![4, 2, 1]);
    assert!(ack.no_more_items);

    client.stop().await.unwrap();
    drop(server);
}

#[tokio::test]
async fn audit_query_continuation_has_no_omission_or_duplication() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 7);
    let parameters = || by_target(device(2), BACnetSuccessFilter::ALL);

    let mut collected = Vec::new();
    let mut start = None;
    for _ in 0..8 {
        let ack = client
            .audit_log_query(&server_mac, &query(log, parameters(), start, 1))
            .await
            .unwrap();
        assert_eq!(ack.audit_log, log);
        if ack.records.is_empty() {
            assert!(ack.no_more_items);
            break;
        }
        // The final non-empty page honestly reports exhaustion.
        start = Some(ack.records[0].sequence_number);
        collected.push(ack.records[0].sequence_number);
        if ack.no_more_items {
            break;
        }
    }
    assert_eq!(collected, vec![4, 2, 1]);

    let exhausted = client
        .audit_log_query(&server_mac, &query(log, parameters(), start, 1))
        .await
        .unwrap();
    assert!(exhausted.records.is_empty());
    assert!(exhausted.no_more_items);

    client.stop().await.unwrap();
    drop(server);
}

#[tokio::test]
async fn audit_query_u64_cursor_pages_identities_above_u32() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 9);
    let base = u64::from(u32::MAX) + 1;
    let parameters = || by_target(device(2), BACnetSuccessFilter::ALL);

    let ack = client
        .audit_log_query(&server_mac, &query(log, parameters(), None, 10))
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![base + 1, base]);
    assert!(ack.no_more_items);

    let ack = client
        .audit_log_query(&server_mac, &query(log, parameters(), Some(base + 1), 10))
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![base]);
    assert!(ack.no_more_items);

    // u64::MAX admits every retained identity without modular wrap.
    let ack = client
        .audit_log_query(&server_mac, &query(log, parameters(), Some(u64::MAX), 10))
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![base + 1, base]);
    assert!(ack.no_more_items);

    // FAILURES_ONLY applies above u32::MAX as well.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(
                log,
                by_target(device(2), BACnetSuccessFilter::FAILURES_ONLY),
                None,
                10,
            ),
        )
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![base + 1]);
    assert!(ack.no_more_items);

    client.stop().await.unwrap();
    drop(server);
}

#[tokio::test]
async fn audit_query_matches_address_recipient_and_unknown_audit_log_errors() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 7);

    // Record 3 targets an Address recipient: it matches through the optional
    // address filter regardless of the identifier value.
    let address_match = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(77),
        target_device_address: Some(BACnetAddress {
            network_number: 9,
            mac_address: MacAddr::from_slice(&[0xAA]),
        }),
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: BACnetSuccessFilter::ALL,
    };
    let ack = client
        .audit_log_query(&server_mac, &query(log, address_match, None, 10))
        .await
        .unwrap();
    assert_eq!(sequences(&ack.records), vec![3]);
    assert!(ack.no_more_items);

    let address_mismatch = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(77),
        target_device_address: Some(BACnetAddress {
            network_number: 9,
            mac_address: MacAddr::from_slice(&[0xBB]),
        }),
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: BACnetSuccessFilter::ALL,
    };
    let ack = client
        .audit_log_query(&server_mac, &query(log, address_mismatch, None, 10))
        .await
        .unwrap();
    assert!(ack.records.is_empty());
    assert!(ack.no_more_items);

    // An unknown Audit Log instance is OBJECT/UNKNOWN_OBJECT, unlike an
    // unknown filter value which is an honest empty page.
    let missing = oid(ObjectType::AUDIT_LOG, 99);
    let error = client
        .audit_log_query(
            &server_mac,
            &query(
                missing,
                by_target(device(2), BACnetSuccessFilter::ALL),
                None,
                10,
            ),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Protocol { class, code }
                if class == u32::from(ErrorClass::OBJECT.to_raw())
                    && code == u32::from(ErrorCode::UNKNOWN_OBJECT.to_raw())
        ),
        "unexpected error: {error}"
    );

    client.stop().await.unwrap();
    drop(server);
}

#[tokio::test]
async fn audit_query_reassembles_segmented_ack_under_small_max_apdu() {
    let (server, mut client, server_mac) = started().await;
    let log = oid(ObjectType::AUDIT_LOG, 8);

    // 40 padded records (~15 KiB ACK) far exceed the 480-octet max-APDU, so
    // the server must use the segmented ComplexACK path and the client must
    // reassemble every record in newest-first order.
    let ack = client
        .audit_log_query(
            &server_mac,
            &query(
                log,
                by_target(device(2), BACnetSuccessFilter::ALL),
                None,
                u16::MAX,
            ),
        )
        .await
        .unwrap();
    assert_eq!(ack.audit_log, log);
    assert_eq!(ack.records.len(), 40);
    assert_eq!(
        sequences(&ack.records),
        (1..=40u64).rev().collect::<Vec<_>>()
    );
    assert!(ack.no_more_items);

    client.stop().await.unwrap();
    drop(server);
}
