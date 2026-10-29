use super::super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendMode, SentFrame, TestTransport};
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::decode_bacnet_property_value;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::{EventNotificationRequest, NotificationParameters};
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, FaultParameters,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, EventType, Reliability};
use bacnet_types::primitives::{BACnetTimeStamp, StatusFlags};
use bytes::Bytes;
use std::borrow::Cow;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

/// Captures the broadcast EventNotifications a [`TestTransport`] sends and
/// ignores unicasts. While `lock_probe` names a database, each broadcast first
/// proves that database's guard is released, and only then is recorded.
#[derive(Clone, Default)]
pub(super) struct NotificationCapture {
    pub(super) sent: StdArc<StdMutex<Vec<Bytes>>>,
    pub(super) lock_probe: StdArc<StdMutex<Option<StdArc<tokio::sync::RwLock<ObjectDatabase>>>>>,
}

impl NotificationCapture {
    pub(super) fn transport(&self) -> TestTransport {
        let capture = self.clone();
        TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .unicast(SendMode::Ignore)
            .on_send(move |frame| capture.clone().record(frame))
            .build()
    }

    async fn record(self, frame: SentFrame) -> Result<(), Error> {
        let database = self.lock_probe.lock().unwrap().clone();
        if let Some(database) = database {
            assert!(
                database.try_read().is_ok(),
                "Event Enrollment database guard must be released before network I/O"
            );
        }
        self.sent.lock().unwrap().push(frame.npdu);
        Ok(())
    }
}

pub(super) struct ObservedObject {
    oid: ObjectIdentifier,
    name: String,
    value: PropertyValue,
    reliability: Option<PropertyValue>,
    status_flags: Option<u8>,
}

impl ObservedObject {
    pub(super) fn new(instance: u32, value: PropertyValue) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            name: format!("observed-{instance}"),
            value,
            reliability: Some(PropertyValue::Enumerated(
                Reliability::NO_FAULT_DETECTED.to_raw(),
            )),
            status_flags: Some(0),
        }
    }

    pub(super) fn with_reliability(mut self, reliability: Reliability) -> Self {
        self.reliability = Some(PropertyValue::Enumerated(reliability.to_raw()));
        self
    }

    pub(super) fn with_reliability_value(mut self, reliability: PropertyValue) -> Self {
        self.reliability = Some(reliability);
        self
    }

    pub(super) fn with_status_flags(mut self, status_flags: u8) -> Self {
        self.status_flags = Some(status_flags);
        self
    }

    pub(super) fn without_reliability(mut self) -> Self {
        self.reliability = None;
        self
    }

    pub(super) fn without_status_flags(mut self) -> Self {
        self.status_flags = None;
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
            p if p == PropertyIdentifier::RELIABILITY => self.reliability.clone().ok_or(
                Error::protocol(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            ),
            p if p == PropertyIdentifier::STATUS_FLAGS => self
                .status_flags
                .map(|status_flags| PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![status_flags],
                })
                .ok_or(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                )),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
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
            (p, value) if p == PropertyIdentifier::RELIABILITY => self.reliability = Some(value),
            (p, PropertyValue::BitString { data, .. })
                if p == PropertyIdentifier::STATUS_FLAGS && data.len() == 1 =>
            {
                self.status_flags = Some(data[0]);
            }
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::WRITE_ACCESS_DENIED,
                ));
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
    let mut enrollment =
        EventEnrollmentObject::new(instance, format!("enrollment-{instance}"), event_type).unwrap();
    enrollment
        .set_object_property_reference(target.map(|oid| {
            BACnetDeviceObjectPropertyReference::new_local(
                oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            )
        }))
        .unwrap();
    enrollment.set_event_parameters(parameters);
    enrollment.set_event_enable(bacnet_types::bitstring::EventTransitionBits::all());
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
            identity: bacnet_objects::DeviceIdentity::default()
                .with_name("Event Enrollment Device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    let mut notification_class = NotificationClass::new(0, "NC-0").unwrap();
    notification_class.ack_required = EventTransitionBits::all();
    if recipients {
        notification_class
            .add_destination(
                crate::server::event_notifications_tests::local_broadcast_destination(),
            )
            .unwrap();
    }
    db.add(Box::new(notification_class)).unwrap();
}

pub(super) async fn start_server(
    mut db: ObjectDatabase,
    recipients: bool,
) -> (BACnetServer<TestTransport>, StdArc<StdMutex<Vec<Bytes>>>) {
    add_server_context(&mut db, recipients);
    let capture = NotificationCapture::default();
    let server = BACnetServer::start_clockless(
        ServerConfig {
            event_enrollment_interval_secs: 1,
            ..ServerConfig::default()
        },
        db,
        capture.transport(),
    )
    .await
    .unwrap();
    *capture.lock_probe.lock().unwrap() = Some(StdArc::clone(server.database()));
    (server, capture.sent)
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
            decode_event_notification(&request.service_request)
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
        assert_eq!(notification.event_type, EventType::CHANGE_OF_RELIABILITY);
        assert_eq!(notification.from_state, from);
        assert_eq!(notification.to_state, to);
        assert!(notification.ack_required);
        assert_eq!(notification.message_text, None);
        let Some(NotificationParameters::ChangeOfReliability {
            status_flags,
            property_values,
            ..
        }) = notification.event_values.as_ref()
        else {
            panic!("Event Enrollment fault transition must carry CHANGE_OF_RELIABILITY values");
        };
        assert_eq!(
            *status_flags,
            if to == EventState::FAULT {
                StatusFlags::IN_ALARM | StatusFlags::FAULT
            } else {
                StatusFlags::empty()
            }
        );
        let mut properties = Vec::new();
        let mut position = 0;
        while position < property_values.len() {
            let (property, next) = decode_bacnet_property_value(property_values, position).unwrap();
            assert!(next > position);
            properties.push(property.property_identifier);
            position = next;
        }
        assert_eq!(
            properties,
            vec![
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::RELIABILITY,
                PropertyIdentifier::STATUS_FLAGS,
            ]
        );
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

#[tokio::test(start_paused = true)]
async fn event_enrollment_reliability_omits_only_unavailable_monitored_entries() {
    let mut db = ObjectDatabase::new();
    let target = ObservedObject::new(70, PropertyValue::Real(-1.0))
        .without_reliability()
        .without_status_flags();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let mut enrollment = enrollment(
        70,
        EventType::OUT_OF_RANGE,
        Some(target_oid),
        out_of_range_parameters(0),
    );
    enrollment.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 10.0,
    }));
    db.add(Box::new(enrollment)).unwrap();

    let (mut server, sent) = start_server(db, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let notifications = drain_notifications(&sent);
    assert_eq!(notifications.len(), 1);
    let Some(NotificationParameters::ChangeOfReliability {
        property_values, ..
    }) = notifications[0].event_values.as_ref()
    else {
        panic!("fault transition must carry CHANGE_OF_RELIABILITY");
    };
    let mut identifiers = Vec::new();
    let mut position = 0;
    while position < property_values.len() {
        let (property, next) = decode_bacnet_property_value(property_values, position).unwrap();
        identifiers.push(property.property_identifier);
        position = next;
    }
    assert_eq!(
        identifiers,
        vec![
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::PRESENT_VALUE,
        ],
        "only unavailable monitored Reliability and Status_Flags may be omitted"
    );
    assert_eq!(notifications[0].message_text, None);
    server.stop().await.unwrap();
}
