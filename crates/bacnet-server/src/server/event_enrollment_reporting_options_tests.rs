//! Event_Algorithm_Inhibit and Event_Message_Texts_Config on an Event
//! Enrollment, end to end in paused time (#1507): the inhibit stops the
//! algorithm's offnormal and normal transitions while fault detection still
//! runs (Clauses 13.2.2.1 and 13.2.2.1.5), and a configured text is the sent
//! notification's message text.

use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::BinaryPV;
use bytes::BytesMut;

const TARGET: u32 = 70;

async fn write(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    server
        .database()
        .write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property(property, None, value, None)
        .unwrap();
}

/// The Binary Value whose Present_Value (its Relinquish_Default, as nothing
/// commands it) the enrollment's inhibit follows.
async fn set_inhibit(server: &BACnetServer<TestTransport>, oid: ObjectIdentifier, on: bool) {
    let value = if on {
        BinaryPV::ACTIVE
    } else {
        BinaryPV::INACTIVE
    };
    write(
        server,
        oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyValue::Enumerated(value.to_raw()),
    )
    .await;
}

async fn settle(seconds: f32) {
    tokio::time::sleep(Duration::from_secs_f32(seconds)).await;
}

fn single(sent: &std::sync::Mutex<Vec<Bytes>>) -> EventNotificationRequest {
    let mut notifications = drain_notifications(sent);
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    notifications.remove(0)
}

#[tokio::test(start_paused = true)]
async fn event_enrollment_inhibit_stops_the_algorithm_but_not_fault_and_texts_go_out() {
    let mut db = ObjectDatabase::new();
    let target = ObservedObject::new(TARGET, PropertyValue::Real(50.0));
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();
    let bv = BinaryValueObject::new(71, "inhibit").unwrap();
    let bv_oid = bv.object_identifier();
    db.add(Box::new(bv)).unwrap();

    let mut ee = enrollment(
        TARGET,
        EventType::OUT_OF_RANGE,
        Some(target_oid),
        out_of_range_parameters(2),
    );
    // A long pTimeDelayNormal: only the inhibit can bring it back quickly.
    ee.set_time_delay_normal(Some(30));
    ee.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    }));
    let mut reference = BytesMut::new();
    encode_object_property_reference(
        &mut reference,
        &BACnetObjectPropertyReference::new(bv_oid, PropertyIdentifier::PRESENT_VALUE.to_raw()),
    );
    ee.write_property(
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
        None,
        PropertyValue::ApplicationData(reference.to_vec()),
        None,
    )
    .unwrap();
    // Sent as written: "{" and "%" are not substitution codes here.
    ee.write_property(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        None,
        PropertyValue::List(
            ["Too hot {name} 100%", "", "Back to normal"]
                .into_iter()
                .map(|text| PropertyValue::CharacterString(text.into()))
                .collect(),
        ),
        None,
    )
    .unwrap();
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let (mut server, sent) = start_server(db, true).await;
    settle(0.5).await;
    assert!(drain_notifications(&sent).is_empty());

    // Not inhibited: the algorithm reports, carrying the configured text.
    write(
        &server,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(90.0),
    )
    .await;
    settle(5.0).await;
    let high = single(&sent);
    assert_eq!(high.to_state, EventState::HIGH_LIMIT);
    assert_eq!(high.message_text.as_deref(), Some("Too hot {name} 100%"));

    // Inhibited: back to NORMAL at once, well inside pTimeDelayNormal.
    set_inhibit(&server, bv_oid, true).await;
    settle(2.5).await;
    let normal = single(&sent);
    assert_eq!(
        (normal.from_state, normal.to_state),
        (EventState::HIGH_LIMIT, EventState::NORMAL)
    );
    assert_eq!(normal.event_type, EventType::OUT_OF_RANGE);
    assert_eq!(normal.message_text.as_deref(), Some("Back to normal"));
    assert_eq!(
        server
            .database()
            .read()
            .await
            .get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_ALGORITHM_INHIBIT, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    // The value is still high, yet nothing more is reported.
    settle(8.0).await;
    assert!(drain_notifications(&sent).is_empty());

    // Fault detection still runs, in and out, with no configured TO_FAULT
    // text to send.
    write(
        &server,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(150.0),
    )
    .await;
    settle(2.5).await;
    let fault = single(&sent);
    assert_eq!(fault.to_state, EventState::FAULT);
    assert_eq!(fault.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(fault.message_text, None);
    write(
        &server,
        target_oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(90.0),
    )
    .await;
    settle(2.5).await;
    let recovered = single(&sent);
    assert_eq!(
        (recovered.from_state, recovered.to_state),
        (EventState::FAULT, EventState::NORMAL)
    );
    settle(8.0).await;
    assert!(
        drain_notifications(&sent).is_empty(),
        "an inhibited enrollment reported its still-high value"
    );

    // Cleared: the condition has to hold its whole Time_Delay from now.
    set_inhibit(&server, bv_oid, false).await;
    settle(1.5).await;
    assert!(drain_notifications(&sent).is_empty());
    settle(5.0).await;
    assert_eq!(single(&sent).to_state, EventState::HIGH_LIMIT);
    server.stop().await.unwrap();
}
