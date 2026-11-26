//! Following points at the edges (#2795): a point added just as a move is
//! taken, handles moved only when the old address was asked, a device that
//! can't say which it is, and frames from the address a device left.

use std::time::Duration;

use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, UnconfirmedServiceChoice};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::State;
use super::hub::Hub;
use super::move_cue_tests::{Handle, Lab};
use super::stream_tests::{number, ready, until};
use super::*;
use crate::device::test_bus::{Bus, N, at, client, requests, server, settle, stop};

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

/// The hub the client's watches of its own settings run on.
fn pooled_hub(client: &crate::Client<TestTransport>) -> Hub<TestTransport> {
    let settings = client.watch_settings().clone();
    let mut pool = client.shared.watch_pool.lock().unwrap();
    pool.hub(&settings, || unreachable!("a live hub")).unwrap()
}

/// Concurrency, made certain: a following point added in the moment the
/// scheduler at the old address takes the move, after it last took in
/// its arrivals. It goes to the new address with the others, never
/// started where the device left.
#[tokio::test(start_paused = true)]
async fn a_point_added_as_a_move_is_taken_goes_with_it() {
    let mut lab = Lab::new(Handle::Follows).await;
    let control = lab.watch.control();
    let added = lab
        .device
        .as_ref()
        .unwrap()
        .point("ai:1 status-flags")
        .unwrap();
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.silence(2);
    let hub = pooled_hub(&lab.client);
    let adding = control.clone();
    let point = added.clone();
    *hub.inner.registry.before_move.lock().unwrap() = Some(Box::new(move || {
        assert_eq!(adding.add([point]).unwrap(), 1);
    }));
    lab.tell(3);
    let update = until(&mut lab.watch, |u| {
        u.point == added && u.state == State::Subscribed
    })
    .await;
    assert_eq!(update.at, at(3), "{update:?}");
    assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 2);
    drop(control);
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// Concurrency: a following point added on another thread at a spread of
/// moments around a move the hub takes. Wherever it lands, it ends
/// subscribed at the new address.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_point_added_racing_a_move_is_never_stranded() {
    for delay in [990u64, 1000, 1010] {
        let mut lab = Lab::new(Handle::Follows).await;
        let control = lab.watch.control();
        let added = lab
            .device
            .as_ref()
            .unwrap()
            .point("ai:1 status-flags")
            .unwrap();
        lab.servers.push(server(lab.bus.port(3), N, "moved").await);
        lab.bus.silence(2);
        lab.tell(3);
        let point = added.clone();
        let adding = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            adding.add([point]).unwrap();
        })
        .await
        .unwrap();
        let wait = Duration::from_secs(20);
        let update = tokio::time::timeout(
            wait,
            until(&mut lab.watch, |u| {
                u.point == added && u.state == State::Subscribed
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("{delay} ms: stranded"));
        assert_eq!(update.at, at(3), "{delay} ms");
        drop(control);
        lab.bus.unsilence(2);
        lab.stop().await;
    }
}

/// The hub's own look finds the device at station 3 after its own request
/// at station 2 went unanswered, and takes the move without asking
/// station 2 again: its following points go there, but the handles don't
/// move, as nothing asked their own address (#2795).
#[tokio::test(start_paused = true)]
async fn the_hubs_own_look_moves_no_handle() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, sent) = client(&bus).await;
    let device = client.device(N).await.unwrap();
    let mut events = client.events();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.silence(2);
    let update = until(&mut watch, |u| u.moved.is_some()).await;
    assert_eq!(update.at, at(3));
    assert_eq!(device.address(), at(2), "the handle stays");
    assert!(events.try_recv().is_err(), "no handle moved");
    // A write goes to station 2, where the handle holds the device, and
    // nowhere else.
    let write = bacnet_types::enums::ConfirmedServiceChoice::WRITE_PROPERTY;
    let error = device.write("ai:1 oos", "true", None).await.unwrap_err();
    assert!(error.timed_out(), "{error:?}");
    assert_eq!(requests(&sent, write, 2), 1);
    assert_eq!(requests(&sent, write, 3), 0, "nothing written at station 3");
    drop((watch, device));
    settle().await;
    bus.unsilence(2);
    stop(servers).await;
}

/// A following point placed at an address whose device refuses the read
/// that names it: nothing there can confirm the device (a read by the
/// instance names nothing, and no address it left was asked), so nothing
/// it answers is taken and nothing is subscribed: the point is unreachable
/// as unconfirmed.
#[tokio::test(start_paused = true)]
async fn a_following_point_at_a_wildcard_refuser_is_unconfirmed() {
    let bus = Bus::new();
    let servers = vec![server(bus.port(2), N, "refuser").await];
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    client.refuse_wildcard(&at(2));
    let point = Point {
        device: Binding::Follows {
            instance: N,
            network: None,
        },
        object: ai(1),
        watched: Watched::Object,
    };
    let mut watch = client.watch([point]).unwrap();
    let update = until(&mut watch, |u| u.state != State::Starting).await;
    assert_eq!(
        update.state,
        State::Unreachable {
            why: Why::Unconfirmed
        }
    );
    assert!(update.value.is_none());
    assert_eq!(servers[0].cov_counters().subscriptions_active, 0);
    drop(watch);
    settle().await;
    stop(servers).await;
}

/// After a move to station 3, frames from station 2 take nothing: a COV
/// notification for the point's process identifier is dropped (no value,
/// no move back), and I-Ams of the device from station 2 move nothing
/// while station 3 answers. The watch goes on at station 3.
#[tokio::test(start_paused = true)]
async fn frames_from_the_address_left_take_nothing() {
    let mut lab = Lab::new(Handle::Follows).await;
    let point = lab.device.as_ref().unwrap().point("ai:1").unwrap();
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.silence(2);
    lab.tell(3);
    until(&mut lab.watch, |u| {
        u.at == at(3) && u.state == State::Subscribed
    })
    .await;
    let hub = pooled_hub(&lab.client);
    let process = {
        let table = hub.inner.registry.table();
        table.entries[&hub.keyed(point.clone())].shared.process
    };
    drop(hub);
    lab.bus.unsilence(2);
    let mut service = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, N).unwrap(),
        monitored_object_identifier: ai(1),
        time_remaining: 300,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: [&[0x44][..], &77.0f32.to_be_bytes()].concat(),
            priority: None,
        }],
    }
    .encode(&mut service)
    .unwrap();
    let mut frame = BytesMut::from(&[1u8, 0][..]);
    let apdu = bacnet_encoding::apdu::Apdu::UnconfirmedRequest(
        bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: service.freeze(),
        },
    );
    bacnet_encoding::apdu::encode_apdu(&mut frame, &apdu).unwrap();
    for _ in 0..30 {
        lab.bus.send_from(2, &frame, 1);
        lab.bus.i_am(2, N);
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(&mut lab.watch) {
            assert_ne!(number(&update), Some(77.0), "{update:?}");
            assert_eq!(update.at, at(3), "{update:?}");
        }
    }
    lab.servers[1]
        .set_present_value_local(&ai(1), PropertyValue::Real(78.0))
        .await
        .unwrap();
    let update = until(&mut lab.watch, |u| number(u) == Some(78.0)).await;
    assert_eq!(update.at, at(3));
    lab.stop().await;
}

/// A following point left unconfirmed at station 2 (its device refuses the
/// read that names it) goes to station 3 when a cue sends the hub there
/// (as another handle's move would) and its read finds the device, but
/// nothing asked station 2 which device it is: this handle stays.
#[tokio::test(start_paused = true)]
async fn an_unconfirmed_point_moves_no_handle() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "refuser").await];
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    client.refuse_wildcard(&at(2));
    let device = client.device(N).await.unwrap();
    let mut events = client.events();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state != State::Starting).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.i_am(3, N);
    pooled_hub(&client).inner.registry.moved(N, &at(2), at(3));
    let update = until(&mut watch, |u| u.moved.is_some()).await;
    assert_eq!(update.at, at(3));
    assert_eq!(device.address(), at(2), "the handle stays");
    assert!(events.try_recv().is_err(), "no handle moved");
    drop((watch, device));
    settle().await;
    stop(servers).await;
}
