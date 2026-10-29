//! Router forwarding overhead stress scenario.
//!
//! Clients on one B/IP network read a Present_Value from a server, scaling
//! the client count, by one of three paths:
//!
//! - `direct`: client and server on the same B/IP network, the baseline;
//! - `bip`: through a [`BACnetRouter`] from B/IP network 1 to a server on
//!   B/IP network 2, all on loopback;
//! - `mstp`: through a [`BACnetRouter`] from B/IP network 1 to a server on
//!   MS/TP network 2, a [`SimWire`] line at the given baud rate.
//!
//! Each routed request carries DNET 2 and the server's MAC as DADR, so it
//! takes the router's real forwarding path both ways.

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::MstpTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{
    make_bip_server_with_db, make_large_db, make_stress_client, stress_device_config,
};
use crate::stress::mstp::{SimWire, station};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

/// The network the clients are on.
const CLIENT_NET: u16 = 1;
/// The network the server is on, behind the router.
const SERVER_NET: u16 = 2;

/// How the clients reach the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Same network, no router.
    Direct,
    /// Through a router, B/IP to B/IP.
    Bip,
    /// Through a router, B/IP to MS/TP.
    Mstp,
}

impl Path {
    /// Parse `direct`, `bip` or `mstp`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "direct" => Some(Self::Direct),
            "bip" => Some(Self::Bip),
            "mstp" => Some(Self::Mstp),
            _ => None,
        }
    }
}

/// Where a client sends its request: the server itself, or the router with
/// the server's network and MAC.
#[derive(Clone)]
enum Target {
    Direct(Vec<u8>),
    Routed { router: Vec<u8>, mac: Vec<u8> },
}

/// What has to be stopped when the scenario ends.
enum Running {
    Direct(BACnetServer<BipTransport>),
    Bip(BACnetRouter, BACnetServer<BipTransport>),
    Mstp(BACnetRouter, BACnetServer<MstpTransport<SimWire>>),
}

impl Running {
    async fn stop(self) {
        match self {
            Self::Direct(mut server) => {
                let _ = server.stop().await;
            }
            Self::Bip(mut router, mut server) => {
                router.stop().await.unwrap();
                let _ = server.stop().await;
            }
            Self::Mstp(mut router, mut server) => {
                router.stop().await.unwrap();
                let _ = server.stop().await;
            }
        }
    }
}

/// A free loopback UDP port, released for the router's transport to bind.
fn free_port() -> u16 {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|socket| socket.local_addr())
        .map(|address| address.port())
        .expect("a free loopback UDP port")
}

/// The router's B/IP MAC on the clients' network: loopback and `port`.
fn bip_mac(port: u16) -> Vec<u8> {
    let mut mac = Ipv4Addr::LOCALHOST.octets().to_vec();
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}

/// Start a router, keeping its local-APDU channel drained.
async fn start_router<T>(ports: Vec<RouterPort<T>>) -> Option<BACnetRouter>
where
    T: bacnet_transport::port::TransportPort + 'static,
{
    let StartedRouter {
        router, mut apdus, ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .ok()?;
    tokio::spawn(async move { while apdus.recv().await.is_some() {} });
    Some(router)
}

/// Start the server, and the router in front of it for a routed path.
/// Starting a router is tried again on fresh ports, as another process can
/// take a picked port before the router binds it.
async fn start(path: Path, baud: u32) -> (Running, Target) {
    let db = || make_large_db(1234, 100);
    match path {
        Path::Direct => {
            let server = make_bip_server_with_db(db()).await.unwrap();
            let mac = server.local_mac().to_vec();
            (Running::Direct(server), Target::Direct(mac))
        }
        Path::Bip => {
            let server = make_bip_server_with_db(db()).await.unwrap();
            let mac = server.local_mac().to_vec();
            for _ in 0..8 {
                let (port_1, port_2) = (free_port(), free_port());
                let ports = [(port_1, CLIENT_NET), (port_2, SERVER_NET)]
                    .into_iter()
                    .map(|(port, network_number)| RouterPort {
                        transport: BipTransport::new(
                            Ipv4Addr::LOCALHOST,
                            port,
                            Ipv4Addr::LOCALHOST,
                        ),
                        network_number,
                    })
                    .collect();
                if let Some(router) = start_router(ports).await {
                    let target = Target::Routed {
                        router: bip_mac(port_1),
                        mac,
                    };
                    return (Running::Bip(router, server), target);
                }
            }
            panic!("no loopback UDP ports the router could bind");
        }
        Path::Mstp => {
            for _ in 0..8 {
                let (router_end, server_end, _) = SimWire::pair(baud);
                let port_1 = free_port();
                let bip = BipTransport::new(Ipv4Addr::LOCALHOST, port_1, Ipv4Addr::LOCALHOST);
                let ports = vec![
                    RouterPort {
                        transport: AnyTransport::Bip(Box::new(bip)),
                        network_number: CLIENT_NET,
                    },
                    RouterPort {
                        transport: MstpTransport::new(router_end, station(0, baud, true)).into(),
                        network_number: SERVER_NET,
                    },
                ];
                let Some(router) = start_router(ports).await else {
                    continue;
                };
                let server = BACnetServer::<MstpTransport<SimWire>>::generic_builder()
                    .device(stress_device_config(1234).identity)
                    .transport(MstpTransport::new(server_end, station(1, baud, true)))
                    .database(db())
                    .build()
                    .await
                    .unwrap();
                let target = Target::Routed {
                    router: bip_mac(port_1),
                    mac: vec![1],
                };
                return (Running::Mstp(router, server), target);
            }
            panic!("no loopback UDP port the router could bind");
        }
    }
}

/// Read once through `target` until a read succeeds, for up to 20 seconds:
/// an MS/TP line forms its token ring first.
async fn warm_up(target: &Target, oid: ObjectIdentifier) {
    let mut client = make_stress_client().await.unwrap();
    let give_up = Instant::now() + Duration::from_secs(20);
    while Instant::now() < give_up && read(&client, target, oid).await.is_err() {}
    let _ = client.stop().await;
}

/// One ReadProperty of `oid`'s Present_Value through `target`.
async fn read(
    client: &BACnetClient<BipTransport>,
    target: &Target,
    oid: ObjectIdentifier,
) -> Result<ReadPropertyACK, Error> {
    let property = PropertyIdentifier::PRESENT_VALUE;
    match target {
        Target::Direct(mac) => client.read_property(mac, oid, property, None).await,
        Target::Routed { router, mac } => {
            client
                .read_property_routed(router, SERVER_NET, mac, oid, property, None)
                .await
        }
    }
}

pub async fn run(
    path: Path,
    baud: u32,
    duration_secs: u64,
    steps: &[u64],
) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    let (running, target) = start(path, baud).await;
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    warm_up(&target, oid).await;

    for &count in steps {
        eprintln!("--- {count} clients ({path:?} path) ---");

        // Every client connects before the clock starts, and each records
        // into its own histogram, so neither setup nor a shared lock is
        // measured.
        let mut pool = Vec::new();
        for _ in 0..count {
            match make_stress_client().await {
                Ok(c) => pool.push(c),
                Err(e) => eprintln!("  client error: {e}"),
            }
        }

        let started = Instant::now();
        let deadline = started + Duration::from_secs(duration_secs);

        let mut handles = Vec::new();
        for mut client in pool {
            let target = target.clone();
            handles.push(tokio::spawn(async move {
                let mut recorder = LatencyRecorder::new();
                while Instant::now() < deadline {
                    let start = Instant::now();
                    match read(&client, &target, oid).await {
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
        let point = DegradationPoint::from_clients(count, started, results);
        point.log();
        curve.push(point);
    }

    running.stop().await;
    curve
}
