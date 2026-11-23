//! A client on a router's application port (#2601): the local scope is
//! each of the router's networks, isolated ports among them, and no
//! network beyond; the devices answer as routed devices on their networks,
//! as when each network is named.

use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{BACnetRouter, RouterApplicationPort, RouterOptions, RouterPort};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::test_bus::{Bus, config, mac, server, stop};
use super::*;
use crate::client::{Discover, Find, Scope};
use crate::limits::RequestLimits;

/// The device on each network: network 1's at station 5, isolated network
/// 2's at 6, isolated network 4's at 7, and network 3's at 8, behind a
/// second router on network 2.
const DEVICES: [(u16, u8, u32); 4] = [(1, 5, 2001), (2, 6, 2002), (4, 7, 2004), (3, 8, 2003)];

/// A router on networks 1, 2 and 4 (2 and 4 isolated) with a client on its
/// application port, a second router from network 2 to 3, and a device on
/// each network.
struct Lab {
    routers: Vec<BACnetRouter>,
    servers: Vec<BACnetServer<TestTransport>>,
    client: Client<RouterApplicationPort>,
}

impl Lab {
    async fn new() -> Self {
        let (one, two, three, four) = (Bus::new(), Bus::new(), Bus::new(), Bus::new());
        let ports = vec![
            RouterPort::new(one.port(254), 1),
            RouterPort::new(two.port(254), 2).isolated(true),
            RouterPort::new(four.port(254), 4).isolated(true),
        ];
        let started = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        let (near, port, _controls) = started.into_application_port();
        // Started second, so the near router hears it announce network 3.
        let far = vec![
            RouterPort::new(two.port(253), 2),
            RouterPort::new(three.port(253), 3),
        ];
        let far = BACnetRouter::start(far, RouterOptions::new())
            .await
            .unwrap()
            .router;
        let buses = [(1, &one), (2, &two), (4, &four), (3, &three)];
        let mut servers = Vec::new();
        for ((network, bus), (_, station, instance)) in buses.into_iter().zip(DEVICES) {
            let name = format!("on {network}");
            servers.push(server(bus.port(station), instance, &name).await);
        }
        let low = BACnetClient::start(config(), port).await.unwrap();
        let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
        Self {
            routers: vec![near, far],
            servers,
            client,
        }
    }

    async fn discover(&self, request: Discover) -> Vec<(DeviceAddress, u32)> {
        let found = self
            .client
            .discover(request.wait(Duration::from_millis(500)))
            .await
            .unwrap();
        found.into_iter().map(|f| (f.address, f.instance)).collect()
    }

    /// The discovery table, each row but the times it was heard.
    async fn table(&self) -> Vec<impl Ord + std::fmt::Debug> {
        let mut rows: Vec<_> = self
            .client
            .devices()
            .await
            .into_iter()
            .map(|row| {
                (
                    row.object_identifier.instance_number(),
                    row.source_network,
                    row.source_address.map(|mac| mac.to_vec()),
                    row.mac_address.to_vec(),
                    row.max_apdu_length,
                    row.max_segments_accepted,
                    row.vendor_id,
                    format!("{:?}", row.segmentation_supported),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    async fn stop(mut self) {
        stop(std::mem::take(&mut self.servers)).await;
        for mut router in self.routers {
            router.stop().await.unwrap();
        }
    }
}

/// The devices on networks `networks`, as each answers.
fn on(networks: &[u16]) -> Vec<(DeviceAddress, u32)> {
    let mut found: Vec<_> = DEVICES
        .iter()
        .filter(|(network, ..)| networks.contains(network))
        .map(|&(network, station, instance)| {
            (
                DeviceAddress::new(network, &mac(station)).unwrap(),
                instance,
            )
        })
        .collect();
    found.sort();
    found
}

/// The default discovery asks networks 1, 2 and 4, isolated or not, and
/// not network 3 beyond; every network is asked only when named so.
#[tokio::test(start_paused = true)]
async fn a_local_discovery_asks_each_attached_network() {
    let lab = Lab::new().await;
    assert_eq!(lab.client.low_level().local_networks(), [1, 2, 4]);
    assert_eq!(lab.discover(Discover::default()).await, on(&[1, 2, 4]));
    assert_eq!(lab.discover(Discover::global()).await, on(&[1, 2, 3, 4]));
    lab.stop().await;
}

/// The local discovery and one naming each attached network leave the
/// same discovery table and find the same devices.
#[tokio::test(start_paused = true)]
async fn local_and_each_network_named_end_with_the_same_table() {
    let local = Lab::new().await;
    let found = local.discover(Discover::local()).await;
    let named = Lab::new().await;
    let mut each = Vec::new();
    for network in [1, 2, 4] {
        each.extend(named.discover(Discover::network(network)).await);
    }
    each.sort();
    assert_eq!(found, each);
    assert_eq!(local.table().await, named.table().await);
    local.stop().await;
    named.stop().await;
}

/// A local search finds the object on each attached network and not
/// beyond; a bare instance is resolved there, and one beyond is unknown,
/// asked locally.
#[tokio::test(start_paused = true)]
async fn a_search_and_a_bare_instance_stay_on_the_attached_networks() {
    let lab = Lab::new().await;
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let found = lab
        .client
        .find(Find::object(ai).wait(Duration::from_millis(500)))
        .await
        .unwrap();
    let mut found: Vec<_> = found.into_iter().map(|f| (f.address, f.device)).collect();
    found.sort();
    assert_eq!(found, on(&[1, 2, 4]));

    let device = lab.client.device(2004).await.unwrap();
    assert_eq!(device.address(), DeviceAddress::new(4, &mac(7)).unwrap());
    let name = (
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        PropertyIdentifier::OBJECT_NAME,
    );
    let value = device.read(name).await.unwrap();
    assert_eq!(value.as_str(), Some("on 4"));
    assert!(matches!(
        lab.client.device(2003).await,
        Err(OpsError::UnknownDevice {
            asked: Some(Scope::Local),
            ..
        })
    ));
    lab.stop().await;
}
