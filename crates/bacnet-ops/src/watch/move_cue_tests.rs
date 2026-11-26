//! A move the hub didn't see answered is only a cue (#2812): a following
//! handle that moved on the discovery table's rows, which unsolicited
//! I-Ams fill, or a new address the hub's own look found, has the hub read
//! the Device object by the wildcard instance there, paced as an ask of
//! which device answers, and the points are given up as moved only when
//! that answer names their instance. On the device tests' bus, with a
//! following handle held, dropped, or a pinned one.

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::Apdu;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::broadcast;

use super::State;
use super::stream_tests::{latest, number, ready, until};
use super::*;
use crate::Device;
use crate::device::test_bus::{Bus, N, at, client, mac, server, settle, stop};
use crate::device::{ClientEvent, DeviceAddress};

/// How the watched point's handle is held while the frames come.
#[derive(Debug, Clone, Copy)]
pub(super) enum Handle {
    /// A handle following the instance, held.
    Follows,
    /// A handle following the instance, dropped once the point was made.
    Dropped,
    /// A handle pinned to the address.
    Pinned,
}

const HANDLES: [Handle; 3] = [Handle::Pinned, Handle::Dropped, Handle::Follows];

/// Device `N` at station 2, its AI 1 watched and subscribed.
pub(super) struct Lab {
    pub(super) bus: Arc<Bus>,
    pub(super) servers: Vec<BACnetServer<TestTransport>>,
    pub(super) client: crate::Client<TestTransport>,
    pub(super) sent: SendLog,
    pub(super) device: Option<Device<TestTransport>>,
    pub(super) watch: Watch,
    pub(super) events: broadcast::Receiver<ClientEvent>,
}

impl Lab {
    pub(super) async fn new(handle: Handle) -> Self {
        let bus = Bus::new();
        let servers = vec![server(bus.port(2), N, "watched").await];
        let (client, sent) = client(&bus).await;
        bus.i_am(2, N);
        settle().await;
        let device = match handle {
            Handle::Pinned => client.device(at(2)).await.unwrap(),
            Handle::Follows | Handle::Dropped => client.device(N).await.unwrap(),
        };
        assert_eq!(device.follows(), !matches!(handle, Handle::Pinned));
        let events = client.events();
        let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| u.state == State::Subscribed).await;
        latest(&mut watch).await;
        let device = match handle {
            Handle::Dropped => None,
            Handle::Follows | Handle::Pinned => Some(device),
        };
        Self {
            bus,
            servers,
            client,
            sent,
            device,
            watch,
            events,
        }
    }

    /// Two forged I-Ams: another device at the watched address, and the
    /// watched instance at station 3, where no device answers.
    pub(super) fn forge(&self) {
        self.bus.i_am(2, N + 1);
        self.bus.i_am(3, N);
    }

    /// The watch goes on: no point is given up, and a new value of the
    /// device at station 2 reaches it, subscribed.
    pub(super) async fn goes_on(&mut self, value: f32) {
        self.servers[0]
            .set_present_value_local(&ai1(), PropertyValue::Real(value))
            .await
            .unwrap();
        let update = until(&mut self.watch, |u| {
            assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
            number(u) == Some(f64::from(value))
        })
        .await;
        assert_eq!(update.state, State::Subscribed);
        assert_eq!(update.at, at(2));
    }

    /// No update for `minutes` gives a point up, or moves a following one.
    pub(super) async fn none_given_up(&mut self, minutes: u64) {
        for _ in 0..minutes * 6 {
            tokio::time::sleep(Duration::from_secs(10)).await;
            while let Some(update) = ready(&mut self.watch) {
                assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
                assert!(update.moved.is_none(), "{update:?}");
                assert_eq!(update.at, at(2), "{update:?}");
            }
        }
    }

    pub(super) fn reads_at(&self, station: u8) -> usize {
        wildcard_reads(&self.sent, station)
    }

    /// Tell the hub that a handle following `N` moved it from station 2 to
    /// `station`, as one does on the table's rows.
    pub(super) fn tell(&self, station: u8) {
        let (from, to) = (at(2), at(station));
        let moved = ClientEvent::Moved {
            instance: N,
            from,
            to,
        };
        self.client.shared.events.send(moved).unwrap();
    }

    pub(super) async fn stop(self) {
        drop((self.watch, self.device));
        settle().await;
        stop(self.servers).await;
    }
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

pub(super) fn moved_to(update: &Update) -> Option<&DeviceAddress> {
    match &update.state {
        State::Gone {
            why: Why::Moved { to },
        } => Some(to),
        _ => None,
    }
}

/// Reads of the Device object by the wildcard instance sent to `station`.
pub(super) fn wildcard_reads(sent: &SendLog, station: u8) -> usize {
    let wildcard = |request: &bacnet_encoding::apdu::ConfirmedRequest| {
        request.service_choice == ConfirmedServiceChoice::READ_PROPERTY
            && ReadPropertyRequest::decode(&request.service_request).is_ok_and(|read| {
                let object = read.object_identifier;
                object.object_type() == ObjectType::DEVICE
                    && object.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
            })
    };
    sent.unicasts()
        .iter()
        .filter(|frame| frame.mac.as_slice() == mac(station))
        .filter(
            |frame| matches!(frame.apdu(), Apdu::ConfirmedRequest(request) if wildcard(&request)),
        )
        .count()
}

/// The issue's pair: a forged I-Am of another device at the watched
/// address and one of the watched instance at station 3. A following
/// handle's next request asks station 2 which device it is first (#2795):
/// still the watched one, the handle stays and its read goes there.
/// Nothing is read at station 3, and the watch goes on there.
#[tokio::test(start_paused = true)]
async fn a_forged_pair_moves_no_handle_and_ends_no_watch() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.forge();
    settle().await;
    let device = lab.device.as_ref().unwrap();
    device.read("ai:1 pv").await.unwrap();
    assert_eq!(device.address(), at(2), "the handle stays");
    assert!(lab.events.try_recv().is_err(), "nothing moved");
    lab.goes_on(21.0).await;
    lab.none_given_up(10).await;
    assert_eq!(lab.reads_at(3), 0, "nothing read at station 3");
    lab.goes_on(22.0).await;
    lab.stop().await;
}

/// A real move: the device answers at station 3, and station 2 is silent.
/// The handle moves for its own request once station 3 names the device;
/// the hub's own read at station 3 names it too, station 2 asked once more
/// is silent, and only then does the following point go there: the same
/// point, its update saying where it went, then values from station 3.
#[tokio::test(start_paused = true)]
async fn a_move_is_followed_once_the_new_address_names_the_device() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.silence(2);
    lab.bus.i_am(3, N);
    let device = lab.device.as_ref().unwrap();
    let point = device.point("ai:1").unwrap();
    // The first read times out and asks; the next, past the resolve wait,
    // finds only station 3 answered, and moves there once it names the
    // device.
    assert!(device.read("ai:1 pv").await.is_err());
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.reads_at(3), 0, "no move yet, no read");
    assert!(device.read("ai:1 pv").await.is_ok());
    assert_eq!(device.address(), at(3));
    assert_eq!(lab.reads_at(3), 1, "the handle asked station 3 first");
    let update = until(&mut lab.watch, |u| u.moved.is_some()).await;
    assert_eq!(update.point, point, "the same point");
    let moved = update.moved.clone().unwrap();
    assert_eq!(
        (&moved.from, &moved.to, &update.at),
        (&at(2), &at(3), &at(3))
    );
    assert!(lab.reads_at(3) >= 2, "the hub read the Device object there");
    assert_eq!(lab.client.said(&at(3)), Some(N), "and took its answer");
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    lab.servers[1]
        .set_present_value_local(&ai1(), PropertyValue::Real(28.0))
        .await
        .unwrap();
    let update = until(&mut lab.watch, |u| number(u) == Some(28.0)).await;
    assert_eq!((update.state, update.at), (State::Subscribed, at(3)));
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// A hub told that a handle moved to station 3 (as one does once station 3
/// named the device to it), where another device answers: the hub's own
/// read there names that one, and nothing changes for the watch.
#[tokio::test(start_paused = true)]
async fn a_new_address_naming_another_device_changes_nothing() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers
        .push(server(lab.bus.port(3), N + 7, "other").await);
    settle().await;
    lab.forge();
    lab.tell(3);
    settle().await;
    lab.goes_on(23.0).await;
    lab.none_given_up(10).await;
    assert_eq!(lab.reads_at(3), 1, "one read at the new address");
    assert_eq!(lab.client.said(&at(3)), Some(N + 7));
    lab.goes_on(24.0).await;
    lab.stop().await;
}

/// The forged pair again every 20 s for an hour, a following handle
/// re-resolved each time, as a front end does for each command, and the
/// hub told each time that a handle moved to station 3: re-resolving moves
/// no handle (#2795), and the hub reads station 3 at once, then on an ask's
/// pace (60 s, doubling up to the five-minute lifetime), 14 times in the
/// hour, and the watch goes on.
#[tokio::test(start_paused = true)]
async fn an_unanswered_new_address_is_read_on_an_asks_pace() {
    let mut lab = Lab::new(Handle::Follows).await;
    let mut moves = 0;
    for _ in 0..180 {
        lab.forge();
        settle().await;
        let there = lab.client.device(N).await.unwrap();
        assert_eq!(there.address(), at(2), "re-resolving moves no handle");
        while let Ok(event) = lab.events.try_recv() {
            moves += usize::from(matches!(event, ClientEvent::Moved { from, .. } if from == at(2)));
        }
        lab.tell(3);
        // The test's own word to the hub, not a handle's move.
        while lab.events.try_recv().is_ok() {}
        // The device announces itself again, and station 3 another.
        lab.bus.i_am(2, N);
        lab.bus.i_am(3, N + 2);
        settle().await;
        drop(there);
        while let Some(update) = ready(&mut lab.watch) {
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
            assert!(update.moved.is_none(), "{update:?}");
        }
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    assert_eq!(moves, 0, "no handle moved");
    // At once, after 60, 120 and 240 s, then every 300 s.
    assert_eq!(lab.reads_at(3), 14, "reads at station 3 in the hour");
    lab.goes_on(25.0).await;
    lab.stop().await;
}

/// The forged pair while the device misses the read its forged claim
/// cues: the hub looks for the device (the handle's rule, or the table's
/// alone), finds station 3, reads it once, hears nothing, and the watch
/// goes on once the device answers again. The hub's look moves no handle
/// (#2795).
#[tokio::test(start_paused = true)]
async fn a_forged_pair_with_a_missed_read_ends_no_watch() {
    for handle in HANDLES {
        let mut lab = Lab::new(handle).await;
        lab.bus.silence(2);
        lab.forge();
        tokio::time::sleep(Duration::from_secs(5)).await;
        lab.bus.unsilence(2);
        lab.goes_on(26.0).await;
        assert_eq!(lab.reads_at(3), 1, "{handle:?}: one read at station 3");
        lab.none_given_up(10).await;
        lab.goes_on(27.0).await;
        if let Handle::Follows = handle {
            let device = lab.device.as_ref().unwrap();
            assert_eq!(device.address(), at(2), "the look moved no handle");
        }
        lab.stop().await;
    }
}

/// A real move with the handle held, dropped or pinned: the device answers
/// at station 3, station 2 is silent and another device announced itself
/// there. Once the hub's read there names the device, a pinned point is
/// given up as moved to station 3, and a following one goes there.
#[tokio::test(start_paused = true)]
async fn a_move_found_by_the_hub_is_taken_after_its_read() {
    for handle in HANDLES {
        let mut lab = Lab::new(handle).await;
        lab.servers.push(server(lab.bus.port(3), N, "moved").await);
        lab.bus.silence(2);
        lab.bus.i_am(3, N);
        lab.bus.i_am(2, N + 1);
        let update = until(&mut lab.watch, |u| {
            matches!(u.state, State::Gone { .. }) || u.moved.is_some()
        })
        .await;
        match handle {
            Handle::Pinned => assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}"),
            Handle::Follows | Handle::Dropped => {
                assert_eq!(update.at, at(3), "{handle:?}: {update:?}");
                let update = until(&mut lab.watch, |u| u.state == State::Subscribed).await;
                assert_eq!(update.at, at(3), "{handle:?}");
                assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 1);
            }
        }
        assert!(lab.reads_at(3) >= 1, "{handle:?}: the hub read there");
        assert_eq!(lab.client.said(&at(3)), Some(N), "{handle:?}: its answer");
        lab.stop().await;
    }
}
