//! Routed stations behind an in-process router (#2560): a station named by
//! its network and MAC is reached there directly, even while its instance
//! is claimed on another network too; handles following an instance on one
//! network stay there; and an address on the client's own network is the
//! local station.

use std::sync::Arc;
use std::time::Duration;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::error::Error;

use super::test_bus::{Bus, N, client_on, mac, name_of, server, settle, stop};
use super::*;
use crate::client::WhoIsScope;

/// The client at station 1 of network 1, a router at station 254 of
/// networks 1, 2 and 3, and on network 2 station 5 and on network 3
/// station 6, both claiming N.
struct Routed {
    near: Arc<Bus>,
    three: Arc<Bus>,
    router: BACnetRouter,
    servers: Vec<BACnetServer<TestTransport>>,
    client: Client<TestTransport>,
    sent: SendLog,
}

impl Routed {
    /// The lab, its client's port made by `port` on the near network.
    async fn new(port: fn(&Arc<Bus>) -> TestTransport) -> Self {
        let (near, two, three) = (Bus::new(), Bus::new(), Bus::new());
        let ports = [(&near, 1), (&two, 2), (&three, 3)]
            .into_iter()
            .map(|(bus, network_number)| RouterPort {
                transport: bus.port(254),
                network_number,
            })
            .collect();
        let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        let servers = vec![
            server(two.port(5), N, "on two").await,
            server(three.port(6), N, "on three").await,
        ];
        let (client, sent) = client_on(port(&near)).await;
        Self {
            near,
            three,
            router,
            servers,
            client,
            sent,
        }
    }

    /// Discover every network.
    async fn discover(&self) -> Vec<Discovered> {
        self.client
            .discover(WhoIsScope::Global, None, Duration::from_millis(500))
            .await
            .unwrap()
    }

    async fn stop(mut self) {
        stop(std::mem::take(&mut self.servers)).await;
        self.router.stop().await.unwrap();
    }
}

/// Station 5 on network 2 and station 6 on network 3 both claim N; the
/// instance alone is refused, `2:0x..` reaches station 5 through the
/// router, and `3:N` follows the one claim on network 3.
#[tokio::test(start_paused = true)]
async fn a_routed_station_is_reached_by_its_address() {
    let lab = Routed::new(|near| near.port(1)).await;
    let client = &lab.client;
    assert_eq!(lab.discover().await.len(), 2);
    assert!(matches!(
        client.device(N).await,
        Err(Error::AmbiguousDevice { instance: N, .. })
    ));
    let station = client.device("2:0x0A000005BAC0").await.unwrap();
    assert_eq!(station.address(), DeviceAddress::new(2, &mac(5)).unwrap());
    assert!(!station.follows());
    assert_eq!(name_of(&station).await.unwrap(), "on two");
    let following = client.device("3:1001").await.unwrap();
    assert!(following.follows());
    assert_eq!(name_of(&following).await.unwrap(), "on three");
    lab.stop().await;
}

/// Handles following one instance on two networks stay apart: making the
/// second moves nothing, and each reads its own network's device (the
/// reviewer's probe A).
#[tokio::test(start_paused = true)]
async fn following_handles_on_two_networks_stay_apart() {
    let lab = Routed::new(|near| near.port(1)).await;
    let client = &lab.client;
    lab.discover().await;
    let mut events = client.events();
    let three = client.device("3:1001").await.unwrap();
    assert_eq!(name_of(&three).await.unwrap(), "on three");
    let two = client.device("2:1001").await.unwrap();
    assert_eq!(name_of(&two).await.unwrap(), "on two");
    assert!(events.try_recv().is_err(), "nothing moved");
    assert_eq!(three.address().network(), 3);
    assert_eq!(name_of(&three).await.unwrap(), "on three");
    lab.stop().await;
}

/// A routed following handle whose row was purged, while its instance
/// came back at another station on its network, finds it there: no route
/// is reason to look again, as a timeout is (the reviewer's probe C).
#[tokio::test(start_paused = true)]
async fn a_routed_following_handle_finds_its_device_after_a_purge() {
    let mut lab = Routed::new(|near| near.port(1)).await;
    lab.discover().await;
    let following = lab.client.device("3:1001").await.unwrap();
    assert_eq!(name_of(&following).await.unwrap(), "on three");
    // Station 6 leaves; station 7 on network 3 holds N now.
    lab.three.silence(6);
    lab.servers
        .push(server(lab.three.port(7), N, "new on three").await);
    lab.client.low_level().clear_devices().await;
    lab.discover().await;
    assert_eq!(name_of(&following).await.unwrap(), "new on three");
    assert_eq!(following.address(), DeviceAddress::new(3, &mac(7)).unwrap());
    lab.stop().await;
}

/// Once the client knows its own network's number, an address written
/// with it is the local station: no Who-Is goes to find it, no routed row
/// appears beside the local one, and the two names share one gate (the
/// reviewer's probe H).
#[tokio::test(start_paused = true)]
async fn an_address_on_the_own_network_is_local() {
    let mut lab = Routed::new(|near| near.numbered_port(1)).await;
    lab.servers
        .push(server(lab.near.port(2), 2002, "near").await);
    let low = lab.client.low_level();
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    lab.discover().await;
    let local = lab.client.device("10.0.0.2:47808").await.unwrap();
    let before = lab.sent.len();
    let aliased = lab.client.device("1:0x0A000002BAC0").await.unwrap();
    assert_eq!(lab.sent.len(), before, "nothing sent to find it");
    assert_eq!(aliased.address(), local.address());
    assert_eq!(aliased.instance(), Some(2002));
    assert_eq!(name_of(&aliased).await.unwrap(), "near");
    let rows = lab.client.devices().await;
    let claims = rows
        .iter()
        .filter(|row| row.object_identifier.instance_number() == 2002)
        .count();
    assert_eq!(claims, 1, "no routed row beside the local one: {rows:?}");
    assert_eq!(
        lab.client.device(2002).await.unwrap().address(),
        local.address()
    );
    let held = lab.client.limits().gate(&local.address()).acquire_owned();
    let held = held.await.unwrap();
    let read = tokio::time::timeout(Duration::from_secs(10), name_of(&aliased)).await;
    assert!(read.is_err(), "the aliased name went past the local gate");
    drop(held);
    // The routed instance form on the own network is local too.
    let routed = lab.client.device("1:2002").await.unwrap();
    assert_eq!(routed.address(), local.address());
    lab.stop().await;
}

/// Handles named with the own network's number before the client learns
/// it reach the local station once it does, at its gate, and the instance
/// alone finds that one station (the reviewer's probe P5').
#[tokio::test(start_paused = true)]
async fn handles_named_before_the_number_is_learned_are_local_after() {
    use bacnet_client::discovery::RoutedDeviceConfig;
    use bacnet_types::enums::Segmentation;

    let mut lab = Routed::new(|near| near.numbered_port(1)).await;
    lab.servers
        .push(server(lab.near.port(2), 2002, "near").await);
    let low = Arc::clone(lab.client.low_level());
    low.add_routed_device(RoutedDeviceConfig {
        instance: 2002,
        router_mac: mac(254).to_vec(),
        remote_network: 1,
        remote_mac: mac(2).to_vec(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
    })
    .await
    .unwrap();
    let pinned = lab.client.device("1:0x0A000002BAC0").await.unwrap();
    let following = lab.client.device("1:2002").await.unwrap();
    let routed = DeviceAddress::new(1, &mac(2)).unwrap();
    assert_eq!(pinned.address(), routed, "the number isn't known yet");
    assert_eq!(following.address(), routed);
    low.what_is_network_number(None).await.unwrap();
    for _ in 0..50 {
        if low.network_number().is_some() {
            break;
        }
        settle().await;
    }
    assert_eq!(low.network_number(), Some(1), "the router said");
    let local = lab.client.device("10.0.0.2:47808").await.unwrap();
    assert_eq!(pinned.address(), local.address());
    assert_eq!(following.address(), local.address());
    let by_instance = lab.client.device(2002).await.unwrap();
    assert_eq!(by_instance.address(), local.address());
    let held = lab.client.limits().gate(&local.address()).acquire_owned();
    let held = held.await.unwrap();
    for handle in [&pinned, &following] {
        let read = tokio::time::timeout(Duration::from_secs(10), name_of(handle)).await;
        assert!(read.is_err(), "{handle:?} went past the local gate");
    }
    drop(held);
    assert_eq!(name_of(&pinned).await.unwrap(), "near");
    assert_eq!(name_of(&following).await.unwrap(), "near");
    lab.stop().await;
}
