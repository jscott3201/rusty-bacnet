//! `Event_Detection_Enable` filtering in the three event-summarization
//! services (ASHRAE 135-2020 Clause 13.2.4 Table 13-2, and the Service
//! Procedure of each of Clauses 13.10, 13.11 and 13.12).
//!
//! Each of those three clauses states the exclusion independently, so each
//! service is tested separately rather than trusting one shared code path. The
//! wording differs and the difference matters: 13.10 and 13.11 say an object
//! with the property FALSE "shall be ignored", while 13.12 inverts it into the
//! search predicate — objects that "do not have an Event_Detection_Enable
//! property with a value of FALSE" — which is what makes absence mean
//! *included* rather than excluded.

use super::*;
use bacnet_objects::traits::BACnetObject;

/// A minimal event-initiating object that is simultaneously in HIGH_LIMIT and
/// has `Event_Detection_Enable` FALSE.
///
/// A mock rather than a real `EventEnrollmentObject`, deliberately.
/// `EventEnrollmentObject` now enforces the Clause 13.2.2.1 invariant on every
/// path — `write_property`, `set_event_state_internal` and the `set_event_state`
/// seeder all refuse to leave it non-NORMAL while detection is off — so it
/// *cannot* reach this state, and a fixture built from it would prove only that
/// the reset works, not that these services filter on the property.
///
/// The state is still reachable in the wild: `AlertEnrollmentObject` exposes
/// `Event_Detection_Enable` and applies no reset (#205), and the standard states
/// the exclusion independently in each of the three Service Procedures rather
/// than deriving it from `Event_State`. So the filter must be tested directly,
/// against an object that can actually be inconsistent.
struct DisabledAlarmingObject {
    oid: ObjectIdentifier,
    detection_enable: Option<bool>,
}

impl BACnetObject for DisabledAlarmingObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "MOCK-1"
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
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                bacnet_types::enums::EventState::HIGH_LIMIT.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_DETECTION_ENABLE => self
                .detection_enable
                .map(PropertyValue::Boolean)
                .ok_or_else(|| Error::Protocol {
                    class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                    code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                }),
            p if p == PropertyIdentifier::EVENT_TYPE => Ok(PropertyValue::Enumerated(
                bacnet_types::enums::EventType::OUT_OF_RANGE.to_raw(),
            )),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(0)),
            _ => Err(Error::Protocol {
                class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
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
            class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
            code: bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ];
        std::borrow::Cow::Borrowed(PROPS)
    }
}

fn db_with_enrollment(detection_enabled: bool) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(DisabledAlarmingObject {
        oid: ObjectIdentifier::new(bacnet_types::enums::ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
        detection_enable: Some(detection_enabled),
    }))
    .unwrap();
    db
}

fn alarm_summary_entry_count(db: &ObjectDatabase) -> usize {
    use bacnet_services::alarm_summary::GetAlarmSummaryAck;
    let mut buf = BytesMut::new();
    handle_get_alarm_summary(db, &mut buf).unwrap();
    GetAlarmSummaryAck::decode(&buf).unwrap().entries.len()
}

/// Clause 13.10, Service Procedure: "Any object that has an Event_Detection_Enable property
/// with a value of FALSE shall be ignored."
#[test]
fn get_alarm_summary_excludes_detection_disabled_object() {
    assert_eq!(
        alarm_summary_entry_count(&db_with_enrollment(true)),
        1,
        "control: an alarming enrollment is reported when detection is enabled"
    );
    assert_eq!(
        alarm_summary_entry_count(&db_with_enrollment(false)),
        0,
        "an object with Event_Detection_Enable FALSE shall be ignored"
    );
}

/// The exclusion tests the property, not the absence of the property.
///
/// Clause 13.12's Service Procedure phrases it as a double negative — objects that "do not have
/// an Event_Detection_Enable property with a value of FALSE" are searched — so
/// an object that does not model the property at all must still be reported.
/// Getting this backwards would silently empty these responses for every object
/// type that lacks the property, which is most of them.
#[test]
fn summarization_includes_objects_without_the_property() {
    let mut db = ObjectDatabase::new();
    let object = DisabledAlarmingObject {
        oid: ObjectIdentifier::new(bacnet_types::enums::ObjectType::EVENT_ENROLLMENT, 2).unwrap(),
        detection_enable: None,
    };

    // Precondition: this object type really does lack the property.
    assert!(
        object
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .is_err(),
        "fixture must not model Event_Detection_Enable"
    );
    db.add(Box::new(object)).unwrap();

    assert_eq!(
        alarm_summary_entry_count(&db),
        1,
        "an object with no Event_Detection_Enable property must still be reported"
    );
}

/// Clause 13.11's Service Procedure carries the same exclusion. This service matters most for
/// the check: unlike the other two it applies no default `Event_State` filter,
/// so the exclusion cannot fall out of the forced-NORMAL invariant and has to
/// be implemented explicitly.
#[test]
fn get_enrollment_summary_excludes_detection_disabled_object() {
    use bacnet_services::enrollment_summary::{
        GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };

    let count = |db: &ObjectDatabase| {
        let request = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 0, // all
            enrollment_filter: None,
            event_state_filter: None,
            event_type_filter: None,
            priority_filter: None,
            notification_class_filter: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack = BytesMut::new();
        handle_get_enrollment_summary(db, &buf, &mut ack).unwrap();
        GetEnrollmentSummaryAck::decode(&ack).unwrap().entries.len()
    };

    assert_eq!(
        count(&db_with_enrollment(true)),
        1,
        "control: the enrollment is summarized when detection is enabled"
    );
    assert_eq!(
        count(&db_with_enrollment(false)),
        0,
        "Event_Detection_Enable FALSE excludes the object from the summary"
    );
}

/// Clause 13.12's Service Procedure carries the exclusion for GetEventInformation, the one
/// summarization service Clause 13.2.4 requires notification-servers to
/// support (the other two are deprecated).
#[test]
fn get_event_information_excludes_detection_disabled_object() {
    use bacnet_services::alarm_event::GetEventInformationRequest;

    let responses = [true, false].map(|enabled| {
        let db = db_with_enrollment(enabled);
        let request = GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack = BytesMut::new();
        handle_get_event_information(&db, &buf, &mut ack).unwrap();
        ack.to_vec()
    });

    assert_ne!(
        responses[0], responses[1],
        "disabling detection must change the GetEventInformation response"
    );
    // The enabled response carries the object identifier; the disabled one is
    // the shorter empty-list form.
    assert!(
        responses[1].len() < responses[0].len(),
        "the disabled object must be absent from the response, got {} vs {} bytes",
        responses[1].len(),
        responses[0].len()
    );
}
