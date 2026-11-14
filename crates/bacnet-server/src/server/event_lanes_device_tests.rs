//! A Device recipient the server holds no fresh binding for (#1368): its lane
//! looks for the device with a targeted Who-Is and waits, the notification at
//! its head and the device's later ones behind it in commit order, until the
//! I-Am binds the device or the wait runs out (#2537). Other destinations go
//! on, the writer's COV fanout waits for the I-Am no longer than its cap, a
//! device that stays silent drops everything that waited for it, and
//! `stop()`, the drop and DCC end the wait.
//!
//! The harness is a running server whose AV-1 reports a high-limit alarm to
//! Notification Class 0: writing 85 raises it, writing 50 clears it. The class
//! names Device 9, which the server holds no binding for, and a peer on this
//! network by address. The clock is paused, and the server's APDU timeout, the
//! probe's wait and the writer's cap alike, is the default 3 seconds.

use super::super::{
    LaneEntry, LanePayload, MAX_LANE_ENTRIES, MAX_LANED_ENTRIES, OwnedDelivery, lanes,
};
use super::LaneLockSpy;
use crate::server::command_remote_write_tests::{deliver, device, disable_initiation};
use crate::server::cov_wire_test_support::*;
use crate::server::device_bindings::{DeviceResolution, OBSERVED_BINDING_TTL, ObservationOutcome};
use crate::server::event_recipient_routing_tests::{address_recipient, destination_for};
use crate::server::remote_write_discovery_tests::{
    everywhere, i_am, next_who_is, targeted, who_is_sent,
};
use crate::server::test_transport::TestTransport;
use crate::server::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{EnableDisable, EngineeringUnits, EventState, EventType};
use tokio::time::Instant;

/// How long a probe waits for the I-Am, and a writer for its notifications:
/// the APDU timeout.
const WAIT: Duration = Duration::from_secs(3);
/// Device 9's MAC, from which it answers.
const DEVICE_9: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
/// Device 10's MAC, from which it answers.
const DEVICE_10: [u8; 6] = [10, 0, 0, 10, 0xBA, 0xC0];
/// The peer the class names by address.
const OTHER: [u8; 6] = [10, 0, 0, 7, 0xBA, 0xC0];

/// A server whose AV-1 alarms to Device 9, confirmed or not, and to
/// [`OTHER`].
async fn start(confirmed: bool) -> Harness {
    start_with(ServerConfig::default(), confirmed).await
}

/// [`start`] with `config`.
async fn start_with(config: ServerConfig, confirmed: bool) -> Harness {
    start_for(config, &[(9, confirmed)], &[OTHER]).await
}

/// A server whose AV-1 alarms to each of `devices` (an instance and whether
/// it is sent confirmed, as the process numbered as the instance), then to
/// each of `peers` by address, unconfirmed, as process 7.
async fn start_for(config: ServerConfig, devices: &[(u32, bool)], peers: &[[u8; 6]]) -> Harness {
    start_for_and(config, devices, peers, |_| {}).await
}

/// [`start_for`], then `extend` on the database.
async fn start_for_and(
    config: ServerConfig,
    devices: &[(u32, bool)],
    peers: &[[u8; 6]],
    extend: impl FnOnce(&mut ObjectDatabase) + Send + 'static,
) -> Harness {
    let devices = devices.to_vec();
    let peers = peers.to_vec();
    Harness::start_with(config, move |db| {
        let mut nc = bacnet_objects::notification_class::NotificationClass::new(0, "NC-0").unwrap();
        for (instance, confirmed) in devices {
            let mut to_device =
                destination_for(BACnetRecipient::Device(device(instance)), confirmed);
            to_device.process_identifier = instance;
            nc.add_destination(to_device).unwrap();
        }
        for peer in peers {
            let mut to_peer = destination_for(address_recipient(0, &peer), false);
            to_peer.process_identifier = 7;
            nc.add_destination(to_peer).unwrap();
        }
        db.add(Box::new(nc)).unwrap();
        alarms_above_80(db, av1());
        extend(db);
    })
    .await
}

/// Make `object`, an Analog Value, report a high-limit alarm above 80.
fn alarms_above_80(db: &mut ObjectDatabase, object: ObjectIdentifier) {
    let object = db.get_mut(&object).unwrap();
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
        let value = PropertyValue::BitString {
            unused_bits,
            data: vec![bits],
        };
        object.write_property(property, None, value, None).unwrap();
    }
}

fn av2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
}

/// AV-2, alarming above 80 through Notification Class `class`.
fn add_av2(db: &mut ObjectDatabase, class: u32) {
    let mut av = AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let class = PropertyValue::Unsigned(class.into());
    BACnetObject::write_property(
        &mut av,
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        class,
        None,
    )
    .unwrap();
    db.add(Box::new(av)).unwrap();
    alarms_above_80(db, av2());
}

/// `(MAC, process identifier, confirmed)` of every event notification sent.
fn notifications(h: &Harness) -> Vec<(Vec<u8>, u32, bool)> {
    let log = h.server.test_network().transport().sent();
    log.frames()
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

/// The object and To_State of each unconfirmed event notification sent to
/// `mac`, in the order they went out.
fn sent_to(h: &Harness, mac: &[u8]) -> Vec<(ObjectIdentifier, EventState)> {
    let log = h.server.test_network().transport().sent();
    log.frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == mac)
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION =>
            {
                let n = decode_event_notification(&request.service_request).unwrap();
                Some((n.event_object_identifier, n.to_state))
            }
            _ => None,
        })
        .collect()
}

/// The To_States [`sent_to`] `mac`.
fn to_states(h: &Harness, mac: &[u8]) -> Vec<EventState> {
    sent_to(h, mac).into_iter().map(|(_, to)| to).collect()
}

/// Whether `frame` is an unconfirmed COV notification.
fn is_cov(frame: &crate::server::test_transport::SentFrame) -> bool {
    matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
        if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION)
}

/// COV notifications sent so far.
fn cov_reports(h: &Harness) -> usize {
    let log = h.server.test_network().transport().sent();
    log.frames().iter().filter(|frame| is_cov(frame)).count()
}

/// `device_recipient_unbound` and `confirmed_unanswered`.
fn counted(h: &Harness) -> (u64, u64) {
    let counters = h.server.event_notification_counters();
    (
        counters.device_recipient_unbound,
        counters.confirmed_unanswered,
    )
}

/// Whether no Who-Is has been sent since the last one taken.
fn next_who_is_none(h: &Harness) -> bool {
    who_is_sent(h).is_empty()
}

/// Entries waiting in every lane, behind the ones their tasks are sending.
fn waiting(h: &Harness) -> usize {
    lanes(&h.server.notification_transactions).waiting()
}

/// Destinations with a lane open.
fn open_lanes(h: &Harness) -> usize {
    lanes(&h.server.notification_transactions).open()
}

/// Wait, on the paused clock, until `done` holds.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..2_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("never {what}");
}

/// The value that makes AV-1's `at`-th transition, from NORMAL: an alarm,
/// then a clear, in turn.
fn flip(at: usize) -> f32 {
    if at.is_multiple_of(2) { 85.0 } else { 50.0 }
}

/// Whether `sent` alternates HIGH_LIMIT and NORMAL from the first.
fn alternates(sent: &[EventState]) -> bool {
    sent.iter().enumerate().all(|(at, state)| {
        *state
            == if at.is_multiple_of(2) {
                EventState::HIGH_LIMIT
            } else {
                EventState::NORMAL
            }
    })
}

/// While the lane waits for Device 9's I-Am the other recipient has its
/// notification at once; the I-Am sends Device 9's, ending its writer's wait.
#[tokio::test(start_paused = true)]
async fn the_lane_sends_once_the_i_am_comes_and_holds_no_other_destination() {
    let mut h = start(false).await;
    let (spy, _guard) = LaneLockSpy::install(&h.server.notification_transactions);
    let started = Instant::now();
    let writer = async {
        h.write_local(85.0).await;
        started.elapsed()
    };
    let (held, ()) = tokio::join!(writer, async {
        // Device 9 is looked for everywhere: the server never heard from it.
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
        h.settle().await;
        assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
        assert_eq!(
            (open_lanes(&h), waiting(&h)),
            (1, 0),
            "Device 9's at its head"
        );
        tokio::time::advance(WAIT / 2).await;
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    assert!(
        held >= WAIT / 2 && held < WAIT,
        "held to the I-Am: {held:?}"
    );
    h.settle().await;
    let both = vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, false)];
    assert_eq!(notifications(&h), both);
    assert_eq!((counted(&h), open_lanes(&h)), ((0, 0), 0));

    // The binding the I-Am made serves the next notification directly.
    let started = Instant::now();
    h.write_local(50.0).await;
    assert!(started.elapsed() < Duration::from_millis(10));
    h.settle().await;
    assert_eq!(notifications(&h).len(), 4);
    assert!(next_who_is_none(&h));
    assert_eq!(h.server.event_notification_counters().cov_wait_expired, 0);
    spy.assert_logged_unlocked(1);
    h.server.stop().await.unwrap();
}

/// A device named both as a Device and by its address has two lanes: the
/// address one doesn't wait for the I-Am the other waits for.
#[tokio::test(start_paused = true)]
async fn a_device_named_by_its_address_too_has_a_lane_that_does_not_wait() {
    let mut h = start_for(ServerConfig::default(), &[(9, false)], &[DEVICE_9]).await;
    tokio::join!(h.write_local(85.0), async {
        next_who_is(&h).await;
        h.settle().await;
        assert_eq!(notifications(&h), vec![(DEVICE_9.to_vec(), 7, false)]);
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    h.settle().await;
    let both = vec![(DEVICE_9.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, false)];
    assert_eq!(notifications(&h), both);
    h.server.stop().await.unwrap();
}

/// A device that stays silent counts once, with one Who-Is, when the wait
/// runs out; within the minute after it the next notification skips it at
/// once, sending no Who-Is and holding its writer for nothing.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_counted_once_with_one_who_is() {
    let mut h = start(false).await;
    tokio::join!(h.write_local(85.0), async {
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
        tokio::time::sleep(WAIT - Duration::from_millis(1)).await;
        assert_eq!(counted(&h), (0, 0), "nothing counted while it waits");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(counted(&h), (1, 0));
    });
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(open_lanes(&h), 0);

    let started = Instant::now();
    h.write_local(50.0).await;
    assert!(started.elapsed() < Duration::from_millis(10));
    h.settle().await;
    assert!(next_who_is_none(&h));
    assert_eq!(counted(&h), (2, 0));
    assert_eq!(notifications(&h).len(), 2);
    h.server.stop().await.unwrap();
}

/// The wait for the I-Am lasts a minute at most, whatever the APDU timeout.
/// When it runs out, the notification at the lane's head and the one waiting
/// behind it go together, each counted, with no second Who-Is for the one
/// behind; both writers' waits end there.
#[tokio::test(start_paused = true)]
async fn a_device_that_stays_silent_drops_everything_that_waited_for_it() {
    // Ten hours: the writers' cap too.
    let config = ServerConfig {
        cov_retry_timeout_ms: 10 * 60 * 60 * 1000,
        ..ServerConfig::default()
    };
    let mut h = start_with(config, false).await;
    let (spy, _guard) = LaneLockSpy::install(&h.server.notification_transactions);
    let minute = Duration::from_secs(60);
    let started = Instant::now();
    let first = async {
        h.write_local(85.0).await;
        started.elapsed()
    };
    let second = async {
        h.settle().await;
        h.write_local(50.0).await;
        started.elapsed()
    };
    let (first, second, ()) = tokio::join!(first, second, async {
        next_who_is(&h).await;
        tokio::time::sleep(minute - Duration::from_millis(1)).await;
        assert_eq!(counted(&h), (0, 0));
        assert_eq!(waiting(&h), 1, "the clear waits behind the alarm");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(counted(&h), (2, 0));
        assert!(next_who_is_none(&h), "none for the one behind");
    });
    for held in [first, second] {
        assert!(
            held >= minute && held < minute + Duration::from_millis(1),
            "{held:?}"
        );
    }
    assert_eq!((waiting(&h), open_lanes(&h)), (0, 0));
    assert_eq!(h.server.event_notification_counters().cov_wait_expired, 0);
    spy.assert_logged_unlocked(1);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_notification_after_the_i_am_counts_as_any_other() {
    let mut h = start(true).await;
    tokio::join!(h.write_local(85.0), async {
        next_who_is(&h).await;
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    h.settle().await;
    let both = vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, true)];
    assert_eq!(notifications(&h), both);
    // Device 9 never acknowledges it: after the last retry it counts as
    // unanswered, not as a recipient with no binding.
    tokio::time::sleep(WAIT * 5).await;
    assert_eq!(notifications(&h).len(), 2 + 3);
    assert_eq!(counted(&h), (0, 1));
    h.server.stop().await.unwrap();
}

/// DCC restricting initiation: from the start no Who-Is goes and nothing
/// waits; by a request while the lane waits, nothing goes to the device once
/// the wait ends, the I-Am come or not, and nothing is counted.
#[tokio::test(start_paused = true)]
async fn dcc_ends_the_wait_with_nothing_sent_or_counted() {
    let mut h = start(false).await;
    disable_initiation(&h);
    let started = Instant::now();
    h.write_local(85.0).await;
    assert!(started.elapsed() < Duration::from_millis(10));
    h.settle().await;
    assert!(next_who_is_none(&h));
    assert!(notifications(&h).is_empty());
    assert_eq!(counted(&h), (0, 0));
    h.server.stop().await.unwrap();

    let config = ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    };
    for answers in [true, false] {
        let mut h = start_with(config.clone(), false).await;
        // By WriteProperty: the request's task waits, not the test.
        h.write_pv(85.0, 0).await;
        next_who_is(&h).await;
        h.dcc(EnableDisable::DISABLE_INITIATION, None).await;
        h.settle().await;
        assert!(h.server.comm_state.initiation_restricted());
        if answers {
            deliver(&h, &i_am(9), &DEVICE_9, None).await;
        }
        tokio::time::sleep(WAIT * 2).await;
        assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
        assert_eq!((counted(&h), open_lanes(&h)), ((0, 0), 0), "{answers}");
        h.server.stop().await.unwrap();
    }
}

/// Transitions of two objects from several writers wait for Device 9's I-Am
/// and go out in commit order after it; one committed while the first of them
/// is held in the transport after the I-Am still goes last.
#[tokio::test(start_paused = true)]
async fn writers_to_an_unbound_device_go_out_in_commit_order() {
    let mut h = start_for_and(ServerConfig::default(), &[(9, false)], &[OTHER], |db| {
        add_av2(db, 0)
    })
    .await;
    let transport = h.server.test_network().transport().handle();
    let (high, normal) = (EventState::HIGH_LIMIT, EventState::NORMAL);
    tokio::join!(
        h.write_local(85.0),
        h.write_local_to(av2(), 85.0),
        h.write_local(50.0),
        async {
            next_who_is(&h).await;
            until("three committed", || sent_to(&h, &OTHER).len() == 3).await;
            assert_eq!(waiting(&h), 2, "two behind the head");
            transport.block_next_send();
            deliver(&h, &i_am(9), &DEVICE_9, None).await;
            transport.wait_blocked().await;
            tokio::join!(h.write_local_to(av2(), 50.0), async {
                until("a fourth committed", || sent_to(&h, &OTHER).len() == 4).await;
                transport.release_sends(1);
            });
        }
    );
    h.settle().await;
    let made = [
        (av1(), high),
        (av2(), high),
        (av1(), normal),
        (av2(), normal),
    ];
    assert_eq!(sent_to(&h, &OTHER), made);
    assert_eq!(sent_to(&h, &DEVICE_9), made);
    assert!(next_who_is_none(&h));
    assert_eq!(counted(&h), (0, 0));
    h.server.stop().await.unwrap();
}

/// A writer whose transition goes to an unbound Device recipient is held
/// until the I-Am, its COV report after the notification; with no I-Am, no
/// longer than its cap, counted in `cov_wait_expired`.
#[tokio::test(start_paused = true)]
async fn the_writer_waits_for_the_i_am_and_no_longer_than_its_cap() {
    let mut h = start_for(ServerConfig::default(), &[(9, false)], &[]).await;
    h.subscribe_cov().await;
    h.settle().await;
    let reports = cov_reports(&h);
    let started = Instant::now();
    let writer = async {
        h.write_local(85.0).await;
        started.elapsed()
    };
    let (held, ()) = tokio::join!(writer, async {
        next_who_is(&h).await;
        tokio::time::sleep(WAIT / 2).await;
        assert_eq!(cov_reports(&h), reports, "the report waits for the I-Am");
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    assert!(held >= WAIT / 2 && held < WAIT, "{held:?}");
    assert_eq!(cov_reports(&h), reports + 1);
    assert_eq!(notifications(&h), vec![(DEVICE_9.to_vec(), 9, false)]);
    let frames = h.server.test_network().transport().sent().frames();
    let event = frames
        .iter()
        .position(|frame| frame.mac.as_slice() == DEVICE_9);
    let report = frames.iter().rposition(is_cov);
    assert!(event < report, "the event notification first");
    assert_eq!(h.server.event_notification_counters().cov_wait_expired, 0);
    h.server.stop().await.unwrap();

    // Device 9 stays silent, and its Who-Is is held 100 ms in the transport,
    // so the probe's wait ends after the writer's cap does.
    let mut h = start_for(ServerConfig::default(), &[(9, false)], &[]).await;
    h.subscribe_cov().await;
    h.settle().await;
    let reports = cov_reports(&h);
    let transport = h.server.test_network().transport().handle();
    transport.block_next_send();
    let started = Instant::now();
    let writer = async {
        h.write_local(85.0).await;
        started.elapsed()
    };
    let (held, ()) = tokio::join!(writer, async {
        transport.wait_blocked().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        transport.release_sends(1);
    });
    assert_eq!(held, WAIT, "held to its cap");
    assert_eq!(cov_reports(&h), reports + 1);
    let counters = h.server.event_notification_counters();
    let counted_then = (counters.cov_wait_expired, counters.device_recipient_unbound);
    assert_eq!(counted_then, (1, 0));
    tokio::time::sleep(Duration::from_millis(101)).await;
    assert_eq!(counted(&h), (1, 0), "the probe's own wait ran out");
    h.server.stop().await.unwrap();
}

/// A lane waiting for its device's I-Am fills: 64 wait behind its head, the
/// newest past them are dropped and counted in `lane_full`, none as unbound,
/// and the head and the 64 go out in commit order once the I-Am comes.
#[tokio::test(start_paused = true)]
async fn a_lane_waiting_for_the_i_am_fills_and_drops_the_newest() {
    let mut h = start(false).await;
    let (spy, _guard) = LaneLockSpy::install(&h.server.notification_transactions);
    let made = MAX_LANE_ENTRIES + 6;
    let writes = futures_util::future::join_all((0..made).map(|at| h.write_local(flip(at))));
    tokio::join!(writes, async {
        next_who_is(&h).await;
        until("all committed", || sent_to(&h, &OTHER).len() == made).await;
        assert_eq!(waiting(&h), MAX_LANE_ENTRIES);
        let counters = h.server.event_notification_counters();
        let dropped = (made - 1 - MAX_LANE_ENTRIES) as u64;
        assert_eq!(
            (counters.lane_full, counters.device_recipient_unbound),
            (dropped, 0)
        );
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    h.settle().await;
    let sent = to_states(&h, &DEVICE_9);
    assert_eq!(sent.len(), MAX_LANE_ENTRIES + 1, "the head and the 64");
    assert!(alternates(&sent), "in the order they were made: {sent:?}");
    assert!(alternates(&to_states(&h, &OTHER)));
    assert_eq!((waiting(&h), open_lanes(&h)), (0, 0));
    spy.assert_logged_unlocked(1);
    h.server.stop().await.unwrap();
}

/// Once the I-Am binds Device 9 its lane may wait for an invoke ID, and holds
/// 64 behind its head there too; while it does, Device 10, which needs a
/// Who-Is, is still looked for and served.
#[tokio::test(start_paused = true)]
async fn a_device_lane_held_for_an_invoke_id_leaves_another_device_looked_for() {
    let mut h = start_for_and(ServerConfig::default(), &[(9, true)], &[OTHER], |db| {
        let mut nc = bacnet_objects::notification_class::NotificationClass::new(1, "NC-1").unwrap();
        let mut to_device = destination_for(BACnetRecipient::Device(device(10)), false);
        to_device.process_identifier = 10;
        nc.add_destination(to_device).unwrap();
        db.add(Box::new(nc)).unwrap();
        add_av2(db, 1);
    })
    .await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    let leases = h.server.notification_transactions.lease_every_invoke_id();
    let made = MAX_LANE_ENTRIES + 2;
    futures_util::future::join_all((0..made).map(|at| h.write_local(flip(at)))).await;
    let counters = h.server.event_notification_counters();
    assert_eq!(waiting(&h), MAX_LANE_ENTRIES);
    assert_eq!(
        (counters.lane_full, counters.confirmed_waited_for_invoke_id),
        (1, 1)
    );
    tokio::join!(h.write_local_to(av2(), 85.0), async {
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(10)));
        deliver(&h, &i_am(10), &DEVICE_10, None).await;
    });
    h.settle().await;
    assert!(notifications(&h).contains(&(DEVICE_10.to_vec(), 10, false)));
    assert_eq!(counted(&h), (0, 0));
    drop(leases);
    h.server.stop().await.unwrap();
}

/// Two unbound devices are looked for at once, each lane on its own: one
/// that answers has its notification while the other still waits, and the
/// silent one counts once.
#[tokio::test(start_paused = true)]
async fn each_device_waits_on_its_own() {
    let mut h = start_for(
        ServerConfig::default(),
        &[(9, false), (10, false)],
        &[OTHER],
    )
    .await;
    tokio::join!(h.write_local(85.0), async {
        let mut looked_for = Vec::new();
        until("two Who-Is", || {
            looked_for.extend(who_is_sent(&h).into_iter().map(|(_, who_is)| who_is));
            looked_for.len() == 2
        })
        .await;
        assert!(looked_for.contains(&targeted(9)) && looked_for.contains(&targeted(10)));
        tokio::time::advance(WAIT / 2).await;
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
        h.settle().await;
        let both = vec![(OTHER.to_vec(), 7, false), (DEVICE_9.to_vec(), 9, false)];
        assert_eq!(notifications(&h), both);
        assert_eq!((counted(&h), open_lanes(&h)), ((0, 0), 1));
        tokio::time::sleep(WAIT).await;
        assert_eq!((counted(&h), open_lanes(&h)), ((1, 0), 0));
    });
    assert_eq!(notifications(&h).len(), 2);
    h.server.stop().await.unwrap();
}

/// With 1,024 entries waiting across the lanes, a notification for an
/// unbound device is dropped at once, counted in `lanes_entries_full`, and
/// no Who-Is goes out for it.
#[tokio::test(start_paused = true)]
async fn with_every_place_taken_a_notification_is_dropped_with_no_who_is() {
    let mut h = start(false).await;
    {
        let filler = Arc::new(LanePayload::local(EventNotificationRequest {
            process_identifier: 1,
            initiating_device_identifier: device(1),
            event_object_identifier: av1(),
            timestamp: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(1),
            notification_class: 0,
            priority: 255,
            event_type: EventType::OUT_OF_RANGE,
            message_text: None,
            notify_type: NotifyType::ALARM,
            ack_required: false,
            from_state: EventState::NORMAL,
            to_state: EventState::HIGH_LIMIT,
            event_values: None,
        }));
        let mut lanes = lanes(&h.server.notification_transactions);
        // Taskless lanes for Devices 50 to 65, 64 each, that never drain.
        for at in 0..MAX_LANED_ENTRIES {
            let instance = 50 + (at / MAX_LANE_ENTRIES) as u32;
            let entry = LaneEntry::new(&filler, 1, false, None);
            lanes.push(&BACnetRecipient::Device(device(instance)), entry);
        }
        assert_eq!(lanes.waiting(), MAX_LANED_ENTRIES);
    }
    let started = Instant::now();
    h.write_local(85.0).await;
    assert!(
        started.elapsed() < Duration::from_millis(10),
        "nothing to wait for"
    );
    h.settle().await;
    let counters = h.server.event_notification_counters();
    let dropped = (
        counters.lanes_entries_full,
        counters.device_recipient_unbound,
    );
    assert_eq!(dropped, (2, 0), "Device 9's and the peer's");
    assert!(next_who_is_none(&h));
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    assert!(notifications(&h).is_empty());
    h.server.stop().await.unwrap();
}

/// `stop()` while a lane waits for its device's I-Am returns at once and the
/// lane sends nothing more; a server dropped then leaves no lane or task.
#[tokio::test(start_paused = true)]
async fn stop_and_the_drop_end_a_lane_waiting_for_the_i_am() {
    let mut h = start(false).await;
    let log = h.server.test_network().transport().sent();
    h.write_pv(85.0, 0).await;
    next_who_is(&h).await;
    assert_eq!(open_lanes(&h), 1);
    let started = Instant::now();
    h.server.stop().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1), "stop waited");
    assert_eq!(open_lanes(&h), 0);
    let sent = log.len();
    tokio::time::sleep(WAIT * 2).await;
    assert_eq!(log.len(), sent, "nothing is sent after stop()");
    let to_device =
        |frame: &crate::server::test_transport::SentFrame| frame.mac.as_slice() == DEVICE_9;
    assert!(!log.frames().iter().any(to_device));

    let mut h = start(false).await;
    let log = h.server.test_network().transport().sent();
    let transactions = Arc::clone(&h.server.notification_transactions);
    h.write_pv(85.0, 0).await;
    next_who_is(&h).await;
    drop(h);
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let lanes = lanes(&transactions);
        assert_eq!((lanes.open(), lanes.tasks.len()), (0, 0), "no lane left");
    }
    tokio::time::sleep(WAIT * 2).await;
    assert!(!log.frames().iter().any(to_device));
}

/// An I-Am that races the start of the lane's wait is never lost: the table
/// starts the probe under the guard it finds no binding under, so the I-Am
/// either comes first, and the look finds the binding, or after, and wakes
/// the wait. Looped on two worker threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_i_am_racing_the_start_of_the_wait_still_ends_it() {
    for round in 0..2_000 {
        // A minute's wait, so a lost wake-up shows as a timeout.
        let owned = Arc::new(OwnedDelivery {
            db: Arc::new(RwLock::new(ObjectDatabase::new())),
            network: Arc::new(NetworkLayer::new(TestTransport::new())),
            comm_state: Arc::default(),
            learned_routers: Arc::new(Mutex::new(LearnedRouterCache::new())),
            notification_transactions: NotificationTransactions::new(),
            device_bindings: Arc::new(RwLock::new(DeviceBindingTable::new())),
            suppressions: Arc::default(),
            retry_timeout_ms: 60_000,
            local_apdu_capacity: 1476,
            device: device(1),
        });
        let bindings = Arc::clone(&owned.device_bindings);
        let lane = tokio::spawn(async move {
            let recipient = BACnetRecipient::Device(device(9));
            let ctx = owned.delivery();
            BACnetServer::<TestTransport>::device_route(&ctx, &recipient, device(9), 0).await
        });
        let answer = tokio::spawn(async move {
            let now = crate::runtime_clock::now();
            let mut table = bindings.write().await;
            table.observe_i_am_at(device(9), &DEVICE_9, None, now, |_| false)
        });
        assert_eq!(answer.await.unwrap(), ObservationOutcome::Inserted);
        let routed = tokio::time::timeout(Duration::from_secs(5), lane).await;
        assert!(
            matches!(routed, Ok(Ok(Some(DeviceResolution::ResolvedLocal { .. })))),
            "round {round}: the wait missed the I-Am: {routed:?}"
        );
    }
}

/// A bound Device recipient whose confirmed notification waits for an
/// invoke ID past its observed binding's lifetime: once an ID comes back,
/// its first attempt finds the binding gone, so it is withdrawn, its ID given
/// back, counted as unbound, with no Who-Is for it. The next notification
/// finds the binding stale and looks for the device where it was last seen.
#[tokio::test(start_paused = true)]
async fn a_binding_that_lapses_during_a_wait_for_an_invoke_id_counts_as_unbound() {
    let mut h = start(true).await;
    deliver(&h, &i_am(9), &DEVICE_9, None).await;
    h.settle().await;
    let transactions = Arc::clone(&h.server.notification_transactions);
    let mut leases = transactions.lease_every_invoke_id();
    h.write_pv(85.0, 0).await;
    let waited = || {
        h.server
            .event_notification_counters()
            .confirmed_waited_for_invoke_id
    };
    until("the wait for an ID", || waited() == 1).await;
    tokio::time::sleep(OBSERVED_BINDING_TTL).await;
    drop(leases.pop());
    h.settle().await;
    assert_eq!(notifications(&h), vec![(OTHER.to_vec(), 7, false)]);
    assert_eq!(counted(&h), (1, 0));
    assert!(next_who_is_none(&h));
    assert_eq!(transactions.active_count(), 255, "its ID given back");
    h.write_pv(50.0, 0).await;
    assert_eq!(next_who_is(&h).await, (None, targeted(9)));
    drop(leases);
    h.server.stop().await.unwrap();
}
