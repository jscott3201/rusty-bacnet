//! An event notification to a Device recipient the server has no binding for
//! looks for the device with a targeted Who-Is and waits for its I-Am (#1368),
//! while the notification's other recipients are served at once.
//!
//! The harness is a running server whose AV-1 reports a high-limit alarm to
//! Notification Class 0. Writing 85 raises it, writing 50 clears it. The
//! class names Device 9, which the server holds no binding for, and a peer on
//! this network by address. The clock is paused, and the server's APDU
//! timeout, the probe's wait, is the default 3 seconds.
use crate::server::command_remote_write_tests::{deliver, device, disable_initiation};
use crate::server::cov_wire_test_support::*;
use crate::server::event_recipient_routing_tests::{address_recipient, destination_for};
use crate::server::remote_write_discovery_tests::{
    everywhere, i_am, next_who_is, targeted, who_is_sent,
};
use crate::server::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_types::constructed::BACnetRecipient;

/// How long a probe waits for the I-Am: the APDU timeout.
const WAIT: Duration = Duration::from_secs(3);
/// Device 9's MAC, from which it answers.
const DEVICE_9: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
/// The peer the class names by address.
const OTHER: [u8; 6] = [10, 0, 0, 7, 0xBA, 0xC0];

/// A server whose AV-1 alarms to Device 9, confirmed or not, as process 9,
/// and to [`OTHER`] as process 7.
async fn start(confirmed: bool) -> Harness {
    start_with(ServerConfig::default(), confirmed).await
}

/// [`start`] with `config`.
async fn start_with(config: ServerConfig, confirmed: bool) -> Harness {
    start_for(config, &[(9, confirmed)]).await
}

/// A server whose AV-1 alarms to each of `devices`, an instance and whether
/// it is sent confirmed, as the process numbered as the instance, then to
/// [`OTHER`] as process 7.
async fn start_for(config: ServerConfig, devices: &[(u32, bool)]) -> Harness {
    let devices = devices.to_vec();
    Harness::start_with(config, move |db| {
        let mut nc = bacnet_objects::notification_class::NotificationClass::new(0, "NC-0").unwrap();
        for (instance, confirmed) in devices {
            let mut to_device =
                destination_for(BACnetRecipient::Device(device(instance)), confirmed);
            to_device.process_identifier = instance;
            nc.add_destination(to_device).unwrap();
        }
        let mut to_other = destination_for(address_recipient(0, &OTHER), false);
        to_other.process_identifier = 7;
        nc.add_destination(to_other).unwrap();
        db.add(Box::new(nc)).unwrap();
        let object = db.get_mut(&av1()).unwrap();
        for (property, value) in [
            (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
            (PropertyIdentifier::LOW_LIMIT, 0.0),
            (PropertyIdentifier::DEADBAND, 1.0),
        ] {
            object
                .write_property(property, None, PropertyValue::Real(value), None)
                .unwrap();
        }
        for (property, unused_bits, bits) in [
            (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
            (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
        ] {
            object
                .write_property(
                    property,
                    None,
                    PropertyValue::BitString {
                        unused_bits,
                        data: vec![bits],
                    },
                    None,
                )
                .unwrap();
        }
    })
    .await
}

/// `(MAC, process identifier, confirmed)` of every event notification sent.
fn notifications(h: &Harness) -> Vec<(Vec<u8>, u32, bool)> {
    h.server
        .test_network()
        .transport()
        .sent()
        .frames()
        .into_iter()
        .filter_map(|frame| {
            let (request, confirmed) = match frame.apdu() {
                Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION =>
                {
                    (request.service_request, false)
                }
                Apdu::ConfirmedRequest(request)
                    if request.service_choice
                        == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION =>
                {
                    (request.service_request, true)
                }
                _ => return None,
            };
            let notification = decode_event_notification(&request).unwrap();
            Some((
                frame.mac.to_vec(),
                notification.process_identifier,
                confirmed,
            ))
        })
        .collect()
}

/// The To_State of each unconfirmed event notification sent to `mac`, in
/// the order they went out.
fn to_states(h: &Harness, mac: &[u8]) -> Vec<bacnet_types::enums::EventState> {
    h.server
        .test_network()
        .transport()
        .sent()
        .frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == mac)
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION =>
            {
                Some(
                    decode_event_notification(&request.service_request)
                        .unwrap()
                        .to_state,
                )
            }
            _ => None,
        })
        .collect()
}

/// `device_recipient_unbound` and `confirmed_unanswered`.
fn counted(h: &Harness) -> (u64, u64) {
    let counters = h.server.event_notification_counters();
    (
        counters.device_recipient_unbound,
        counters.confirmed_unanswered,
    )
}

#[tokio::test(start_paused = true)]
async fn an_unbound_recipient_that_answers_gets_the_notification_and_the_others_do_not_wait() {
    let mut h = start(false).await;
    h.write_local(85.0).await;
    // The other recipient has its notification at once, and Device 9 is
    // looked for everywhere, since the server has never heard from it.
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    assert_eq!(waiting(&h), 1);

    tokio::time::advance(WAIT / 2).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    assert_eq!(
        notifications(&h),
        vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, false)]
    );
    assert_eq!(counted(&h), (0, 0));
    // Drained: the queue gives its place back.
    assert_eq!(waiting(&h), 0);

    // The binding the I-Am made serves the next notification directly.
    h.write_local(50.0).await;
    h.settle().await;
    assert_eq!(notifications(&h).len(), 4);
    assert!(next_who_is_none(&h));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_silent_recipient_is_skipped_and_counted_once_with_one_who_is() {
    let mut h = start(false).await;
    h.write_local(85.0).await;
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    // Nothing is counted while the probe is out.
    tokio::time::sleep(WAIT - Duration::from_millis(1)).await;
    assert_eq!(counted(&h), (0, 0));
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(counted(&h), (1, 0));
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);

    // Within the minute after a Who-Is that drew nothing, the next
    // notification asks again for nothing: it skips Device 9 at once.
    h.write_local(50.0).await;
    h.settle().await;
    assert!(next_who_is_none(&h));
    assert_eq!(counted(&h), (2, 0));
    assert_eq!(notifications(&h).len(), 2);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_notification_after_the_i_am_counts_as_any_other() {
    let mut h = start(true).await;
    h.write_local(85.0).await;
    next_who_is(&h).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    assert_eq!(
        notifications(&h),
        vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, true)]
    );
    // Device 9 never acknowledges it: after the last retry it counts as
    // unanswered, not as a recipient with no binding.
    tokio::time::sleep(WAIT * 5).await;
    assert_eq!(notifications(&h).len(), 2 + 3);
    assert_eq!(counted(&h), (0, 1));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn nothing_is_sent_under_disable_initiation() {
    // Restricted from the start: no Who-Is, no notification.
    let mut h = start(false).await;
    disable_initiation(&h);
    h.write_local(85.0).await;
    h.settle().await;
    assert!(next_who_is_none(&h));
    assert!(notifications(&h).is_empty());
    assert_eq!(counted(&h), (0, 0));
    h.server.stop().await.unwrap();

    // Restricted while the notification waits: the I-Am binds the device,
    // but nothing goes to it and nothing is counted.
    let mut h = start(false).await;
    h.write_local(85.0).await;
    next_who_is(&h).await;
    disable_initiation(&h);
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    tokio::time::sleep(WAIT * 2).await;
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(counted(&h), (0, 0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn notifications_to_one_device_go_out_in_the_order_they_were_made() {
    use bacnet_types::enums::EventState;
    let mut h = start(false).await;
    // Two transitions made while Device 9 is looked for wait in its queue.
    h.write_local(85.0).await;
    next_who_is(&h).await;
    h.write_local(50.0).await;
    // The first goes out after the I-Am and is held in the transport there,
    // so the second still waits when a third transition is made.
    let transport = h.server.test_network().transport().handle();
    transport.block_next_send();
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    tokio::time::timeout(Duration::from_secs(1), transport.wait_blocked())
        .await
        .expect("the first waiting notification goes out");
    h.write_local(85.0).await;
    transport.release_sends(1);
    h.settle().await;
    let made = [
        EventState::HIGH_LIMIT,
        EventState::NORMAL,
        EventState::HIGH_LIMIT,
    ];
    assert_eq!(to_states(&h, &DEVICE_9), made);
    assert_eq!(to_states(&h, &OTHER), made);
    assert_eq!(waiting(&h), 0);
    assert!(next_who_is_none(&h));
    assert_eq!(counted(&h), (0, 0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn past_the_cap_a_notification_is_skipped_and_counted_at_once() {
    let mut h = start(false).await;
    // Notifications for Device 50 take every place.
    {
        let mut awaiting = super::awaiting(&h.server.notification_transactions);
        let filler = || super::Pending {
            process_id: 1,
            confirmed: false,
            request: Bytes::new(),
            notification_class: 0,
            priority: 255,
            admits: Arc::new(|_: &super::super::super::event_recipient_route::RecipientRoute| true),
            budget: None,
        };
        for _ in 0..super::MAX_AWAITING_NOTIFICATIONS {
            awaiting.enqueue(device(50), true, filler());
        }
        assert_eq!(awaiting.waiting(), super::MAX_AWAITING_NOTIFICATIONS);
    }
    h.write_local(85.0).await;
    h.settle().await;
    assert_eq!(counted(&h), (1, 0));
    // A notification that can only be skipped sends no Who-Is.
    assert!(next_who_is_none(&h));
    // The I-Am binds Device 9, but the skipped notification is gone.
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_wait_for_the_i_am_lasts_at_most_a_minute() {
    // An APDU timeout of ten hours still waits only a minute.
    let config = ServerConfig {
        cov_retry_timeout_ms: 10 * 60 * 60 * 1000,
        ..ServerConfig::default()
    };
    let mut h = start_with(config, false).await;
    h.write_local(85.0).await;
    next_who_is(&h).await;
    tokio::time::sleep(Duration::from_secs(60) - Duration::from_millis(1)).await;
    assert_eq!(counted(&h), (0, 0));
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(counted(&h), (1, 0));
    h.server.stop().await.unwrap();
}

/// Whether no Who-Is has been sent since the last one taken.
fn next_who_is_none(h: &Harness) -> bool {
    crate::server::remote_write_discovery_tests::who_is_sent(h).is_empty()
}

/// How many notifications wait for their devices, across every device.
fn waiting(h: &Harness) -> usize {
    super::awaiting(&h.server.notification_transactions).waiting()
}

#[tokio::test(start_paused = true)]
async fn stop_while_a_device_is_looked_for_closes_its_queue() {
    let mut h = start(false).await;
    let log = h.server.test_network().transport().sent();
    h.write_local(85.0).await;
    next_who_is(&h).await;
    assert_eq!(waiting(&h), 1);
    h.server.stop().await.unwrap();
    h.settle().await;
    // `stop()` aborts the queue's task, and the queue closes with it.
    assert_eq!(waiting(&h), 0);
    let sent = log.len();
    tokio::time::sleep(WAIT * 2).await;
    assert_eq!(log.len(), sent, "nothing is sent after stop()");
    assert!(
        log.frames()
            .iter()
            .all(|frame| frame.mac.as_slice() != DEVICE_9)
    );
}

#[tokio::test(start_paused = true)]
async fn each_device_drains_on_its_own() {
    let mut h = start_for(ServerConfig::default(), &[(9, false), (10, false)]).await;
    h.write_local(85.0).await;
    let looked_for: Vec<_> = who_is_sent(&h)
        .into_iter()
        .map(|(_, who_is)| who_is)
        .collect();
    assert_eq!(looked_for, [targeted(9), targeted(10)]);
    assert_eq!(waiting(&h), 2);
    // Device 9 answers halfway through the wait and has its notification at
    // once; Device 10, still looked for, holds nothing up.
    tokio::time::advance(WAIT / 2).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    assert_eq!(
        notifications(&h),
        vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, false)]
    );
    assert_eq!((counted(&h), waiting(&h)), ((0, 0), 1));
    // Device 10 stays silent and counts once.
    tokio::time::sleep(WAIT).await;
    assert_eq!((counted(&h), waiting(&h)), ((1, 0), 0));
    tokio::time::sleep(WAIT * 2).await;
    assert_eq!(counted(&h), (1, 0));
    assert_eq!(notifications(&h).len(), 2);
    h.server.stop().await.unwrap();
}
