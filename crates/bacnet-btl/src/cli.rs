//! CLI argument parsing via clap.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bacnet-test",
    about = "BTL compliance test harness for BACnet devices"
)]
pub struct Cli {
    /// Bind interface address
    #[arg(long, default_value = "0.0.0.0")]
    pub interface: Ipv4Addr,

    /// BACnet port
    #[arg(long, default_value = "47808")]
    pub port: u16,

    /// Broadcast address (BIP only)
    #[arg(long, default_value = "255.255.255.255")]
    pub broadcast: Ipv4Addr,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List available tests
    List {
        /// Filter by section number (e.g., "2", "3.1")
        #[arg(long)]
        section: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Search test names and references
        #[arg(long)]
        search: Option<String>,
    },
    /// Execute tests against an external IUT
    Run {
        /// IUT target address (IP:port for BIP, or VMAC hex for SC)
        #[arg(long)]
        target: String,
        /// SC hub WebSocket URL — enables SC transport for test client
        #[arg(long)]
        sc_hub: Option<String>,
        /// Skip TLS certificate verification for SC (testing only)
        #[arg(long)]
        sc_no_verify: bool,
        /// Filter by section
        #[arg(long)]
        section: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Run a single test by ID
        #[arg(long)]
        test: Option<String>,
        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,
        /// Show which tests would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Save report to file (JSON)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format (terminal, json)
        #[arg(long, default_value = "terminal")]
        format: String,
    },
    /// Test our own BACnet server
    SelfTest {
        /// Self-test mode (in-process, subprocess)
        #[arg(long, default_value = "in-process")]
        mode: String,
        /// Filter by section
        #[arg(long)]
        section: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Run a single test by ID
        #[arg(long)]
        test: Option<String>,
        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,
        /// Show which tests would run without executing
        #[arg(long)]
        dry_run: bool,
        /// Save report to file (JSON)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format (terminal, json)
        #[arg(long, default_value = "terminal")]
        format: String,
        /// Verbose output
        #[arg(long)]
        verbose: bool,
    },
    /// Interactive REPL mode
    Shell,
    /// Run a standalone BTL-compliant BACnet server (for Docker/external testing)
    Serve {
        /// Device instance number
        #[arg(long, default_value_t = 99999)]
        device_instance: u32,
        /// SC hub WebSocket URL (e.g., wss://hub:47809) — enables SC transport
        #[arg(long)]
        sc_hub: Option<String>,
        /// Skip TLS certificate verification for SC (testing only)
        #[arg(long)]
        sc_no_verify: bool,
    },
}
