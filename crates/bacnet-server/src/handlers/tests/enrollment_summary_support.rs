use std::borrow::Cow;

use bacnet_objects::event::{EnrollmentSummaryCapability, EventTransition};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::EventType;
use bacnet_types::primitives::Time;

use super::*;

pub(super) struct SummaryFixture {
    oid: ObjectIdentifier,
    name: String,
    advertised: Vec<PropertyIdentifier>,
    values: Vec<(PropertyIdentifier, PropertyValue)>,
    capability: Option<EnrollmentSummaryCapability>,
}

impl SummaryFixture {
    pub(super) fn candidate(
        instance: u32,
        event_type: EventType,
        event_state: EventState,
        acknowledged_transitions: u8,
        notification_class: u32,
        last_transition: Option<EventTransition>,
    ) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            name: format!("SUMMARY-{instance}"),
            advertised: vec![
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::ACKED_TRANSITIONS,
                PropertyIdentifier::NOTIFICATION_CLASS,
            ],
            values: vec![
                (
                    PropertyIdentifier::EVENT_STATE,
                    PropertyValue::Enumerated(event_state.to_raw()),
                ),
                (
                    PropertyIdentifier::ACKED_TRANSITIONS,
                    transition_bits(acknowledged_transitions),
                ),
                (
                    PropertyIdentifier::NOTIFICATION_CLASS,
                    PropertyValue::Unsigned(notification_class as u64),
                ),
            ],
            capability: Some(EnrollmentSummaryCapability {
                event_type,
                last_transition,
            }),
        }
    }

    pub(super) fn notification_class(
        instance: u32,
        class_value: Option<PropertyValue>,
        priority: Option<PropertyValue>,
        recipient_list: Option<PropertyValue>,
    ) -> Self {
        let mut values = Vec::new();
        for (property, value) in [
            (PropertyIdentifier::NOTIFICATION_CLASS, class_value),
            (PropertyIdentifier::PRIORITY, priority),
            (PropertyIdentifier::RECIPIENT_LIST, recipient_list),
        ] {
            if let Some(value) = value {
                values.push((property, value));
            }
        }
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, instance).unwrap(),
            name: format!("SUMMARY-NC-{instance}"),
            advertised: Vec::new(),
            values,
            capability: None,
        }
    }

    pub(super) fn advertise(&mut self, property: PropertyIdentifier) {
        if !self.advertised.contains(&property) {
            self.advertised.push(property);
        }
    }

    pub(super) fn set(&mut self, property: PropertyIdentifier, value: PropertyValue) {
        self.values.retain(|(candidate, _)| *candidate != property);
        self.values.push((property, value));
    }

    pub(super) fn without_capability(mut self) -> Self {
        self.capability = None;
        self
    }
}

impl BACnetObject for SummaryFixture {
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
        self.values
            .iter()
            .find(|(candidate, _)| *candidate == property)
            .map(|(_, value)| value.clone())
            .ok_or(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
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

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(self.advertised.clone())
    }

    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        self.capability
    }
}

pub(super) fn transition_bits(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bacnet_types::bitstring::pack_octet(bits)],
    }
}

pub(super) fn class(
    instance: u32,
    intrinsic_notification_class: u32,
    priority: [u8; 3],
    recipient_list: Vec<BACnetDestination>,
) -> NotificationClass {
    let mut class = NotificationClass::new(instance, format!("NC-{instance}")).unwrap();
    class.notification_class = intrinsic_notification_class;
    class.priority = priority;
    class.recipient_list = recipient_list;
    class
}

pub(super) fn destination(
    recipient: BACnetRecipient,
    process_identifier: u32,
) -> BACnetDestination {
    BACnetDestination {
        valid_days: 0,
        from_time: Time {
            hour: 23,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 1,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        recipient,
        process_identifier,
        issue_confirmed_notifications: false,
        transitions: 0,
    }
}

pub(super) fn request() -> GetEnrollmentSummaryRequest {
    GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    }
}

pub(super) fn response(
    db: &ObjectDatabase,
    request: &GetEnrollmentSummaryRequest,
) -> Result<GetEnrollmentSummaryAck, Error> {
    let mut encoded_request = BytesMut::new();
    request.encode(&mut encoded_request);
    let mut encoded_ack = BytesMut::new();
    handle_get_enrollment_summary(db, &encoded_request, &mut encoded_ack)?;
    GetEnrollmentSummaryAck::decode(&encoded_ack)
}

pub(super) fn assert_operational_problem(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::DEVICE.to_raw() as u32
                && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32
    ));
}
