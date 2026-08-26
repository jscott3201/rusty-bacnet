use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::primitives::BACnetTimeStamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SampledEventClock {
    Valid(ClockFrame),
    Unavailable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EventTimestampSample {
    pub(super) timestamp: BACnetTimeStamp,
    pub(super) clock: SampledEventClock,
}

/// Sample the Device clock once and select one valid EventNotification
/// timestamp. A missing or malformed wall clock consumes the database-local
/// sequence source instead of inventing a DateTime.
pub(super) fn sample_event_timestamp(db: &mut ObjectDatabase) -> EventTimestampSample {
    match db.clock_frame() {
        Some(frame) if frame.is_valid_actual_datetime() => EventTimestampSample {
            timestamp: BACnetTimeStamp::DateTime {
                date: frame.local_date,
                time: frame.local_time,
            },
            clock: SampledEventClock::Valid(frame),
        },
        Some(_) => EventTimestampSample {
            timestamp: BACnetTimeStamp::SequenceNumber(db.next_event_sequence_number()),
            clock: SampledEventClock::Invalid,
        },
        None => EventTimestampSample {
            timestamp: BACnetTimeStamp::SequenceNumber(db.next_event_sequence_number()),
            clock: SampledEventClock::Unavailable,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use bacnet_encoding::apdu::{decode_apdu, Apdu};
    use bacnet_encoding::npdu::decode_npdu;
    use bacnet_network::layer::NetworkLayer;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::clock::ClockReader;
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::event::EventStateChange;
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_services::alarm_event::EventNotificationRequest;
    use bacnet_types::enums::{EventState, EventType, ObjectType, UnconfirmedServiceChoice};
    use bacnet_types::primitives::{Date, Time};
    use bytes::Bytes;
    use tokio::sync::{Mutex, RwLock};

    use super::super::event_notifications_tests::{
        local_broadcast_destination, RecordingTransport,
    };
    use super::super::{BACnetServer, NotificationTransactions, ServerTsm};
    use super::*;

    struct FixedClock(ClockFrame);

    impl ClockReader for FixedClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            Some(self.0)
        }
    }

    fn fixed_frame() -> ClockFrame {
        ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
            utc_offset: 300,
            daylight_savings_status: true,
        }
    }

    #[test]
    fn valid_clock_frame_selects_exact_local_datetime() {
        let frame = fixed_frame();
        let mut db = ObjectDatabase::new();
        db.set_clock_reader(Some(Arc::new(FixedClock(frame))));

        assert_eq!(
            sample_event_timestamp(&mut db),
            EventTimestampSample {
                timestamp: BACnetTimeStamp::DateTime {
                    date: frame.local_date,
                    time: frame.local_time,
                },
                clock: SampledEventClock::Valid(frame),
            }
        );
    }

    #[test]
    fn clockless_source_increments_independently_per_database() {
        let mut first = ObjectDatabase::new();
        let mut second = ObjectDatabase::new();

        assert_eq!(
            sample_event_timestamp(&mut first).timestamp,
            BACnetTimeStamp::SequenceNumber(0)
        );
        assert_eq!(
            sample_event_timestamp(&mut first).timestamp,
            BACnetTimeStamp::SequenceNumber(1)
        );
        assert_eq!(
            sample_event_timestamp(&mut second).timestamp,
            BACnetTimeStamp::SequenceNumber(0)
        );
    }

    #[test]
    fn malformed_clock_frame_falls_back_to_sequence_and_is_flagged() {
        let mut frame = fixed_frame();
        frame.local_time.hour = Time::UNSPECIFIED;
        let mut db = ObjectDatabase::new();
        db.set_clock_reader(Some(Arc::new(FixedClock(frame))));

        assert_eq!(
            sample_event_timestamp(&mut db),
            EventTimestampSample {
                timestamp: BACnetTimeStamp::SequenceNumber(0),
                clock: SampledEventClock::Invalid,
            }
        );
    }

    fn outbound_database(frame: Option<ClockFrame>, recipients: usize) -> ObjectDatabase {
        let mut db = ObjectDatabase::new();
        if let Some(frame) = frame {
            db.set_clock_reader(Some(Arc::new(FixedClock(frame))));
        }
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 1,
                name: "Timestamp-Policy-Device".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        db.add(Box::new(AnalogInputObject::new(1, "AI-1", 0).unwrap()))
            .unwrap();
        let mut notification_class = NotificationClass::new(0, "NC-0").unwrap();
        for _ in 0..recipients {
            notification_class.add_destination(local_broadcast_destination());
        }
        db.add(Box::new(notification_class)).unwrap();
        db
    }

    fn decode_notification(frame: &Bytes) -> EventNotificationRequest {
        let npdu = decode_npdu(frame.clone()).expect("decode NPDU");
        let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).expect("decode APDU")
        else {
            panic!("expected unconfirmed EventNotification");
        };
        assert_eq!(
            request.service_choice,
            UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
        );
        EventNotificationRequest::decode(&request.service_request)
            .expect("decode EventNotification")
    }

    async fn send_event(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<RecordingTransport>>,
    ) {
        BACnetServer::<RecordingTransport>::build_and_send_event_notification(
            db,
            network,
            &Arc::new(std::sync::atomic::AtomicU8::new(0)),
            &Arc::new(Mutex::new(ServerTsm::new())),
            &NotificationTransactions::new(),
            &bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            (
                EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::HIGH_LIMIT,
                },
                EventType::OUT_OF_RANGE,
            ),
            1000,
        )
        .await;
    }

    #[tokio::test]
    async fn outbound_event_uses_exact_sampled_device_datetime() {
        let frame = fixed_frame();
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let network = Arc::new(NetworkLayer::new(RecordingTransport {
            sent_broadcast: Arc::clone(&sent),
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        }));
        let db = Arc::new(RwLock::new(outbound_database(Some(frame), 1)));

        send_event(&db, &network).await;

        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            decode_notification(&sent[0]).timestamp,
            BACnetTimeStamp::DateTime {
                date: frame.local_date,
                time: frame.local_time,
            }
        );
    }

    #[tokio::test]
    async fn outbound_event_with_invalid_clock_uses_sequence_without_suppressing_delivery() {
        let mut frame = fixed_frame();
        frame.local_time.hour = Time::UNSPECIFIED;
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let network = Arc::new(NetworkLayer::new(RecordingTransport {
            sent_broadcast: Arc::clone(&sent),
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        }));
        let db = Arc::new(RwLock::new(outbound_database(Some(frame), 1)));

        send_event(&db, &network).await;

        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            decode_notification(&sent[0]).timestamp,
            BACnetTimeStamp::SequenceNumber(0)
        );
    }

    #[tokio::test]
    async fn clockless_event_sequence_is_shared_by_all_recipients_and_increments_per_event() {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let network = Arc::new(NetworkLayer::new(RecordingTransport {
            sent_broadcast: Arc::clone(&sent),
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        }));
        let db = Arc::new(RwLock::new(outbound_database(None, 2)));

        send_event(&db, &network).await;
        {
            let first = sent.lock().unwrap();
            assert_eq!(first.len(), 2);
            assert!(first.iter().all(|frame| {
                decode_notification(frame).timestamp == BACnetTimeStamp::SequenceNumber(0)
            }));
        }

        sent.lock().unwrap().clear();
        send_event(&db, &network).await;
        let second = sent.lock().unwrap();
        assert_eq!(second.len(), 2);
        assert!(second.iter().all(|frame| {
            decode_notification(frame).timestamp == BACnetTimeStamp::SequenceNumber(1)
        }));
    }
}
