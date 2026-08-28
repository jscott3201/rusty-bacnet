use super::*;
use crate::event_enrollment::{
    CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult,
    EventEnrollmentReliabilityCause, EventEnrollmentReliabilityResult, EventEnrollmentTransition,
};
use crate::server::event_timestamp::SampledEventClock;
use bacnet_objects::event::{
    EventStateChange, EventTransitionCommit, EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

#[derive(Clone)]
enum IndexedHistoryRead {
    Value(PropertyValue),
    Missing,
    Unreadable,
}

struct AtomicHistoryObject {
    oid: ObjectIdentifier,
    event_type: Option<PropertyValue>,
    timestamps: [IndexedHistoryRead; 3],
    messages: [IndexedHistoryRead; 3],
}

impl AtomicHistoryObject {
    fn indexed(
        values: &[IndexedHistoryRead; 3],
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let Some(index @ 1..=3) = array_index else {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
            });
        };
        match &values[index as usize - 1] {
            IndexedHistoryRead::Value(value) => Ok(value.clone()),
            IndexedHistoryRead::Missing => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
            IndexedHistoryRead::Unreadable => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::READ_ACCESS_DENIED.to_raw() as u32,
            }),
        }
    }
}

impl BACnetObject for AtomicHistoryObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "atomic-history"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
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
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(0)),
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(NotifyType::ALARM.to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_TYPE => {
                self.event_type.clone().ok_or_else(|| Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                })
            }
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => {
                Self::indexed(&self.timestamps, array_index)
            }
            p if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS => {
                Self::indexed(&self.messages, array_index)
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
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ])
    }

    fn intrinsic_reporting_requires_atomic_commit(&self) -> bool {
        true
    }

    fn commit_event_transition_internal(
        &mut self,
        _commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        Ok(())
    }
}

fn timestamp_read(timestamp: BACnetTimeStamp) -> IndexedHistoryRead {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_timestamp_choice(&mut encoded, &timestamp).unwrap();
    IndexedHistoryRead::Value(PropertyValue::ApplicationData(encoded.to_vec()))
}

fn empty_message_read() -> IndexedHistoryRead {
    IndexedHistoryRead::Value(PropertyValue::CharacterString(String::new()))
}

fn atomic_history_database(
    timestamps: [IndexedHistoryRead; 3],
    messages: [IndexedHistoryRead; 3],
    clocked: bool,
) -> (Arc<RwLock<ObjectDatabase>>, ObjectIdentifier) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 77).unwrap();
    let mut db = if clocked {
        clocked_test_database()
    } else {
        ObjectDatabase::new()
    };
    db.add(Box::new(AtomicHistoryObject {
        oid,
        event_type: Some(PropertyValue::Enumerated(EventType::OUT_OF_RANGE.to_raw())),
        timestamps,
        messages,
    }))
    .unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "History Device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    (Arc::new(RwLock::new(db)), oid)
}

async fn commit_and_capture_history_notification(
    timestamps: [IndexedHistoryRead; 3],
    messages: [IndexedHistoryRead; 3],
    change: EventStateChange,
    clocked: bool,
) -> (Arc<RwLock<ObjectDatabase>>, Vec<bytes::Bytes>) {
    let (db, oid) = atomic_history_database(timestamps, messages, clocked);
    let committed = {
        let mut guard = db.write().await;
        BACnetServer::<RecordingTransport>::commit_intrinsic_transition(
            &mut guard,
            &oid,
            TransitionOutcome {
                change,
                event_type: EventType::OUT_OF_RANGE,
                distribute: true,
            },
        )
    };

    let sent = StdArc::new(StdMutex::new(Vec::new()));
    if let Some(committed) = committed {
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
    }

    let captured = sent.lock().unwrap().clone();
    (db, captured)
}

fn repeated_timestamp_reads(timestamp: BACnetTimeStamp) -> [IndexedHistoryRead; 3] {
    std::array::from_fn(|_| timestamp_read(timestamp.clone()))
}

fn empty_message_reads() -> [IndexedHistoryRead; 3] {
    std::array::from_fn(|_| empty_message_read())
}

fn committed_enrollment_normal(
    oid: ObjectIdentifier,
    change: EventStateChange,
) -> CommittedEventEnrollmentDelivery {
    CommittedEventEnrollmentDelivery {
        result: CommittedEventEnrollmentResult::Normal(EventEnrollmentTransition {
            enrollment_oid: oid,
            monitored_oid: oid,
            change,
            event_type: EventType::OUT_OF_RANGE,
            distribute: true,
        }),
        ack_required: true,
        recipient_clock: SampledEventClock::Unavailable,
    }
}

fn committed_enrollment_reliability(oid: ObjectIdentifier) -> CommittedEventEnrollmentDelivery {
    CommittedEventEnrollmentDelivery {
        result: CommittedEventEnrollmentResult::Reliability(EventEnrollmentReliabilityResult {
            enrollment_oid: oid,
            monitored_oid: Some(oid),
            previous_reliability: bacnet_types::enums::Reliability::NO_FAULT_DETECTED,
            new_reliability: bacnet_types::enums::Reliability::OVER_RANGE,
            state_change: Some(EventStateChange {
                from: EventState::NORMAL,
                to: EventState::FAULT,
            }),
            distribute: true,
            cause: EventEnrollmentReliabilityCause::FaultAlgorithm,
        }),
        ack_required: true,
        recipient_clock: SampledEventClock::Unavailable,
    }
}

#[test]
fn event_enrollment_projection_accepts_intentionally_absent_message_without_mutation() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 77).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AtomicHistoryObject {
        oid,
        event_type: Some(PropertyValue::Enumerated(EventType::OUT_OF_RANGE.to_raw())),
        timestamps: repeated_timestamp_reads(BACnetTimeStamp::SequenceNumber(19)),
        messages: std::array::from_fn(|_| IndexedHistoryRead::Missing),
    }))
    .unwrap();

    let before = db.reserve_event_sequence_number().number();
    let resolved =
        crate::server::event_notifications::resolve_committed_event_enrollment_transition(
            &db,
            committed_enrollment_normal(
                oid,
                EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::HIGH_LIMIT,
                },
            ),
        );

    assert!(resolved.is_some());
    assert_eq!(db.reserve_event_sequence_number().number(), before);
}

#[test]
fn malformed_or_unreadable_event_enrollment_history_fails_closed_without_mutation() {
    let failures = [
        IndexedHistoryRead::Missing,
        IndexedHistoryRead::Unreadable,
        IndexedHistoryRead::Value(PropertyValue::Unsigned(7)),
        IndexedHistoryRead::Value(PropertyValue::ApplicationData(vec![0x19])),
    ];

    for (instance, failure) in failures.into_iter().enumerate() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 80 + instance as u32).unwrap();
        let mut db = ObjectDatabase::new();
        let timestamps = std::array::from_fn(|index| {
            if index == 0 {
                failure.clone()
            } else {
                timestamp_read(BACnetTimeStamp::SequenceNumber(index as u16))
            }
        });
        db.add(Box::new(AtomicHistoryObject {
            oid,
            event_type: Some(PropertyValue::Enumerated(EventType::OUT_OF_RANGE.to_raw())),
            timestamps,
            messages: std::array::from_fn(|_| IndexedHistoryRead::Missing),
        }))
        .unwrap();

        assert!(
            crate::server::event_notifications::resolve_committed_event_enrollment_transition(
                &db,
                committed_enrollment_normal(
                    oid,
                    EventStateChange {
                        from: EventState::NORMAL,
                        to: EventState::HIGH_LIMIT,
                    },
                ),
            )
            .is_none()
        );
        assert_eq!(db.reserve_event_sequence_number().number(), 0);
    }
}

#[test]
fn unreadable_or_malformed_reliability_event_type_fails_closed_without_mutation() {
    for (instance, event_type) in [
        None,
        Some(PropertyValue::Unsigned(2)),
        Some(PropertyValue::Enumerated(999)),
    ]
    .into_iter()
    .enumerate()
    {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 90 + instance as u32).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(AtomicHistoryObject {
            oid,
            event_type,
            timestamps: repeated_timestamp_reads(BACnetTimeStamp::SequenceNumber(23)),
            messages: std::array::from_fn(|_| IndexedHistoryRead::Missing),
        }))
        .unwrap();

        assert!(
            crate::server::event_notifications::resolve_committed_event_enrollment_transition(
                &db,
                committed_enrollment_reliability(oid),
            )
            .is_none()
        );
        assert_eq!(db.reserve_event_sequence_number().number(), 0);
    }
}

#[tokio::test]
async fn committed_history_preserves_each_timestamp_choice_on_the_wire() {
    let choices = [
        BACnetTimeStamp::Time(Time {
            hour: 4,
            minute: 5,
            second: 6,
            hundredths: 7,
        }),
        BACnetTimeStamp::SequenceNumber(44_444),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
        },
    ];

    for expected in choices {
        let (_, sent) = commit_and_capture_history_notification(
            repeated_timestamp_reads(expected.clone()),
            empty_message_reads(),
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            true,
        )
        .await;
        let notification = decode_broadcast_notification(&StdMutex::new(sent));
        assert_eq!(notification.timestamp, expected);
        assert_eq!(
            notification.message_text, None,
            "empty committed message history must remain absent on the wire"
        );
    }
}

#[tokio::test]
async fn committed_history_selects_only_the_transition_coordinate() {
    let timestamps = [
        timestamp_read(BACnetTimeStamp::SequenceNumber(11)),
        timestamp_read(BACnetTimeStamp::SequenceNumber(22)),
        timestamp_read(BACnetTimeStamp::SequenceNumber(33)),
    ];
    let cases = [
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            11,
        ),
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::FAULT,
            },
            22,
        ),
        (
            EventStateChange {
                from: EventState::HIGH_LIMIT,
                to: EventState::NORMAL,
            },
            33,
        ),
    ];

    for (change, expected) in cases {
        let (_, sent) = commit_and_capture_history_notification(
            timestamps.clone(),
            empty_message_reads(),
            change,
            true,
        )
        .await;
        assert_eq!(
            decode_broadcast_notification(&StdMutex::new(sent)).timestamp,
            BACnetTimeStamp::SequenceNumber(expected)
        );
    }
}

#[tokio::test]
async fn committed_history_timestamp_is_distinct_from_the_staged_device_clock_sample() {
    let expected = BACnetTimeStamp::SequenceNumber(54_321);
    let (_, sent) = commit_and_capture_history_notification(
        repeated_timestamp_reads(expected.clone()),
        empty_message_reads(),
        EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        },
        true,
    )
    .await;

    assert_eq!(
        decode_broadcast_notification(&StdMutex::new(sent)).timestamp,
        expected,
        "the stored coordinate, not the staged Device DateTime, is wire authority"
    );
}

#[tokio::test]
async fn nonempty_committed_message_is_captured_with_its_timestamp() {
    let messages = std::array::from_fn(|index| {
        IndexedHistoryRead::Value(PropertyValue::CharacterString(format!("message-{index}")))
    });
    let (_, sent) = commit_and_capture_history_notification(
        repeated_timestamp_reads(BACnetTimeStamp::SequenceNumber(9)),
        messages,
        EventStateChange {
            from: EventState::NORMAL,
            to: EventState::FAULT,
        },
        true,
    )
    .await;

    assert_eq!(
        decode_broadcast_notification(&StdMutex::new(sent)).message_text,
        Some("message-1".into())
    );
}

#[tokio::test]
async fn invalid_committed_history_suppresses_emission_after_consuming_sequence() {
    let mut trailing = BytesMut::new();
    bacnet_encoding::primitives::encode_timestamp_choice(
        &mut trailing,
        &BACnetTimeStamp::SequenceNumber(7),
    )
    .unwrap();
    trailing.extend_from_slice(&[0]);

    let failures = [
        IndexedHistoryRead::Value(PropertyValue::ApplicationData(vec![0x19])),
        IndexedHistoryRead::Value(PropertyValue::Unsigned(7)),
        IndexedHistoryRead::Missing,
        IndexedHistoryRead::Unreadable,
        IndexedHistoryRead::Value(PropertyValue::ApplicationData(trailing.to_vec())),
    ];

    for failure in failures {
        let timestamps = std::array::from_fn(|index| {
            if index == 0 {
                failure.clone()
            } else {
                timestamp_read(BACnetTimeStamp::SequenceNumber(index as u16))
            }
        });
        let (db, sent) = commit_and_capture_history_notification(
            timestamps,
            empty_message_reads(),
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            false,
        )
        .await;

        assert!(
            sent.is_empty(),
            "invalid committed history must fail closed"
        );
        assert_eq!(
            db.write().await.reserve_event_sequence_number().number(),
            1,
            "a successful commit permanently consumes its staged sequence"
        );
    }
}

#[tokio::test]
async fn invalid_committed_message_history_suppresses_emission_after_consuming_sequence() {
    let failures = [
        IndexedHistoryRead::Value(PropertyValue::Unsigned(7)),
        IndexedHistoryRead::Missing,
        IndexedHistoryRead::Unreadable,
    ];

    for failure in failures {
        let messages = std::array::from_fn(|index| {
            if index == 0 {
                failure.clone()
            } else {
                empty_message_read()
            }
        });
        let (db, sent) = commit_and_capture_history_notification(
            repeated_timestamp_reads(BACnetTimeStamp::SequenceNumber(17)),
            messages,
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            false,
        )
        .await;

        assert!(
            sent.is_empty(),
            "invalid committed message history must fail closed"
        );
        assert_eq!(
            db.write().await.reserve_event_sequence_number().number(),
            1,
            "a successful commit permanently consumes its staged sequence"
        );
    }
}
