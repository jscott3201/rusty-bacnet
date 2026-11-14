//! Routed stations behind an in-process router (#2560): a station named by
//! its network and MAC is reached there directly, even while its instance
//! is claimed on another network too.

use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::test_bus::{Bus, mac, server};
use super::*;
use crate::client::{Client, WhoIsScope};
use crate::format::decode_all;
use crate::limits::RequestLimits;

const N: u32 = 1001;

/// Station 5 on network 2 and station 6 on network 3 both claim N; the
/// instance alone is refused, `2:0x..` reaches station 5 through the
/// router, and `3:N` follows the one claim on network 3.
#[tokio::test(start_paused = true)]
async fn a_routed_station_is_reached_by_its_address() {
    let (near, two, three) = (Bus::new(), Bus::new(), Bus::new());
    let ports = vec![
        RouterPort {
            transport: near.port(254),
            network_number: 1,
        },
        RouterPort {
            transport: two.port(254),
            network_number: 2,
        },
        RouterPort {
            transport: three.port(254),
            network_number: 3,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let servers = vec![
        server(two.port(5), N, "on two").await,
        server(three.port(6), N, "on three").await,
    ];
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let low = BACnetClient::start(config, near.port(1)).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    let found = client
        .discover(WhoIsScope::Global, None, Duration::from_millis(500))
        .await
        .unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(matches!(
        client.device(N).await,
        Err(Error::AmbiguousDevice { instance: N, .. })
    ));
    let name = async |device: &Device<_>| {
        let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let data = device
            .read_property(ai, PropertyIdentifier::OBJECT_NAME, None)
            .await
            .unwrap();
        match decode_all(&data).as_deref() {
            Some([PropertyValue::CharacterString(name)]) => name.clone(),
            other => panic!("a name: {other:?}"),
        }
    };
    let station = client.device("2:0x0A000005BAC0").await.unwrap();
    assert_eq!(station.address(), DeviceAddress::new(2, &mac(5)).unwrap());
    assert!(!station.follows());
    assert_eq!(name(&station).await, "on two");
    let following = client.device("3:1001").await.unwrap();
    assert!(following.follows());
    assert_eq!(name(&following).await, "on three");
    for mut server in servers {
        server.stop().await.unwrap();
    }
    router.stop().await.unwrap();
}
