// Same B/IP-only entry point for all three immutable comparison inputs.
use clap::{Parser, Subcommand};
use stress::output::{StressMetrics, StressResult, print_results};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Clients {
        #[arg(long, default_value = "bip")]
        transport: String,
        #[arg(long, default_value = "100")]
        objects: u32,
        #[arg(long, default_value = "10")]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_value = "100")]
        steps: Vec<u64>,
    },
}
fn main() {
    let Cli {
        command:
            Command::Clients {
                transport,
                objects,
                duration,
                steps,
            },
    } = Cli::parse();
    if transport != "bip" {
        eprintln!("normalized-clients supports only bip");
        std::process::exit(2);
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let run = runtime.block_on(stress::client_run::run_steps(
        objects,
        duration,
        &steps,
        |index, count, time| stress::client_bip::step(index, objects, count, time),
    ));
    let valid = run.diagnostics.valid;
    let result = StressResult {
        scenario: "max_clients".into(),
        transport,
        parameters: serde_json::json!({ "objects": objects, "duration_secs": duration, "steps": steps }),
        results: StressMetrics::from_curve(run.curve, current_rss_kb()),
        clients_diagnostics: Some(run.diagnostics),
    };
    print_results(&result);
    if !valid {
        std::process::exit(1);
    }
}
fn current_rss_kb() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory() / 1024).unwrap_or(0)
}

#[cfg(test)]
mod equivalence {
    include!("encoding_guard.rs");
}
