//! The tail every change runs (#2524): a request's write evaluates
//! intrinsic reporting as the host's change does, and a change's event
//! notifications go ahead of its COV report (#889).

use super::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_types::enums::EventState;

/// The next notification is an event notification to `state`, and the one
/// after it the COV report of the same change, carrying `value`.
fn assert_event_then_cov(device: &mut Device, state: EventState, value: f32, now: Instant) {
    let (event, apdu) = device.pop(now).expect("the event notification");
    assert_eq!(event.service, NotifyService::Event, "{apdu:?}");
    let params = match &apdu {
        Apdu::UnconfirmedRequest(request) => &request.service_request,
        other => panic!("not an unconfirmed notification: {other:?}"),
    };
    let notification = decode_event_notification(params).unwrap();
    assert_eq!(notification.to_state, state);
    let (cov, apdu) = device.pop(now).expect("the COV report after it");
    assert_eq!(cov.service, NotifyService::Cov);
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(value));
    assert!(device.pop(now).is_none());
}

/// A device sweeping every ten seconds, whose AI-3 has no Time_Delay and
/// one unconfirmed COV subscriber, its initial report sent.
fn subscribed() -> Device {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(false)],
        10,
    );
    device.subscribe_to(alarm(), 1, false, ms(10));
    let (initial, _) = device.pop(ms(10)).expect("the initial report");
    assert_eq!(initial.service, NotifyService::Cov);
    device
}

#[test]
fn a_write_request_evaluates_intrinsic_reporting_at_once() {
    let mut device = subscribed();
    // High_Limit under the reading: the write commits the transition, not
    // the sweep ten seconds on.
    device.write_to(alarm(), P::HIGH_LIMIT, PropertyValue::Real(5.0), ms(100));
    assert_eq!(device.reporter.next_deadline(), Some(ms(10_000)));
    assert_event_then_cov(&mut device, EventState::HIGH_LIMIT, 10.0, ms(100));
}

#[test]
fn a_host_change_queues_its_event_notification_ahead_of_its_cov_report() {
    let mut device = subscribed();
    device.raise(alarm(), ms(100));
    assert_event_then_cov(&mut device, EventState::HIGH_LIMIT, 90.0, ms(100));
}
