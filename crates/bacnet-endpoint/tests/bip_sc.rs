//! RB-15 concrete transport controls (no real-transport proofs).
//!
//! B/IP modes + SC builder validation live at endpoint level as concrete
//! types (not generic-trait erasure). Real one-socket B/IP + SC proofs are
//! RB-16/17 and stay out of scope.

use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{BbmdOptions, BipMode, FanoutPolicy, ForeignDeviceConfig};
use bacnet_transport::port::TransportPort;
use bacnet_types::bip_port::BipPortMode;

fn bdt_entry(ip: [u8; 4], port: u16) -> BdtEntry {
    BdtEntry {
        ip,
        port,
        broadcast_mask: [255, 255, 255, 255],
    }
}

fn foreign() -> BipMode {
    BipMode::Foreign(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: 47808,
        renewal_interval: None,
        ttl: 60,
    })
}

#[test]
fn bip_builder_applies_the_mode_concretely() {
    let bbmd = BipMode::Bbmd(BbmdOptions {
        bdt: vec![bdt_entry([127, 0, 0, 1], 47808)],
        foreign_device_policy: Some(ForeignDevicePolicy::default()),
        management_acl: vec![[127, 0, 0, 1]],
        fanout_policy: FanoutPolicy::default(),
        bdt_persist_path: None,
        alone_on_subnet: false,
        nat: None,
    });
    // The transport builds unstarted, in the mode given: a BBMD's tables
    // appear only once `start()` creates them.
    let transport = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .queue_capacity(8)
        .mode(bbmd)
        .build_transport()
        .expect("B/IP transport must build");
    assert!(matches!(
        transport.bip_port().unwrap().mode,
        BipPortMode::Bbmd { tables: None }
    ));
    let transport = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .mode(foreign())
        .build_transport()
        .unwrap();
    let BipPortMode::Foreign { registration } = transport.bip_port().unwrap().mode else {
        panic!("foreign mode");
    };
    assert_eq!(registration.subscription_lifetime(), 60);
}

/// One mode at a time: a later `mode` replaces an earlier one, so a BBMD
/// that also registers as a foreign device can't be built (#938).
#[test]
fn bip_builder_keeps_only_the_last_mode() {
    let transport = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .mode(BipMode::Bbmd(BbmdOptions::default()))
        .mode(foreign())
        .build_transport()
        .unwrap();
    assert!(matches!(
        transport.bip_port().unwrap().mode,
        BipPortMode::Foreign { .. }
    ));
    let transport = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .mode(foreign())
        .mode(BipMode::Normal)
        .build_transport()
        .unwrap();
    assert!(matches!(
        transport.bip_port().unwrap().mode,
        BipPortMode::Normal
    ));
}

#[test]
fn bip_builder_builds_unstarted_session() {
    let session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .build_session()
        .expect("unstarted B/IP session must build");
    // Unstarted session owns no leases and no policy outcomes yet.
    assert_eq!(session.active_leases(), 0);
}

#[test]
fn sc_builder_validates_concrete_identity_without_hub_io() {
    let good = ScEndpointBuilder::new([0x02; 6], [0x11; 16]);
    assert!(good.validate_only().is_ok());

    let zero_vmac = ScEndpointBuilder::new([0; 6], [0x11; 16]);
    assert!(zero_vmac.validate_only().is_err());

    let broadcast_vmac =
        ScEndpointBuilder::new(bacnet_transport::sc_frame::BROADCAST_VMAC, [0x11; 16]);
    assert!(broadcast_vmac.validate_only().is_err());

    let zero_uuid = ScEndpointBuilder::new([0x02; 6], [0; 16]);
    assert!(zero_uuid.validate_only().is_err());
}

#[test]
fn sc_builder_validates_heartbeat_and_reconnect() {
    let bad_interval = ScEndpointBuilder::new([0x02; 6], [0x11; 16]).heartbeat(1_000, 60_000);
    assert!(bad_interval.validate_only().is_err());

    let inverted = ScEndpointBuilder::new([0x02; 6], [0x11; 16]).heartbeat(30_000, 30_000);
    assert!(inverted.validate_only().is_err());

    let bad_reconnect = ScEndpointBuilder::new([0x02; 6], [0x11; 16]).reconnect(
        bacnet_transport::sc::ScReconnectConfig {
            initial_delay_ms: 0,
            max_delay_ms: 1,
            max_retries: 3,
        },
    );
    assert!(bad_reconnect.validate_only().is_err());
}

#[test]
fn sc_loopback_builder_constructs_typed_session() {
    let (ws_client, _ws_hub) = bacnet_transport::sc::LoopbackWebSocket::pair();
    let session = ScEndpointBuilder::new([0x02; 6], [0x22; 16])
        .role(SessionRole::ClientOnly)
        .build_loopback_session(ws_client)
        .expect("typed SC loopback session must build");
    assert_eq!(session.active_leases(), 0);
}
