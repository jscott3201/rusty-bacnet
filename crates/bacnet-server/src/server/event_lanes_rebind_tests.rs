//! A Device recipient's binding that lapses or moves while its confirmed
//! notification waits for an invoke ID (#2569). Once an ID comes back the
//! binding is read again: one an I-Am refreshed meanwhile still takes the
//! notification; one gone is looked for with a targeted Who-Is, as at a
//! first send, the ID given back while it looks, and counts once in
//! `device_recipient_unbound` when the device stays silent, or takes the
//! notification when it answers; one that moved takes it at its new
//! address, unless that address is a broadcast form on another network,
//! which no confirmed request goes to (#2732).
//!
//! Each test binds Device 9 from its I-Am, leases every invoke ID, and has
//! AV-1 alarm confirmed to Device 9 (unconfirmed to the peer named by
//! address), so the alarm waits for an ID.

use super::*;
use crate::server::device_bindings::OBSERVED_BINDING_TTL;
use crate::server::notification_transactions::NotificationReservation;

/// The MAC Device 9 answers from once it has moved.
const MOVED: [u8; 6] = [10, 0, 0, 19, 0xBA, 0xC0];

/// Confirmed notifications that waited for an invoke ID.
fn waited(h: &Harness) -> u64 {
    h.server
        .event_notification_counters()
        .confirmed_waited_for_invoke_id
}

/// A confirmed notification to Device 9 at `mac` was sent.
fn sent_confirmed_to(h: &Harness, mac: &[u8]) -> bool {
    notifications(h).contains(&(mac.to_vec(), 9, true))
}

/// The server as above, AV-1's alarm waiting for an ID, and the leases.
async fn waiting_for_an_id() -> (Harness, Vec<NotificationReservation>) {
    let mut h = start(true).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    let leases = h.server.notification_transactions.lease_every_invoke_id();
    h.write_pv(85.0, 0).await;
    until("the wait for an ID", || waited(&h) == 1).await;
    (h, leases)
}

/// The binding lapses during the wait and the device's I-Am refreshes it
/// before an ID comes back: the notification goes to it, nothing counted,
/// with no Who-Is.
#[tokio::test(start_paused = true)]
async fn a_binding_refreshed_during_the_wait_still_takes_the_notification() {
    let (mut h, mut leases) = waiting_for_an_id().await;
    tokio::time::sleep(OBSERVED_BINDING_TTL).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    drop(leases.pop());
    until("the notification", || sent_confirmed_to(&h, &DEVICE_9)).await;
    assert_eq!(counted(&h), (0, 0));
    assert!(next_who_is_none(&h));
    drop(leases);
    h.server.stop().await.unwrap();
}

/// The binding lapses during the wait and stays gone: once an ID comes
/// back, the device is looked for where it was last seen, the ID given back
/// meanwhile, and, silent, counts once, with nothing sent to it.
#[tokio::test(start_paused = true)]
async fn a_binding_still_gone_is_looked_for_and_counted_once() {
    let (mut h, mut leases) = waiting_for_an_id().await;
    tokio::time::sleep(OBSERVED_BINDING_TTL).await;
    drop(leases.pop());
    assert_eq!(next_who_is(&h).await, (None, targeted(9)));
    let transactions = &h.server.notification_transactions;
    assert_eq!(transactions.active_count(), 255, "its ID given back");
    tokio::time::sleep(WAIT).await;
    h.settle().await;
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(counted(&h), (1, 0));
    assert_eq!(open_lanes(&h), 0);
    drop(leases);
    h.server.stop().await.unwrap();
}

/// The binding lapses during the wait, and the Who-Is sent once an ID comes
/// back draws the I-Am: the notification goes to the device.
#[tokio::test(start_paused = true)]
async fn a_lapsed_binding_that_answers_the_who_is_takes_the_notification() {
    let (mut h, mut leases) = waiting_for_an_id().await;
    tokio::time::sleep(OBSERVED_BINDING_TTL).await;
    drop(leases.pop());
    assert_eq!(next_who_is(&h).await, (None, targeted(9)));
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    until("the notification", || sent_confirmed_to(&h, &DEVICE_9)).await;
    assert_eq!(counted(&h), (0, 0));
    drop(leases);
    h.server.stop().await.unwrap();
}

/// Device 9 answers from another MAC during the wait: the notification goes
/// there, not to the old address, with no Who-Is.
#[tokio::test(start_paused = true)]
async fn a_binding_moved_during_the_wait_takes_the_notification_there() {
    let (mut h, mut leases) = waiting_for_an_id().await;
    deliver(&h, &i_am(9), &MOVED, None).await;
    h.settle().await;
    drop(leases.pop());
    until("the notification", || sent_confirmed_to(&h, &MOVED)).await;
    assert!(!sent_confirmed_to(&h, &DEVICE_9));
    assert_eq!(counted(&h), (0, 0));
    assert!(next_who_is_none(&h));
    drop(leases);
    h.server.stop().await.unwrap();
}

/// Device 9 answers during the wait through a router from `5:0xFF`, a
/// broadcast form there: the address actually used is checked, so the
/// confirmed notification is skipped and counted, its ID given back.
#[tokio::test(start_paused = true)]
async fn a_binding_moved_to_a_broadcast_form_is_skipped_and_counted() {
    let (mut h, mut leases) = waiting_for_an_id().await;
    let station = NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(&[0xFF]),
    };
    deliver(&h, &i_am(9), &MOVED, Some(station)).await;
    h.settle().await;
    drop(leases.pop());
    let skipped = || {
        h.server
            .event_notification_counters()
            .confirmed_broadcast_recipient
            == 1
    };
    until("the skip", skipped).await;
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(counted(&h), (0, 0));
    let transactions = &h.server.notification_transactions;
    assert_eq!(transactions.active_count(), 255, "its ID given back");
    drop(leases);
    h.server.stop().await.unwrap();
}

/// The confirmed event notifications first sent to `mac`: one per invoke
/// ID, retries left out.
fn first_sends_to(h: &Harness, mac: &[u8]) -> usize {
    let log = h.server.test_network().transport().sent();
    let mut invoke_ids = Vec::new();
    for frame in log.frames() {
        if let Apdu::ConfirmedRequest(request) = frame.apdu()
            && request.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
            && frame.mac.as_slice() == mac
            && !invoke_ids.contains(&request.invoke_id)
        {
            invoke_ids.push(request.invoke_id);
        }
    }
    invoke_ids.len()
}

/// Device 9's binding lapses while its alarm waits for an ID, Device 10's
/// alarm waiting behind it. The first ID to come back goes to Device 9's,
/// which gives it back to look for the device, and Device 10's takes it.
/// Device 9 answers, and its alarm waits again, uncounted, since its wait
/// was counted once. It moves while it waits, but the binding is read again
/// once only: the alarm goes where it answered first, with no third look.
#[tokio::test(start_paused = true)]
async fn a_rerouted_notification_waits_again_uncounted_and_once() {
    let mut h = start_for_and(ServerConfig::default(), &[(9, true)], &[], |db| {
        let mut nc = bacnet_objects::notification_class::NotificationClass::new(1, "NC-1").unwrap();
        let mut to_device = destination_for(BACnetRecipient::Device(device(10)), true);
        to_device.process_identifier = 10;
        nc.add_destination(to_device).unwrap();
        db.add(Box::new(nc)).unwrap();
        add_av2(db, 1);
    })
    .await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    let transactions = Arc::clone(&h.server.notification_transactions);
    let mut leases = transactions.lease_every_invoke_id();
    h.write_pv(85.0, 0).await;
    until("Device 9's wait", || waited(&h) == 1).await;
    tokio::time::sleep(OBSERVED_BINDING_TTL).await;
    tokio::join!(h.write_local_to(av2(), 85.0), async {
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(10)));
        deliver(&h, &i_am(10), &DEVICE_10, None).await;
        until("Device 10's wait", || waited(&h) == 2).await;
    });
    drop(leases.pop());
    until("Device 10's alarm", || first_sends_to(&h, &DEVICE_10) == 1).await;
    assert_eq!(next_who_is(&h).await, (None, targeted(9)));
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    let second_wait = || transactions.waiting_for_ids() == 1;
    until("Device 9's second wait", second_wait).await;
    deliver(&h, &i_am(9), &MOVED, None).await;
    h.settle().await;
    drop(leases.pop());
    until("Device 9's alarm", || first_sends_to(&h, &DEVICE_9) == 1).await;
    assert_eq!(first_sends_to(&h, &MOVED), 0, "no third look");
    assert_eq!(waited(&h), 2, "Device 9's wait counted once");
    assert_eq!(counted(&h), (0, 0));
    assert!(next_who_is_none(&h));
    drop(leases);
    h.server.stop().await.unwrap();
}
