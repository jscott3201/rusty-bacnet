//! Concurrent ReadProperty clients, with one clock and complete accounting.
use super::client_measure::{StepReport, drive, stop_clients};
use super::client_run::{ClientsRun, run_steps};
use super::harness::{make_large_db, stress_device_config};
use crate::sc_helpers::{
    CertMaterial, generate_test_certs, try_make_hub_tls_config, try_make_node_tls_config,
};
use bacnet_client::client::BACnetClient;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts};
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::error::Error;
use std::net::Ipv6Addr;
use std::time::Duration;
const DEVICE_INSTANCE: u32 = 1234;

#[path = "client_bip6_setup.rs"]
mod bip6_setup;

/// The data link shared by clients and server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientTransport {
    /// B/IP on loopback.
    Bip,
    /// B/IPv6 on loopback.
    Bip6,
    /// Mutually authenticated BACnet/SC through an in-process hub.
    Sc,
}
impl ClientTransport {
    /// Parse the command-line transport name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "bip" => Some(Self::Bip),
            "bip6" => Some(Self::Bip6),
            "sc" => Some(Self::Sc),
            _ => None,
        }
    }
}

pub async fn run(
    transport: ClientTransport,
    object_count: u32,
    duration_secs: u64,
    steps: &[u64],
) -> ClientsRun {
    run_steps(
        object_count,
        duration_secs,
        steps,
        |index, count, duration| async move {
            match transport {
                ClientTransport::Bip => {
                    super::client_bip::step(index, object_count, count, duration).await
                }
                ClientTransport::Bip6 => {
                    bip6_step(
                        make_large_db(DEVICE_INSTANCE, object_count),
                        index,
                        count,
                        duration,
                    )
                    .await
                }
                ClientTransport::Sc => {
                    sc_step(
                        make_large_db(DEVICE_INSTANCE, object_count),
                        index,
                        count,
                        duration,
                    )
                    .await
                }
            }
        },
    )
    .await
}

async fn bip6_step(db: ObjectDatabase, index: usize, count: u64, duration: Duration) -> StepReport {
    let mut report = StepReport::new(index, count);
    let mut server = match BACnetServer::<Bip6Transport>::generic_builder()
        .device(stress_device_config(DEVICE_INSTANCE).identity)
        .transport(Bip6Transport::new(
            Ipv6Addr::LOCALHOST,
            0,
            Some(DEVICE_INSTANCE),
        ))
        .database(db)
        .build()
        .await
    {
        Ok(server) => server,
        Err(error) => {
            report.diagnostics.fail("server_setup", None, error);
            report.finish();
            return report;
        }
    };
    let mut clients = Vec::new();
    let address = server
        .transport()
        .and_then(Bip6Transport::local_address)
        .ok_or_else(|| Error::Encoding("the B/IPv6 server is not running".into()));
    let setup = bip6_setup::setup(
        address,
        count,
        &mut clients,
        async || {
            BACnetClient::<Bip6Transport>::generic_builder()
                .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None))
                .apdu_timeout_ms(5000)
                .build()
                .await
        },
        async |client: &BACnetClient<Bip6Transport>, address| {
            client
                .transport()
                .resolve_virtual_address(address)
                .await
                .map(|vmac| vmac.octets().to_vec())
        },
        &mut report,
    )
    .await;
    match setup {
        Some(mac) => drive(clients, mac, duration, |_| None, &mut report).await,
        None => stop_clients(&mut clients, &mut report).await,
    }
    if let Err(error) = server.stop().await {
        report.diagnostics.fail("server_stop", None, error);
    }
    report.finish();
    report
}

fn sc_identity(role: u8, n: u64) -> ([u8; 6], [u8; 16]) {
    let [.., hi, lo] = n.to_be_bytes();
    let vmac = [0x02, role, 0, 0, hi, lo];
    let mut uuid = [0x5c; 16];
    uuid[13] = role;
    uuid[14] = hi;
    uuid[15] = lo;
    (vmac, uuid)
}

async fn sc_step(db: ObjectDatabase, index: usize, count: u64, duration: Duration) -> StepReport {
    let mut report = StepReport::new(index, count);
    if count > 65536 {
        report.diagnostics.fail(
            "arguments",
            None,
            "SC client identities support at most 65536 clients",
        );
        report.finish();
        return report;
    }
    let certs = generate_test_certs();
    let setup_hub = async {
        ScHub::start_with_tls_config(
            "127.0.0.1:0",
            try_make_hub_tls_config(&certs)?,
            [0x02, 0, 0, 0, 0, 1],
            [0x48; 16],
            ScHubHandshakeTimeouts::default(),
        )
        .await
    }
    .await;
    let mut hub = match setup_hub {
        Ok(hub) => hub,
        Err(error) => {
            report.diagnostics.fail("hub_setup", None, error);
            report.finish();
            return report;
        }
    };
    let setup_server = async {
        let address = hub
            .local_addr()
            .ok_or_else(|| Error::Encoding("SC hub address unavailable".into()))?;
        let url = format!("wss://localhost:{}", address.port());
        let (vmac, uuid) = sc_identity(1, 0);
        let transport = sc_transport(&url, &certs, vmac, uuid).await?;
        let server = BACnetServer::<ScTransport<TlsWebSocket>>::generic_builder()
            .device(stress_device_config(DEVICE_INSTANCE).identity)
            .transport(transport)
            .database(db)
            .build()
            .await?;
        Ok::<_, Error>((server, url))
    }
    .await;
    let (mut server, url) = match setup_server {
        Ok(result) => result,
        Err(error) => {
            report.diagnostics.fail("server_setup", None, error);
            hub.stop().await;
            report.finish();
            return report;
        }
    };
    let mac = server.local_mac().to_vec();
    let mut clients = Vec::new();
    for n in 0..count {
        let (vmac, uuid) = sc_identity(2, n);
        let setup_client = async {
            let transport = sc_transport(&url, &certs, vmac, uuid).await?;
            BACnetClient::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(transport)
                .apdu_timeout_ms(5000)
                .build()
                .await
        }
        .await;
        match setup_client {
            Ok(client) => clients.push(client),
            Err(error) => {
                report
                    .diagnostics
                    .fail("client_setup", Some(n as usize), error);
                break;
            }
        }
    }

    if report.diagnostics.lifecycle_failures.is_empty() {
        // Existing setup settling interval, outside the measured clock.
        tokio::time::sleep(Duration::from_millis(100)).await;
        drive(clients, mac, duration, |_| None, &mut report).await;
    } else {
        report.diagnostics.started_clients = clients.len();
        stop_clients(&mut clients, &mut report).await;
    }
    if let Err(error) = server.stop().await {
        report.diagnostics.fail("server_stop", None, error);
    }
    hub.stop().await;
    report.finish();
    report
}

async fn sc_transport(
    url: &str,
    certs: &CertMaterial,
    vmac: [u8; 6],
    uuid: [u8; 16],
) -> Result<ScTransport<TlsWebSocket>, Error> {
    let ws = TlsWebSocket::connect(url, try_make_node_tls_config(certs)?).await?;
    Ok(ScTransport::new(ws, vmac).with_device_uuid(uuid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_parse_by_name() {
        assert_eq!(ClientTransport::parse("bip"), Some(ClientTransport::Bip));
        assert_eq!(ClientTransport::parse("bip6"), Some(ClientTransport::Bip6));
        assert_eq!(ClientTransport::parse("sc"), Some(ClientTransport::Sc));
        assert_eq!(ClientTransport::parse("mstp"), None);
    }

    #[test]
    fn sc_identities_differ_per_node_and_role() {
        let (server, server_uuid) = sc_identity(1, 0);
        let (first, first_uuid) = sc_identity(2, 0);
        let (last, last_uuid) = sc_identity(2, 299);
        assert_ne!(server, first);
        assert_ne!(first, last);
        assert_ne!(server_uuid, first_uuid);
        assert_ne!(first_uuid, last_uuid);
        assert_eq!(last, [0x02, 2, 0, 0, 1, 43]);
    }
}
