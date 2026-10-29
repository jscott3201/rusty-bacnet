//! Endpoint MS/TP example (one serial owner, simulator evidence).
//!
//! Old path: `BACnetClient::generic_builder().transport(MstpTransport...)`.
//! New path: `MstpEndpointBuilder::new(serial, station)` + `start()`.
//!
//! Evidence level: simulator only (`LoopbackSerial` ownership + lifecycle).
//! No bench or on-wire conformance; timing qualification is RB-26.
//!
//! Run with: `cargo run -p bacnet-endpoint --example composed_mstp`
#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::mstp::{LoopbackSerial, MstpExecutionMode, SerialPort};

fn drain_peer(peer: LoopbackSerial) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 512];
        while let Ok(n) = peer.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Addressing validates without consuming the serial owner.
    assert!(
        MstpEndpointBuilder::new(LoopbackSerial::pair().0, 3)
            .validate_only()
            .is_ok()
    );
    assert!(
        MstpEndpointBuilder::new(LoopbackSerial::pair().0, 200)
            .validate_only()
            .is_err()
    );

    // One serial owner enters the session; a drain task keeps the bounded
    // loopback channel flowing while the MAC loop organizes.
    let (session_end, peer_end) = LoopbackSerial::pair();
    let drain = drain_peer(peer_end);
    let identity = DeviceIdentity::new(2001, 42)?.with_max_apdu(480)?;
    let db = identity.build_database()?;
    let mut endpoint = MstpEndpointBuilder::new(session_end, 3)
        .execution_mode(MstpExecutionMode::Tokio)
        .role(SessionRole::Both)
        .database(db)
        .device(identity)
        .build_session()?;
    endpoint.start().await?;
    assert!(endpoint.is_running());

    endpoint.stop().await?;
    assert!(endpoint.stop().await.is_err(), "stop-once");
    drop(endpoint);
    tokio::time::timeout(std::time::Duration::from_secs(2), drain).await??;

    // An omitted database generates a Device after the requested ceiling
    // resolves to the standard-frame transport's stable receive capacity.
    let requested = DeviceIdentity::new(2002, 42)?.with_max_apdu(1476)?;
    let (serial2, peer2) = LoopbackSerial::pair();
    let drain2 = drain_peer(peer2);
    let mut generated = MstpEndpointBuilder::new(serial2, 4)
        .execution_mode(MstpExecutionMode::Tokio)
        .device(requested)
        .build_session()?;
    generated.start().await?;
    assert!(generated.is_running());
    assert_eq!(generated.identity().unwrap().max_apdu_length(), 480);
    generated.stop().await?;
    drop(generated);
    tokio::time::timeout(std::time::Duration::from_secs(2), drain2).await??;
    println!("done (simulator evidence only; timing is RB-26).");
    Ok(())
}
