use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};

use crate::clock::{ClockFrame, ClockReader};
use crate::traits::BACnetObject;

use super::{AuditLogNotificationSink, AuditLogObject, AuditLogPersistence, AuditLogSnapshot};

#[derive(Default)]
struct MemoryPersistence {
    snapshot: Mutex<Option<AuditLogSnapshot>>,
    commits: AtomicUsize,
    fail: AtomicBool,
}

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::Acquire) {
            return Err(Error::Transport(std::io::Error::other(
                "injected commit failure",
            )));
        }
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        self.commits.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

struct FixedClock(Option<ClockFrame>);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.0
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn date() -> Date {
    Date {
        year: 124,
        month: 2,
        day: 29,
        day_of_week: 4,
    }
}

fn time(second: u8) -> Time {
    Time {
        hour: 12,
        minute: 0,
        second,
        hundredths: 0,
    }
}

fn clock() -> Arc<dyn ClockReader> {
    Arc::new(FixedClock(Some(ClockFrame {
        local_date: date(),
        local_time: time(30),
        utc_offset: 0,
        daylight_savings_status: false,
    })))
}

fn invalid_clock_frame() -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 30,
            day_of_week: 5,
        },
        local_time: time(30),
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

fn notification(
    source: Option<BACnetTimeStamp>,
    target: Option<BACnetTimeStamp>,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: source,
        target_timestamp: target,
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
        source_object: Some(oid(ObjectType::ANALOG_INPUT, 1)),
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: Some(7),
        source_user_id: Some(9),
        source_user_role: Some(2),
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
        target_object: Some(oid(ObjectType::ANALOG_VALUE, 3)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }),
        target_priority: Some(8),
        target_value: Some(vec![0x21, 0x05]),
        current_value: None,
        result: None,
    }
}

fn log(capacity: u32) -> (AuditLogObject, Arc<MemoryPersistence>) {
    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "audit", capacity, persistence.clone()).unwrap();
    log.bind_clock_internal(Some(clock()));
    (log, persistence)
}

#[test]
fn complementary_batch_merges_once_and_target_current_value_wins() {
    let (mut log, persistence) = log(10);
    let mut source = notification(Some(BACnetTimeStamp::Time(time(0))), None);
    source.current_value = Some(vec![0x21, 0x01]);
    let mut target = notification(None, Some(BACnetTimeStamp::Time(time(4))));
    target.current_value = Some(vec![0x21, 0x02]);

    log.store_notifications(&[source, target], 2_000).unwrap();

    assert_eq!(persistence.commits.load(Ordering::Acquire), 2); // initialization + batch
    assert_eq!(log.total_record_count(), 1);
    assert_eq!(log.records().len(), 1);
    let result = log.records().front().unwrap();
    assert_eq!(result.sequence_number, 1);
    assert_eq!(result.record.timestamp, (date(), time(30)));
    let BACnetAuditLogDatum::AuditNotification(stored) = &result.record.datum else {
        panic!("expected notification")
    };
    assert!(stored.source_timestamp.is_some());
    assert!(stored.target_timestamp.is_some());
    assert_eq!(stored.current_value, Some(vec![0x21, 0x02]));
}

#[test]
fn completed_match_is_dropped_and_merge_preserves_record_identity() {
    let (mut log, persistence) = log(10);
    let source = notification(Some(BACnetTimeStamp::Time(time(0))), None);
    log.store_notifications(&[source], 2_000).unwrap();
    let identity = log.records().front().unwrap().clone();
    let target = notification(None, Some(BACnetTimeStamp::Time(time(3))));
    log.store_notifications(std::slice::from_ref(&target), 2_000)
        .unwrap();
    assert_eq!(log.total_record_count(), 1);
    assert_eq!(
        log.records().front().unwrap().sequence_number,
        identity.sequence_number
    );
    assert_eq!(
        log.records().front().unwrap().record.timestamp,
        identity.record.timestamp
    );
    let generation = log.generation();
    let commits = persistence.commits.load(Ordering::Acquire);

    log.store_notifications(&[target], 2_000).unwrap();
    assert_eq!(log.generation(), generation);
    assert_eq!(persistence.commits.load(Ordering::Acquire), commits);
    assert_eq!(log.total_record_count(), 1);
}

#[test]
fn match_identity_excludes_the_separate_source_and_target_object_parameters() {
    let (mut log, _) = log(10);
    let source = notification(Some(BACnetTimeStamp::Time(time(0))), None);
    let source_object = source.source_object;
    let target_object = source.target_object;
    let mut target = notification(None, Some(BACnetTimeStamp::Time(time(2))));
    target.source_object = Some(oid(ObjectType::ANALOG_INPUT, 99));
    target.target_object = Some(oid(ObjectType::ANALOG_VALUE, 99));

    log.store_notifications(&[source, target], 2_000).unwrap();

    assert_eq!(log.total_record_count(), 1);
    let BACnetAuditLogDatum::AuditNotification(stored) =
        &log.records().front().unwrap().record.datum
    else {
        panic!("expected notification")
    };
    assert_eq!(stored.source_object, source_object);
    assert_eq!(stored.target_object, target_object);
    assert!(stored.source_timestamp.is_some());
    assert!(stored.target_timestamp.is_some());
}

#[test]
fn timestamp_variants_use_configured_window_and_never_cross_compare() {
    let cases = [
        (
            BACnetTimeStamp::DateTime {
                date: date(),
                time: time(0),
            },
            BACnetTimeStamp::DateTime {
                date: date(),
                time: time(5),
            },
            2_500,
            1,
        ),
        (
            BACnetTimeStamp::SequenceNumber(7),
            BACnetTimeStamp::SequenceNumber(7),
            1,
            1,
        ),
        (
            BACnetTimeStamp::SequenceNumber(7),
            BACnetTimeStamp::SequenceNumber(8),
            60_000,
            2,
        ),
        (
            BACnetTimeStamp::Time(time(0)),
            BACnetTimeStamp::SequenceNumber(0),
            60_000,
            2,
        ),
    ];
    for (source_timestamp, target_timestamp, timeout, expected_count) in cases {
        let (mut log, _) = log(10);
        log.store_notifications(
            &[
                notification(Some(source_timestamp), None),
                notification(None, Some(target_timestamp)),
            ],
            timeout,
        )
        .unwrap();
        assert_eq!(log.total_record_count(), expected_count);
    }
}

#[test]
fn commit_and_clock_failures_leave_memory_and_durable_snapshot_unchanged() {
    let (mut log, persistence) = log(10);
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();
    persistence.fail.store(true, Ordering::Release);
    assert!(log
        .store_notifications(
            &[notification(Some(BACnetTimeStamp::Time(time(0))), None)],
            2_000
        )
        .is_err());
    assert_eq!(log.total_record_count(), 0);
    assert_eq!(*persistence.snapshot.lock().unwrap(), Some(before));

    persistence.fail.store(false, Ordering::Release);
    for frame in [None, Some(invalid_clock_frame())] {
        log.bind_clock_internal(Some(Arc::new(FixedClock(frame))));
        let durable = persistence.snapshot.lock().unwrap().clone();
        let error = log
            .store_notifications(
                &[notification(Some(BACnetTimeStamp::Time(time(0))), None)],
                2_000,
            )
            .unwrap_err();
        assert!(matches!(error, Error::Protocol { class, code }
            if class == bacnet_types::enums::ErrorClass::DEVICE.to_raw() as u32
                && code == bacnet_types::enums::ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32));
        assert_eq!(log.total_record_count(), 0);
        assert_eq!(*persistence.snapshot.lock().unwrap(), durable);
    }
}

#[test]
fn disabled_sink_denies_before_clock_or_persistence_change() {
    let (mut log, persistence) = log(10);
    log.write_property(
        bacnet_types::enums::PropertyIdentifier::LOG_ENABLE,
        None,
        bacnet_types::primitives::PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let generation = log.generation();
    let error = log
        .store_notifications(&[notification(None, None)], 2_000)
        .unwrap_err();
    assert!(matches!(error, Error::Protocol { class, code }
        if class == bacnet_types::enums::ErrorClass::SERVICES.to_raw() as u32
            && code == bacnet_types::enums::ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32));
    assert_eq!(log.generation(), generation);
    assert_eq!(
        persistence
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .generation,
        generation
    );
}
