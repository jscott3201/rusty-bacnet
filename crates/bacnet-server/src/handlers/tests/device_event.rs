use super::*;
use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_types::primitives::{Date, Time};

struct EventSummaryFixture {
    oid: ObjectIdentifier,
    name: String,
    timestamps: [PropertyValue; 3],
}

impl BACnetObject for EventSummaryFixture {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                Ok(PropertyValue::Boolean(true))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()))
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(0)),
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xa0],
            }),
            p if p == PropertyIdentifier::NOTIFY_TYPE => Ok(PropertyValue::Enumerated(1)),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x40],
            }),
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => {
                Ok(PropertyValue::List(self.timestamps.to_vec()))
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
        static PROPERTIES: &[PropertyIdentifier] = &[
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
        ];
        std::borrow::Cow::Borrowed(PROPERTIES)
    }
}

fn event_summary_fixture(instance: u32, timestamps: [u64; 3]) -> EventSummaryFixture {
    event_summary_fixture_with_values(instance, timestamps.map(PropertyValue::Unsigned))
}

fn event_summary_fixture_with_values(
    instance: u32,
    timestamps: [PropertyValue; 3],
) -> EventSummaryFixture {
    EventSummaryFixture {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        name: format!("AI-{instance}"),
        timestamps,
    }
}

fn timestamp_value(timestamp: &BACnetTimeStamp) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_timestamp_choice(&mut encoded, timestamp).unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

fn get_event_information_ack(
    db: &ObjectDatabase,
    cursor: Option<ObjectIdentifier>,
) -> GetEventInformationAck {
    let request = GetEventInformationRequest {
        last_received_object_identifier: cursor,
    };
    let mut request_buf = BytesMut::new();
    request.encode(&mut request_buf);
    let mut ack_buf = BytesMut::new();
    handle_get_event_information(db, &request_buf, &mut ack_buf).unwrap();
    GetEventInformationAck::decode(&ack_buf).unwrap()
}

#[test]
fn device_communication_control_handler() {
    let comm_state = AtomicU8::new(0);

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: Some(60),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(duration, Some(60));
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn device_communication_control_enable() {
    let comm_state = AtomicU8::new(1); // start disabled

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::ENABLE);
    assert_eq!(duration, None);
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}

#[test]
fn device_communication_control_disable_initiation() {
    let comm_state = AtomicU8::new(0);

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(duration, None);
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn reinitialize_device_handler() {
    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    handle_reinitialize_device(&buf, &None).unwrap();
}

#[test]
fn get_event_information_empty() {
    let db = make_db_with_ai();
    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    assert!(!ack_bytes.is_empty());
}

#[test]
fn get_event_information_reports_non_normal_objects() {
    use bacnet_objects::event::LimitEnable;

    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // Configure intrinsic reporting
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    // Push value above high limit and evaluate
    ai.set_present_value(85.0);
    ai.evaluate_intrinsic_reporting(); // → HIGH_LIMIT
    db.add(Box::new(ai)).unwrap();

    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    // The ACK should contain one event summary for AI-1
    assert!(ack_bytes.len() > 5); // non-trivial response
}

#[test]
fn get_event_information_reads_event_enable_notify_type_and_priorities() {
    use bacnet_objects::event::LimitEnable;
    use bacnet_objects::notification_class::NotificationClass;

    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    // Set EVENT_ENABLE to 0x05 (TO_OFFNORMAL + TO_NORMAL, not TO_FAULT)
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0], // TO_OFFNORMAL|TO_NORMAL, MSB-first
        },
        None,
    )
    .unwrap();
    // Set NOTIFY_TYPE to 1 (EVENT, not ALARM)
    ai.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(7),
        None,
    )
    .unwrap();
    // Push above high limit and evaluate to trigger alarm
    ai.set_present_value(85.0);
    ai.evaluate_intrinsic_reporting();
    db.add(Box::new(ai)).unwrap();

    // Add NotificationClass object with custom priorities
    let mut nc = NotificationClass::new(5, "NC-5").unwrap();
    nc.notification_class = 7;
    nc.priority = [100, 150, 200];
    db.add(Box::new(nc)).unwrap();

    let request = bacnet_services::alarm_event::GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
    let ack = GetEventInformationAck::decode(&ack_buf).unwrap();
    let summary = &ack.list_of_event_summaries[0];
    assert_eq!(summary.event_enable, 0x05);
    assert_eq!(summary.notify_type, 1);
    assert_eq!(summary.event_priorities, [100, 150, 200]);
}

#[test]
fn get_event_information_forwards_valid_sequence_timestamps_and_defaults_invalid_array() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(event_summary_fixture(1, [11, 65535, 33])))
        .unwrap();
    db.add(Box::new(event_summary_fixture(2, [65536, 44, 55])))
        .unwrap();
    let mut trailing_data = timestamp_value(&BACnetTimeStamp::SequenceNumber(66));
    let PropertyValue::ApplicationData(encoded) = &mut trailing_data else {
        unreachable!("timestamp helper must return ApplicationData")
    };
    encoded.push(0);
    db.add(Box::new(event_summary_fixture_with_values(
        3,
        [
            trailing_data,
            timestamp_value(&BACnetTimeStamp::SequenceNumber(77)),
            timestamp_value(&BACnetTimeStamp::SequenceNumber(88)),
        ],
    )))
    .unwrap();

    let ack = get_event_information_ack(&db, None);
    assert_eq!(
        ack.list_of_event_summaries[0].event_timestamps,
        [
            BACnetTimeStamp::SequenceNumber(11),
            BACnetTimeStamp::SequenceNumber(65535),
            BACnetTimeStamp::SequenceNumber(33),
        ]
    );
    assert_eq!(
        ack.list_of_event_summaries[1].event_timestamps,
        [
            BACnetTimeStamp::SequenceNumber(0),
            BACnetTimeStamp::SequenceNumber(0),
            BACnetTimeStamp::SequenceNumber(0),
        ]
    );
    assert_eq!(
        ack.list_of_event_summaries[2].event_timestamps,
        [
            BACnetTimeStamp::SequenceNumber(0),
            BACnetTimeStamp::SequenceNumber(0),
            BACnetTimeStamp::SequenceNumber(0),
        ]
    );
}

#[test]
fn get_event_information_preserves_timestamp_choices() {
    let expected = [
        BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        }),
        BACnetTimeStamp::SequenceNumber(22),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 7,
                day: 30,
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
    let mut db = ObjectDatabase::new();
    db.add(Box::new(event_summary_fixture_with_values(
        1,
        expected
            .clone()
            .map(|timestamp| timestamp_value(&timestamp)),
    )))
    .unwrap();

    let ack = get_event_information_ack(&db, None);
    assert_eq!(ack.list_of_event_summaries[0].event_timestamps, expected);
}

#[test]
fn get_event_information_paginates_by_object_identifier_after_cursor_removal() {
    let mut db = ObjectDatabase::new();
    for instance in (1..=27).rev() {
        db.add(Box::new(event_summary_fixture(instance, [0; 3])))
            .unwrap();
    }

    let first_page = get_event_information_ack(&db, None);
    let first_instances: Vec<_> = first_page
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert_eq!(first_instances, (1..=25).collect::<Vec<_>>());
    assert!(first_page.more_events);

    let cursor = first_page.list_of_event_summaries[24].object_identifier;
    db.remove(&cursor).unwrap();
    let second_page = get_event_information_ack(&db, Some(cursor));
    let second_instances: Vec<_> = second_page
        .list_of_event_summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect();
    assert_eq!(second_instances, vec![26, 27]);
    assert!(!second_page.more_events);
}
