//! Start and stop one shared B/IP owner with observable role and socket state.
#![forbid(unsafe_code)]

use bacnet_endpoint::{DeviceIdentity, SessionRole, bip::BipEndpointBuilder};
use std::io::{self, Write};
use std::net::{Ipv4Addr, UdpSocket};
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("usage: shared-endpoint-guide (press Enter or send EOF to stop)".into());
    }
    let identity = DeviceIdentity::new(1001, 42)?.with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let database = identity.build_database()?;
    // LOCALHOST is the announced interface and synthetic broadcast destination.
    // The default OS listener still binds wildcard; it is not loopback-confined.
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
        .role(SessionRole::Both)
        .database(database)
        .device(identity)
        .build_session()?;
    session.start().await?;

    let address = session.bip_local_address();

    // Keep reporting and input errors on the same awaited cleanup path.
    let observed = async {
        let address = address.ok_or("started endpoint did not publish its B/IP address")?;
        writeln!(io::stdout(), "Started endpoint: {address}")?;
        writeln!(
            io::stdout(),
            "Roles: requester={} responder={}",
            session.client().is_some(),
            session.server().is_some()
        )?;
        writeln!(io::stdout(), "Press Enter to stop (or send EOF).")?;
        io::stdout().flush()?;
        let mut input = String::new();
        tokio::io::BufReader::new(tokio::io::stdin())
            .read_line(&mut input)
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    session.stop().await?;
    writeln!(
        io::stdout(),
        "Stopped: address={:?} requester={} responder={}",
        session.bip_local_address(),
        session.client().is_some(),
        session.server().is_some()
    )?;
    if let Some(address) = address {
        let rebound = UdpSocket::bind(address)?;
        writeln!(
            io::stdout(),
            "Rebound endpoint: {}; owner running={}",
            rebound.local_addr()?,
            session.is_running()
        )?;
    }
    observed?;
    Ok(())
}
