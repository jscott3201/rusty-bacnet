//! Cross-network stress orchestrator for the Docker topology.
//!
//! Runs scenarios across the `examples/docker` networks: a same-network
//! baseline, a hop through the BACnet router, Who-Is discovery of a fleet
//! of devices on one broadcast domain, and a foreign device that registers
//! with a BBMD and reaches a server on its network. The foreign-device
//! scenario must run from a host on the foreign network only (the
//! `foreign-bench` service), so the orchestrator leaves it out unless asked.
#![allow(clippy::print_stdout, clippy::print_stderr)] // benchmark binaries report progress and results on the console
#![forbid(unsafe_code)]

#[path = "orchestrator/scenarios.rs"]
mod scenarios;

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bacnet_benchmarks::stress::output::{print_results, StressMetrics, StressResult};
use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use clap::Parser;

use scenarios::{discovery, foreign, run_rp_scenario, Via};

/// Every scenario, by the name `--only` and `--skip` take.
const SCENARIOS: [&str; 4] = ["same-subnet", "router", "discovery", "foreign"];

#[derive(Parser)]
#[command(
    name = "stress-orchestrator",
    about = "Cross-network BACnet stress orchestrator"
)]
struct Args {
    /// Seconds to wait for all services to be ready
    #[arg(long, default_value_t = 10)]
    wait_for_ready: u64,

    /// Duration per test in seconds
    #[arg(long, default_value_t = 10)]
    duration: u64,

    /// Scenarios to run (comma-separated: same-subnet,router,discovery,foreign).
    /// Without it, every scenario but foreign runs
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Scenarios to skip (comma-separated, as for --only)
    #[arg(long, value_delimiter = ',')]
    skip: Vec<String>,

    /// Devices the discovery scenario expects on the fleet network (the
    /// `fleet` service's replicas)
    #[arg(long, default_value_t = 20)]
    fleet: u64,
}

/// The scenarios to run, in order: `only` (or every one but `foreign`),
/// less `skip`. An unknown name is an error.
fn selected(only: &[String], skip: &[String]) -> Result<Vec<&'static str>, String> {
    for name in only.iter().chain(skip) {
        if !SCENARIOS.contains(&name.as_str()) {
            return Err(format!(
                "unknown scenario {name:?}; expected one of {}",
                SCENARIOS.join(", ")
            ));
        }
    }
    Ok(SCENARIOS
        .into_iter()
        .filter(|s| {
            if only.is_empty() {
                *s != "foreign"
            } else {
                only.iter().any(|o| o == s)
            }
        })
        .filter(|s| !skip.iter().any(|k| k == s))
        .collect())
}

/// Wait for a BIP device to respond to a ReadProperty.
async fn wait_for_device(
    client: &BACnetClient<BipTransport>,
    target_ip: Ipv4Addr,
    target_port: u16,
    device_instance: u32,
    timeout: Duration,
) -> bool {
    let mac = bacnet_transport::bvll::encode_bip_mac(target_ip.octets(), target_port);
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        let result = client
            .read_property(&mac, oid, PropertyIdentifier::OBJECT_NAME, None)
            .await;
        if result.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let scenarios = selected(&args.only, &args.skip)?;

    eprintln!(
        "Waiting {}s for services to be ready...",
        args.wait_for_ready
    );
    tokio::time::sleep(Duration::from_secs(args.wait_for_ready)).await;

    // Probe server-a from network 1, unless only the foreign scenario runs:
    // its host is on the foreign network alone, and probes for itself.
    if scenarios.iter().any(|s| *s != "foreign") {
        let probe_client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::new(172, 20, 0, 100))
            .port(0)
            .broadcast_address(Ipv4Addr::new(172, 20, 0, 255))
            .apdu_timeout_ms(5000)
            .build()
            .await?;
        let ready = wait_for_device(
            &probe_client,
            Ipv4Addr::new(172, 20, 0, 10),
            47808,
            1000,
            Duration::from_secs(30),
        )
        .await;
        if !ready {
            eprintln!("ERROR: server-a not responding after 30s");
            std::process::exit(1);
        }
        eprintln!("server-a is ready");
    }

    let duration = Duration::from_secs(args.duration);
    let mut all_points = Vec::new();
    let mut discovery_point = None;
    for (index, scenario) in scenarios.iter().enumerate() {
        let param = index as u64 + 1;
        let point = match *scenario {
            // Client on network 1 reads from server-a on the same network.
            "same-subnet" => {
                eprintln!("\n=== Same-Subnet Baseline ===");
                run_rp_scenario(
                    "same_subnet_rp",
                    param,
                    Ipv4Addr::new(172, 20, 0, 100),
                    Ipv4Addr::new(172, 20, 0, 255),
                    [172, 20, 0, 10],
                    None,
                    duration,
                )
                .await
            }
            // Client on network 1 reads from server-b on network 2 through
            // the router service.
            "router" => {
                eprintln!("\n=== Router Hop Latency ===");
                run_rp_scenario(
                    "routed_rp",
                    param,
                    Ipv4Addr::new(172, 20, 0, 100),
                    Ipv4Addr::new(172, 20, 0, 255),
                    [172, 20, 1, 10],
                    Some(Via {
                        router_ip: [172, 20, 0, 1],
                        router_port: 47808,
                        network: 2,
                    }),
                    duration,
                )
                .await
            }
            "discovery" => {
                eprintln!("\n=== Who-Is Discovery of {} Devices ===", args.fleet);
                discovery(args.fleet, duration).await
            }
            "foreign" => {
                eprintln!("\n=== Foreign Device registered with bbmd-a ===");
                foreign(param, duration).await
            }
            _ => unreachable!("selected() returns known scenarios"),
        };
        match point {
            // Discovery counts rounds, not requests, so it stays out of the
            // combined summary; it printed its own line, and the JSON holds
            // its point under `parameters.discovery`.
            Ok(point) if *scenario == "discovery" => discovery_point = Some(point),
            Ok(point) => all_points.push(point),
            Err(e) => eprintln!("  {scenario} failed: {e}"),
        }
    }

    let result = StressResult {
        scenario: "docker-cross-network".into(),
        transport: "bip".into(),
        parameters: serde_json::json!({
            "duration_secs": args.duration,
            "scenarios": scenarios,
            "fleet": args.fleet,
            "discovery": discovery_point,
        }),
        results: StressMetrics::from_curve(
            all_points,
            bacnet_benchmarks::stress::harness::current_rss_kb(),
        ),
    };
    print_results(&result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn foreign_runs_only_when_asked() {
        assert_eq!(
            selected(&[], &[]).unwrap(),
            vec!["same-subnet", "router", "discovery"]
        );
        assert_eq!(
            selected(&names(&["foreign"]), &[]).unwrap(),
            vec!["foreign"]
        );
        assert_eq!(
            selected(&[], &names(&["router"])).unwrap(),
            vec!["same-subnet", "discovery"]
        );
        assert!(selected(&names(&["bbmd"]), &[]).is_err());
    }
}
