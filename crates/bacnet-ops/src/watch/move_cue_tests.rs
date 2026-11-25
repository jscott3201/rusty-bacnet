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
enum Handle {
    /// A handle following the instance, held.
    Follows,
    /// A handle following the instance, dropped once the point was made.
    Dropped,
    /// A handle pinned to the address.
    Pinned,
}

const HANDLES: [Handle; 3] = [Handle::Pinned, Handle::Dropped, Handle::Follows];

/// Device `N` at station 2, its AI 1 watched and subscribed.
struct Lab {
    bus: Arc<Bus>,
    servers: Vec<BACnetServer<TestTransport>>,
    client: crate::Client<TestTransport>,
    sent: SendLog,
    device: Option<Device<TestTransport>>,
    watch: Watch,
    events: broadcast::Receiver<ClientEvent>,
}

impl Lab {
    async fn new(handle: Handle) -> Self {
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
    fn forge(&self) {
        self.bus.i_am(2, N + 1);
        self.bus.i_am(3, N);
    }

    /// The watch goes on: no point is given up, and a new value of the
    /// device at station 2 reaches it, subscribed.
    async fn goes_on(&mut self, value: f32) {
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
        assert_eq!(update.point.device, at(2));
    }

    /// No update for `minutes` gives a point up.
    async fn none_given_up(&mut self, minutes: u64) {
        for _ in 0..minutes * 6 {
            tokio::time::sleep(Duration::from_secs(10)).await;
            while let Some(update) = ready(&mut self.watch) {
                assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
            }
        }
    }

    fn reads_at(&self, station: u8) -> usize {
        wildcard_reads(&self.sent, station)
    }

    async fn stop(self) {
        drop((self.watch, self.device));
        settle().await;
        stop(self.servers).await;
    }
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn moved_to(update: &Update) -> Option<&DeviceAddress> {
    match &update.state {
        State::Gone {
            why: Why::Moved { to },
        } => Some(to),
        _ => None,
    }
}

/// Reads of the Device object by the wildcard instance sent to `station`.
fn wildcard_reads(sent: &SendLog, station: u8) -> usize {
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
/// address and one of the watched instance at station 3 move a following
/// handle there before its next request. The hub reads station 3 once,
/// hears nothing, and the watch goes on, taking the device's values.
#[tokio::test(start_paused = true)]
async fn a_forged_pair_moving_a_handle_ends_no_watch() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.forge();
    settle().await;
    let device = lab.device.as_ref().unwrap();
    assert!(
        device.read("ai:1 pv").await.is_err(),
        "nothing at station 3"
    );
    let moved = ClientEvent::Moved {
        instance: N,
        from: at(2),
        to: at(3),
    };
    assert_eq!(lab.events.try_recv().unwrap(), moved, "the handle moved");
    lab.goes_on(21.0).await;
    lab.none_given_up(10).await;
    assert_eq!(lab.reads_at(3), 1, "one read at the new address");
    lab.goes_on(22.0).await;
    lab.stop().await;
}

/// A real move: the device answers at station 3, and station 2 is silent.
/// The handle moves for its own request; the hub's read at station 3 names
/// the device, and only then is the point given up as moved there.
#[tokio::test(start_paused = true)]
async fn a_move_is_given_up_once_the_new_address_names_the_device() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.silence(2);
    lab.bus.i_am(3, N);
    let device = lab.device.as_ref().unwrap();
    // The first read times out and asks; the next, past the resolve wait,
    // finds only station 3 answered and moves there.
    assert!(device.read("ai:1 pv").await.is_err());
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.reads_at(3), 0, "no move yet, no read");
    assert!(device.read("ai:1 pv").await.is_ok());
    assert_eq!(device.address(), at(3));
    let update = until(&mut lab.watch, |u| matches!(u.state, State::Gone { .. })).await;
    assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
    assert_eq!(lab.reads_at(3), 1, "the hub read the Device object there");
    assert_eq!(lab.client.said(&at(3)), Some(N), "and took its answer");
    lab.stop().await;
}

/// The handle moves on the forged pair to station 3, where another device
/// answers: the hub's read there names that one, and nothing changes for
/// the watch.
#[tokio::test(start_paused = true)]
async fn a_new_address_naming_another_device_changes_nothing() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers
        .push(server(lab.bus.port(3), N + 7, "other").await);
    settle().await;
    lab.forge();
    settle().await;
    lab.device.as_ref().unwrap().read("ai:1 pv").await.ok();
    assert_eq!(lab.device.as_ref().unwrap().address(), at(3));
    lab.goes_on(23.0).await;
    lab.none_given_up(10).await;
    assert_eq!(lab.reads_at(3), 1, "one read at the new address");
    assert_eq!(lab.client.said(&at(3)), Some(N + 7));
    lab.goes_on(24.0).await;
    lab.stop().await;
}

/// The forged pair again every 20 s for an hour, a following handle
/// moving to station 3 and back each time (re-resolved, as a front end
/// does for each command): the hub reads station 3 at once, then on an
/// ask's pace (60 s, doubling up to the five-minute lifetime), 14 times in
/// the hour, and the watch goes on.
#[tokio::test(start_paused = true)]
async fn an_unanswered_new_address_is_read_on_an_asks_pace() {
    let mut lab = Lab::new(Handle::Follows).await;
    let mut moves = 0;
    for _ in 0..180 {
        lab.forge();
        settle().await;
        let there = lab.client.device(N).await.unwrap();
        // The device announces itself again, and station 3 another.
        lab.bus.i_am(2, N);
        lab.bus.i_am(3, N + 2);
        settle().await;
        let back = lab.client.device(N).await.unwrap();
        drop((there, back));
        while let Ok(event) = lab.events.try_recv() {
            moves += usize::from(matches!(event, ClientEvent::Moved { from, .. } if from == at(2)));
        }
        while let Some(update) = ready(&mut lab.watch) {
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
        }
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    assert_eq!(moves, 180, "the handle moved away each time");
    // At once, after 60, 120 and 240 s, then every 300 s.
    assert_eq!(lab.reads_at(3), 14, "reads at station 3 in the hour");
    lab.goes_on(25.0).await;
    lab.stop().await;
}

/// The forged pair while the device misses the read its forged claim
/// cues: the hub looks for the device (the handle's rule, or the table's
/// alone), finds station 3, reads it once, hears nothing, and the watch
/// goes on once the device answers again. A held following handle was
/// moved by the look; the watch stays.
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
            assert_eq!(device.address(), at(3), "the handle and the watch apart");
        }
        lab.stop().await;
    }
}

/// A real move with the handle held, dropped or pinned: the device answers
/// at station 3, station 2 is silent and another device announced itself
/// there. The point is given up as moved to station 3 once the hub's read
/// there names the device.
#[tokio::test(start_paused = true)]
async fn a_move_found_by_the_hub_is_given_up_after_its_read() {
    for handle in HANDLES {
        let mut lab = Lab::new(handle).await;
        lab.servers.push(server(lab.bus.port(3), N, "moved").await);
        lab.bus.silence(2);
        lab.bus.i_am(3, N);
        lab.bus.i_am(2, N + 1);
        let update = until(&mut lab.watch, |u| matches!(u.state, State::Gone { .. })).await;
        assert_eq!(moved_to(&update), Some(&at(3)), "{handle:?}: {update:?}");
        assert_eq!(lab.reads_at(3), 1, "{handle:?}: the hub read there");
        assert_eq!(lab.client.said(&at(3)), Some(N), "{handle:?}: its answer");
        lab.stop().await;
    }
}
