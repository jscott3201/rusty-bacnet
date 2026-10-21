//! Max concurrent clients stress scenario.
//!
//! Scales the number of concurrent clients doing ReadProperty against a
//! single server to find the throughput ceiling and error threshold, over
//! B/IP, B/IPv6 or BACnet/SC (mutual TLS, through a hub in this process).
//! Every client connects before the step's clock starts, and each records
//! into its own histogram, so neither setup nor a shared lock is measured.

use std::net::Ipv6Addr;
use std::time::{Duration, Instant};

use bacnet_client::client::BACnetClient;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::sc_helpers::{generate_test_certs, make_sc_transport_mtls, start_sc_hub_mtls};
use crate::stress::harness::{
    make_bip_server_with_db, make_large_db, make_stress_client, stress_device_config,
};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

const DEVICE_INSTANCE: u32 = 1234;

/// The data link the clients and the server share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientTransport {
    /// B/IP on 127.0.0.1.
    Bip,
    /// B/IPv6 on ::1.
    Bip6,
    /// BACnet/SC with mutual TLS, through a hub in this process.
    Sc,
}

impl ClientTransport {
    /// `bip`, `bip6` or `sc`.
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
) -> Vec<DegradationPoint> {
    let duration = Duration::from_secs(duration_secs);
    let mut curve = Vec::new();
    for &count in steps {
        eprintln!("--- {count} clients ({transport:?}) ---");
        let db = make_large_db(DEVICE_INSTANCE, object_count);
        let result = match transport {
            ClientTransport::Bip => bip_step(db, count, duration).await,
            ClientTransport::Bip6 => bip6_step(db, count, duration).await,
            ClientTransport::Sc => sc_step(db, count, duration).await,
        };
        match result {
            Ok(point) => {
                point.log();
                curve.push(point);
            }
            Err(e) => eprintln!("  step skipped: {e}"),
        }
    }
    curve
}

async fn bip_step(
    db: ObjectDatabase,
    count: u64,
    duration: Duration,
) -> Result<DegradationPoint, Error> {
    let mut server = make_bip_server_with_db(db).await?;
    let mac = server.local_mac().to_vec();
    let mut clients = Vec::new();
    for _ in 0..count {
        clients.push(make_stress_client().await?);
    }
    let point = drive(clients, mac, count, duration).await;
    let _ = server.stop().await;
    Ok(point)
}

async fn bip6_step(
    db: ObjectDatabase,
    count: u64,
    duration: Duration,
) -> Result<DegradationPoint, Error> {
    let mut server = BACnetServer::<Bip6Transport>::generic_builder()
        .device(stress_device_config(DEVICE_INSTANCE).identity)
        .transport(Bip6Transport::new(
            Ipv6Addr::LOCALHOST,
            0,
            Some(DEVICE_INSTANCE),
        ))
        .database(db)
        .build()
        .await?;
    let address = server
        .transport()
        .and_then(Bip6Transport::local_address)
        .ok_or_else(|| Error::Encoding("the B/IPv6 server is not running".into()))?;
    let mut clients = Vec::new();
    let mut mac = Vec::new();
    for _ in 0..count {
        let client = BACnetClient::<Bip6Transport>::generic_builder()
            .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None))
            .apdu_timeout_ms(5000)
            .build()
            .await?;
        // Each client learns the server's VMAC before the clock starts, so
        // no request waits on address resolution.
        let vmac = client.transport().resolve_virtual_address(address).await?;
        mac = vmac.octets().to_vec();
        clients.push(client);
    }
    let point = drive(clients, mac, count, duration).await;
    let _ = server.stop().await;
    Ok(point)
}

/// A locally administered VMAC (and a matching Device UUID) for node `n`
/// of `role`: 1 for the server, 2 for the clients.
fn sc_identity(role: u8, n: u64) -> ([u8; 6], [u8; 16]) {
    let [.., hi, lo] = n.to_be_bytes();
    let vmac = [0x02, role, 0, 0, hi, lo];
    let mut uuid = [0x5c; 16];
    uuid[13] = role;
    uuid[14] = hi;
    uuid[15] = lo;
    (vmac, uuid)
}

async fn sc_step(
    db: ObjectDatabase,
    count: u64,
    duration: Duration,
) -> Result<DegradationPoint, Error> {
    let certs = generate_test_certs();
    let (mut hub, url) = start_sc_hub_mtls(&certs, [0x02, 0, 0, 0, 0, 1]).await;
    let (vmac, uuid) = sc_identity(1, 0);
    let server_transport = make_sc_transport_mtls(&url, &certs, vmac)
        .await
        .with_device_uuid(uuid);
    let mut server = BACnetServer::<ScTransport<TlsWebSocket>>::generic_builder()
        .device(stress_device_config(DEVICE_INSTANCE).identity)
        .transport(server_transport)
        .database(db)
        .build()
        .await?;
    let mac = server.local_mac().to_vec();
    let mut clients = Vec::new();
    for n in 0..count {
        let (vmac, uuid) = sc_identity(2, n);
        let transport = make_sc_transport_mtls(&url, &certs, vmac)
            .await
            .with_device_uuid(uuid);
        clients.push(
            BACnetClient::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(transport)
                .apdu_timeout_ms(5000)
                .build()
                .await?,
        );
    }
    // Let the hub finish admitting the last connections.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let point = drive(clients, mac, count, duration).await;
    let _ = server.stop().await;
    hub.stop().await;
    Ok(point)
}

/// Run every client's ReadProperty loop for `duration`, from one start, and
/// time the step from that start to the last loop's end.
async fn drive<T: TransportPort + 'static>(
    clients: Vec<BACnetClient<T>>,
    server_mac: Vec<u8>,
    parameter: u64,
    duration: Duration,
) -> DegradationPoint {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let started = Instant::now();
    let deadline = started + duration;
    let mut handles = Vec::new();
    for mut client in clients {
        let mac = server_mac.clone();
        handles.push(tokio::spawn(async move {
            let mut recorder = LatencyRecorder::new();
            while Instant::now() < deadline {
                let start = Instant::now();
                match client
                    .read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                    .await
                {
                    Ok(_) => recorder.record_success(start),
                    Err(_) => recorder.record_failure(),
                }
            }
            let ended = Instant::now();
            let _ = client.stop().await;
            (recorder, ended)
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        if let Ok(result) = h.await {
            results.push(result);
        }
    }
    DegradationPoint::from_clients(parameter, started, results)
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
