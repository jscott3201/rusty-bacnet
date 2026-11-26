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
use crate::write::{Priority, WritePolicy};

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
        .write_policy(WritePolicy::default().allow_priorities([Priority::CRITICAL_EQUIPMENT]))
        .watch_settings(WatchSettings::default().with_subscribe(false))
        .connect()
        .await
        .unwrap();
    assert_eq!(client.resolve_scope(), ResolveScope::Network(5));
    assert!(client.write_policy().allows(Priority::CRITICAL_EQUIPMENT));
    assert!(!client.watch_settings().subscribe, "the builder's");
    // A 2 s timeout sent twice, from the low-level client's own timers:
    // what the watches' renewals leave room for.
    assert_eq!(client.request_budget(), Duration::from_secs(4));
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
    assert_eq!(client.resolve_scope(), ResolveScope::Local);
}

/// A builder's client resolves a bare instance on this network alone
/// unless the builder says otherwise, as a client made by hand does.
#[tokio::test]
async fn a_builder_client_resolves_locally_by_default() {
    let client = Client::bip()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .connect()
        .await
        .unwrap();
    assert_eq!(client.resolve_scope(), ResolveScope::Local);
    assert_eq!(client.write_policy(), WritePolicy::default());
    assert_eq!(client.watch_settings(), &WatchSettings::default());
    // The default timers: 6 s, sent four times.
    assert_eq!(client.request_budget(), Duration::from_secs(24));
}

/// A client's watches leave room in each renewal for its own timers: a
/// low-level client built with a 20 s timeout and three retries takes up
/// to 80 s a request, so a five-minute subscription renews 150 s before it
/// ends (two such requests, at most half the lifetime), not the 48 s the
/// default timers leave.
#[tokio::test]
async fn a_watch_renews_early_enough_for_the_clients_own_timers() {
    let low = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(20_000)
        .apdu_retries(3)
        .build()
        .await
        .unwrap();
    assert_eq!(low.apdu_timeout(), Duration::from_secs(20));
    assert_eq!(low.apdu_retries(), 3);
    let client: Client<BipTransport> = Client::over(low);
    assert_eq!(client.request_budget(), Duration::from_secs(80));
    let watch = client.watch([]).unwrap();
    let control = watch.control();
    let hubs = client.shared.watch_pool.lock().unwrap().drain();
    assert_eq!(hubs.len(), 1);
    assert_eq!(hubs[0].options().renewal_margin, Duration::from_secs(150));
    drop((hubs, control, watch));
}

/// Watch settings no hub takes are refused by `connect`, before anything
/// is bound: a poll interval past an hour.
#[tokio::test]
async fn a_builder_refuses_watch_settings_before_binding() {
    let slow = WatchSettings::default().with_poll_every(Duration::from_secs(3601));
    let Err(refused) = Client::bip()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .watch_settings(slow)
        .connect()
        .await
    else {
        panic!("refused watch settings connected");
    };
    assert!(
        matches!(
            refused,
            OpsError::Watch(crate::watch::WatchError::Settings(_))
        ),
        "{refused:?}"
    );
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

/// TLS material the SC configuration refuses reaches an `OpsError` by `?`,
/// still naming the input at fault (#2600).
#[cfg(feature = "sc-tls")]
#[test]
fn sc_tls_material_errors_convert_for_the_sc_builder() {
    use crate::{ScNodeTlsConfig, ScTlsConfigError, ScTlsInput};
    use std::error::Error as _;
    fn load(dir: &std::path::Path) -> Result<ScNodeTlsConfig, OpsError> {
        Ok(ScNodeTlsConfig::from_pem_files(
            dir.join("site-ca.pem"),
            dir.join("node.pem"),
            dir.join("node.key"),
        )?)
    }
    let error = load(std::path::Path::new("/no-such-bacnet-dir")).unwrap_err();
    let OpsError::Tls(tls) = &error else {
        panic!("expected OpsError::Tls, got {error:?}");
    };
    assert!(
        matches!(
            tls,
            ScTlsConfigError::Read {
                input: ScTlsInput::CaBundle,
                ..
            }
        ),
        "{tls:?}"
    );
    // The text is the configuration's, and the chain goes on to its cause
    // (the operating system's error), as for the client's errors.
    assert_eq!(error.to_string(), tls.to_string());
    assert!(
        error
            .to_string()
            .contains("CA bundle file /no-such-bacnet-dir"),
        "{error}"
    );
    assert_eq!(
        error.source().map(ToString::to_string),
        tls.source().map(ToString::to_string)
    );
    assert!(!error.unreachable() && !error.timed_out());
    assert!(error.remote().is_none() && error.written().is_none());
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
        Client::mstp(port, 5).baud(50_000),
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
