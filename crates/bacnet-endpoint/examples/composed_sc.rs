//! Endpoint SC example (validation + loopback composition, no hub I/O).
//!
//! Old path: `BACnetClient::sc_builder()...build().await` (dials a hub).
//! New path: `ScEndpointBuilder::new(vmac, uuid)` + `build_hub_session(ws)`
//! after the caller dials (`sc-tls`, proven against the local
//! constrained-TLS hub in `rb16_sc_proof`). This example exercises the
//! loopback composition locally: validation + unstarted build, which is also
//! what unit proofs use. Real hub dial needs `sc-tls` + a running `ScHub`.
//!
//! Run with: `cargo run -p bacnet-endpoint --example composed_sc`

use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::SessionRole;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Validation without hub I/O: good config passes, reserved VMAC and
    // zero UUID fail fast with typed errors.
    assert!(ScEndpointBuilder::new([0x02; 6], [0x11; 16])
        .validate_only()
        .is_ok());
    assert!(ScEndpointBuilder::new([0; 6], [0x11; 16])
        .validate_only()
        .is_err());
    assert!(ScEndpointBuilder::new([0x02; 6], [0; 16])
        .validate_only()
        .is_err());
    assert!(ScEndpointBuilder::new([0x02; 6], [0x11; 16])
        .heartbeat(1_000, 60_000)
        .validate_only()
        .is_err());
    println!("SC validation OK.");

    // Loopback composition: no TLS, no hub dial. The caller drives the hub
    // side for handshake in unit proofs; here we prove the typed session
    // builds with no leases and no policy outcomes yet.
    let (ws_client, _ws_hub) = bacnet_transport::sc::LoopbackWebSocket::pair();
    let session = ScEndpointBuilder::new([0x02; 6], [0x22; 16])
        .role(SessionRole::ClientOnly)
        .build_loopback_session(ws_client)?;
    assert_eq!(session.active_leases(), 0);
    assert!(
        session.client().is_none(),
        "unstarted session has no roles yet"
    );
    println!("SC loopback session builds OK (unstarted; hub dial is sc-tls).");

    // Identity agreement is checked at hub-dial build time (sc-tls): a
    // mismatched UUID/VMAC fails before any I/O. Documented here without a
    // live hub; see `rb16_sc_proof` for the exercised path.
    println!("done.");
    Ok(())
}
