//! BACnet stress testing suite — find breaking points across transports.

use clap::{Parser, Subcommand};

use bacnet_benchmarks::stress::harness::current_rss_kb;
use bacnet_benchmarks::stress::output::{print_results, LatencyStats, StressMetrics, StressResult};

#[derive(Parser)]
#[command(name = "stress-test", about = "BACnet stress & limits testing suite")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Enable tokio-console (requires --features console)
    #[arg(long, global = true)]
    console: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Scale concurrent clients against a single server
    Clients {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, default_value = "100")]
        objects: u32,
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,5,10,25,50,100")]
        steps: Vec<u64>,
    },
    /// Scale object count on a single server
    Objects {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "100,500,1000,2500,5000,10000"
        )]
        steps: Vec<u64>,
    },
    /// COV subscription saturation
    Cov {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, default_value = "10")]
        change_rate_hz: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,10,50,100,200,255")]
        steps: Vec<u64>,
    },
    /// Segmented RPM stress
    Segmentation {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, value_delimiter = ',', default_value = "10,25,50,100")]
        steps: Vec<u64>,
    },
    /// Mixed realistic workload
    Mixed {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, default_value = "10")]
        clients: u64,
        #[arg(long, default_value = "30")]
        duration: u64,
    },
    /// Router forwarding overhead
    Router {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,5,10,25,50")]
        steps: Vec<u64>,
    },
    /// BBMD foreign device stress
    Bbmd {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,5,10,25")]
        steps: Vec<u64>,
    },
    /// WhoIs broadcast storm
    Whois {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "5,10,25,50")]
        steps: Vec<u64>,
    },
}

fn main() {
    let cli = Cli::parse();

    let rt = if cli.console {
        #[cfg(feature = "console")]
        {
            console_subscriber::init();
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
        }
        #[cfg(not(feature = "console"))]
        {
            eprintln!("Error: --console requires --features console");
            std::process::exit(1);
        }
    } else {
        tokio::runtime::Runtime::new().unwrap()
    };

    rt.block_on(async {
        let rss_before = current_rss_kb();

        let (scenario, transport, params, curve) = match cli.command {
            Command::Clients {
                transport,
                objects,
                duration,
                steps,
            } => {
                let curve =
                    bacnet_benchmarks::stress::clients::run(&transport, objects, duration, &steps)
                        .await;
                let params = serde_json::json!({
                    "objects": objects,
                    "duration_secs": duration,
                    "steps": steps,
                });
                ("max_clients".into(), transport, params, curve)
            }
            Command::Objects {
                transport,
                duration,
                steps,
            } => {
                let curve =
                    bacnet_benchmarks::stress::objects::run(&transport, duration, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "steps": steps,
                });
                ("object_scale".into(), transport, params, curve)
            }
            Command::Cov {
                duration,
                change_rate_hz,
                steps,
            } => {
                let curve =
                    bacnet_benchmarks::stress::cov::run(duration, change_rate_hz, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "change_rate_hz": change_rate_hz,
                    "steps": steps,
                });
                ("cov_saturation".into(), "bip".into(), params, curve)
            }
            Command::Segmentation { transport, steps } => {
                let curve = bacnet_benchmarks::stress::segmentation::run(&transport, &steps).await;
                let params = serde_json::json!({ "steps": steps });
                ("segmentation".into(), transport, params, curve)
            }
            Command::Mixed {
                transport,
                clients,
                duration,
            } => {
                let curve =
                    bacnet_benchmarks::stress::mixed::run(&transport, clients, duration).await;
                let params = serde_json::json!({
                    "clients": clients,
                    "duration_secs": duration,
                });
                ("mixed_workload".into(), transport, params, curve)
            }
            Command::Router { duration, steps } => {
                let curve = bacnet_benchmarks::stress::router::run(duration, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "steps": steps,
                });
                ("router_forwarding".into(), "bip".into(), params, curve)
            }
            Command::Bbmd { duration, steps } => {
                let curve = bacnet_benchmarks::stress::bbmd::run(duration, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "steps": steps,
                });
                ("bbmd_foreign_device".into(), "bip".into(), params, curve)
            }
            Command::Whois { duration, steps } => {
                let curve = bacnet_benchmarks::stress::whois::run(duration, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "steps": steps,
                });
                ("whois_storm".into(), "bip".into(), params, curve)
            }
        };

        let rss_after = current_rss_kb();

        // Build aggregate metrics from degradation curve
        let (total_ok, total_err) = curve.iter().fold((0u64, 0u64), |(ok, err), pt| {
            (ok + (pt.throughput * 10.0) as u64, err + pt.errors)
        });
        let total = total_ok + total_err;
        let overall_throughput = if let Some(last) = curve.last() {
            last.throughput
        } else {
            0.0
        };
        let latency = if let Some(last) = curve.last() {
            LatencyStats {
                min: 0,
                p50: last.p50_us,
                p95: 0,
                p99: last.p99_us,
                p999: 0,
                max: 0,
            }
        } else {
            LatencyStats {
                min: 0,
                p50: 0,
                p95: 0,
                p99: 0,
                p999: 0,
                max: 0,
            }
        };

        let result = StressResult {
            scenario,
            transport,
            parameters: params,
            results: StressMetrics {
                total_requests: total,
                successful: total_ok,
                failed: total_err,
                error_rate_pct: if total > 0 {
                    (total_err as f64 / total as f64) * 100.0
                } else {
                    0.0
                },
                throughput_ops_sec: overall_throughput,
                latency_us: latency,
                peak_rss_kb: rss_after.max(rss_before),
                degradation_curve: curve,
            },
        };

        print_results(&result);
    });
}
