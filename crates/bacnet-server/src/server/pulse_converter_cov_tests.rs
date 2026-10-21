//! Pulse Converter COV reporting over the wire (#1092, #1061).
//!
//! Table 13-1 has a Pulse Converter notification carry Update_Time after
//! Present_Value and Status_Flags. Update_Time is reported, not a trigger:
//! the notifications here come from Present_Value changes, and a pulse that
//! moves Update_Time but not Present_Value by COV_Increment sends none.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use std::sync::Mutex as StdMutex;

fn pc1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::PULSE_CONVERTER, 1).unwrap()
}

/// `(property, value bytes)` of a Pulse Converter notification, in wire order.
fn values(notification: &COVNotificationRequest) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    assert_eq!(notification.monitored_object_identifier, pc1());
    notification
        .list_of_values
        .iter()
        .map(|value| (value.property_identifier, value.value.clone()))
        .collect()
}

/// Update_Time as stamped at `at(7)`: Date 2026-09-29 (Tuesday), Time 15:00:07.00.
fn update_time() -> Vec<u8> {
    vec![0xA4, 126, 9, 29, 2, 0xB4, 15, 0, 7, 0]
}

#[tokio::test(start_paused = true)]
async fn pulse_converter_cov_notification_reports_update_time() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        let mut object =
            PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        // Accumulate under a clock reading 15:00:07 so Update_Time is set.
        let clock = SharedClock(Arc::new(StdMutex::new(at(7))));
        object.bind_clock_internal(Some(Arc::new(clock)));
        object.add_pulses(4).unwrap();
        db.add(Box::new(object)).unwrap();
    })
    .await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1092,
        monitored_object_identifier: pc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    let flags = vec![0x82, 0x04, 0x00];
    assert_eq!(
        values(&h.cov_notification().await),
        vec![
            (PV, real(4.0)),
            (SF, flags.clone()),
            (PropertyIdentifier::UPDATE_TIME, update_time()),
        ]
    );

    // Doubling Scale_Factor moves Present_Value from 4 to 8; the report
    // carries the unchanged Update_Time with it.
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &PropertyValue::Real(2.0))
        .unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: pc1(),
        property_identifier: PropertyIdentifier::SCALE_FACTOR,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    assert_eq!(
        values(&h.cov_notification().await),
        vec![
            (PV, real(8.0)),
            (SF, flags),
            (PropertyIdentifier::UPDATE_TIME, update_time()),
        ]
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// PC-1 reporting Present_Value moves of at least 10, whose Count took 4
/// pulses at 15:00:07 and `more` at 15:00:09.
fn counted(more: u64) -> Box<dyn BACnetObject> {
    let mut object =
        PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    object
        .write_property(
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(10.0),
            None,
        )
        .unwrap();
    for (second, pulses) in [(7, 4), (9, more)] {
        let clock = SharedClock(Arc::new(StdMutex::new(at(second))));
        object.bind_clock_internal(Some(Arc::new(clock)));
        object.add_pulses(pulses).unwrap();
    }
    Box::new(object)
}

#[tokio::test(start_paused = true)]
async fn pulse_converter_update_time_alone_sends_no_cov_notification() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(counted(0)).unwrap();
    })
    .await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1061,
        monitored_object_identifier: pc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    let flags = vec![0x82, 0x04, 0x00];
    assert_eq!(
        values(&h.cov_notification().await),
        vec![
            (PV, real(4.0)),
            (SF, flags.clone()),
            (PropertyIdentifier::UPDATE_TIME, update_time()),
        ]
    );

    // One more pulse stamps Update_Time 15:00:09 but moves Present_Value by
    // only 1, under COV_Increment: nothing goes out.
    h.replace_and_fan_out(counted(1)).await;
    h.no_notification().await;

    // Scale_Factor 10 moves Present_Value from 5 to 50; that report carries
    // the newer Update_Time.
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &PropertyValue::Real(10.0))
        .unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: pc1(),
        property_identifier: PropertyIdentifier::SCALE_FACTOR,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    let mut later = update_time();
    later[8] = 9;
    assert_eq!(
        values(&h.cov_notification().await),
        vec![
            (PV, real(50.0)),
            (SF, flags),
            (PropertyIdentifier::UPDATE_TIME, later),
        ]
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
