//! Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair on Event
//! Enrollment (#1507): the shared `ReportingOptions` serves them, as it does
//! on the intrinsic reporters, and a committed transition stores the
//! configured text in Event_Message_Texts.

use super::super::*;
use crate::binary::BinaryValueObject;
use crate::database::ObjectDatabase;
use crate::event::{EventStateChange, EventTransition};
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::BinaryPV;
use bacnet_types::error::RemoteError;
use bytes::BytesMut;

fn enrollment() -> EventEnrollmentObject {
    EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap()
}

fn write(ee: &mut EventEnrollmentObject, property: PropertyIdentifier, value: PropertyValue) {
    ee.write_property(property, None, value, None).unwrap();
}

fn texts(texts: [&str; 3]) -> PropertyValue {
    PropertyValue::List(
        texts
            .iter()
            .map(|t| PropertyValue::CharacterString((*t).into()))
            .collect(),
    )
}

fn inhibit(ee: &dyn BACnetObject) -> PropertyValue {
    ee.read_property(PropertyIdentifier::EVENT_ALGORITHM_INHIBIT, None)
        .unwrap()
}

fn code(result: Result<(), Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { code, .. })) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn commit(ee: &mut EventEnrollmentObject, from: EventState, to: EventState) {
    ee.commit_event_transition_internal(EventTransitionCommit {
        coordinate: EventTransition::for_target_state(to),
        change: EventStateChange { from, to },
        ack_required: false,
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        // What the server's enrollment commit passes (#1507).
        message_text: Some(String::new()),
    })
    .unwrap();
}

fn message_text(ee: &EventEnrollmentObject, index: u32) -> PropertyValue {
    ee.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(index))
        .unwrap()
}

#[test]
fn event_enrollment_serves_and_takes_the_three_rows() {
    let mut ee = enrollment();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG, None)
            .unwrap(),
        texts(["", "", ""])
    );
    write(
        &mut ee,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        texts(["High", "", "Back"]),
    );
    ee.write_property(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        Some(2),
        PropertyValue::CharacterString("Broken".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG, None)
            .unwrap(),
        texts(["High", "Broken", "Back"])
    );

    // No reference: the unset form reads back and the inhibit is writable.
    let PropertyValue::ApplicationData(_) = ee
        .read_property(PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF, None)
        .unwrap()
    else {
        panic!("the reference reads as an encoded BACnetObjectPropertyReference");
    };
    assert_eq!(inhibit(&ee), PropertyValue::Boolean(false));
    write(
        &mut ee,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        PropertyValue::Boolean(true),
    );
    assert_eq!(inhibit(&ee), PropertyValue::Boolean(true));

    // Detection off: the configuration stays, and the inhibit takes no write.
    write(
        &mut ee,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    assert_eq!(
        code(ee.write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    assert_eq!(inhibit(&ee), PropertyValue::Boolean(true));
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("High".into())
    );
}

#[test]
fn event_enrollment_inhibit_follows_its_reference_through_the_database() {
    let mut db = ObjectDatabase::new();
    let mut bv = BinaryValueObject::new(4, "inhibit").unwrap();
    let bv_oid = bv.object_identifier();
    // Nothing commands it, so Present_Value is Relinquish_Default.
    bv.set_relinquish_default(BinaryPV::ACTIVE.to_raw())
        .unwrap();
    db.add(Box::new(bv)).unwrap();
    let mut ee = enrollment();
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(bv_oid, PropertyIdentifier::PRESENT_VALUE.to_raw()),
    );
    write(
        &mut ee,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
        PropertyValue::ApplicationData(octets.to_vec()),
    );
    // With a reference, the inhibit is the referenced value's to set.
    assert_eq!(
        code(ee.write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    assert!(db.follow_event_algorithm_inhibit(&ee_oid));
    assert_eq!(
        inhibit(db.get(&ee_oid).unwrap()),
        PropertyValue::Boolean(true)
    );
    db.get_mut(&bv_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Enumerated(BinaryPV::INACTIVE.to_raw()),
            None,
        )
        .unwrap();
    assert!(db.follow_event_algorithm_inhibit(&ee_oid));
    assert_eq!(
        inhibit(db.get(&ee_oid).unwrap()),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn event_enrollment_commit_stores_the_configured_text_or_none() {
    let mut ee = enrollment();
    write(
        &mut ee,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        texts(["Too hot", "", ""]),
    );
    commit(&mut ee, EventState::NORMAL, EventState::HIGH_LIMIT);
    assert_eq!(
        message_text(&ee, 1),
        PropertyValue::CharacterString("Too hot".into())
    );
    // An empty entry leaves the transition without a text.
    commit(&mut ee, EventState::HIGH_LIMIT, EventState::NORMAL);
    assert_eq!(
        message_text(&ee, 3),
        PropertyValue::CharacterString(String::new())
    );
    // Detection off returns the texts to their initial condition.
    write(
        &mut ee,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None)
            .unwrap(),
        texts(["", "", ""])
    );
}

/// Alert Enrollment shares the rows (#1865): Table 12-61 lists them as plain
/// optional rows, so the configuration survives a detection reset, the
/// inhibit takes no write while detection is off, and with a reference the
/// database refreshes it from the referenced property.
#[test]
fn alert_enrollment_shares_the_reporting_options() {
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut alert = AlertEnrollmentObject::new(1, "AE-1", source).unwrap();
    for property in [
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
    ] {
        assert!(alert.is_writable_property(property), "{property:?}");
    }
    alert
        .write_property(
            PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            texts(["Alert", "", ""]),
            None,
        )
        .unwrap();
    alert
        .write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(inhibit(&alert), PropertyValue::Boolean(true));

    alert.set_event_detection_enable(false);
    assert_eq!(
        code(alert.write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None,
        )),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    assert_eq!(
        alert
            .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("Alert".into())
    );
    assert_eq!(
        alert
            .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None)
            .unwrap(),
        texts(["", "", ""])
    );
    alert.set_event_detection_enable(true);

    // A reference hands the inhibit to the database's refresh.
    let mut db = ObjectDatabase::new();
    let bv = BinaryValueObject::new(4, "inhibit").unwrap();
    let bv_oid = bv.object_identifier();
    db.add(Box::new(bv)).unwrap();
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(bv_oid, PropertyIdentifier::PRESENT_VALUE.to_raw()),
    );
    alert
        .write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            PropertyValue::ApplicationData(octets.to_vec()),
            None,
        )
        .unwrap();
    let alert_oid = alert.object_identifier();
    db.add(Box::new(alert)).unwrap();
    // The Binary Value reads INACTIVE, so the inhibit follows it to FALSE.
    db.follow_event_algorithm_inhibit(&alert_oid);
    assert_eq!(
        inhibit(db.get(&alert_oid).unwrap()),
        PropertyValue::Boolean(false)
    );
    db.get_mut(&bv_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Enumerated(BinaryPV::ACTIVE.to_raw()),
            None,
        )
        .unwrap();
    assert!(db.follow_event_algorithm_inhibit(&alert_oid));
    assert_eq!(
        inhibit(db.get(&alert_oid).unwrap()),
        PropertyValue::Boolean(true)
    );
}
