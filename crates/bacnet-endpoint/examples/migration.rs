//! Migration adapter example (docs-level, no facade code).
//!
//! Maps old standalone `BACnetClient` / `BACnetServer` builder calls to the
//! three endpoint builders. This is documentation as runnable code: each
//! adapter builds an UNSTARTED endpoint session (or validates config) with
//! the same identity/database the standalone path would use. There is no
//! facade, no parts API, and no second hidden owner — the standalone types
//! stay as untouched compat surfaces.
//!
//! Run with: `cargo run -p bacnet-endpoint --example migration`

use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::mstp::{LoopbackSerial, MstpTransport};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport};

/// Old: `BACnetClient::bip_builder().interface(i).port(p).broadcast_address(b).build().await`
/// New: client-only endpoint session over the same addressing.
fn migrate_bip_client(
    interface: Ipv4Addr,
    port: u16,
    broadcast: Ipv4Addr,
) -> Result<EndpointSession<BipTransport>, bacnet_types::error::Error> {
    BipEndpointBuilder::new(interface, port, broadcast)
        .role(SessionRole::ClientOnly)
        .build_session()
}

/// Old: `BACnetServer::bip_builder().database(db)...build().await`
/// New: server-only endpoint session with the identity-built database.
fn migrate_bip_server(
    interface: Ipv4Addr,
    port: u16,
    broadcast: Ipv4Addr,
    identity: DeviceIdentity,
) -> Result<EndpointSession<BipTransport>, bacnet_types::error::Error> {
    let db = build_database_with_extra(&identity, Vec::new())?;
    BipEndpointBuilder::new(interface, port, broadcast)
        .role(SessionRole::ServerOnly)
        .database(db)
        .identity(identity)
        .build_session()
}

/// Old: `BACnetClient::sc_builder()...build().await` (after hub dial).
/// New: `ScEndpointBuilder` over the caller-dialed socket; loopback shown
/// here so the adapter runs without a hub.
fn migrate_sc_loopback(
    vmac: [u8; 6],
    uuid: [u8; 16],
    ws: LoopbackWebSocket,
) -> Result<EndpointSession<ScTransport<LoopbackWebSocket>>, bacnet_types::error::Error> {
    ScEndpointBuilder::new(vmac, uuid)
        .role(SessionRole::Both)
        .build_loopback_session(ws)
}

/// Old: `generic_builder().transport(MstpTransport::new(serial, ...))`.
/// New: `MstpEndpointBuilder::new(serial, station)` (serial taken once).
fn migrate_mstp<S: bacnet_transport::mstp::SerialPort>(
    serial: S,
    station: u8,
) -> Result<EndpointSession<MstpTransport<S>>, bacnet_types::error::Error> {
    MstpEndpointBuilder::new(serial, station)
        .role(SessionRole::Both)
        .build_session()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Standalone Loopback composition keeps working untouched; the endpoint
    // equivalent owns the same transport shape above both roles.
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let standalone_shape =
        EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())?;
    assert_eq!(standalone_shape.active_leases(), 0);
    println!("loopback shape OK.");

    let client = migrate_bip_client(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)?;
    assert_eq!(client.active_leases(), 0);
    println!("migrate: BACnetClient::bip_builder -> BipEndpointBuilder(ClientOnly) OK.");

    let identity = DeviceIdentity::new(1001, 42)?.with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let server = migrate_bip_server(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST, identity)?;
    assert_eq!(server.active_leases(), 0);
    println!("migrate: BACnetServer::bip_builder -> BipEndpointBuilder(ServerOnly) OK.");

    let (ws_client, _ws_hub) = LoopbackWebSocket::pair();
    let sc = migrate_sc_loopback([0x02; 6], [0x33; 16], ws_client)?;
    assert_eq!(sc.active_leases(), 0);
    println!("migrate: sc_builder -> ScEndpointBuilder(loopback) OK.");

    let (serial, _peer_serial) = LoopbackSerial::pair();
    let mstp = migrate_mstp(serial, 5)?;
    assert_eq!(mstp.active_leases(), 0);
    println!("migrate: generic_builder(mstp) -> MstpEndpointBuilder OK.");

    println!("done (adapters build unstarted sessions; caller drives start/stop).");
    Ok(())
}
