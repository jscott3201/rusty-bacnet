//! A confirmed notification that finds every invoke ID leased (#2537): it
//! waits at the head of its lane, the destination's later entries behind it,
//! until an ID comes back, while every other destination's lane goes on. The
//! wait misses no release, a full lane still drops its newest, `stop()` and
//! the drop end it, the writer's COV fanout waits no longer than its cap, and
//! a worker that panics gives its ID back.

use super::wire::{class_0, subscribe_cov};
use super::*;
use crate::server::notification_transactions::{NotificationReservation, canonical_direct_peer};
use bacnet_types::bitstring::EventTransitionBits;

/// A destination whose confirmed notifications wait for an invoke ID.
const HELD: [u8; 6] = [10, 0, 0, 11, 0xBA, 0xC0];

/// Every invoke ID, leased to another peer.
fn lease_every_id(transactions: &NotificationTransactions) -> Vec<NotificationReservation> {
    transactions.lease_every_invoke_id()
}

/// Confirmed notifications that waited for an invoke ID, so far.
fn waited(rig: &Rig) -> u64 {
    rig.server
        .event_notification_counters()
        .confirmed_waited_for_invoke_id
}

/// An Address recipient at `mac`, confirmed or not, for `transitions`.
fn to_mac(mac: &[u8], confirmed: bool, transitions: EventTransitionBits) -> BACnetDestination {
    BACnetDestination {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(mac),
        }),
        issue_confirmed_notifications: confirmed,
        transitions,
        ..local_broadcast_destination()
    }
}

/// The event notifications first sent to `mac`, in order: confirmed or not,
/// and the To State. A confirmed one's retries, under the invoke ID of its
/// first send, are left out.
fn sent_to(rig: &Rig, mac: &[u8]) -> Vec<(bool, u32)> {
    let mut first_sends = Vec::new();
    let mut invoke_ids = Vec::new();
    for frame in rig.sent.frames() {
        if frame.mac.as_slice() != mac {
            continue;
        }
        match frame.apdu() {
            Apdu::ConfirmedRequest(request)
                if request.service_choice
                    == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION =>
            {
                if invoke_ids.contains(&request.invoke_id) {
                    continue;
                }
                invoke_ids.push(request.invoke_id);
                let n = decode_event_notification(&request.service_request).unwrap();
                first_sends.push((true, n.to_state.to_raw()));
            }
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION =>
            {
                let n = decode_event_notification(&request.service_request).unwrap();
                first_sends.push((false, n.to_state.to_raw()));
            }
            _ => {}
        }
    }
    first_sends
}

/// Wait, on the paused clock, until `count` notifications were first sent
/// to `mac`; what was sent then.
async fn wait_sent_to(rig: &Rig, mac: &[u8], count: usize) -> Vec<(bool, u32)> {
    for _ in 0..2_000 {
        let sent = sent_to(rig, mac);
        if sent.len() >= count {
            return sent;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    sent_to(rig, mac)
}

/// With every invoke ID leased, the next confirmed notification waits at its
/// lane's head, uncounted, and a later unconfirmed one to the same
/// destination waits behind it; the broadcast destination's lane goes on.
/// One ID coming back sends the held entry, then the rest, in commit order.
#[tokio::test(start_paused = true)]
async fn a_confirmed_notification_with_no_invoke_id_waits_at_its_lanes_head() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        class_0(
            db,
            vec![
                local_broadcast_destination(),
                to_mac(&HELD, true, EventTransitionBits::TO_FAULT),
                to_mac(&HELD, false, EventTransitionBits::TO_NORMAL),
            ],
        );
    })
    .await;
    let (spy, _guard) = LaneLockSpy::install(&rig.server.notification_transactions);
    let mut leases = lease_every_id(&rig.server.notification_transactions);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    assert_eq!(
        rig.wait_wire(2).await,
        [ev(ai1(), FAULT), ev(ai1(), NORMAL)],
        "the broadcast lane goes on"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sent_to(&rig, &HELD), [], "the held lane waits");
    assert_eq!(
        lanes(&rig.server.notification_transactions).waiting(),
        1,
        "the unconfirmed one waits behind the held head"
    );
    assert_eq!(waited(&rig), 1, "it waits, counted once");
    drop(leases.pop());
    assert_eq!(
        wait_sent_to(&rig, &HELD, 2).await,
        [(true, FAULT), (false, NORMAL)],
        "the held entry first, then the one behind it"
    );
    spy.assert_logged_unlocked(1);
}

/// A release that races the start of the wait is never lost: the wait is
/// registered before each attempt, so a lease that comes back after the
/// attempt fails still ends it. Looped on two worker threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_release_racing_the_start_of_the_wait_still_ends_it() {
    let transactions = NotificationTransactions::new();
    for round in 0..500 {
        let mut leases = lease_every_id(&transactions);
        let waiter = tokio::spawn({
            let transactions = Arc::clone(&transactions);
            async move {
                transactions
                    .reserve_waiting(
                        canonical_direct_peer(&OTHER),
                        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                        || {},
                    )
                    .await
                    .is_ok()
            }
        });
        let release = tokio::spawn(async move {
            drop(leases.pop());
            leases
        });
        let leases = release.await.unwrap();
        let reserved = tokio::time::timeout(Duration::from_secs(5), waiter).await;
        assert!(
            matches!(reserved, Ok(Ok(true))),
            "round {round}: the wait missed the release"
        );
        drop(leases);
    }
}

/// A lane held for an invoke ID still fills at 64 behind its head: the next
/// one is dropped and counted.
#[tokio::test(start_paused = true)]
async fn a_lane_held_for_an_invoke_id_still_fills_and_drops_the_newest() {
    let config = ServerConfig {
        cov_retry_timeout_ms: 20,
        ..ServerConfig::default()
    };
    let rig = Rig::start_with(config, |db| {
        ai_objects(db);
        class_0(db, vec![to_mac(&HELD, true, EventTransitionBits::all())]);
    })
    .await;
    let (spy, _guard) = LaneLockSpy::install(&rig.server.notification_transactions);
    let _leases = lease_every_id(&rig.server.notification_transactions);
    for flip in 0..MAX_LANE_ENTRIES + 2 {
        let reliability = if flip % 2 == 0 {
            Reliability::NO_SENSOR
        } else {
            Reliability::NO_FAULT_DETECTED
        };
        rig.reliability(reliability).await;
        // Past each request's capped COV wait: one peer has 16 slots.
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let counters = rig.server.event_notification_counters();
    assert_eq!(
        counters.lane_full, 1,
        "the head held, 64 behind it, one more"
    );
    assert_eq!(counters.confirmed_waited_for_invoke_id, 1, "the head");
    assert_eq!(
        lanes(&rig.server.notification_transactions).waiting(),
        MAX_LANE_ENTRIES
    );
    assert_eq!(sent_to(&rig, &HELD), []);
    spy.assert_logged_unlocked(1);
}

/// `stop()` with a lane held for an invoke ID returns at once, its writer's
/// wait ended, and nothing goes out once IDs come back.
#[tokio::test(start_paused = true)]
async fn stop_ends_a_lane_held_for_an_invoke_id() {
    let mut rig = Rig::start(|db| {
        ai_objects(db);
        class_0(db, vec![to_mac(&HELD, true, EventTransitionBits::all())]);
    })
    .await;
    let leases = lease_every_id(&rig.server.notification_transactions);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(lanes(&rig.server.notification_transactions).open(), 1);
    assert_eq!(waited(&rig), 1, "held for an ID");
    let started = tokio::time::Instant::now();
    rig.server.stop().await.unwrap();
    // The writers' COV waits run to 3 s; sealing ends them first.
    assert!(started.elapsed() < Duration::from_secs(1), "stop waited");
    drop(leases);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sent_to(&rig, &HELD), [], "nothing sent after stop");
}

/// A server dropped with a lane held for an invoke ID: the lane and its task
/// go, and nothing goes out once IDs come back.
#[tokio::test(start_paused = true)]
async fn a_drop_ends_a_lane_held_for_an_invoke_id() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        class_0(db, vec![to_mac(&HELD, true, EventTransitionBits::all())]);
    })
    .await;
    let transactions = Arc::clone(&rig.server.notification_transactions);
    let leases = lease_every_id(&transactions);
    rig.reliability(Reliability::NO_SENSOR).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(lanes(&transactions).open(), 1);
    assert_eq!(waited(&rig), 1, "held for an ID");
    let Rig { server, sent, .. } = rig;
    drop(server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let lanes = lanes(&transactions);
        assert_eq!((lanes.open(), lanes.tasks.len()), (0, 0), "no lane left");
    }
    drop(leases);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let to_held = sent
        .frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == HELD)
        .count();
    assert_eq!(to_held, 0, "nothing sent after the drop");
}

/// The writer of a notification held for an invoke ID fans out its COV once
/// its capped wait runs out, and the wait is counted.
#[tokio::test(start_paused = true)]
async fn the_cov_wait_expires_while_a_notification_waits_for_an_invoke_id() {
    let config = ServerConfig {
        cov_retry_timeout_ms: 300,
        ..ServerConfig::default()
    };
    let rig = Rig::start_with(config, |db| {
        ai_objects(db);
        class_0(db, vec![to_mac(&HELD, true, EventTransitionBits::all())]);
    })
    .await;
    subscribe_cov(&rig).await;
    let mut leases = lease_every_id(&rig.server.notification_transactions);
    let started = tokio::time::Instant::now();
    rig.reliability(Reliability::NO_SENSOR).await;
    assert_eq!(rig.wait_wire(1).await, [Sent::Cov(ai1())], "past the cap");
    assert!(started.elapsed() >= Duration::from_millis(300), "it waited");
    let counters = rig.server.event_notification_counters();
    assert_eq!(counters.cov_wait_expired, 1);
    assert_eq!(sent_to(&rig, &HELD), []);
    drop(leases.pop());
    assert_eq!(wait_sent_to(&rig, &HELD, 1).await, [(true, FAULT)]);
}

/// DCC restricting initiation while a notification waits for an invoke ID:
/// once one comes back the notification is withdrawn, uncounted, as any send
/// DCC holds back, and the ID is given back.
#[tokio::test(start_paused = true)]
async fn dcc_withdraws_a_notification_that_waited_for_an_invoke_id() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        class_0(db, vec![to_mac(&HELD, true, EventTransitionBits::all())]);
    })
    .await;
    let transactions = Arc::clone(&rig.server.notification_transactions);
    let mut leases = lease_every_id(&transactions);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.server
        .comm_state
        .set_for_test(crate::server::DccState::DisableInitiation);
    drop(leases.pop());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sent_to(&rig, &HELD), [], "withdrawn");
    assert_eq!(lanes(&transactions).open(), 0, "the lane woke and closed");
    assert_eq!(transactions.active_count(), 255, "its ID given back");
    assert_eq!(
        rig.server.event_notification_counters(),
        EventNotificationCounters {
            confirmed_waited_for_invoke_id: 1,
            ..EventNotificationCounters::default()
        }
    );
}

/// A notification worker that panics in its send gives its invoke ID back,
/// which wakes the lane waiting for one: with one ID free for two
/// destinations, the second still goes out.
#[tokio::test(start_paused = true)]
async fn a_worker_that_panics_gives_its_invoke_id_back() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        class_0(
            db,
            vec![
                to_mac(&OTHER, true, EventTransitionBits::all()),
                to_mac(&HELD, true, EventTransitionBits::all()),
            ],
        );
    })
    .await;
    let (spy, _guard) = LaneLockSpy::install(&rig.server.notification_transactions);
    let mut leases = lease_every_id(&rig.server.notification_transactions);
    drop(leases.pop());
    *rig.probe.panic_on.lock().unwrap() = Some(FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    // One destination took the free ID and its worker's send panicked; the
    // other waited for that ID and then sent.
    let other = wait_sent_to(&rig, &OTHER, 1).await;
    let held = wait_sent_to(&rig, &HELD, 1).await;
    assert_eq!((other, held), (vec![(true, FAULT)], vec![(true, FAULT)]));
    assert_eq!(*rig.probe.panic_on.lock().unwrap(), None, "a send panicked");
    assert_eq!(waited(&rig), 1, "the other destination waited");
    spy.assert_logged_unlocked(1);
}
