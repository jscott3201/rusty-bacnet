//! The event timestamp on the server's send path (the timestamps themselves
//! are `bacnet_device_core::event`'s).

use bacnet_encoding::constructed::decode_event_notification;
use std::sync::Arc;

use bacnet_encoding::apdu::{Apdu, decode_apdu};
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

use super::event_notifications_tests::{local_broadcast_destination, recording_transport};
use super::{BACnetServer, LearnedRouterCache, NotificationTransactions};
use crate::server::test_transport::TestTransport;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::BACnetTimeStamp;

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

fn outbound_database(frame: Option<ClockFrame>, recipients: usize) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    if let Some(frame) = frame {
        db.set_clock_reader(Some(Arc::new(FixedClock(frame))));
    }
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default()
                .with_name("Timestamp-Policy-Device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    let mut notification_class = NotificationClass::new(0, "NC-0").unwrap();
    for _ in 0..recipients {
        notification_class
            .add_destination(local_broadcast_destination())
            .unwrap();
    }
    db.add(Box::new(notification_class)).unwrap();
    db
}

fn decode_notification(frame: &Bytes) -> EventNotificationRequest {
    let npdu = decode_npdu(frame.clone()).expect("decode NPDU");
    let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).expect("decode APDU") else {
        panic!("expected unconfirmed EventNotification");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
    );
    decode_event_notification(&request.service_request).expect("decode EventNotification")
}

async fn send_event(db: &Arc<RwLock<ObjectDatabase>>, network: &Arc<NetworkLayer<TestTransport>>) {
    BACnetServer::<TestTransport>::build_and_send_event_notification_with_bindings(
        &crate::server::event_delivery::EventDelivery {
            db,
            network,
            comm_state: &Arc::default(),
            learned_routers: &Arc::new(Mutex::new(LearnedRouterCache::new())),
            notification_transactions: &NotificationTransactions::new(),
            device_bindings: &Arc::new(RwLock::new(
                crate::server::device_bindings::DeviceBindingTable::new(),
            )),
            suppressions: &Default::default(),
            retry_timeout_ms: 1000,
            local_apdu_capacity: 1476,
            device: crate::server::event_delivery::fixture_device(db).await,
        },
        &bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        (
            EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            EventType::OUT_OF_RANGE,
        ),
    )
    .await;
}

#[tokio::test]
async fn outbound_event_uses_exact_sampled_device_datetime() {
    let frame = fixed_frame();
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let db = Arc::new(RwLock::new(outbound_database(Some(frame), 1)));

    send_event(&db, &network).await;

    let sent = sent.npdus();
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
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let db = Arc::new(RwLock::new(outbound_database(Some(frame), 1)));

    send_event(&db, &network).await;

    let sent = sent.npdus();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        decode_notification(&sent[0]).timestamp,
        BACnetTimeStamp::SequenceNumber(0)
    );
}

#[tokio::test]
async fn clockless_event_sequence_is_shared_by_all_recipients_and_increments_per_event() {
    let (transport, sent) = recording_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let db = Arc::new(RwLock::new(outbound_database(None, 2)));

    send_event(&db, &network).await;
    {
        let first = sent.npdus();
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|frame| {
            decode_notification(frame).timestamp == BACnetTimeStamp::SequenceNumber(0)
        }));
    }

    sent.clear();
    send_event(&db, &network).await;
    let second = sent.npdus();
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|frame| {
        decode_notification(frame).timestamp == BACnetTimeStamp::SequenceNumber(1)
    }));
}
