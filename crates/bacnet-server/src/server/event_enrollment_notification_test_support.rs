use super::super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, EventType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;
use bytes::Bytes;
use std::borrow::Cow;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
pub(super) struct RecordingTransport {
    pub(super) sent: StdArc<StdMutex<Vec<Bytes>>>,
    pub(super) lock_probe: StdArc<StdMutex<Option<StdArc<tokio::sync::RwLock<ObjectDatabase>>>>>,
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let database = self.lock_probe.lock().unwrap().clone();
        if let Some(database) = database {
            assert!(
                database.try_read().is_ok(),
                "Event Enrollment database guard must be released before network I/O"
            );
        }
        self.sent.lock().unwrap().push(Bytes::copy_from_slice(npdu));
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xBA, 0xC0]
    }
}

pub(super) struct ObservedObject {
    oid: ObjectIdentifier,
    name: String,
    value: PropertyValue,
    reliability: PropertyValue,
    status_flags: u8,
}

impl ObservedObject {
    pub(super) fn new(instance: u32, value: PropertyValue) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            name: format!("observed-{instance}"),
            value,
            reliability: PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
            status_flags: 0,
        }
    }

    pub(super) fn with_reliability(mut self, reliability: Reliability) -> Self {
        self.reliability = PropertyValue::Enumerated(reliability.to_raw());
        self
    }

    pub(super) fn with_reliability_value(mut self, reliability: PropertyValue) -> Self {
        self.reliability = reliability;
        self
    }

    pub(super) fn with_status_flags(mut self, status_flags: u8) -> Self {
        self.status_flags = status_flags;
        self
    }
}

impl BACnetObject for ObservedObject {
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
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => Ok(self.value.clone()),
            p if p == PropertyIdentifier::RELIABILITY => Ok(self.reliability.clone()),
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags],
            }),
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        match (property, value) {
            (p, value) if p == PropertyIdentifier::PRESENT_VALUE => self.value = value,
            (p, value) if p == PropertyIdentifier::RELIABILITY => self.reliability = value,
            (p, PropertyValue::BitString { data, .. })
                if p == PropertyIdentifier::STATUS_FLAGS && data.len() == 1 =>
            {
                self.status_flags = data[0];
            }
            _ => {
                return Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
                });
            }
        }
        Ok(())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATUS_FLAGS,
        ])
    }
}

pub(super) fn enrollment(
    instance: u32,
    event_type: EventType,
    target: Option<ObjectIdentifier>,
    parameters: BACnetEventParameter,
) -> EventEnrollmentObject {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("enrollment-{instance}"),
        event_type.to_raw(),
    )
    .unwrap();
    enrollment.set_object_property_reference(target.map(|oid| {
        BACnetDeviceObjectPropertyReference::new_local(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )
    }));
    enrollment.set_event_parameters(parameters);
    enrollment.set_event_enable(0x07);
    enrollment
}

pub(super) fn out_of_range_parameters(time_delay: u32) -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    }
}

pub(super) fn add_server_context(db: &mut ObjectDatabase, recipients: bool) {
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Event Enrollment Device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    let mut notification_class = NotificationClass::new(0, "NC-0").unwrap();
    notification_class.ack_required = [true; 3];
    if recipients {
        notification_class.add_destination(
            crate::server::event_notifications_tests::local_broadcast_destination(),
        );
    }
    db.add(Box::new(notification_class)).unwrap();
}

pub(super) async fn start_server(
    mut db: ObjectDatabase,
    recipients: bool,
) -> (
    BACnetServer<RecordingTransport>,
    StdArc<StdMutex<Vec<Bytes>>>,
) {
    add_server_context(&mut db, recipients);
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let lock_probe = StdArc::new(StdMutex::new(None));
    let server = BACnetServer::start_clockless(
        ServerConfig {
            event_enrollment_interval_secs: 1,
            ..ServerConfig::default()
        },
        db,
        RecordingTransport {
            sent: StdArc::clone(&sent),
            lock_probe: StdArc::clone(&lock_probe),
        },
    )
    .await
    .unwrap();
    *lock_probe.lock().unwrap() = Some(StdArc::clone(server.database()));
    (server, sent)
}

pub(super) fn drain_notifications(sent: &StdMutex<Vec<Bytes>>) -> Vec<EventNotificationRequest> {
    std::mem::take(&mut *sent.lock().unwrap())
        .into_iter()
        .map(|frame| {
            let npdu = decode_npdu(frame).expect("decode notification NPDU");
            let Apdu::UnconfirmedRequest(request) =
                decode_apdu(npdu.payload).expect("decode notification APDU")
            else {
                panic!("expected unconfirmed EventNotification");
            };
            assert_eq!(
                request.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
            EventNotificationRequest::decode(&request.service_request)
                .expect("decode EventNotification service request")
        })
        .collect()
}

pub(super) fn history_timestamp(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    to_state: EventState,
) -> BACnetTimeStamp {
    let index = if to_state == EventState::FAULT {
        2
    } else if to_state == EventState::NORMAL {
        3
    } else {
        1
    };
    let PropertyValue::ApplicationData(bytes) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(index))
        .unwrap()
    else {
        panic!("Event_Time_Stamps coordinate must be application data");
    };
    let (timestamp, consumed) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(consumed, bytes.len());
    timestamp
}

pub(super) fn assert_committed_reliability_notifications(
    db: &ObjectDatabase,
    notifications: &[EventNotificationRequest],
    expected_instances: &[u32],
    from: EventState,
    to: EventState,
    first_sequence: u16,
) {
    assert_eq!(notifications.len(), expected_instances.len());
    assert_eq!(
        notifications
            .iter()
            .map(|notification| notification.event_object_identifier.instance_number())
            .collect::<Vec<_>>(),
        expected_instances
    );
    for (offset, notification) in notifications.iter().enumerate() {
        assert_eq!(
            notification.event_type,
            EventType::CHANGE_OF_RELIABILITY.to_raw()
        );
        assert_eq!(notification.from_state, from.to_raw());
        assert_eq!(notification.to_state, to.to_raw());
        assert!(notification.ack_required);
        assert_eq!(notification.message_text, None);
        assert_eq!(notification.event_values, None);
        assert_eq!(
            notification.timestamp,
            BACnetTimeStamp::SequenceNumber(first_sequence + offset as u16)
        );
        assert_eq!(
            notification.timestamp,
            history_timestamp(db, notification.event_object_identifier, to)
        );
    }
}

pub(super) fn event_state(db: &ObjectDatabase, oid: ObjectIdentifier) -> EventState {
    let PropertyValue::Enumerated(raw) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
    else {
        panic!("Event_State must be Enumerated");
    };
    EventState::from_raw(raw)
}
