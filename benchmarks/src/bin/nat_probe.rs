//! A foreign device that checks a NAT topology (#940, #942): it registers
//! with a BBMD, broadcasts a Who-Is through it, and waits for an I-Am from
//! every device instance it is told to expect. It exits 0 when all answer
//! in time and 1 otherwise, printing who answered and from where.
//!
//! `examples/docker/nat` runs it against two BBMD sites behind NAT routers.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a check tool reports on the console
#![forbid(unsafe_code)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::process::ExitCode;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::{BipMode, ForeignDeviceConfig};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "nat-probe",
    about = "Who-Is through a BBMD, as a foreign device"
)]
struct Args {
    /// The BBMD to register with, ip:port.
    #[arg(long)]
    bbmd: SocketAddrV4,

    /// Comma-separated device instances that must answer.
    #[arg(long, value_delimiter = ',', required = true)]
    expect: Vec<u32>,

    /// Seconds to wait for every answer.
    #[arg(long, default_value_t = 15)]
    timeout: u64,

    /// Seconds between registrations: short behind a NAT router.
    #[arg(long, default_value_t = 20)]
    renewal: u64,
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: *args.bbmd.ip(),
            bbmd_port: args.bbmd.port(),
            ttl: 300,
            renewal_interval: Some(Duration::from_secs(args.renewal)),
        }))
        .build()
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout);
    let mut missing = args.expect.clone();
    while !missing.is_empty() && tokio::time::Instant::now() < deadline {
        client.who_is(None).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let found = client.discovered_devices().await;
        missing.retain(|instance| {
            !found
                .iter()
                .any(|device| device.object_identifier.instance_number() == *instance)
        });
    }
    for device in client.discovered_devices().await {
        println!(
            "I-Am from device {} at {}",
            device.object_identifier.instance_number(),
            mac_text(device.mac_address.as_slice())
        );
    }
    if missing.is_empty() {
        println!("nat-probe: every expected device answered");
        return Ok(ExitCode::SUCCESS);
    }
    // Name the hop that failed: registration first, then the I-Ams.
    let registration = client
        .transport()
        .bvlc_client_snapshot()
        .foreign_registration;
    if registration.registered {
        eprintln!(
            "nat-probe: hop 1 ok: registered with the BBMD at {}",
            args.bbmd
        );
        eprintln!(
            "nat-probe: no I-Am from {missing:?}: the Who-Is didn't reach that \
             device through the BBMDs (check the BDTs and the routers' port \
             forwards), or its I-Am didn't come back (a unicast I-Am from \
             another site is dropped by a router that forwards nothing; a \
             broadcast one comes back through the BBMDs)"
        );
    } else {
        eprintln!(
            "nat-probe: hop 1 failed: not registered with the BBMD at {} \
             after {} attempts ({} refused): the BBMD or its router's port \
             forward isn't reachable, or the answers go around the router",
            args.bbmd, registration.attempts, registration.refused
        );
        eprintln!("nat-probe: no answer from {missing:?}");
    }
    Ok(ExitCode::FAILURE)
}

/// A 6-octet B/IP MAC as `ip:port`, anything else as hex.
fn mac_text(mac: &[u8]) -> String {
    match mac {
        [a, b, c, d, high, low] => SocketAddrV4::new(
            Ipv4Addr::new(*a, *b, *c, *d),
            u16::from_be_bytes([*high, *low]),
        )
        .to_string(),
        other => format!("{other:02x?}"),
    }
}
