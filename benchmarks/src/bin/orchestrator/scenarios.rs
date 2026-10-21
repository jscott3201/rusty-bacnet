//! The orchestrator's scenarios.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bacnet_benchmarks::stress::output::{DegradationPoint, LatencyRecorder};
use bacnet_client::client::{BACnetClient, DeviceEventKind};
use bacnet_transport::bip::{BipMode, BipTransport, ForeignDeviceConfig};
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// A BACnet router to send through: its B/IP address on the client's
/// network, and the network the target is on.
#[derive(Clone, Copy)]
pub struct Via {
    pub router_ip: [u8; 4],
    pub router_port: u16,
    pub network: u16,
}

async fn client_on(
    interface: Ipv4Addr,
    port: u16,
    broadcast: Ipv4Addr,
) -> Result<BACnetClient<BipTransport>, Error> {
    BACnetClient::bip_builder()
        .interface(interface)
        .port(port)
        .broadcast_address(broadcast)
        .apdu_timeout_ms(5000)
        .build()
        .await
}

/// ReadProperty of Analog Input 1 one after another for `duration`. Every
/// request goes to `target_ip`; with `via`, each names the target's network
/// and MAC and goes to the router, which forwards it (Docker's bridge
/// networks don't route IP between each other, so only a BACnet router
/// joins them).
pub async fn read_loop(
    client: &BACnetClient<BipTransport>,
    label: &str,
    param: u64,
    target_ip: [u8; 4],
    via: Option<Via>,
    duration: Duration,
) -> DegradationPoint {
    let mac = encode_bip_mac(target_ip, 47808);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let property = PropertyIdentifier::PRESENT_VALUE;
    let mut recorder = LatencyRecorder::new();
    let start = Instant::now();
    let deadline = start + duration;
    while Instant::now() < deadline {
        let t0 = Instant::now();
        let result = match via {
            None => client.read_property(&mac, oid, property, None).await,
            Some(via) => {
                let router = encode_bip_mac(via.router_ip, via.router_port);
                client
                    .read_property_routed(&router, via.network, &mac, oid, property, None)
                    .await
            }
        };
        match result {
            Ok(_) => recorder.record_success(t0),
            Err(_) => recorder.record_failure(),
        }
    }
    let point = DegradationPoint::from_recorder(param, &recorder, start.elapsed());
    eprint!("  {label}:");
    point.log();
    point
}

/// [`read_loop`] from a new client on `interface`.
pub async fn run_rp_scenario(
    label: &str,
    param: u64,
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    target_ip: [u8; 4],
    via: Option<Via>,
    duration: Duration,
) -> Result<DegradationPoint, Error> {
    let mut client = client_on(interface, 0, broadcast).await?;
    let point = read_loop(&client, label, param, target_ip, via, duration).await;
    let _ = client.stop().await;
    Ok(point)
}

/// How long one discovery round waits for the whole fleet to answer.
const ROUND_TIMEOUT: Duration = Duration::from_secs(2);

/// The pause between discovery rounds: 50 ms longer than a server's
/// default window for coalescing repeated Who-Is requests.
fn round_gap() -> Duration {
    bacnet_server::server::DiscoveryPolicy::default().coalesce_window + Duration::from_millis(50)
}

/// Who-Is discovery: the client on the fleet network broadcasts a Who-Is to
/// every device and times how long until `fleet` distinct devices have
/// answered with an I-Am, round after round for `duration`. A round in
/// which some device didn't answer within 2 seconds counts as failed. The
/// step's rate is rounds per second of the rounds' own time (not the pause
/// between them); its percentiles are the time to the last I-Am. A device
/// may broadcast its I-Am to UDP port 47808, so the client binds that port.
pub async fn discovery(fleet: u64, duration: Duration) -> Result<DegradationPoint, Error> {
    let mut client = client_on(
        Ipv4Addr::new(172, 20, 5, 100),
        47808,
        Ipv4Addr::new(172, 20, 5, 255),
    )
    .await?;
    let mut to_all = LatencyRecorder::new();
    let mut to_first = LatencyRecorder::new();
    let mut missing = 0u64;
    let start = Instant::now();
    let deadline = start + duration;
    let mut round = 0u64;
    let mut timed = Duration::ZERO;
    while round == 0 || Instant::now() < deadline {
        if round > 0 {
            // A server drops a repeat of the same Who-Is from one source
            // within its discovery policy's coalescing window (200 ms by
            // default); wait it out, off the clock.
            tokio::time::sleep(round_gap()).await;
        }
        round += 1;
        client.clear_devices().await;
        let mut events = client.device_events();
        let mut seen = HashSet::new();
        let t0 = Instant::now();
        client.who_is(None).await?;
        let round_end = t0 + ROUND_TIMEOUT;
        while (seen.len() as u64) < fleet {
            let left = round_end.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, events.recv()).await {
                Ok(Ok(event)) => {
                    let new = matches!(
                        event.kind,
                        DeviceEventKind::Discovered | DeviceEventKind::Updated
                    ) && seen.insert(event.device.object_identifier.instance_number());
                    if new && seen.len() == 1 {
                        to_first.record_success(t0);
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(_)) | Err(_) => break,
            }
        }
        timed += t0.elapsed();
        if seen.len() as u64 >= fleet {
            to_all.record_success(t0);
        } else {
            to_all.record_failure();
            missing += fleet - seen.len() as u64;
        }
    }
    let point = DegradationPoint::from_recorder(fleet, &to_all, timed);
    let first = to_first.stats();
    eprintln!(
        "  {round} rounds: all {fleet} devices answered in {} of them; time to the last \
         I-Am p50={} µs p99={} µs, to the first p50={} µs; {missing} answers missing",
        to_all.successful(),
        point.p50_us,
        point.p99_us,
        first.p50,
    );
    point.log();
    let _ = client.stop().await;
    Ok(point)
}

/// Device instance and address of server-a on network 1.
const SERVER_A: (u32, [u8; 4]) = (1000, [172, 20, 0, 10]);

/// A foreign device on the foreign network (172.20.4.0/24) registers with
/// `bbmd-a`, finds server-a with a Who-Is the BBMD distributes on network 1,
/// then reads from it for `duration`. Server-a answers the Who-Is with an
/// I-Am sent straight back to the foreign device (a server prefers directed
/// replies by default, `DiscoveryPolicy::prefer_directed_responses`), and
/// that reply and the reads are unicast over the `ip-router` service that
/// joins the foreign network to network 1; only the Who-Is goes through
/// the BBMD. This must run on a host whose only network is the foreign one,
/// or it would reach network 1 directly.
pub async fn foreign(param: u64, duration: Duration) -> Result<DegradationPoint, Error> {
    let interface = Ipv4Addr::new(172, 20, 4, 10);
    let mut client = BACnetClient::bip_builder()
        .interface(interface)
        .port(0)
        .broadcast_address(Ipv4Addr::new(172, 20, 4, 255))
        .apdu_timeout_ms(5000)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::new(172, 20, 0, 2),
            bbmd_port: 47808,
            ttl: 300,
            renewal_interval: None,
        }))
        .build()
        .await?;

    // Registration with the BBMD.
    let t0 = Instant::now();
    let registered = loop {
        if client.foreign_registration()?.registered {
            break Some(t0.elapsed());
        }
        if t0.elapsed() > Duration::from_secs(10) {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let Some(registered) = registered else {
        let snapshot = client.foreign_registration()?;
        let _ = client.stop().await;
        return Err(Error::Encoding(format!(
            "no registration with bbmd-a within 10 seconds (last outcome {:?})",
            snapshot.last_outcome
        )));
    };
    eprintln!(
        "  registered with bbmd-a in {:.1} ms",
        registered.as_secs_f64() * 1000.0
    );

    // Discovery of server-a through the BBMD.
    let (instance, address) = SERVER_A;
    let range = bacnet_services::who_is::DeviceInstanceRange::new(instance, instance)?;
    let mut events = client.device_events();
    let t0 = Instant::now();
    client.who_is(Some(range)).await?;
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(event) if event.device.object_identifier.instance_number() == instance => {
                    break Some(event.device.mac_address.to_vec());
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break None,
            }
        }
    })
    .await
    .ok()
    .flatten();
    match &found {
        Some(mac) => eprintln!(
            "  found server-a (Who-Is through bbmd-a) in {:.1} ms, at MAC {mac:02x?}",
            t0.elapsed().as_secs_f64() * 1000.0
        ),
        None => eprintln!("  no I-Am from server-a within 5 seconds"),
    }
    if found.as_deref() != Some(&encode_bip_mac(address, 47808)[..]) {
        let _ = client.stop().await;
        return Err(Error::Encoding(
            "the foreign device did not discover server-a at its address".into(),
        ));
    }

    let point = read_loop(&client, "foreign_rp", param, address, None, duration).await;
    let _ = client.stop().await;
    Ok(point)
}
