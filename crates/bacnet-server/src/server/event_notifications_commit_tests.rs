use super::*;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::BACnetTimeStamp;

struct UnsupportedFaultReindication {
    oid: ObjectIdentifier,
}

impl BACnetObject for UnsupportedFaultReindication {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "unsupported-fault"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            Ok(PropertyValue::Unsigned(0))
        } else {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(&[PropertyIdentifier::NOTIFICATION_CLASS])
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<bacnet_objects::event::TransitionOutcome> {
        Some(bacnet_objects::event::TransitionOutcome {
            change: EventStateChange {
                from: EventState::FAULT,
                to: EventState::FAULT,
            },
            event_type: EventType::CHANGE_OF_RELIABILITY,
            distribute: true,
        })
    }
}

#[tokio::test]
async fn no_recipient_transition_commits_locally_without_sending() {
    let db = db_with_high_limit_transition(0x80);
    {
        let mut guard = db.write().await;
        guard
            .add(Box::new(
                bacnet_objects::notification_class::NotificationClass::new(0, "NC-0").unwrap(),
            ))
            .unwrap();
    }

    let sent = broadcasts_from_per_write_path(&db, 0).await;
    assert!(sent.is_empty());
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

#[test]
fn unsupported_fault_reindication_is_retryable_without_sequence_consumption() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(UnsupportedFaultReindication { oid }))
        .unwrap();

    let proposal = db
        .get_mut(&oid)
        .unwrap()
        .evaluate_intrinsic_reporting()
        .unwrap();
    assert!(
        BACnetServer::<RecordingTransport>::commit_intrinsic_transition(
            &mut db,
            &oid,
            proposal.clone(),
        )
        .is_none()
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    assert_eq!(
        db.get_mut(&oid).unwrap().evaluate_intrinsic_reporting(),
        Some(proposal)
    );
}

#[tokio::test]
async fn dcc_suppressed_intrinsic_transition_commits_locally_without_sending() {
    let db = db_with_high_limit_transition(0x80);
    let sent = broadcasts_from_per_write_path(&db, 1).await;

    assert!(sent.is_empty(), "DCC must suppress external distribution");
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "DCC must not suppress the local transition commit"
    );
}

#[tokio::test]
async fn clockless_intrinsic_commit_stores_and_sends_one_reserved_sequence() {
    let db = db_with_high_limit_transition(0x80);
    {
        let mut guard = db.write().await;
        guard.set_clock_reader(None);
        let mut notification_class = notification_class_0_broadcasting();
        notification_class.ack_required = [true, false, false];
        guard.add(Box::new(notification_class)).unwrap();
    }

    let sent = broadcasts_from_per_write_path(&db, 0).await;
    let notification = decode_broadcast_notification(&StdMutex::new(sent));
    assert_eq!(
        notification.timestamp,
        BACnetTimeStamp::SequenceNumber(0),
        "the outbound request must reuse the timestamp committed under the lock"
    );

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_timestamp_choice(
        &mut encoded,
        &BACnetTimeStamp::SequenceNumber(0),
    )
    .unwrap();
    let mut guard = db.write().await;
    let object = guard.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(1))
            .unwrap(),
        PropertyValue::ApplicationData(encoded.to_vec())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x60],
        },
        "Ack_Required clears only TO_OFFNORMAL"
    );
    assert_eq!(guard.reserve_event_sequence_number().number(), 1);

    let mut replacement = notification_class_0_broadcasting();
    replacement.ack_required = [false; 3];
    guard.add(Box::new(replacement)).unwrap();
    assert_eq!(
        guard
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x60],
        },
        "later Notification Class edits must not rewrite committed acknowledgment state"
    );
}
