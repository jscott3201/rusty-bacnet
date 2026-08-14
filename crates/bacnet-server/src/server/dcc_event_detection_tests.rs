use super::event_notifications_tests::{
    broadcasts_from_per_write_path, db_with_high_limit_transition, RecordingTransport,
};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::EventState;
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

#[tokio::test]
async fn dcc_states_preserve_per_write_detection_but_suppress_distribution() {
    for comm_state in [1, 2] {
        let db = db_with_high_limit_transition(0x80);
        let sent = broadcasts_from_per_write_path(&db, comm_state).await;

        assert!(
            sent.is_empty(),
            "DCC state {comm_state} must suppress event notification distribution"
        );
        let db = db.read().await;
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
            "DCC state {comm_state} must not suppress event-state detection"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn dcc_states_preserve_delayed_detection_but_suppress_distribution() {
    for comm_state in [1, 2] {
        let sent = StdArc::new(StdMutex::new(Vec::<Bytes>::new()));
        let transport = RecordingTransport {
            sent_broadcast: StdArc::clone(&sent),
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        };
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
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
                instance: 1,
                name: "Dev".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();

        let mut server = BACnetServer::start(ServerConfig::default(), db, transport)
            .await
            .expect("server should start");
        server.comm_state.store(comm_state, Ordering::Release);
        server
            .write_local(
                &oid,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(81.0),
                None,
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
                "DCC state {comm_state} must honor Time_Delay before transitioning"
            );
        }

        tokio::time::sleep(Duration::from_secs(5)).await;

        assert!(
            sent.lock().unwrap().is_empty(),
            "DCC state {comm_state} must suppress delayed event notification distribution"
        );
        {
            let db = server.database().read().await;
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::EVENT_STATE, None)
                    .unwrap(),
                PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()),
                "DCC state {comm_state} must not pause the Time_Delay countdown"
            );
        }
        server.stop().await.unwrap();
    }
}
