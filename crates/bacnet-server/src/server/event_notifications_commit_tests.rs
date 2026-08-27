use super::*;
use bacnet_objects::event::{EventStateChange, LimitEnable, OutOfRangeDetector};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::BACnetTimeStamp;

struct LegacyMacroObject {
    oid: ObjectIdentifier,
    event_detector: OutOfRangeDetector,
    present_value: f32,
    reliability: u32,
    event_detection_enable: bool,
}

impl BACnetObject for LegacyMacroObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "legacy-macro"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_INPUT.to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(
                self.event_detector.notification_class as u64,
            )),
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(self.event_detector.notify_type))
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
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
        std::borrow::Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::NOTIFY_TYPE,
        ])
    }

    bacnet_objects::impl_intrinsic_reporting!(
        event_detector,
        present_value,
        reliability,
        event_detection_enable
    );
}

fn legacy_macro_object(time_delay: u32) -> LegacyMacroObject {
    let event_detector = OutOfRangeDetector {
        high_limit: 80.0,
        low_limit: 20.0,
        limit_enable: LimitEnable::BOTH,
        event_enable: 0x07,
        time_delay,
        ..OutOfRangeDetector::default()
    };
    LegacyMacroObject {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        event_detector,
        present_value: 81.0,
        reliability: bacnet_types::enums::Reliability::NO_FAULT_DETECTED.to_raw(),
        event_detection_enable: true,
    }
}

fn legacy_macro_database(time_delay: u32) -> ObjectDatabase {
    let mut db = clocked_test_database();
    db.add(Box::new(legacy_macro_object(time_delay))).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Legacy Device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    db
}

struct UnsupportedFaultReindication {
    oid: ObjectIdentifier,
}

#[tokio::test]
async fn legacy_exported_macro_per_write_outcome_still_sends() {
    let db = Arc::new(RwLock::new(legacy_macro_database(0)));

    let sent = broadcasts_from_per_write_path(&db, 0).await;

    assert_eq!(
        sent.len(),
        1,
        "legacy immediate outcomes must bypass commit"
    );
    let notification = decode_broadcast_notification(&StdMutex::new(sent));
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT.to_raw());
}

#[tokio::test(start_paused = true)]
async fn legacy_exported_macro_delayed_periodic_outcome_still_sends() {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let mut db = legacy_macro_database(1);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        db.get_mut(&oid).unwrap().evaluate_intrinsic_reporting(),
        None,
        "legacy probe should seed rather than advance the delay"
    );

    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert_eq!(
        sent.lock().unwrap().len(),
        1,
        "legacy delayed outcomes must bypass commit"
    );
    assert_eq!(
        server
            .database()
            .read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
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

#[tokio::test]
async fn committed_ack_required_snapshot_survives_notification_class_replacement() {
    let db = db_with_high_limit_transition(0x80);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let committed = {
        let mut guard = db.write().await;
        let mut notification_class = notification_class_0_broadcasting();
        notification_class.ack_required = [true, false, false];
        guard.add(Box::new(notification_class)).unwrap();
        let outcome = guard
            .get_mut(&oid)
            .unwrap()
            .evaluate_intrinsic_reporting()
            .unwrap();
        BACnetServer::<RecordingTransport>::commit_intrinsic_transition(&mut guard, &oid, outcome)
            .unwrap()
    };

    {
        let mut guard = db.write().await;
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
            "the committed TO_OFFNORMAL acknowledgment bit stays cleared"
        );
    }

    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    }));
    BACnetServer::<RecordingTransport>::build_and_send_event_notification(
        &db,
        &network,
        &Arc::new(AtomicU8::new(0)),
        &Arc::new(Mutex::new(ServerTsm::new())),
        &NotificationTransactions::new(),
        &oid,
        committed,
        1000,
    )
    .await;

    assert!(
        decode_broadcast_notification(&sent).ack_required,
        "wire Ack_Required must use the commit-time snapshot"
    );
}
