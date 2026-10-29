//! BACnet stress testing suite — find breaking points across transports.
#![allow(clippy::print_stdout, clippy::print_stderr)] // benchmark binaries report progress and results on the console
#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};

use bacnet_benchmarks::stress::clients::ClientTransport;
use bacnet_benchmarks::stress::harness::current_rss_kb;
use bacnet_benchmarks::stress::mstp::Reply;
use bacnet_benchmarks::stress::output::{StressMetrics, StressResult, print_results};
use bacnet_benchmarks::stress::router::Path;

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
        /// `bip`, `bip6` (B/IPv6 on ::1) or `sc` (BACnet/SC with mutual TLS,
        /// through an in-process hub)
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
    /// COV subscriptions up to the server's total, spread over as many
    /// subscribers as the per-subscriber limit needs
    Cov {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, default_value = "10")]
        change_rate_hz: u64,
        /// Ask for confirmed notifications, which each subscriber acknowledges
        #[arg(long)]
        confirmed: bool,
        #[arg(long, value_delimiter = ',', default_value = "1,10,64,256,1024")]
        steps: Vec<u64>,
    },
    /// Segmented RPM stress
    Segmentation {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, value_delimiter = ',', default_value = "10,50,100,250")]
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
    /// Router forwarding overhead: the `direct` baseline, or through a
    /// BACnetRouter to a `bip` or `mstp` network
    Router {
        #[arg(long, default_value = "bip")]
        path: String,
        /// MS/TP baud rate for `--path mstp`
        #[arg(long, default_value = "115200")]
        baud: u32,
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,5,10,25,50")]
        steps: Vec<u64>,
    },
    /// MS/TP confirmed-request rate against the wire limit, per baud rate
    Mstp {
        /// `small`, `standard` (near 480 octets) or `extended` (near 1,476
        /// octets, over extended frames)
        #[arg(long, default_value = "small")]
        reply: String,
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "9600,38400,76800,115200")]
        steps: Vec<u64>,
    },
    /// BBMD forwarding capacity: foreign devices send broadcasts through
    /// it as fast as they can. On Linux each foreign device has its own
    /// loopback address (run it in Docker on other hosts)
    Bbmd {
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "1,5,10,25,50,100")]
        steps: Vec<u64>,
    },
    /// Device scan: one client reads every Device's Object_Name from N
    /// servers, one after another (no Who-Is; see the Docker orchestrator's
    /// discovery scenario)
    DeviceScan {
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

    let valid = rt.block_on(async {
        let rss_before = current_rss_kb();

        let mut clients_diagnostics = None;
        let (scenario, transport, params, curve) = match cli.command {
            Command::Clients {
                transport,
                objects,
                duration,
                steps,
            } => {
                let Some(kind) = ClientTransport::parse(&transport) else {
                    eprintln!("Error: --transport must be bip, bip6 or sc");
                    std::process::exit(2);
                };
                let run =
                    bacnet_benchmarks::stress::clients::run(kind, objects, duration, &steps).await;
                clients_diagnostics = Some(run.diagnostics);
                let curve = run.curve;
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
                confirmed,
                steps,
            } => {
                let curve = bacnet_benchmarks::stress::cov::run(
                    duration,
                    change_rate_hz,
                    confirmed,
                    &steps,
                )
                .await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "change_rate_hz": change_rate_hz,
                    "confirmed": confirmed,
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
            Command::Router {
                path,
                baud,
                duration,
                steps,
            } => {
                let Some(route) = Path::parse(&path) else {
                    eprintln!("Error: --path must be direct, bip or mstp");
                    std::process::exit(2);
                };
                let curve =
                    bacnet_benchmarks::stress::router::run(route, baud, duration, &steps).await;
                let mut params = serde_json::json!({
                    "path": path,
                    "duration_secs": duration,
                    "steps": steps,
                });
                if route == Path::Mstp {
                    params["baud"] = baud.into();
                }
                ("router_forwarding".into(), "bip".into(), params, curve)
            }
            Command::Mstp {
                reply,
                duration,
                steps,
            } => {
                let Some(size) = Reply::parse(&reply) else {
                    eprintln!("Error: --reply must be small, standard or extended");
                    std::process::exit(2);
                };
                let (curve, wire) =
                    bacnet_benchmarks::stress::mstp::run(size, duration, &steps).await;
                let params = serde_json::json!({
                    "reply": reply,
                    "duration_secs": duration,
                    "baud_rates": steps,
                    "wire": wire,
                });
                ("mstp_wire".into(), "mstp".into(), params, curve)
            }
            Command::Bbmd { duration, steps } => {
                let curve = bacnet_benchmarks::stress::bbmd::run(duration, &steps).await;
                let params = serde_json::json!({
                    "duration_secs": duration,
                    "steps": steps,
                    "distinct_addresses": bacnet_benchmarks::stress::bbmd::DISTINCT_ADDRESSES,
                });
                ("bbmd_capacity".into(), "bip".into(), params, curve)
            }
            Command::DeviceScan { steps } => {
                let curve = bacnet_benchmarks::stress::device_scan::run(&steps).await;
                let params = serde_json::json!({ "steps": steps });
                ("device_scan".into(), "bip".into(), params, curve)
            }
        };

        let rss_after = current_rss_kb();

        // Summarise every step, from what each step counted and timed.
        let result = StressResult {
            scenario,
            transport,
            parameters: params,
            results: StressMetrics::from_curve(curve, rss_after.max(rss_before)),
            clients_diagnostics,
        };

        print_results(&result);
        result
            .clients_diagnostics
            .as_ref()
            .is_none_or(|diagnostics| diagnostics.valid)
    });
    if !valid {
        std::process::exit(1);
    }
}
