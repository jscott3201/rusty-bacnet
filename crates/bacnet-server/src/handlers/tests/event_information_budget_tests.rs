use super::*;
use crate::server::GetEventInformationBudget;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Counted {
    inner: ProjectionFixture,
    calls: Arc<AtomicUsize>,
    projections: Arc<AtomicUsize>,
}
impl BACnetObject for Counted {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.object_name()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.projections.fetch_add(1, Ordering::SeqCst);
        self.inner.property_list()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(property, index, value, priority)
    }
}

#[test]
fn get_event_information_preflight_no_callbacks_and_full_scan_after_capacity() {
    let calls = Arc::new(AtomicUsize::new(0));
    let projections = Arc::new(AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    for inner in [
        ProjectionFixture::summary(1),
        ProjectionFixture::summary(2),
        ProjectionFixture::notification_class(
            99,
            Some(PropertyValue::Unsigned(42)),
            Some(PropertyValue::List(vec![PropertyValue::Unsigned(1); 3])),
        ),
    ] {
        db.add(Box::new(Counted {
            inner,
            calls: calls.clone(),
            projections: projections.clone(),
        }))
        .unwrap();
    }
    calls.store(0, Ordering::SeqCst);
    let budget = GetEventInformationBudget {
        max_objects: 2,
        ..Default::default()
    };
    for cursor in [
        None,
        Some(ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 999).unwrap()),
    ] {
        assert!(matches!(
            configured(&db, cursor, budget),
            Err(EventInformationFailure::Objects)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    for bytes in [1, 16384] {
        projections.store(0, Ordering::SeqCst);
        let result = configured(
            &db,
            None,
            GetEventInformationBudget {
                max_objects: 3,
                max_returned_summaries: 1,
                max_service_ack_bytes: bytes,
            },
        );
        assert_eq!(projections.load(Ordering::SeqCst), 3);
        if bytes == 1 {
            assert!(matches!(result, Err(EventInformationFailure::Bytes)));
        } else {
            assert!(
                GetEventInformationAck::decode(&result.unwrap())
                    .unwrap()
                    .more_events
            );
        }
    }
}

fn configured(
    db: &ObjectDatabase,
    cursor: Option<ObjectIdentifier>,
    budget: GetEventInformationBudget,
) -> Result<BytesMut, EventInformationFailure> {
    let mut encoded = BytesMut::new();
    handle_get_event_information_configured(db, &request(cursor), &mut encoded, budget)?;
    Ok(encoded)
}

fn database(count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    add_class(&mut db, 99, [1, 100, 255]);
    for instance in (1..=count).rev() {
        db.add(Box::new(ProjectionFixture::summary(instance)))
            .unwrap();
    }
    db
}

#[test]
fn get_event_information_configured_256_257_exact_cursor_pages() {
    for count in [0, 1, 256, 257] {
        let db = database(count);
        let encoded = configured(&db, None, GetEventInformationBudget::default()).unwrap();
        let ack = GetEventInformationAck::decode(&encoded).unwrap();
        assert_eq!(ack.list_of_event_summaries.len(), count.min(256) as usize);
        assert_eq!(ack.more_events, count > 256);
        for (i, summary) in ack.list_of_event_summaries.iter().enumerate() {
            assert_eq!(summary.object_identifier.instance_number(), i as u32 + 1);
        }
        if count == 257 {
            let cursor = ack
                .list_of_event_summaries
                .last()
                .unwrap()
                .object_identifier;
            let final_page =
                configured(&db, Some(cursor), GetEventInformationBudget::default()).unwrap();
            let last = GetEventInformationAck::decode(&final_page).unwrap();
            assert!(!last.more_events);
            assert_eq!(last.list_of_event_summaries.len(), 1);
            assert_eq!(
                last.list_of_event_summaries[0]
                    .object_identifier
                    .instance_number(),
                257
            );
        } else {
            let mut legacy = BytesMut::new();
            handle_get_event_information(&db, &[], &mut legacy).unwrap();
            assert_eq!(encoded, legacy);
        }
    }
}

#[test]
fn get_event_information_configured_exact_bytes_tiny_empty_and_prefix() {
    let one = configured(&database(1), None, GetEventInformationBudget::default()).unwrap();
    let two = configured(&database(2), None, GetEventInformationBudget::default()).unwrap();
    for (bytes, expected, more) in [
        (one.len(), 1, true),
        (two.len() - 1, 1, true),
        (two.len(), 2, false),
    ] {
        let encoded = configured(
            &database(2),
            None,
            GetEventInformationBudget {
                max_service_ack_bytes: bytes,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(encoded.len() <= bytes);
        let ack = GetEventInformationAck::decode(&encoded).unwrap();
        assert_eq!(ack.list_of_event_summaries.len(), expected);
        assert_eq!(ack.more_events, more);
    }
    for bytes in [0, 1, 3, one.len() - 1] {
        assert!(matches!(
            configured(
                &database(1),
                None,
                GetEventInformationBudget {
                    max_service_ack_bytes: bytes,
                    ..Default::default()
                }
            ),
            Err(EventInformationFailure::Bytes)
        ));
    }
    let empty = configured(
        &database(0),
        None,
        GetEventInformationBudget {
            max_service_ack_bytes: 4,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(&empty[..], &[0x0e, 0x0f, 0x19, 0]);
    assert!(matches!(
        configured(
            &database(0),
            None,
            GetEventInformationBudget {
                max_service_ack_bytes: 3,
                ..Default::default()
            }
        ),
        Err(EventInformationFailure::Bytes)
    ));
}

#[test]
fn get_event_information_configured_late_errors_win_and_normals_do_not_imply_more() {
    for bytes in [1, 16384] {
        for normal in [false, true] {
            let mut db = database(1);
            let mut late = ProjectionFixture::summary(2);
            if normal {
                late.set(
                    PropertyIdentifier::EVENT_STATE,
                    PropertyValue::Enumerated(0),
                );
            }
            late.remove(PropertyIdentifier::NOTIFY_TYPE);
            db.add(Box::new(late)).unwrap();
            let budget = GetEventInformationBudget {
                max_returned_summaries: 1,
                max_service_ack_bytes: bytes,
                ..Default::default()
            };
            assert!(
                matches!(configured(&db, None, budget), Err(EventInformationFailure::Service(Error::Protocol { class, code }))
                if class == ErrorClass::DEVICE.to_raw() as u32 && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32)
            );
            assert!(handle_get_event_information(&db, &[], &mut BytesMut::new()).is_err());
        }
    }
    let mut db = database(1);
    let mut normal = ProjectionFixture::summary(2);
    normal.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(0),
    );
    db.add(Box::new(normal)).unwrap();
    let mut disabled = ProjectionFixture::summary(3);
    disabled.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    disabled.set(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    disabled.remove(PropertyIdentifier::NOTIFY_TYPE);
    db.add(Box::new(disabled)).unwrap();
    let ack = configured(
        &db,
        None,
        GetEventInformationBudget {
            max_returned_summaries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!GetEventInformationAck::decode(&ack).unwrap().more_events);
    db.add(Box::new(ProjectionFixture::summary(4))).unwrap();
    let ack = configured(
        &db,
        None,
        GetEventInformationBudget {
            max_returned_summaries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(GetEventInformationAck::decode(&ack).unwrap().more_events);
}

#[test]
fn get_event_information_configured_cardinality_decode_and_cursor_precedence() {
    let mut db = database(1);
    // Malformed projection must never be touched on rejected cardinality.
    let mut malformed = ProjectionFixture::summary(2);
    malformed.remove(PropertyIdentifier::NOTIFY_TYPE);
    db.add(Box::new(malformed)).unwrap();
    let budget = GetEventInformationBudget {
        max_objects: 2,
        ..Default::default()
    };
    for cursor in [
        None,
        Some(ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 999).unwrap()),
    ] {
        assert!(matches!(
            configured(&db, cursor, budget),
            Err(EventInformationFailure::Objects)
        ));
    }
    let mut out = BytesMut::from(&b"sentinel"[..]);
    assert!(matches!(
        handle_get_event_information_configured(&db, &[0xff], &mut out, budget),
        Err(EventInformationFailure::Service(_))
    ));
    assert_eq!(&out[..], b"sentinel");
}

#[test]
fn get_event_information_configured_mixed_types_missing_cursor_and_class_errors() {
    let mut db = database(1);
    let mut unacked = ProjectionFixture::summary(3);
    unacked.oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 3).unwrap();
    unacked.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Enumerated(0),
    );
    unacked.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        transition_bits(0b110),
    );
    db.add(Box::new(unacked)).unwrap();
    for cursor in [
        None,
        Some(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()),
        Some(ObjectIdentifier::new(ObjectType::BINARY_INPUT, 3).unwrap()),
    ] {
        let expected = response(&db, cursor, None).unwrap();
        let encoded = configured(&db, cursor, GetEventInformationBudget::default()).unwrap();
        let ack = GetEventInformationAck::decode(&encoded).unwrap();
        assert_eq!(encoded.len(), expected.1);
        assert_eq!(format!("{ack:?}"), format!("{:?}", expected.0));
    }
    add_class(&mut db, 98, [2, 3, 4]);
    assert!(matches!(
        configured(&db, None, GetEventInformationBudget::default()),
        Err(EventInformationFailure::Service(_))
    ));
    let mut missing = ObjectDatabase::new();
    missing
        .add(Box::new(ProjectionFixture::summary(1)))
        .unwrap();
    assert!(matches!(
        configured(&missing, None, GetEventInformationBudget::default()),
        Err(EventInformationFailure::Service(_))
    ));
}
