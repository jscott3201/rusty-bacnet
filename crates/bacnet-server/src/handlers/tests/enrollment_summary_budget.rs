use super::enrollment_summary_support::*;
use super::*;
use crate::server::GetEnrollmentSummaryBudget;
use bacnet_objects::event::EnrollmentSummaryCapability;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::EventType;
use std::borrow::Cow;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Counted(SummaryFixture, Arc<AtomicUsize>);
impl BACnetObject for Counted {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.write_property(p, i, v, priority)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.property_list()
    }
    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.enrollment_summary_capability_internal()
    }
}
fn candidate(instance: u32) -> SummaryFixture {
    SummaryFixture::candidate(
        instance,
        EventType::OUT_OF_RANGE,
        EventState::NORMAL,
        7,
        7,
        None,
    )
}
fn budget(objects: usize, bytes: usize) -> GetEnrollmentSummaryBudget {
    GetEnrollmentSummaryBudget {
        max_objects: objects,
        max_service_ack_bytes: bytes,
    }
}

#[test]
fn enrollment_summary_object_preflight_and_decode_precedence_zero_callbacks() {
    for capable in [false, true] {
        let count = Arc::new(AtomicUsize::new(0));
        let mut db = ObjectDatabase::new();
        for instance in 1..=4097 {
            let object = candidate(instance);
            let object = if capable {
                object
            } else {
                object.without_capability()
            };
            db.add(Box::new(Counted(object, count.clone()))).unwrap();
        }
        count.store(0, Ordering::SeqCst);
        let mut out = BytesMut::from(&b"caller"[..]);
        assert!(matches!(
            handle_get_enrollment_summary_budgeted(
                &db,
                &[9, 0],
                &mut out,
                GetEnrollmentSummaryBudget::default()
            ),
            Err(EnrollmentSummaryFailure::Work)
        ));
        // Bad tag, undefined acknowledgment, reversed priority interval, wide priority.
        for request in [
            &[0x19, 0][..],
            &[9, 3],
            &[9, 0, 0x4e, 9, 2, 0x19, 1, 0x4f],
            &[9, 0, 0x4e, 0x0a, 1, 0, 0x19, 1, 0x4f],
        ] {
            let legacy = handle_get_enrollment_summary(&db, request, &mut out).unwrap_err();
            let Err(EnrollmentSummaryFailure::Service(actual)) =
                handle_get_enrollment_summary_budgeted(
                    &db,
                    request,
                    &mut out,
                    GetEnrollmentSummaryBudget::default(),
                )
            else {
                panic!("decode must precede work refusal")
            };
            assert_eq!(format!("{actual:?}"), format!("{legacy:?}"));
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(&out[..], b"caller");
    }
}

#[test]
fn enrollment_summary_exact_bytes_independent_vector_and_all_object_count() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate(1))).unwrap();
    db.add(Box::new(candidate(2).without_capability())).unwrap();
    db.add(Box::new(class(7, 7, [11, 22, 33], vec![]))).unwrap();
    // Clause 21 bare sequence: AI:1, out-of-range(5), normal(0), priority33, class7.
    let expected = [0xc4, 0, 0, 0, 1, 0x91, 5, 0x91, 0, 0x21, 33, 0x21, 7];
    for cap in [1, 12, 13, 14] {
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_get_enrollment_summary_budgeted(&db, &[9, 0], &mut out, budget(3, cap));
        if cap < 13 {
            assert!(matches!(result, Err(EnrollmentSummaryFailure::Bytes)));
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            assert_eq!(&out[6..], &expected);
        }
    }
    assert!(matches!(
        handle_get_enrollment_summary_budgeted(&db, &[9, 0], &mut BytesMut::new(), budget(2, 100)),
        Err(EnrollmentSummaryFailure::Work)
    ));
    let mut out = BytesMut::from(&b"prefix"[..]);
    handle_get_enrollment_summary_budgeted(&ObjectDatabase::new(), &[9, 0], &mut out, budget(1, 1))
        .unwrap();
    assert_eq!(&out[..], b"prefix");
}

#[test]
fn enrollment_summary_byte_refusal_stops_later_candidate_and_class_reads() {
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let class_reads = Arc::new(AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(Counted(candidate(1), first.clone())))
        .unwrap();
    db.add(Box::new(Counted(candidate(2), second.clone())))
        .unwrap();
    db.add(Box::new(Counted(
        SummaryFixture::notification_class(
            7,
            None,
            Some(PropertyValue::List(vec![
                PropertyValue::Unsigned(11),
                PropertyValue::Unsigned(22),
                PropertyValue::Unsigned(33),
            ])),
            None,
        ),
        class_reads.clone(),
    )))
    .unwrap();
    let order: Vec<_> = db
        .iter_objects()
        .filter_map(|(_, object)| {
            let oid = object.object_identifier();
            (oid.object_type() == ObjectType::ANALOG_INPUT).then_some(oid)
        })
        .collect();
    let later = if order[1] == ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap() {
        &first
    } else {
        &second
    };
    first.store(0, Ordering::SeqCst);
    second.store(0, Ordering::SeqCst);
    let class_visited_first = db
        .iter_objects()
        .next()
        .unwrap()
        .1
        .object_identifier()
        .object_type()
        == ObjectType::NOTIFICATION_CLASS;
    class_reads.store(0, Ordering::SeqCst);
    let mut out = BytesMut::from(&b"unchanged"[..]);
    assert!(matches!(
        handle_get_enrollment_summary_budgeted(&db, &[9, 0], &mut out, budget(3, 12)),
        Err(EnrollmentSummaryFailure::Bytes)
    ));
    assert_eq!(later.load(Ordering::SeqCst), 0);
    assert_eq!(
        class_reads.load(Ordering::SeqCst),
        1 + usize::from(class_visited_first)
    );
    assert_eq!(&out[..], b"unchanged");
}

#[test]
fn enrollment_summary_variable_enum_and_class_widths() {
    for (value, enum_bytes) in [
        (255, vec![0x91, 255]),
        (256, vec![0x92, 1, 0]),
        (65536, vec![0x93, 1, 0, 0]),
        (u32::MAX, vec![0x94, 255, 255, 255, 255]),
    ] {
        for (class_id, class_bytes) in [
            (255, vec![0x21, 255]),
            (256, vec![0x22, 1, 0]),
            (65536, vec![0x23, 1, 0, 0]),
        ] {
            let mut db = ObjectDatabase::new();
            db.add(Box::new(SummaryFixture::candidate(
                1,
                EventType::from_raw(value),
                EventState::from_raw(value),
                7,
                class_id,
                Some(bacnet_objects::event::EventTransition::ToNormal),
            )))
            .unwrap();
            db.add(Box::new(class(class_id, class_id, [1, 2, 3], vec![])))
                .unwrap();
            let mut expected = vec![0xc4, 0, 0, 0, 1];
            expected.extend_from_slice(&enum_bytes);
            expected.extend_from_slice(&enum_bytes);
            expected.extend_from_slice(&[0x21, 3]);
            expected.extend_from_slice(&class_bytes);
            for cap in [expected.len() - 1, expected.len()] {
                let mut out = BytesMut::from(&b"prefix"[..]);
                let result =
                    handle_get_enrollment_summary_budgeted(&db, &[9, 0], &mut out, budget(2, cap));
                if cap < expected.len() {
                    assert!(matches!(result, Err(EnrollmentSummaryFailure::Bytes)));
                    assert_eq!(&out[..], b"prefix");
                } else {
                    result.unwrap();
                    assert_eq!(&out[6..], expected);
                }
            }
        }
    }
}

#[test]
fn enrollment_summary_discards_accumulated_scratch_on_late_overflow() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate(1))).unwrap();
    db.add(Box::new(candidate(2))).unwrap();
    db.add(Box::new(class(7, 7, [11, 22, 33], vec![]))).unwrap();
    for cap in [13, 25, 26] {
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_get_enrollment_summary_budgeted(&db, &[9, 0], &mut out, budget(3, cap));
        if cap < 26 {
            assert!(matches!(result, Err(EnrollmentSummaryFailure::Bytes)));
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            let mut legacy = BytesMut::from(&b"prefix"[..]);
            handle_get_enrollment_summary(&db, &[9, 0], &mut legacy).unwrap();
            assert_eq!(out, legacy);
        }
    }
}
