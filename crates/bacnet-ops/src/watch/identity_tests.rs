//! Another device at a watched address (#2603), on the device tests' bus:
//! it is never taken for the watched one, whether it announces itself, or
//! only answers (its notifications name it, a poll asks it), or sends before
//! announcing; a pinned watch gives its point up until its device comes
//! back, and reports it moved once it turns up elsewhere; one made through
//! a following handle reports the move; announcements fighting over an
//! address cost one read a lifetime.

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_server::server::BACnetServer;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use tokio::time::Instant;

use super::State;
use super::hub::PROCESS_BASE;
use super::stream_tests::{number, ready, until};
use super::*;
use crate::device::test_bus::{Bus, N, at, client, requests, server, settle, stop};

pub(super) fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

pub(super) async fn set(server: &BACnetServer<TestTransport>, value: f32) {
    server
        .set_present_value_local(&ai1(), PropertyValue::Real(value))
        .await
        .unwrap();
}

pub(super) fn subscribes(sent: &SendLog, station: u8) -> usize {
    requests(sent, ConfirmedServiceChoice::SUBSCRIBE_COV, station)
}

pub(super) fn reads(sent: &SendLog, station: u8) -> usize {
    requests(sent, ConfirmedServiceChoice::READ_PROPERTY, station)
}

fn gone_for(update: &Update) -> Option<&Why> {
    match &update.state {
        State::Gone { why } => Some(why),
        _ => None,
    }
}

const REPLACED: Why = Why::Replaced {
    watched: N,
    found: N + 1,
};

/// A pinned watch of device `N`'s AI 1 at station 2, subscribed, on a new
/// bus, with every update it has had.
pub(super) struct Pinned {
    pub(super) bus: Arc<Bus>,
    pub(super) client: crate::Client<TestTransport>,
    pub(super) sent: SendLog,
    pub(super) device: crate::Device<TestTransport>,
    pub(super) watch: Watch,
    pub(super) seen: Vec<Update>,
}

impl Pinned {
    pub(super) async fn new(first: &mut Option<BACnetServer<TestTransport>>) -> Self {
        Self::timed(first, None).await
    }

    /// As [`new`](Self::new), the device answering its Time_Of_Device_Restart
    /// with `time` when set.
    pub(super) async fn timed(
        first: &mut Option<BACnetServer<TestTransport>>,
        time: Option<u8>,
    ) -> Self {
        let bus = Bus::new();
        bus.restart_time(2, time);
        *first = Some(server(bus.port(2), N, "first").await);
        let (client, sent) = client(&bus).await;
        bus.i_am(2, N);
        settle().await;
        let device = client.device(at(2)).await.unwrap();
        assert!(!device.follows());
        let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
        let mut seen = Vec::new();
        loop {
            let update = until(&mut watch, |_| true).await;
            let done = update.state == State::Subscribed;
            seen.push(update);
            if done {
                break;
            }
        }
        // Its first notification, and the read of its restart time that
        // follows its first subscription.
        settle().await;
        Self {
            bus,
            client,
            sent,
            device,
            watch,
            seen,
        }
    }

    /// Every update waiting now, kept.
    pub(super) fn take(&mut self) -> Vec<Update> {
        let mut new = Vec::new();
        while let Some(update) = ready(&mut self.watch) {
            new.push(update);
        }
        self.seen.extend(new.iter().cloned());
        new
    }

    /// Run `seconds` of paused time, a few at a time, setting `server`'s
    /// value from `from` up each step, and keep every update.
    async fn run(&mut self, seconds: u64, server: &BACnetServer<TestTransport>, from: f32) {
        let end = Instant::now() + Duration::from_secs(seconds);
        let mut value = from;
        while Instant::now() < end {
            set(server, value).await;
            value += 1.0;
            tokio::time::sleep(Duration::from_secs(5)).await;
            self.take();
        }
    }

    /// No update has carried a value of 99 or more: the new device's.
    fn none_foreign(&self) {
        for update in &self.seen {
            assert!(
                number(update).is_none_or(|v| v < 99.0),
                "the new device's value as the point's: {update:?}"
            );
        }
    }
}

/// An UnconfirmedCOVNotification from station `n` to the client, under
/// subscriber process `process`, sent by device `sender`: AI 1 is `value`.
fn notify_from(bus: &Bus, n: u8, process: u32, sender: u32, value: f32) {
    let mut service = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, sender).unwrap(),
        monitored_object_identifier: ai1(),
        time_remaining: 300,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: [&[0x44][..], &value.to_be_bytes()].concat(),
            priority: None,
        }],
    }
    .encode(&mut service)
    .unwrap();
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
        service_request: service.freeze(),
    });
    let mut npdu = BytesMut::from(&[1u8, 0][..]);
    encode_apdu(&mut npdu, &apdu).unwrap();
    bus.send_from(n, &npdu, 1);
}

/// Acceptance (#2603 A): the watched device leaves its address and another,
/// with the same objects, takes it and announces itself. The device there
/// is asked once and says it is the other: the point is given up for it,
/// its values dropped, and over the old subscription's renewal and
/// lifetime no update ever carries a value of the new device's, and
/// nothing is sent to it but that one read. Let go and watched again, the
/// point watches the device there now.
#[tokio::test(start_paused = true)]
async fn a_device_taking_a_watched_address_is_never_the_watched_one() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    let first = first.unwrap();
    set(&first, 21.0).await;
    until(&mut lab.watch, |u| number(u) == Some(21.0)).await;
    stop(vec![first]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    let (subscribed, read) = (subscribes(&lab.sent, 2), reads(&lab.sent, 2));
    lab.bus.i_am(2, N + 1);
    lab.run(400, &second, 99.0).await;
    lab.none_foreign();
    let gone = lab.seen.iter().find(|u| gone_for(u).is_some()).unwrap();
    assert_eq!(gone_for(gone), Some(&REPLACED));
    assert_eq!(gone.value, None, "its last values dropped");
    assert_eq!(subscribes(&lab.sent, 2), subscribed, "nothing but one read");
    assert_eq!(reads(&lab.sent, 2), read + 1);
    assert_eq!(second.cov_counters().subscriptions_active, 0);
    drop(lab.watch);
    settle().await;
    assert_eq!(subscribes(&lab.sent, 2), subscribed, "no cancel either");
    let mut late = lab
        .client
        .watch([lab.device.point("ai:1").unwrap()])
        .unwrap();
    let update = until(&mut late, |u| u.state == State::Subscribed).await;
    assert!(number(&update).is_some_and(|v| v >= 99.0), "{update:?}");
    drop((late, lab.device));
    settle().await;
    stop(vec![second]).await;
}

/// Another device takes the address without announcing itself. The old
/// subscription's renewal reaches it and it takes it; its notifications
/// name it as their sender, so none is taken, and the device there is
/// asked and the point given up for it: no later than the renewal, with no
/// request but that one read.
#[tokio::test(start_paused = true)]
async fn a_silent_takeover_is_found_by_the_new_devices_notifications() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    let read = reads(&lab.sent, 2);
    lab.run(400, &second, 99.0).await;
    lab.none_foreign();
    let gone = lab.seen.iter().find(|u| gone_for(u).is_some());
    assert_eq!(gone.and_then(gone_for), Some(&REPLACED), "{:?}", lab.seen);
    assert_eq!(reads(&lab.sent, 2), read + 1, "one read");
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![second]).await;
}

/// The new device's notification under the point's own process identifier
/// reaches the client before its announcement: it is dropped all the same.
#[tokio::test(start_paused = true)]
async fn a_notification_before_the_announcement_is_dropped() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    notify_from(&lab.bus, 2, PROCESS_BASE, N + 1, 150.0);
    lab.bus.i_am(2, N + 1);
    lab.run(30, &second, 99.0).await;
    lab.none_foreign();
    let gone = lab.seen.iter().find(|u| gone_for(u).is_some());
    assert_eq!(gone.and_then(gone_for), Some(&REPLACED));
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![second]).await;
}

/// The device of a pinned watch given up for another device at its address
/// comes back there and announces itself: once the address's one read a
/// lifetime allows, it is asked, and the point starts again and takes its
/// values.
#[tokio::test(start_paused = true)]
async fn the_watched_device_coming_back_brings_a_pinned_watch_back() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    lab.bus.i_am(2, N + 1);
    until(&mut lab.watch, |u| gone_for(u).is_some()).await;
    stop(vec![second]).await;
    let back = server(lab.bus.port(2), N, "back").await;
    lab.bus.i_am(2, N);
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    set(&back, 33.0).await;
    let update = until(&mut lab.watch, |u| number(u) == Some(33.0)).await;
    assert_eq!(update.via, Some(Via::Notification));
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![back]).await;
}

/// A pinned watch given up for another device at its address: its device
/// turns up at station 3 and announces itself there, and the point is
/// reported moved there once one read there names it (#2812).
#[tokio::test(start_paused = true)]
async fn a_replaced_point_is_moved_once_its_device_turns_up_elsewhere() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    lab.bus.i_am(2, N + 1);
    until(&mut lab.watch, |u| gone_for(u) == Some(&REPLACED)).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let moved = server(lab.bus.port(3), N, "moved").await;
    let sent = lab.sent.len();
    lab.bus.i_am(3, N);
    let update = until(&mut lab.watch, |u| {
        gone_for(u).is_some_and(|why| *why != REPLACED)
    })
    .await;
    assert_eq!(gone_for(&update), Some(&Why::Moved { to: at(3) }));
    assert_eq!(lab.sent.len(), sent + 1, "one read there");
    assert_eq!(lab.client.said(&at(3)), Some(N), "naming the device");
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![second, moved]).await;
}

/// A watch made through a following handle, whose device moved to station
/// 3 while another device took station 2 (#2795): the point, unreachable
/// while the hub looks for its device, follows it there once the hub's
/// read names it, the handle with it; nothing of the device now at station
/// 2 is taken, and nothing is subscribed or cancelled there. The point
/// watched again from the handle is the same point.
#[tokio::test(start_paused = true)]
async fn a_following_watch_whose_address_was_taken_follows_the_device() {
    let bus = Bus::new();
    let old = server(bus.port(2), N, "old").await;
    let (client, sent) = client(&bus).await;
    let device = client.device(N).await.unwrap();
    assert!(device.follows());
    let point = device.point("ai:1").unwrap();
    let mut watch = client.watch([point.clone()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    stop(vec![old]).await;
    let moved = server(bus.port(3), N, "moved").await;
    bus.i_am(3, N);
    let taker = server(bus.port(2), N + 1, "taker").await;
    let cov = bacnet_types::enums::ConfirmedServiceChoice::SUBSCRIBE_COV;
    let before = crate::device::test_bus::requests(&sent, cov, 2);
    bus.i_am(2, N + 1);
    let update = until(&mut watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.moved.is_some()
    })
    .await;
    assert_eq!(update.point, point);
    assert_eq!(update.at, at(3));
    assert_eq!(device.address(), at(3), "the handle followed it");
    let mut again = client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut again, |u| u.state == State::Subscribed).await;
    assert_eq!((update.point, update.at), (point, at(3)));
    assert_eq!(moved.cov_counters().subscriptions_active, 1, "one point");
    assert_eq!(taker.cov_counters().subscriptions_active, 0);
    assert_eq!(
        crate::device::test_bus::requests(&sent, cov, 2),
        before,
        "nothing subscribed or cancelled at station 2"
    );
    drop((watch, again, device));
    settle().await;
    stop(vec![moved, taker]).await;
}

/// A polled point (its device has no COV for it), polled once, so its
/// device named itself: another device takes the address without
/// announcing itself. The next poll asks the device which it is in the same
/// request, and the point is given up, its answer not taken.
#[tokio::test(start_paused = true)]
async fn a_polled_device_taken_silently_is_found_by_the_next_poll() {
    let bus = Bus::new();
    let first = server(bus.port(2), N, "first").await;
    let (client, sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    let point = device.point("device:1001 object-name").unwrap();
    let mut watch = client.watch([point]).unwrap();
    let polled = until(&mut watch, |u| u.state != State::Starting).await;
    assert!(matches!(polled.state, State::Polled { .. }), "{polled:?}");
    tokio::time::sleep(Duration::from_secs(15)).await;
    stop(vec![first]).await;
    let second = server(bus.port(2), N + 1, "second").await;
    let read = reads(&sent, 2);
    let update = until(&mut watch, |u| gone_for(u).is_some()).await;
    assert_eq!(gone_for(&update), Some(&REPLACED));
    assert_eq!(reads(&sent, 2), read, "no read of its own");
    drop((watch, device));
    settle().await;
    stop(vec![second]).await;
}

/// Two devices fight over an address: announcements naming each in turn,
/// every five seconds for twenty minutes, while the watched one answers.
/// The device is asked at once, then after 60, 120 and 240 s, then once a
/// lifetime (the wait doubles up to the lifetime and stays there), and the
/// watch goes on.
#[tokio::test(start_paused = true)]
async fn announcements_fighting_over_an_address_are_asked_ever_less_often() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    let first = first.unwrap();
    let read = reads(&lab.sent, 2);
    for step in 0..240u32 {
        lab.bus.i_am(2, N + step % 2);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let asked = reads(&lab.sent, 2) - read;
    // At 5, 65, 185, 425, 725 and 1025 s.
    assert_eq!(asked, 6, "reads in twenty minutes");
    assert!(lab.take().iter().all(|u| gone_for(u).is_none()));
    set(&first, 44.0).await;
    until(&mut lab.watch, |u| number(u) == Some(44.0)).await;
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![first]).await;
}

/// A forged announcement of another device at the watched address is
/// refuted by the device there, and repeated, so the table goes on naming
/// the other. A watch started there afterwards takes what the device says,
/// not the table's row, and works: subscribed, taking its values, over the
/// lifetime and its renewal.
#[tokio::test(start_paused = true)]
async fn a_watch_started_after_a_refuted_claim_takes_the_device_there() {
    let mut first = None;
    let lab = Pinned::new(&mut first).await;
    let first = first.unwrap();
    lab.bus.i_am(2, N + 1);
    tokio::time::sleep(Duration::from_secs(5)).await;
    // The forged announcement again, after the refutation.
    for _ in 0..2 {
        lab.bus.i_am(2, N + 1);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    drop(lab.watch);
    settle().await;
    let mut watch = lab
        .client
        .watch([lab.device.point("ai:1").unwrap()])
        .unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    let end = Instant::now() + Duration::from_secs(400);
    let mut value = 50.0;
    while Instant::now() < end {
        set(&first, value).await;
        let update = until(&mut watch, |u| number(u) == Some(f64::from(value))).await;
        assert_eq!(update.state, State::Subscribed, "{update:?}");
        value += 1.0;
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    drop((watch, lab.device));
    settle().await;
    stop(vec![first]).await;
}

/// A forged notification naming device N+1 under the point's process
/// identifier: dropped, the device there asked (it is N), and N+1 found not
/// to answer there, so the name is taken as N's own. Device N+1 then takes
/// the address and announces itself: the name is no longer taken, what N+1
/// sends is dropped, and once asked the point is given up for it.
#[tokio::test(start_paused = true)]
async fn a_forged_notification_does_not_let_a_later_device_in() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    notify_from(&lab.bus, 2, PROCESS_BASE, N + 1, 150.0);
    tokio::time::sleep(Duration::from_secs(5)).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    lab.bus.i_am(2, N + 1);
    settle().await;
    notify_from(&lab.bus, 2, PROCESS_BASE, N + 1, 151.0);
    lab.run(120, &second, 99.0).await;
    lab.none_foreign();
    let gone = lab.seen.iter().find(|u| gone_for(u).is_some());
    assert_eq!(gone.and_then(gone_for), Some(&REPLACED), "{:?}", lab.seen);
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![second]).await;
}

/// Sixteen polled points of one device, a full ReadPropertyMultiple: the
/// question of which device answers still rides in it, at no request of
/// its own, so another device taking the address silently (once the device
/// named itself in a poll) is found at the next poll.
#[tokio::test(start_paused = true)]
async fn sixteen_polled_points_still_ask_which_device_answers() {
    const PROPERTIES: [&str; 16] = [
        "object-name",
        "vendor-name",
        "vendor-identifier",
        "model-name",
        "firmware-revision",
        "application-software-version",
        "protocol-version",
        "protocol-revision",
        "system-status",
        "max-apdu-length-accepted",
        "segmentation-supported",
        "database-revision",
        "description",
        "object-type",
        "apdu-timeout",
        "number-of-apdu-retries",
    ];
    let bus = Bus::new();
    let first = server(bus.port(2), N, "first").await;
    let (client, sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    let points = PROPERTIES.map(|p| device.point(format!("device:{N} {p}").as_str()).unwrap());
    let mut watch = client.watch(points).unwrap();
    for _ in 0..PROPERTIES.len() {
        let update = until(&mut watch, |u| u.state != State::Starting).await;
        assert!(matches!(update.state, State::Polled { .. }), "{update:?}");
    }
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    tokio::time::sleep(Duration::from_secs(15)).await;
    let polls = requests(&sent, rpm, 2);
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(requests(&sent, rpm, 2) - polls, 1, "one request a poll");
    stop(vec![first]).await;
    let second = server(bus.port(2), N + 1, "second").await;
    let update = until(&mut watch, |u| gone_for(u).is_some()).await;
    assert_eq!(gone_for(&update), Some(&REPLACED));
    drop((watch, device));
    settle().await;
    stop(vec![second]).await;
}

/// A forged announcement names device N+1 at the address before a point
/// there is watched: the point starts with the table's N+1, which no device
/// confirmed, and the device's own first answer (in its first poll) names
/// N. The point takes N as its device and goes on, polled, not given up.
#[tokio::test(start_paused = true)]
async fn a_point_started_on_a_forged_row_takes_the_devices_own_answer() {
    let bus = Bus::new();
    let first = server(bus.port(2), N, "first").await;
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    bus.i_am(2, N + 1);
    settle().await;
    let point = device.point(format!("device:{N} object-name").as_str());
    let mut watch = client.watch([point.unwrap()]).unwrap();
    let polled = until(&mut watch, |u| u.state != State::Starting).await;
    assert!(matches!(polled.state, State::Polled { .. }), "{polled:?}");
    tokio::time::sleep(Duration::from_secs(60)).await;
    let mut seen = Vec::new();
    while let Some(update) = ready(&mut watch) {
        seen.push(update);
    }
    assert!(seen.iter().all(|u| gone_for(u).is_none()), "{seen:?}");
    assert!(
        seen.last()
            .is_none_or(|u| matches!(u.state, State::Polled { .. })),
        "{seen:?}"
    );
    drop((watch, device));
    settle().await;
    stop(vec![first]).await;
}
