use super::*;
use crate::server::GetAlarmSummaryBudget;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Counted {
    object: AlarmSummaryFixture,
    callbacks: Arc<AtomicUsize>,
}

impl BACnetObject for Counted {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.object.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.object.object_name()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.object.property_list()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.object.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.object.write_property(p, i, v, priority)
    }
}

fn counted(db: &mut ObjectDatabase, object: AlarmSummaryFixture) -> Arc<AtomicUsize> {
    let callbacks = Arc::new(AtomicUsize::new(0));
    db.add(Box::new(Counted {
        object,
        callbacks: callbacks.clone(),
    }))
    .unwrap();
    callbacks.store(0, Ordering::SeqCst);
    callbacks
}

fn limits(objects: usize, bytes: usize) -> GetAlarmSummaryBudget {
    GetAlarmSummaryBudget {
        max_objects: objects,
        max_service_ack_bytes: bytes,
    }
}

#[test]
fn all_object_preflight_exact_over_and_no_callbacks() {
    let mut db = ObjectDatabase::new();
    let alarm = counted(&mut db, AlarmSummaryFixture::alarm(1));
    let mut non_alarm = AlarmSummaryFixture::alarm(2);
    non_alarm.advertised.clear();
    let other = counted(&mut db, non_alarm);
    // Reset after all database registration callbacks, outside the service.
    alarm.store(0, Ordering::SeqCst);
    let mut out = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_get_alarm_summary_budgeted(&db, &mut out, limits(1, 100)),
        Err(AlarmSummaryFailure::Work)
    ));
    assert_eq!(&out[..], b"prefix");
    assert_eq!(alarm.load(Ordering::SeqCst), 0);
    assert_eq!(other.load(Ordering::SeqCst), 0);
    handle_get_alarm_summary_budgeted(&db, &mut out, limits(2, 100)).unwrap();
    assert_eq!(
        GetAlarmSummaryAck::decode(&out[6..]).unwrap().entries.len(),
        1
    );
}

#[test]
fn byte_exact_over_tiny_empty_and_legacy_compatibility() {
    let mut db = ObjectDatabase::new();
    add(&mut db, AlarmSummaryFixture::alarm(1));
    add(&mut db, AlarmSummaryFixture::alarm(2));
    // Independently specified triples: application object-id(5), enum(2), bits(3).
    let expected: Vec<u8> = db
        .iter_objects()
        .flat_map(|(oid, _)| {
            [
                0xc4,
                0,
                0,
                0,
                oid.instance_number() as u8,
                0x91,
                2,
                0x82,
                5,
                0xe0,
            ]
        })
        .collect();
    for cap in [1, 9, 10, 19, 20, 21] {
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_get_alarm_summary_budgeted(&db, &mut out, limits(2, cap));
        if cap < 20 {
            assert!(matches!(result, Err(AlarmSummaryFailure::Bytes)));
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            assert_eq!(&out[6..], &expected);
        }
    }
    let mut legacy = BytesMut::new();
    handle_get_alarm_summary(&db, &mut legacy).unwrap();
    assert_eq!(&legacy[..], &expected);
    let mut empty = BytesMut::from(&b"prefix"[..]);
    handle_get_alarm_summary_budgeted(&ObjectDatabase::new(), &mut empty, limits(1, 1)).unwrap();
    assert_eq!(&empty[..], b"prefix");
}

#[test]
fn byte_failure_stops_later_callbacks_and_service_errors_do_not_commit() {
    let mut db = ObjectDatabase::new();
    let first = counted(&mut db, AlarmSummaryFixture::alarm(1));
    let second = counted(&mut db, AlarmSummaryFixture::alarm(2));
    first.store(0, Ordering::SeqCst);
    let later = if db.iter_objects().nth(1).unwrap().0.instance_number() == 1 {
        first
    } else {
        second
    };
    let mut out = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_get_alarm_summary_budgeted(&db, &mut out, limits(2, 9)),
        Err(AlarmSummaryFailure::Bytes)
    ));
    assert_eq!(later.load(Ordering::SeqCst), 0);
    assert_eq!(&out[..], b"prefix");
    let mut db = ObjectDatabase::new();
    let mut malformed = AlarmSummaryFixture::alarm(2);
    malformed.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Boolean(true),
    );
    add(&mut db, malformed);
    let Err(AlarmSummaryFailure::Service(error)) =
        handle_get_alarm_summary_budgeted(&db, &mut out, limits(2, 20))
    else {
        panic!("genuine projection error must survive");
    };
    assert_operational_problem(error);
    assert_eq!(&out[..], b"prefix");
}

#[test]
fn raised_limits_and_variable_enum_width() {
    let mut db = ObjectDatabase::new();
    let mut object = AlarmSummaryFixture::alarm(1);
    object.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(u32::MAX),
    );
    add(&mut db, object);
    let mut out = BytesMut::new();
    assert!(matches!(
        handle_get_alarm_summary_budgeted(&db, &mut out, limits(1, 12)),
        Err(AlarmSummaryFailure::Bytes)
    ));
    handle_get_alarm_summary_budgeted(&db, &mut out, limits(usize::MAX, 13)).unwrap();
    assert_eq!(out.len(), 13);
    assert_eq!(
        GetAlarmSummaryAck::decode(&out).unwrap().entries[0]
            .alarm_state
            .to_raw(),
        u32::MAX
    );
}
