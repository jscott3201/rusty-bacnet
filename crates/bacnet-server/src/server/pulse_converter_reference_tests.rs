//! On a running server, a Pulse Converter's Input_Reference fault follows
//! the object the reference names as it comes and goes (#1341): the
//! application's own `add`, a CreateObject and a DeleteObject each judge the
//! reference again, and a SubscribeCOV on the converter hears of the
//! Status_Flags change without any write to it. Time is paused.

use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, Reliability};

const FAULT: u8 = 0x40;

fn pc1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::PULSE_CONVERTER, 1).unwrap()
}

/// Multi-state Value 5, whose Present_Value is an Unsigned and which a
/// client can create.
fn msv5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 5).unwrap()
}

/// PC-1 counting from MSV-5's Present_Value, which doesn't exist yet.
fn with_converter(db: &mut ObjectDatabase) {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::NO_UNITS).unwrap();
    pc.set_input_reference(BACnetObjectPropertyReference::new(msv5(), PV.to_raw()));
    db.add(Box::new(pc)).unwrap();
}

async fn subscribe(h: &mut Harness) {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1341,
        monitored_object_identifier: pc1(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
}

/// The converter's Status_Flags in a notification about it.
fn flags(notification: &COVNotificationRequest) -> u8 {
    assert_eq!(notification.monitored_object_identifier, pc1());
    let value = &notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == SF)
        .expect("Status_Flags reported")
        .value;
    assert_eq!(&value[..2], &[0x82, 0x04], "four-bit Status_Flags");
    value[2]
}

async fn reliability(h: &Harness) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&pc1())
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
}

fn configuration_error() -> PropertyValue {
    PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
}

fn no_fault() -> PropertyValue {
    PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
}

#[tokio::test(start_paused = true)]
async fn an_applications_add_and_remove_report_the_fault_without_a_write() {
    let mut h = Harness::start_with(ServerConfig::default(), with_converter).await;
    subscribe(&mut h).await;
    assert_eq!(flags(&h.cov_notification().await), FAULT);
    assert_eq!(reliability(&h).await, configuration_error());

    // The application adds the object under its own guard; the server takes
    // the work the add queued once that guard is dropped.
    h.server
        .database()
        .write()
        .await
        .add(Box::new(MultiStateValueObject::new(5, "MSV-5", 3).unwrap()))
        .unwrap();
    assert_eq!(flags(&h.cov_notification().await), 0);
    assert_eq!(reliability(&h).await, no_fault());

    h.server.database().write().await.remove(&msv5()).unwrap();
    assert_eq!(flags(&h.cov_notification().await), FAULT);
    assert_eq!(reliability(&h).await, configuration_error());
}

#[tokio::test(start_paused = true)]
async fn create_object_and_delete_object_report_the_fault_they_move() {
    let mut h = Harness::start_with(ServerConfig::default(), with_converter).await;
    subscribe(&mut h).await;
    assert_eq!(flags(&h.cov_notification().await), FAULT);
    // Without the waker the background task never takes the queue, so only
    // each request's own settle, under its guard, can report the change.
    h.server
        .database()
        .write()
        .await
        .set_membership_waker_internal(None);

    let mut body = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(msv5()),
        list_of_initial_values: Vec::new(),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::CREATE_OBJECT, body).await;
    assert_eq!(flags(&h.cov_notification().await), 0);
    assert_eq!(reliability(&h).await, no_fault());

    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: msv5(),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    assert_eq!(flags(&h.cov_notification().await), FAULT);
    assert_eq!(reliability(&h).await, configuration_error());
}

#[tokio::test(start_paused = true)]
async fn a_local_write_of_input_reference_is_judged_at_once() {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        with_converter(db);
        db.add(Box::new(MultiStateValueObject::new(5, "MSV-5", 3).unwrap()))
            .unwrap();
    })
    .await;
    assert_eq!(reliability(&h).await, no_fault());
    // [0] analog-value 1, [1] present-value: a REAL.
    let analog_value = vec![0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55];
    // [0] multi-state-value 5, [1] present-value: an Unsigned.
    let multi_state_value = vec![0x0C, 0x04, 0xC0, 0x00, 0x05, 0x19, 0x55];
    for (octets, expected) in [
        (analog_value, configuration_error()),
        (multi_state_value, no_fault()),
    ] {
        h.server
            .write_local(
                &pc1(),
                PropertyIdentifier::INPUT_REFERENCE,
                None,
                PropertyValue::ApplicationData(octets),
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
        assert_eq!(reliability(&h).await, expected);
    }
}

#[tokio::test(start_paused = true)]
async fn a_running_server_counts_the_input_into_count_and_reports_it() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        with_converter(db);
        db.add(Box::new(
            MultiStateValueObject::new(5, "MSV-5", 10).unwrap(),
        ))
        .unwrap();
    })
    .await;
    subscribe(&mut h).await;
    h.cov_notification().await;
    // MSV-5 starts at state 1; the first reading sets the baseline, and the
    // move to 4 is three pulses (Clause 12.23.14).
    h.server
        .write_local(
            &msv5(),
            PV,
            None,
            PropertyValue::Unsigned(4),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let count = h
        .server
        .database()
        .read()
        .await
        .get(&pc1())
        .unwrap()
        .read_property(PropertyIdentifier::COUNT, None)
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3));
    // Present_Value, Count times Scale_Factor 1.0, reaches the subscriber.
    let notification = h.cov_notification().await;
    let present_value = &notification
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == PV)
        .expect("Present_Value reported")
        .value;
    assert_eq!(present_value, &[0x44, 0x40, 0x40, 0x00, 0x00]);
}
