//! A change back before a queued report goes (#2508): a change is measured
//! from the newest observation decided for its subscription, the queued or
//! in-flight report's when there is one, so the subscriber ends with the
//! current value. The newer report takes the queued one's place; a
//! confirmed subscription still waits for its Ack and a fresh look.

use super::*;
use crate::report::LocalValue;
use crate::write::local::LocalChange;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::traits::{BACnetObject, LocalInputValue};
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_types::enums::{BinaryPV, LifeSafetyState};

fn bi() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap()
}

fn lsp() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

fn av() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

/// AI-1 at 10.0 with COV_Increment `increment`, BI-1, LSP-1 and AV-1, a
/// commandable value.
fn device(increment: f32) -> Device {
    let identity = DeviceIdentity::new(9, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut sensor = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    sensor.set_present_value(10.0);
    sensor.set_cov_increment(increment).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(sensor),
        Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()),
        Box::new(AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap()),
    ];
    let db = build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::new(crate::cov::CovPolicy::default())
        .with_confirmed(FlightTiming::new(Duration::from_secs(3), 1));
    Device {
        reporter: Reporter::new(responder, config).unwrap(),
        db,
        ids: Ids {
            limit: 4,
            ..Ids::default()
        },
        invoke_id: 0,
    }
}

impl Device {
    /// The application gives `object` the Present_Value `value`.
    fn input(&mut self, object: ObjectIdentifier, value: PropertyValue) {
        let change = LocalChange::Input(LocalInputValue::present_value(value));
        self.reporter
            .apply_local(&mut self.db, object, change, T0)
            .unwrap();
    }

    /// Every notification queued now, reported sent: the Present_Value
    /// each carries.
    fn send_all(&mut self) -> Vec<PropertyValue> {
        let mut values = Vec::new();
        while let Some((sent, apdu)) = self.pop(T0) {
            values.push(Device::reported_pv(&apdu));
            self.reporter
                .delivered(sent.token, Delivery::Sent, &mut self.ids, T0);
        }
        values
    }

    /// A subscription to `object` whose first notification has been sent.
    fn subscribed(&mut self, object: ObjectIdentifier, process: u32, confirmed: bool) {
        self.subscribe_to(object, process, confirmed, T0);
        if confirmed {
            let (initial, _) = self.pop(T0).unwrap();
            assert!(self.ack(initial.invoke_id.unwrap(), T0));
        } else {
            assert_eq!(self.send_all().len(), 1);
        }
    }
}

fn real(value: f32) -> PropertyValue {
    PropertyValue::Real(value)
}

/// AI-1 goes to 15.0 and back to 10.0 before the host sends: the subscriber
/// hears 10.0, once, and the baseline is 10.0.
#[test]
fn an_analog_change_back_sends_the_current_value() {
    let mut device = device(0.0);
    device.subscribed(ai(), 1, false);
    device.input(ai(), real(15.0));
    device.input(ai(), real(10.0));
    assert_eq!(device.send_all(), [real(10.0)], "the newer took its place");
    device.reporter.changed(&device.db, &[ai()], T0);
    assert!(device.pop(T0).is_none(), "10.0 is the baseline");
}

/// The change back comes while the first report is already out, handed to
/// the host and not yet reported sent: both go, newest last.
#[test]
fn a_change_back_while_a_report_is_out_follows_it() {
    let mut device = device(0.0);
    device.subscribed(ai(), 1, false);
    device.input(ai(), real(15.0));
    let (out, apdu) = device.pop(T0).unwrap();
    assert_eq!(Device::reported_pv(&apdu), real(15.0));
    device.input(ai(), real(10.0));
    device
        .reporter
        .delivered(out.token, Delivery::Sent, &mut device.ids, T0);
    assert_eq!(device.send_all(), [real(10.0)]);
}

/// With COV_Increment 2.0 a change is measured from the queued report's
/// value: 15.0 then 14.0 sends 15.0, within the increment of the object;
/// from 15.0 delivered, 18.0 then 15.5 sends 15.5 in 18.0's place, though
/// 15.5 is within the increment of what was delivered.
#[test]
fn the_increment_is_measured_from_the_queued_report() {
    let mut device = device(2.0);
    device.subscribed(ai(), 1, false);
    device.input(ai(), real(15.0));
    device.input(ai(), real(14.0));
    assert_eq!(device.send_all(), [real(15.0)]);
    device.input(ai(), real(18.0));
    device.input(ai(), real(15.5));
    assert_eq!(device.send_all(), [real(15.5)]);
}

/// BI-1 goes ACTIVE and straight back: the subscriber hears INACTIVE.
#[test]
fn a_binary_change_back_sends_the_current_state() {
    let mut device = device(0.0);
    device.subscribed(bi(), 1, false);
    let state = |state: BinaryPV| PropertyValue::Enumerated(state.to_raw());
    let value = |state: BinaryPV| LocalValue::Binary(state);
    for step in [BinaryPV::ACTIVE, BinaryPV::INACTIVE] {
        device
            .reporter
            .update(&mut device.db, bi(), value(step), T0)
            .unwrap();
    }
    assert_eq!(device.send_all(), [state(BinaryPV::INACTIVE)]);
}

/// A property subscription to AI-1's Present_Value ends current too.
#[test]
fn a_property_subscription_change_back_sends_the_current_value() {
    let mut device = device(0.0);
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: ai(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: P::PRESENT_VALUE,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
    let reply = device.request(service, &body, T0);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(device.send_all().len(), 1);
    device.input(ai(), real(15.0));
    device.input(ai(), real(10.0));
    assert_eq!(device.send_all(), [real(10.0)]);
}

/// LSP-1 goes to ALARM and back to QUIET: its captured change reports
/// QUIET in ALARM's place.
#[test]
fn a_life_safety_change_back_sends_the_current_state() {
    let mut device = device(0.0);
    device.subscribed(lsp(), 1, false);
    let state = |state: LifeSafetyState| PropertyValue::Enumerated(state.to_raw());
    device.input(lsp(), state(LifeSafetyState::ALARM));
    device.input(lsp(), state(LifeSafetyState::QUIET));
    assert_eq!(device.send_all(), [state(LifeSafetyState::QUIET)]);
}

/// A report a change was measured from that ends unsent owes that change a
/// fresh look, from what was delivered; one nothing was measured from owes
/// nothing, so a failing link isn't retried in a loop.
#[test]
fn an_unsent_report_a_change_relied_on_owes_a_look() {
    let mut device = device(2.0);
    device.subscribed(ai(), 1, false);
    device.input(ai(), real(15.0));
    let (out, _) = device.pop(T0).unwrap();
    // Within the increment of 15.0, the report out: nothing new.
    device.input(ai(), real(14.0));
    assert!(device.pop(T0).is_none());
    device
        .reporter
        .delivered(out.token, Delivery::NotSent, &mut device.ids, T0);
    assert_eq!(device.reporter.next_deadline(), Some(T0), "a look is owed");
    device.deadline();
    let (again, apdu) = device.pop(T0).unwrap();
    assert_eq!(
        Device::reported_pv(&apdu),
        real(14.0),
        "from 10.0 delivered"
    );
    // This one fails too, and no change was measured from it.
    device
        .reporter
        .delivered(again.token, Delivery::NotSent, &mut device.ids, T0);
    assert!(device.reporter.next_deadline().is_none_or(|due| due > T0));
}

/// A confirmed subscription keeps its rule: a change back while its report
/// is out waits, and the Ack's look reports the current value.
#[test]
fn a_confirmed_change_back_follows_the_ack() {
    let mut device = device(0.0);
    device.subscribed(ai(), 1, true);
    device.input(ai(), real(15.0));
    let (out, apdu) = device.pop(T0).unwrap();
    assert_eq!(Device::reported_pv(&apdu), real(15.0));
    device.input(ai(), real(10.0));
    assert!(device.pop(T0).is_none(), "held behind the report out");
    assert!(device.ack(out.invoke_id.unwrap(), T0));
    device.deadline();
    let (_, apdu) = device.pop(T0).unwrap();
    assert_eq!(Device::reported_pv(&apdu), real(10.0));
}

/// The comparison is with the newest report still to go, not the oldest:
/// 15.0 is out, 10.0 queued behind it, then back to 15.0. Measured from the
/// queued 10.0, 15.0 takes its place, and the subscriber ends at 15.0, not
/// at 10.0 once the first send ends.
#[test]
fn a_change_is_measured_from_the_newest_report_still_to_go() {
    let mut device = device(0.0);
    device.subscribed(ai(), 1, false);
    device.input(ai(), real(15.0));
    let (out, _) = device.pop(T0).unwrap();
    device.input(ai(), real(10.0));
    device.input(ai(), real(15.0));
    device
        .reporter
        .delivered(out.token, Delivery::Sent, &mut device.ids, T0);
    assert_eq!(device.send_all(), [real(15.0)]);
}

/// A property subscription to a commandable value's Value_Source (Table
/// 13-1a-2): a command at 8 and its relinquish before the host sends end
/// where the subscriber started, with the commanding priority and the
/// source the first notification gave.
#[test]
fn a_value_source_change_back_sends_the_current_source() {
    let mut device = device(0.0);
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: av(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: P::VALUE_SOURCE,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
    let reply = device.request(service, &body, T0);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    let source = |device: &mut Device| {
        let mut sent = Vec::new();
        while let Some((notification, apdu)) = device.pop(T0) {
            let Apdu::UnconfirmedRequest(request) = apdu else {
                panic!("unconfirmed")
            };
            let values = COVNotificationRequest::decode(&request.service_request)
                .unwrap()
                .list_of_values;
            let wanted = [P::VALUE_SOURCE, P::CURRENT_COMMAND_PRIORITY];
            sent.push(
                values
                    .into_iter()
                    .filter(|value| wanted.contains(&value.property_identifier))
                    .map(|value| value.value)
                    .collect::<Vec<_>>(),
            );
            device
                .reporter
                .delivered(notification.token, Delivery::Sent, &mut device.ids, T0);
        }
        sent
    };
    let initial = source(&mut device);
    assert_eq!(initial.len(), 1);
    device
        .reporter
        .command(&mut device.db, av(), 8, LocalValue::Real(5.0), T0)
        .unwrap();
    device
        .reporter
        .relinquish(&mut device.db, av(), 8, T0)
        .unwrap();
    assert_eq!(source(&mut device), initial, "back where it started");
}
