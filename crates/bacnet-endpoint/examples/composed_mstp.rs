//! Endpoint MS/TP example (one serial owner, simulator evidence).
//!
//! Old path: `BACnetClient::generic_builder().transport(MstpTransport...)`.
//! New path: `MstpEndpointBuilder::new(serial, station)` + `start()`.
//!
//! Evidence level: simulator only (`LoopbackSerial` ownership + lifecycle).
//! No bench or on-wire conformance; timing qualification is RB-26.
//!
//! Run with: `cargo run -p bacnet-endpoint --example composed_mstp`

use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_transport::mstp::{LoopbackSerial, MstpExecutionMode, SerialPort};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Addressing validates without consuming the serial owner.
    assert!(MstpEndpointBuilder::new(LoopbackSerial::pair().0, 3)
        .validate_only()
        .is_ok());
    assert!(MstpEndpointBuilder::new(LoopbackSerial::pair().0, 200)
        .validate_only()
        .is_err());

    // One serial owner enters the session; a drain task keeps the bounded
    // loopback channel flowing while the MAC loop organizes.
    let (session_end, peer_end) = LoopbackSerial::pair();
    let identity = DeviceIdentity::new(2001, 42)?.with_max_apdu(480)?;
    let db = identity.build_database()?;
    let mut endpoint = MstpEndpointBuilder::new(session_end, 3)
        .execution_mode(MstpExecutionMode::Tokio)
        .role(SessionRole::Both)
        .database(db)
        .identity(identity)
        .build_session()?;
    endpoint.start().await?;
    assert!(endpoint.is_running());

    // Drain peer-side MAC chatter so loopback writes never stall.
    let drain = tokio::spawn(async move {
        let mut buf = vec![0u8; 512];
        // Read until the session end drops (channel close ends this).
        loop {
            match peer_end.read(&mut buf).await {
                Ok(0) => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    // Identity above the 480 transport bound is rejected at build time.
    let oversize = DeviceIdentity::new(2002, 42)?.with_max_apdu(1476)?;
    let (serial2, _peer2) = LoopbackSerial::pair();
    assert!(MstpEndpointBuilder::new(serial2, 4)
        .identity(oversize)
        .build_session()
        .is_err());

    endpoint.stop().await?;
    assert!(endpoint.stop().await.is_err(), "stop-once");
    drain.abort();
    println!("done (simulator evidence only; timing is RB-26).");
    Ok(())
}
