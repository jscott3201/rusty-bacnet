//! A removed point's updates held back by the binding (#2603): an update a
//! cancelled `__anext__` puts back after `remove` is dropped as it comes
//! ([`Pruned`]), and one put back before is dropped from the receiver's
//! slot by `remove` itself; an update of a point still held goes back as
//! before. A real watch of a device on loopback makes the updates; the
//! put-backs are driven by hand, as a cancel found before delivery makes
//! them ([`Via::BeforeDelivery`]), so the order is the test's.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_ops::target::{AddressStyle, format_mac};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::EngineeringUnits;
use pyo3::prelude::*;

use super::{Pruned, WatchShared};
use crate::event_stream::Next;
use crate::high_level::watch_update::PyWatchUpdate;
use crate::py_async::{PutBack, Via};

/// A device with two analog inputs on 127.0.0.1, and a watch of both
/// from a client there.
async fn lab() -> (
    BACnetServer<BipTransport>,
    Arc<WatchShared>,
    [bacnet_ops::watch::Point; 2],
) {
    let identity = DeviceIdentity::new(2603, 42).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = (1..=2)
        .map(|n| {
            let ai = AnalogInputObject::new(n, format!("AI {n}"), EngineeringUnits::PERCENT);
            Box::new(ai.unwrap()) as Box<dyn BACnetObject>
        })
        .collect();
    let database = build_database_with_extra(&identity, objects).unwrap();
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(identity)
        .database(database)
        .build()
        .await
        .unwrap();
    let low = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await
        .unwrap();
    let client = bacnet_ops::Client::over(low);
    let address = format_mac(server.local_mac());
    let device = client.device(address.as_str()).await.unwrap();
    let points = [device.point("ai:1").unwrap(), device.point("ai:2").unwrap()];
    let watch = client.watch(points.clone()).unwrap();
    (server, Arc::new(WatchShared::new(watch)), points)
}

/// The next update the receiver hands out, fresh from the watch, with the
/// hook its delivery puts it back through.
async fn take(shared: &WatchShared) -> (Box<bacnet_ops::Update>, Pruned) {
    let (next, inner) = shared.stream.next();
    let Some(Next::Fresh(update)) = tokio::time::timeout(Duration::from_secs(10), next)
        .await
        .expect("an update")
    else {
        panic!("a fresh update");
    };
    let pruned = Pruned {
        inner,
        control: shared.control.clone(),
    };
    (update, pruned)
}

/// Put `update` back, as a cancel found before delivery does.
fn put_back(pruned: &Pruned, update: &bacnet_ops::Update) {
    Python::attach(|py| {
        let object = Bound::new(
            py,
            PyWatchUpdate::new(py, update, AddressStyle::Bip).unwrap(),
        )
        .unwrap()
        .into_any()
        .unbind();
        pruned.put_back(py, object, Via::BeforeDelivery);
        pruned.settled();
    });
}

/// What the receiver hands out next within a short wait: `None` when
/// nothing waits.
async fn waiting(shared: &WatchShared) -> Option<Next<Box<bacnet_ops::Update>>> {
    let (next, hook) = shared.stream.next();
    let next = tokio::time::timeout(Duration::from_millis(300), next)
        .await
        .ok()
        .flatten();
    hook.settled();
    next
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_put_back_after_its_removal_is_dropped() {
    Python::initialize();
    let (mut server, shared, points) = lab().await;
    let (update, pruned) = take(&shared).await;
    let removed =
        Python::attach(|py| shared.remove_points(py, std::slice::from_ref(&update.point)));
    assert_eq!(removed.unwrap(), 1);
    put_back(&pruned, &update);
    // The other point's first update may come; the removed one's never.
    while let Some(next) = waiting(&shared).await {
        let Next::Fresh(next) = next else {
            panic!("the removed point's update came back");
        };
        assert_ne!(next.point, update.point, "{next:?}");
    }
    assert!(points.contains(&update.point));
    shared
        .end(super::Ended::Closed, Duration::from_secs(5))
        .await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_put_back_before_its_removal_is_purged() {
    Python::initialize();
    let (mut server, shared, _) = lab().await;
    let (update, pruned) = take(&shared).await;
    // Still held: back into the slot it goes.
    put_back(&pruned, &update);
    let removed =
        Python::attach(|py| shared.remove_points(py, std::slice::from_ref(&update.point)));
    assert_eq!(removed.unwrap(), 1);
    while let Some(next) = waiting(&shared).await {
        let Next::Fresh(next) = next else {
            panic!("the removed point's update stayed in the slot");
        };
        assert_ne!(next.point, update.point, "{next:?}");
    }
    shared
        .end(super::Ended::Closed, Duration::from_secs(5))
        .await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_of_a_point_still_held_goes_back() {
    Python::initialize();
    let (mut server, shared, _) = lab().await;
    let (update, pruned) = take(&shared).await;
    put_back(&pruned, &update);
    match waiting(&shared).await {
        Some(Next::Again(_)) => {}
        _ => panic!("the put-back update first"),
    }
    shared
        .end(super::Ended::Closed, Duration::from_secs(5))
        .await;
    server.stop().await.unwrap();
}
