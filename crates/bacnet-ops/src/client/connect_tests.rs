//! The connect builders (#2560): a B/IP client end to end over loopback,
//! and the others built to the point of connecting.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;
use crate::device::Device;
use crate::target::format_mac;

/// A device with one analog input on 127.0.0.1, at a port of its own.
async fn lab_device(instance: u32) -> BACnetServer<BipTransport> {
    let identity = DeviceIdentity::new(instance, 42).unwrap();
    let ai = AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![Box::new(ai)];
    let database = build_database_with_extra(&identity, objects).unwrap();
    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(identity)
        .database(database)
        .build()
        .await
        .unwrap()
}

/// `Client::bip()` connects without naming a transport type, and the
/// client it gives reads a device on loopback by its address.
#[tokio::test]
async fn a_bip_client_connects_and_reads_over_loopback() {
    let mut server = lab_device(2560).await;
    let address = format_mac(server.local_mac());
    let client: Client = Client::bip()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout(Duration::from_secs(2))
        .apdu_retries(1)
        .limits(RequestLimits::new(50))
        .resolve_scope(ResolveScope::Network(5))
        .connect()
        .await
        .unwrap();
    assert_eq!(client.resolve_scope(), ResolveScope::Network(5));
    let device: Device = client.device(address.as_str()).await.unwrap();
    assert_eq!(device.resolve_instance().await.unwrap(), 2560);
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let name = device
        .read((ai, PropertyIdentifier::OBJECT_NAME))
        .await
        .unwrap();
    assert_eq!(name.as_str(), Some("Zone Temp"));
    drop((device, client));
    server.stop().await.unwrap();
}

/// A low-level client wraps as it is, its own transport type kept.
#[tokio::test]
async fn a_low_level_client_wraps_over() {
    let low = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let client: Client<BipTransport> = Client::over(low);
    assert!(client.low_level().transport().local_mac().len() == 6);
}

/// The B/IPv6 builder takes its settings and makes a connect future;
/// compile-level coverage, since a CI host may have no IPv6 multicast.
#[test]
fn the_bip6_builder_takes_its_settings() {
    let foreign = bacnet_transport::bip6::Bip6ForeignDeviceConfig {
        bbmd: "[::1]:47808".parse().unwrap(),
        ttl: 60,
    };
    let connect = Client::bip6()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        .foreign_device(foreign)
        .apdu_timeout(Duration::from_secs(1))
        .apdu_retries(0)
        .limits(RequestLimits::new(0))
        .connect();
    drop(connect);
}

/// The SC builder's settings, to the point of dialling: compile-level
/// coverage, since a hub needs TLS material.
#[cfg(feature = "sc-tls")]
#[test]
fn the_sc_builder_takes_its_settings() {
    use bacnet_transport::sc::ScReconnectConfig;
    use bacnet_transport::sc_tls::ScNodeTlsConfig;
    let _connect = |tls: ScNodeTlsConfig| {
        Client::sc("wss://hub.example.com/bacnet", tls)
            .vmac([0x02, 0, 0, 0, 0, 0x01])
            .device_uuid([0x42; 16])
            .failover_hub_url("wss://failover.example.com/bacnet")
            .reconnect(ScReconnectConfig::default())
            .apdu_timeout(Duration::from_secs(3))
            .apdu_retries(2)
            .limits(RequestLimits::new(10))
            .connect()
    };
}

/// The MS/TP builder checks its settings before it opens the port, then
/// fails on a port that isn't there.
#[cfg(feature = "serial")]
#[tokio::test]
async fn the_mstp_builder_checks_then_opens_its_port() {
    let port = "/dev/no-such-bacnet-port";
    // Settings the transport refuses are text to fix, and the port is
    // never opened for them: a MAC above Max_Master (the default 127, or
    // one set lower) and no information frame per token.
    for builder in [
        Client::mstp(port, 200),
        Client::mstp(port, 5).max_master(3),
        Client::mstp(port, 5).max_info_frames(0),
    ] {
        let Err(refused) = builder.connect().await else {
            panic!("refused settings joined");
        };
        assert!(
            matches!(
                &refused,
                OpsError::Parse {
                    what: "MS/TP settings",
                    ..
                }
            ),
            "{refused:?}"
        );
        assert!(!refused.to_string().contains(port), "{refused}");
    }
    let Err(missing) = Client::mstp(port, 5)
        .baud(76_800)
        .apdu_timeout(Duration::from_secs(2))
        .connect()
        .await
    else {
        panic!("a missing port opened");
    };
    assert!(missing.to_string().contains(port), "{missing}");
}
