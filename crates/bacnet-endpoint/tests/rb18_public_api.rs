//! RB-18 public API tests (Loopback + real B/IP loopback, deterministic).
//!
//! Covers the promotion contract: public paths resolve, role handles are
//! `Send + Sync` (clones survive session drop as fail-closed values), owner
//! shutdown with live handles, use-after-stop, role teardown while others
//! remain, migration-adapter builds, and BBMD experimental ordering.
//! Wire proofs stay in `rb16_*` / `rb17_*`; this file asserts the API shape.

use std::net::Ipv4Addr;
use std::time::Duration;

// Promotion list resolves at both module and top-level paths.
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity, NetworkPortEntry};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::roles::{ClientRoleHandle, ServerRoleHandle};
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::{
    EndpointSession, PolicyCountersSnapshot, SessionConfig, SessionExit, SessionRole,
};
use bacnet_endpoint::{ClientRoleHandle as TopClient, DeviceIdentity as TopIdentity};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::mstp::LoopbackSerial;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const WAIT: Duration = Duration::from_secs(5);

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn promoted_paths_and_thread_safety_resolve() {
    // Top-level re-exports name the same types (pass-by-value proves it).
    fn takes_top(_: TopClient) {}
    fn takes_top_identity(_: TopIdentity) {}
    fn takes_mod(handle: ClientRoleHandle, identity: DeviceIdentity) {
        takes_top(handle);
        takes_top_identity(identity);
    }
    let _ = takes_mod;
    // Role handles are Send + Sync, including clones that outlive the borrow.
    assert_send_sync::<ClientRoleHandle>();
    assert_send_sync::<ServerRoleHandle>();
    // Session-level types resolve.
    let _ = SessionRole::Both;
    let _ = SessionConfig::default();
    let _ = PolicyCountersSnapshot::default();
    let _ = SessionExit::ReceiversClosed;
    let _ = NetworkPortEntry::bip(1, 0, Ipv4Addr::LOCALHOST, 47808);
    let _ = ScEndpointBuilder::new([0x02; 6], [0x11; 16]);
    let _ = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _ = MstpEndpointBuilder::new(LoopbackSerial::pair().0, 3);
}

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 8,
        apdu_timeout_ms: 200,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn object_id(instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[tokio::test]
async fn owner_shutdown_with_live_handles_fails_closed() {
    let (ta, tb) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut db = bacnet_objects::database::ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "rb18", 0).unwrap();
    ai.set_present_value(1.0);
    db.add(Box::new(ai)).unwrap();
    let mut a = EndpointSession::new(ta, SessionRole::Both, session_config())
        .unwrap()
        .with_database(db);
    let mut b = EndpointSession::new(tb, SessionRole::Both, session_config()).unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();

    // Live clones prove binding while running.
    let live_client = a.cloned_client_handle().unwrap();
    let live_server = a.cloned_server_handle().unwrap();
    assert!(live_server.is_session_alive());

    // Owner shutdown with handles alive: stop joins, clones fail closed.
    a.stop().await.unwrap();
    assert!(!live_server.is_session_alive());
    assert!(live_client
        .read_property(
            &[0x02],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_err());
    assert!(live_server.suspend_next_reply().is_err());
    // Session borrows are released at termination.
    assert!(a.client().is_none());
    assert!(a.server().is_none());

    b.stop().await.unwrap();
}

#[tokio::test]
async fn use_after_stop_fails_closed() {
    let (t, _p) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut s = EndpointSession::new(t, SessionRole::Both, session_config()).unwrap();
    s.start().await.unwrap();
    let client = s.cloned_client_handle().unwrap();
    s.stop().await.unwrap();
    // Second stop is stop-once error, not a hang.
    assert!(s.stop().await.is_err());
    // Broadcast without running errors (no invented device, no send).
    assert!(s.broadcast_i_am().await.is_err());
    // Cloned client fails closed after stop.
    assert!(tokio::time::timeout(
        WAIT,
        client.read_property(
            &[0x02],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
    )
    .await
    .expect("post-stop call hung")
    .is_err());
}

#[tokio::test]
async fn role_teardown_while_other_session_remains() {
    // Client-only + server-only pair: stopping the server leaves the client
    // session running (is_running) while its reads fail; the client owner
    // still drives its own stop.
    let (tc, ts) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut db = bacnet_objects::database::ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(9, "rb18-teardown", 0).unwrap();
    ai.set_present_value(9.0);
    db.add(Box::new(ai)).unwrap();
    let mut client = EndpointSession::new(tc, SessionRole::ClientOnly, session_config()).unwrap();
    let mut server = EndpointSession::new(ts, SessionRole::ServerOnly, session_config())
        .unwrap()
        .with_database(db);
    client.start().await.unwrap();
    server.start().await.unwrap();

    let ack = tokio::time::timeout(
        WAIT,
        client.client().unwrap().read_property(
            &[0x02],
            object_id(9),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("pre-teardown read hung")
    .expect("pre-teardown read failed");
    assert_eq!(ack.object_identifier, object_id(9));

    // Tear down the server role owner; the client session remains running.
    server.stop().await.unwrap();
    assert!(client.is_running());
    assert!(tokio::time::timeout(
        WAIT,
        client.client().unwrap().read_property(
            &[0x02],
            object_id(9),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
    )
    .await
    .expect("post-teardown read hung")
    .is_err());
    client.stop().await.unwrap();
}

#[tokio::test]
async fn detached_handles_survive_drop_as_fail_closed_values() {
    let (ta, tb) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut a = EndpointSession::new(ta, SessionRole::Both, session_config()).unwrap();
    let mut b = EndpointSession::new(tb, SessionRole::Both, session_config()).unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let detached_client = a.cloned_client_handle().unwrap();
    let detached_server = a.cloned_server_handle().unwrap();
    // `Send + Sync` clones move across tasks while bound.
    let moved = tokio::spawn(async move { detached_server.is_session_alive() })
        .await
        .unwrap();
    assert!(moved);
    drop(a);
    assert!(!detached_client
        .read_property(
            &[0x02],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_ok());
    b.stop().await.unwrap();
}

#[test]
fn migration_adapters_build_unstarted() {
    // Doc-level adapters (no facade code): each old standalone shape maps to
    // an unstarted endpoint build with zero leases.
    let bip = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ClientOnly)
        .build_session()
        .unwrap();
    assert_eq!(bip.active_leases(), 0);

    let id = DeviceIdentity::new(1001, 42).unwrap();
    let db = build_database_with_extra(&id, Vec::new()).unwrap();
    let server = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(db)
        .identity(id)
        .build_session()
        .unwrap();
    assert_eq!(server.active_leases(), 0);

    let (ws, _hub) = bacnet_transport::sc::LoopbackWebSocket::pair();
    let sc = ScEndpointBuilder::new([0x02; 6], [0x44; 16])
        .build_loopback_session(ws)
        .unwrap();
    assert_eq!(sc.active_leases(), 0);

    let (serial, _peer) = LoopbackSerial::pair();
    let mstp = MstpEndpointBuilder::new(serial, 5).build_session().unwrap();
    assert_eq!(mstp.active_leases(), 0);
}

#[test]
fn bbmd_mode_without_enable_stays_fail_closed_experimental() {
    // BBMD-mode is experimental/construction-only: ordering violations fail
    // fast with a typed error instead of silently ignoring the controls.
    let builder = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .foreign_device_policy(bacnet_transport::bbmd::ForeignDevicePolicy::default());
    assert!(builder.build_transport().is_err());
}
