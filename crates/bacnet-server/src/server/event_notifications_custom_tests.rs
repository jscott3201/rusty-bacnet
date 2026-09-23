//! A custom implementation using only public object-contract types.
use super::*;
use bacnet_objects::event::{EventTransitionCommit, EventTransitionCommitError, TransitionOutcome};
use bacnet_services::alarm_event::NotificationParameters;
use std::borrow::Cow;

struct CustomProposal {
    oid: ObjectIdentifier,
    state: EventState,
    delay: u32,
    remaining: Option<u32>,
    acknowledged: u8,
    timestamps: [BACnetTimeStamp; 3],
    messages: [String; 3],
    mode: Arc<AtomicU8>,
    commits: Arc<StdMutex<Vec<EventTransitionCommit>>>,
    distribute: bool,
}
impl CustomProposal {
    fn proposal(&self) -> Option<TransitionOutcome> {
        (self.state == EventState::NORMAL).then(|| TransitionOutcome {
            change: EventStateChange {
                from: self.state,
                to: EventState::HIGH_LIMIT,
            },
            event_type: EventType::OUT_OF_RANGE,
            distribute: self.distribute,
        })
    }
}
impl BACnetObject for CustomProposal {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "Custom proposal"
    }
    fn read_property(
        &self,
        p: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let value = match p {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                PropertyValue::ObjectIdentifier(self.oid)
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                PropertyValue::CharacterString(self.object_name().into())
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                PropertyValue::Enumerated(ObjectType::ANALOG_INPUT.to_raw())
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                PropertyValue::Enumerated(self.state.to_raw())
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => PropertyValue::Unsigned(0),
            p if p == PropertyIdentifier::NOTIFY_TYPE => {
                PropertyValue::Enumerated(NotifyType::ALARM.to_raw())
            }
            p if p == PropertyIdentifier::EVENT_TYPE => {
                PropertyValue::Enumerated(EventType::OUT_OF_RANGE.to_raw())
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => PropertyValue::Real(81.0),
            p if p == PropertyIdentifier::HIGH_LIMIT => PropertyValue::Real(80.0),
            p if p == PropertyIdentifier::LOW_LIMIT => PropertyValue::Real(20.0),
            p if p == PropertyIdentifier::DEADBAND => PropertyValue::Real(2.0),
            p if p == PropertyIdentifier::RELIABILITY => PropertyValue::Enumerated(0),
            p if p == PropertyIdentifier::STATUS_FLAGS => PropertyValue::BitString {
                unused_bits: 4,
                data: vec![if self.state == EventState::NORMAL {
                    0
                } else {
                    0x80
                }],
            },
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.acknowledged.reverse_bits()],
            },
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS
                || p == PropertyIdentifier::EVENT_MESSAGE_TEXTS =>
            {
                let Some(index @ 1..=3) = index else {
                    return Err(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw().into(),
                        code: ErrorCode::INVALID_ARRAY_INDEX.to_raw().into(),
                    });
                };
                let index = index as usize - 1;
                if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS {
                    PropertyValue::CharacterString(self.messages[index].clone())
                } else {
                    let mut bytes = BytesMut::new();
                    bacnet_encoding::primitives::encode_timestamp_choice(
                        &mut bytes,
                        &self.timestamps[index],
                    )
                    .unwrap();
                    PropertyValue::ApplicationData(bytes.to_vec())
                }
            }
            _ => {
                return Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw().into(),
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw().into(),
                })
            }
        };
        Ok(value)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw().into(),
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw().into(),
        })
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        if self.delay == 0 {
            return self.proposal();
        }
        if self.state == EventState::NORMAL {
            self.remaining.get_or_insert(self.delay);
        }
        None
    }
    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        let remaining = self.remaining.as_mut()?;
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            self.proposal()
        } else {
            None
        }
    }
    fn commit_event_transition_internal(
        &mut self,
        commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        if self.mode.load(Ordering::SeqCst) == 0 {
            return Err(EventTransitionCommitError::Unsupported);
        }
        if commit.coordinate != commit.change.transition() {
            return Err(EventTransitionCommitError::CoordinateTargetMismatch {
                coordinate: commit.coordinate,
                target: commit.change.to,
            });
        }
        if self.state != commit.change.from || self.mode.load(Ordering::SeqCst) == 2 {
            return Err(EventTransitionCommitError::CurrentStateMismatch {
                expected: commit.change.from,
                actual: self.state,
            });
        }
        let index = commit.coordinate.index();
        self.state = commit.change.to;
        if commit.ack_required {
            self.acknowledged &= !commit.coordinate.bit_mask();
        } else {
            self.acknowledged |= commit.coordinate.bit_mask();
        }
        self.timestamps[index] = commit.timestamp.clone();
        if let Some(message) = &commit.message_text {
            self.messages[index] = message.clone();
        }
        self.remaining = None;
        self.commits.lock().unwrap().push(commit);
        Ok(())
    }
}

// This object deliberately inherits the public trait's unsupported commit hook.
struct UnsupportedProposal(CustomProposal);
impl BACnetObject for UnsupportedProposal {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.0.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.0.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(p, i, v, priority)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.0.property_list()
    }
    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.0.evaluate_intrinsic_reporting()
    }
    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.0.tick_intrinsic_reporting()
    }
}

fn database(
    delay: u32,
    mode: Arc<AtomicU8>,
    unsupported: bool,
    distribute: bool,
) -> (ObjectDatabase, Arc<StdMutex<Vec<EventTransitionCommit>>>) {
    let commits = Arc::new(StdMutex::new(Vec::new()));
    let object = CustomProposal {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        state: EventState::NORMAL,
        delay,
        remaining: None,
        acknowledged: 7,
        timestamps: std::array::from_fn(|_| BACnetTimeStamp::SequenceNumber(99)),
        messages: std::array::from_fn(|_| "initial".into()),
        mode,
        commits: commits.clone(),
        distribute,
    };
    let mut db = ObjectDatabase::new();
    db.add(if unsupported {
        Box::new(UnsupportedProposal(object))
    } else {
        Box::new(object)
    })
    .unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Custom Device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut class = notification_class_0_broadcasting();
    class.ack_required = [true, false, false];
    db.add(Box::new(class)).unwrap();
    (db, commits)
}
fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}
fn snapshot(db: &ObjectDatabase) -> Vec<PropertyValue> {
    let object = db.get(&oid()).unwrap();
    let mut values = vec![
        object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
    ];
    for p in [
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
    ] {
        for i in 1..=3 {
            values.push(object.read_property(p, Some(i)).unwrap());
        }
    }
    values
}
fn assert_committed(
    db: &ObjectDatabase,
    notification: &EventNotificationRequest,
    commits: &[EventTransitionCommit],
) {
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(
        commit.change,
        EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT
        }
    );
    assert_eq!(commit.timestamp, BACnetTimeStamp::SequenceNumber(0));
    assert_eq!(notification.timestamp, commit.timestamp);
    assert_eq!(
        commit.message_text.as_deref(),
        Some("ANALOG_INPUT,1: NORMAL -> HIGH_LIMIT")
    );
    assert_eq!(notification.message_text, commit.message_text);
    assert_eq!(notification.ack_required, commit.ack_required);
    assert!(notification.ack_required);
    assert_eq!(notification.from_state, EventState::NORMAL.to_raw());
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT.to_raw());
    assert_eq!(
        notification.event_values,
        Some(NotificationParameters::OutOfRange {
            exceeding_value: 81.0,
            status_flags: 8,
            deadband: 2.0,
            exceeded_limit: 80.0
        })
    );
    let state = snapshot(db);
    assert_eq!(
        state[0],
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
    assert_eq!(
        state[1],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x60]
        }
    );
    assert_eq!(
        state[5],
        PropertyValue::CharacterString(commit.message_text.clone().unwrap())
    );
    for (index, timestamp) in [0, 99, 99].into_iter().enumerate() {
        let mut bytes = BytesMut::new();
        bacnet_encoding::primitives::encode_timestamp_choice(
            &mut bytes,
            &BACnetTimeStamp::SequenceNumber(timestamp),
        )
        .unwrap();
        assert_eq!(
            state[index + 2],
            PropertyValue::ApplicationData(bytes.to_vec())
        );
    }
    assert_eq!(state[6], PropertyValue::CharacterString("initial".into()));
    assert_eq!(state[7], PropertyValue::CharacterString("initial".into()));
    assert_eq!(db.reserve_event_sequence_number().number(), 1);
}

#[tokio::test]
async fn unsupported_custom_proposal_runtime_is_silent_unchanged_and_retryable() {
    let (db, commits) = database(0, Arc::new(AtomicU8::new(1)), true, true);
    let before = snapshot(&db);
    let db = Arc::new(RwLock::new(db));
    for _ in 0..2 {
        let proposal = db
            .write()
            .await
            .get_mut(&oid())
            .unwrap()
            .evaluate_intrinsic_reporting()
            .unwrap();
        let sent = broadcasts_from_per_write_path(&db, 0).await;
        assert!(
            sent.is_empty(),
            "unsupported proposal must never be distributed"
        );
        let mut guard = db.write().await;
        assert_eq!(snapshot(&guard), before);
        assert_eq!(guard.reserve_event_sequence_number().number(), 0);
        assert_eq!(
            guard
                .get_mut(&oid())
                .unwrap()
                .evaluate_intrinsic_reporting(),
            Some(proposal)
        );
        assert!(commits.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn failed_custom_commit_retries_through_per_write_runtime() {
    let mode = Arc::new(AtomicU8::new(2));
    let (db, commits) = database(0, mode.clone(), false, true);
    let before = snapshot(&db);
    let db = Arc::new(RwLock::new(db));
    assert!(broadcasts_from_per_write_path(&db, 0).await.is_empty());
    assert_eq!(snapshot(&*db.read().await), before);
    assert_eq!(db.read().await.reserve_event_sequence_number().number(), 0);
    assert!(commits.lock().unwrap().is_empty());
    mode.store(1, Ordering::SeqCst);
    let sent = broadcasts_from_per_write_path(&db, 0).await;
    let notification = decode_broadcast_notification(&StdMutex::new(sent));
    assert_committed(&*db.read().await, &notification, &commits.lock().unwrap());
}

#[tokio::test]
async fn custom_immediate_proposal_commits_exact_history_before_distribution() {
    let (db, commits) = database(0, Arc::new(AtomicU8::new(1)), false, true);
    let db = Arc::new(RwLock::new(db));
    let sent = broadcasts_from_per_write_path(&db, 0).await;
    let notification = decode_broadcast_notification(&StdMutex::new(sent));
    assert_committed(&*db.read().await, &notification, &commits.lock().unwrap());
    assert!(broadcasts_from_per_write_path(&db, 0).await.is_empty());
    assert_eq!(commits.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn custom_delayed_proposal_commits_exact_history_before_distribution() {
    delayed_runtime(false, false).await;
}

#[tokio::test(start_paused = true)]
async fn failed_custom_delayed_commit_is_unchanged_and_retries() {
    delayed_runtime(true, false).await;
}

#[tokio::test(start_paused = true)]
async fn unsupported_custom_delayed_proposal_is_silent_and_retryable() {
    delayed_runtime(false, true).await;
}

async fn delayed_runtime(fail_first: bool, unsupported: bool) {
    let mode = Arc::new(AtomicU8::new(if fail_first { 2 } else { 1 }));
    let (mut db, commits) = database(2, mode.clone(), unsupported, true);
    let before = snapshot(&db);
    assert!(db
        .get_mut(&oid())
        .unwrap()
        .evaluate_intrinsic_reporting()
        .is_none());
    assert_eq!(snapshot(&db), before);
    let sent = Arc::new(StdMutex::new(Vec::new()));
    let transport = RecordingTransport {
        sent_broadcast: sent.clone(),
        local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
    };
    let mut server = BACnetServer::start_clockless(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert!(
        sent.lock().unwrap().is_empty(),
        "delay must not fire immediately"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    if fail_first || unsupported {
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(snapshot(&*server.database().read().await), before);
        assert_eq!(
            server
                .database()
                .read()
                .await
                .reserve_event_sequence_number()
                .number(),
            0
        );
        assert!(commits.lock().unwrap().is_empty());
        if unsupported {
            let proposal = server
                .database()
                .write()
                .await
                .get_mut(&oid())
                .unwrap()
                .tick_intrinsic_reporting()
                .unwrap();
            assert_eq!(proposal.change.from, EventState::NORMAL);
            assert_eq!(proposal.change.to, EventState::HIGH_LIMIT);
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(sent.lock().unwrap().is_empty());
            assert_eq!(snapshot(&*server.database().read().await), before);
            server.stop().await.unwrap();
            return;
        }
        mode.store(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let notification = decode_broadcast_notification(&sent);
    assert_committed(
        &*server.database().read().await,
        &notification,
        &commits.lock().unwrap(),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(commits.lock().unwrap().len(), 1);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn custom_local_commit_survives_distribution_gates() {
    for (dcc, recipient, distribute) in [(1, true, true), (0, false, true), (0, true, false)] {
        let (mut db, commits) = database(0, Arc::new(AtomicU8::new(1)), false, distribute);
        if !recipient {
            db.get_mut(&ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 0).unwrap())
                .unwrap()
                .write_property(
                    PropertyIdentifier::RECIPIENT_LIST,
                    None,
                    PropertyValue::ApplicationData(Vec::new()),
                    None,
                )
                .unwrap();
        }
        let db = Arc::new(RwLock::new(db));
        assert!(broadcasts_from_per_write_path(&db, dcc).await.is_empty());
        let state = snapshot(&*db.read().await);
        assert_eq!(
            state[0],
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );
        assert_eq!(commits.lock().unwrap().len(), 1);
        assert_eq!(db.read().await.reserve_event_sequence_number().number(), 1);
    }
}
