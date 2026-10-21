use super::event_notifications_tests::{
    broadcasts_from_per_write_path, db_with_high_limit_transition, recording_transport,
};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState};

#[tokio::test]
async fn disable_initiation_preserves_per_write_detection_but_suppresses_distribution() {
    let db = db_with_high_limit_transition(0x80);
    let sent = broadcasts_from_per_write_path(&db, DccState::DisableInitiation).await;

    assert!(
        sent.is_empty(),
        "DISABLE_INITIATION must suppress event notification distribution"
    );
    let db = db.read().await;
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
        "DISABLE_INITIATION must not suppress event-state detection"
    );
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_preserves_delayed_detection_but_suppresses_distribution() {
    let (transport, sent) = recording_transport();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x80], // TO_OFFNORMAL at wire bit 0
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(2),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.set_present_value(62.0);
    let oid = ai.object_identifier();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // A recipient that WOULD be broadcast to, so the empty assertion can
    // only be satisfied by the DCC gate rather than by an unnamed recipient.
    db.add(Box::new(
        super::event_notifications_tests::notification_class_0_broadcasting(),
    ))
    .unwrap();

    let mut server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    server.comm_state.set_for_test(DccState::DisableInitiation);
    server
        .write_local(
            &oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(81.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("local Present_Value write should seed delayed detection");

    {
        let db = server.database().read().await;
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
            "DISABLE_INITIATION must honor Time_Delay before transitioning"
        );
    }

    tokio::time::sleep(Duration::from_secs(5)).await;

    assert!(
        sent.is_empty(),
        "DISABLE_INITIATION must suppress delayed event notification distribution"
    );
    {
        let db = server.database().read().await;
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
            "DISABLE_INITIATION must not pause the Time_Delay countdown"
        );
    }
    server.stop().await.unwrap();
}
