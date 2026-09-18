//! RB-15 concrete transport controls (no real-transport proofs).
//!
//! B/IP BBMD/FDT controls + SC builder validation live at endpoint level as
//! concrete types (not generic-trait erasure). Real one-socket B/IP + SC
//! proofs are RB-16/17 and stay out of scope.

use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::ForeignDeviceConfig;

fn bdt_entry(ip: [u8; 4], port: u16) -> BdtEntry {
    BdtEntry {
        ip,
        port,
        broadcast_mask: [255, 255, 255, 255],
    }
}

#[test]
fn bip_builder_applies_bbmd_controls_concretely() {
    let builder = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .queue_capacity(8)
        .enable_bbmd(vec![bdt_entry([127, 0, 0, 1], 47808)])
        .foreign_device_policy(ForeignDevicePolicy::default())
        .bbmd_management_acl(vec![[127, 0, 0, 1]])
        .fanout_policy(bacnet_transport::bip::FanoutPolicy::default())
        .register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: 47808,
            ttl: 60,
        });
    // Concrete transport builds without generic erasure; BBMD config is
    // accepted pre-start (fail-closed ACL + policy staged for `start()`).
    // `bbmd_state()` appears only after `start()` binds the socket, so a
    // Loopback-first unit test asserts construction, not live state.
    let _transport = builder
        .build_transport()
        .expect("B/IP transport must build");
}

#[test]
fn bip_builder_rejects_bbmd_controls_without_enable() {
    let builder = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .foreign_device_policy(ForeignDevicePolicy::default());
    assert!(builder.build_transport().is_err());
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
