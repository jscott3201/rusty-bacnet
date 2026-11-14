//! The fault pass (#2535): the server's fault-detection task and the
//! `Reporter`'s fault pass, each every 10 s, commit the transition a
//! Reliability they change causes in the same pass, so both send its event
//! notification and then one COV report carrying FAULT with IN_ALARM, in
//! the same order, octet for octet, and the return to NORMAL with both flags
//! clear. Before, each reported FAULT alone, and the sweep sent the
//! transition and a second report. What both sent is kept sorted by
//! recipient, so the test checks what was told, not its order.
use super::*;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_types::enums::UnconfirmedServiceChoice;

/// An Analog Value faulting outside 0 to 100 by its own reliability
/// evaluation, reporting every transition through class 1 to the recipient
/// open all day.
fn faulting(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut value = AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    value.configure_fault_out_of_range(0.0, 100.0).unwrap();
    let enable = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0xE0],
    };
    value
        .write_property(P::EVENT_ENABLE, None, enable, None)
        .unwrap();
    let class = PropertyValue::Unsigned(1);
    value
        .write_property(P::NOTIFICATION_CLASS, None, class, None)
        .unwrap();
    let always = recipients().swap_remove(0);
    let class0 = NotificationClass::new(0, "NC-0").unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(value),
        Box::new(alarm_class(vec![always])),
        Box::new(class0),
    ];
    database_with(identity, objects)
}

/// What one notification the server sent says: the To State of an event
/// notification, or the Status_Flags a COV report carries.
#[derive(Debug, PartialEq)]
enum Told {
    Event(EventState),
    Cov(u8),
}

fn told(wire: &Wire) -> Told {
    let apdu = Bytes::copy_from_slice(&wire.2);
    let Apdu::UnconfirmedRequest(request) = bacnet_encoding::apdu::decode_apdu(apdu).unwrap()
    else {
        panic!("not unconfirmed: {wire:?}");
    };
    let params = &request.service_request;
    if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION {
        let notification = bacnet_encoding::constructed::decode_event_notification(params);
        return Told::Event(notification.unwrap().to_state);
    }
    let values = COVNotificationRequest::decode(params)
        .unwrap()
        .list_of_values;
    let flags = values
        .iter()
        .find(|value| value.property_identifier == P::STATUS_FLAGS)
        .expect("Status_Flags reported");
    Told::Cov(flags.value[2])
}

#[tokio::test(start_paused = true)]
async fn the_fault_pass_commits_its_transition_alike() {
    const IN_ALARM: u8 = 0x80;
    const FAULT: u8 = 0x40;
    let mut both = EventParity::start_with(faulting, true, None, true).await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: av(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(0),
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV;
    let (reply, sent) = both.request("subscribe", 100, service, &body, None).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(sent, 1, "the first report");
    // Out of range: a COV report now; only the fault pass judges the range.
    assert_eq!(both.value("out of range", 200, 150.0).await, 1);
    both.compared.clear();
    assert_eq!(both.run_to("the fault pass", 10_100).await, 2);
    let compared: Vec<_> = both.compared.drain(..).map(|wire| told(&wire)).collect();
    let faulted = [Told::Cov(IN_ALARM | FAULT), Told::Event(EventState::FAULT)];
    assert_eq!(compared, faulted);
    assert_eq!(both.value("back in range", 15_000, 50.0).await, 1);
    both.compared.clear();
    assert_eq!(both.run_to("the return", 20_100).await, 2);
    let compared: Vec<_> = both.compared.drain(..).map(|wire| told(&wire)).collect();
    assert_eq!(compared, [Told::Cov(0), Told::Event(EventState::NORMAL)]);
    assert_eq!(both.run_to("the sweeps after", 23_000).await, 0);
}
