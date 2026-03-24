//! bacnet-test — BTL compliance test harness for BACnet devices.

mod cli;
mod shell;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use bacnet_btl::engine::registry::TestRegistry;
use bacnet_btl::engine::runner::{RunConfig, TestRunner};
use bacnet_btl::engine::selector::TestFilter;
use bacnet_btl::report::{json, terminal};
use bacnet_btl::self_test::in_process::InProcessServer;
use bacnet_btl::tests;

use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::List {
            section,
            tag,
            search,
        } => cmd_list(section, tag, search),

        Command::SelfTest {
            mode: _,
            section,
            tag,
            test,
            fail_fast,
            dry_run,
            report,
            format,
            verbose,
        } => {
            cmd_self_test(
                section, tag, test, fail_fast, dry_run, report, &format, verbose,
            )
            .await;
        }

        Command::Run {
            target,
            sc_hub,
            sc_no_verify,
            section,
            tag,
            test,
            fail_fast,
            dry_run,
            report,
            format,
        } => {
            cmd_run(
                &target,
                cli.interface,
                cli.port,
                cli.broadcast,
                sc_hub,
                sc_no_verify,
                section,
                tag,
                test,
                fail_fast,
                dry_run,
                report,
                &format,
            )
            .await;
        }

        Command::Shell => {
            shell::run_shell().await;
        }

        Command::Serve {
            device_instance,
            sc_hub,
            sc_no_verify,
        } => {
            cmd_serve(
                cli.interface,
                cli.port,
                cli.broadcast,
                device_instance,
                sc_hub,
                sc_no_verify,
            )
            .await;
        }
    }
}

async fn cmd_serve(
    interface: std::net::Ipv4Addr,
    port: u16,
    broadcast: std::net::Ipv4Addr,
    device_instance: u32,
    sc_hub: Option<String>,
    _sc_no_verify: bool,
) {
    use bacnet_server::server::BACnetServer;

    let db = InProcessServer::build_test_database();
    let obj_count = db.list_objects().len();

    if let Some(_hub_url) = sc_hub {
        #[cfg(feature = "sc-tls")]
        {
            let tls_config = if _sc_no_verify {
                // No-verify TLS for testing (TLS 1.3 per spec AB.7.4)
                let config = tokio_rustls::rustls::ClientConfig::builder_with_protocol_versions(&[
                    &tokio_rustls::rustls::version::TLS13,
                ])
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(NoVerify))
                .with_no_client_auth();
                std::sync::Arc::new(config)
            } else {
                let root_store = tokio_rustls::rustls::RootCertStore::empty();
                let config = tokio_rustls::rustls::ClientConfig::builder_with_protocol_versions(&[
                    &tokio_rustls::rustls::version::TLS13,
                ])
                .with_root_certificates(root_store)
                .with_no_client_auth();
                std::sync::Arc::new(config)
            };

            let vmac: [u8; 6] = rand::random();
            let mut server = BACnetServer::sc_builder()
                .hub_url(&_hub_url)
                .tls_config(tls_config)
                .vmac(vmac)
                .database(db)
                .build()
                .await
                .expect("Failed to start SC server");

            let vmac_str: String = vmac
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(":");
            eprintln!(
                "BTL server (SC) connected to {} — instance={}, vmac={}, objects={}",
                _hub_url, device_instance, vmac_str, obj_count
            );
            eprintln!("Press Ctrl+C to stop.");

            tokio::signal::ctrl_c().await.ok();
            server.stop().await.ok();
        }
        #[cfg(not(feature = "sc-tls"))]
        {
            eprintln!("SC transport requires the 'sc-tls' feature. Rebuild with:");
            eprintln!("  cargo build -p bacnet-btl --features sc-tls");
            std::process::exit(1);
        }
    } else {
        // BIP mode
        let mut server = BACnetServer::bip_builder()
            .interface(interface)
            .port(port)
            .broadcast_address(broadcast)
            .database(db)
            .build()
            .await
            .expect("Failed to start BIP server");

        let mac: String = server
            .local_mac()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        eprintln!(
            "BTL server (BIP) listening on {}:{} — instance={}, mac={}, objects={}",
            interface, port, device_instance, mac, obj_count
        );
        eprintln!("Press Ctrl+C to stop.");

        tokio::signal::ctrl_c().await.ok();
        server.stop().await.ok();
    }
}

#[cfg(feature = "sc-tls")]
#[derive(Debug)]
struct NoVerify;

#[cfg(feature = "sc-tls")]
impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    target: &str,
    interface: std::net::Ipv4Addr,
    port: u16,
    broadcast: std::net::Ipv4Addr,
    sc_hub: Option<String>,
    _sc_no_verify: bool,
    section: Option<String>,
    tag: Option<String>,
    test: Option<String>,
    fail_fast: bool,
    dry_run: bool,
    report: Option<std::path::PathBuf>,
    format: &str,
) {
    use bacnet_btl::engine::context::{ClientHandle, TestContext};
    use bacnet_btl::report::model::TestMode;

    // Build capabilities from the test database (assumes target runs our BTL server)
    let db = InProcessServer::build_test_database();
    let capabilities = InProcessServer::build_capabilities(&db);

    let (client_handle, target_mac) = if let Some(_hub_url) = sc_hub {
        // SC transport — connect through hub
        #[cfg(feature = "sc-tls")]
        {
            use bacnet_client::client::BACnetClient;

            let tls_config = if _sc_no_verify {
                let config = tokio_rustls::rustls::ClientConfig::builder_with_protocol_versions(&[
                    &tokio_rustls::rustls::version::TLS13,
                ])
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(NoVerify))
                .with_no_client_auth();
                std::sync::Arc::new(config)
            } else {
                let root_store = tokio_rustls::rustls::RootCertStore::empty();
                let config = tokio_rustls::rustls::ClientConfig::builder_with_protocol_versions(&[
                    &tokio_rustls::rustls::version::TLS13,
                ])
                .with_root_certificates(root_store)
                .with_no_client_auth();
                std::sync::Arc::new(config)
            };

            let vmac: [u8; 6] = rand::random();
            eprintln!("Connecting SC client to hub {_hub_url}...");
            let client = BACnetClient::sc_builder()
                .hub_url(&_hub_url)
                .tls_config(tls_config)
                .vmac(vmac)
                .apdu_timeout_ms(3000)
                .build()
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to connect SC client: {e}");
                    std::process::exit(1);
                });

            // Parse target as VMAC hex (e.g., "8f:36:1c:d4:97:c7") or discover
            let target_mac: Vec<u8> = target
                .split(':')
                .map(|h| u8::from_str_radix(h, 16).unwrap_or(0))
                .collect();

            if target_mac.len() != 6 {
                eprintln!(
                    "SC target must be a 6-byte VMAC (e.g., aa:bb:cc:dd:ee:ff), got: {target}"
                );
                std::process::exit(1);
            }

            eprintln!("SC client connected. Testing target VMAC {target}");
            (ClientHandle::Sc(client), target_mac)
        }
        #[cfg(not(feature = "sc-tls"))]
        {
            eprintln!("SC transport requires the 'sc-tls' feature. Rebuild with:");
            eprintln!("  cargo build -p bacnet-btl --features sc-tls");
            std::process::exit(1);
        }
    } else {
        // BIP transport
        use bacnet_client::client::BACnetClient;

        let parts: Vec<&str> = target.split(':').collect();
        let target_ip: std::net::Ipv4Addr = parts[0].parse().unwrap_or_else(|_| {
            eprintln!("Invalid target IP: {}", parts[0]);
            std::process::exit(1);
        });
        let target_port: u16 = parts.get(1).unwrap_or(&"47808").parse().unwrap_or(47808);

        let ip_bytes = target_ip.octets();
        let port_bytes = target_port.to_be_bytes();
        let target_mac: Vec<u8> = vec![
            ip_bytes[0],
            ip_bytes[1],
            ip_bytes[2],
            ip_bytes[3],
            port_bytes[0],
            port_bytes[1],
        ];

        eprintln!("Connecting BIP client to {target_ip}:{target_port}...");
        let client = BACnetClient::bip_builder()
            .interface(interface)
            .port(port)
            .broadcast_address(broadcast)
            .apdu_timeout_ms(3000)
            .build()
            .await
            .unwrap_or_else(|e| {
                eprintln!("Failed to create BIP client: {e}");
                std::process::exit(1);
            });

        (ClientHandle::Bip(client), target_mac)
    };

    let ctx = TestContext::new(
        client_handle,
        target_mac.into(),
        capabilities,
        None,
        TestMode::External,
    );

    let mut registry = TestRegistry::new();
    tests::register_all(&mut registry);
    let runner = TestRunner::new(registry);

    let config = RunConfig {
        filter: TestFilter {
            section,
            tag,
            test_id: test,
            ..Default::default()
        },
        fail_fast,
        dry_run,
        ..Default::default()
    };

    let run = runner.run(&mut { ctx }, &config).await;

    match format {
        "json" => println!("{}", json::to_json_string(&run).unwrap()),
        _ => terminal::print_test_run(&run, false),
    }

    if let Some(path) = report {
        if let Err(e) = json::save_json(&run, &path) {
            eprintln!("Failed to save report: {e}");
        }
    }

    if run.summary.failed > 0 || run.summary.errors > 0 {
        std::process::exit(1);
    }
}

fn cmd_list(section: Option<String>, tag: Option<String>, search: Option<String>) {
    let mut registry = TestRegistry::new();
    tests::register_all(&mut registry);

    let filter = TestFilter {
        section,
        tag,
        search,
        ..Default::default()
    };

    // Use a dummy capabilities set to show all tests (MustExecute only for filtering)
    let caps = bacnet_btl::iut::capabilities::IutCapabilities::default();
    let selected = bacnet_btl::engine::selector::TestSelector::select(&registry, &caps, &filter);

    if selected.is_empty() {
        println!("No tests match the given filters.");
        return;
    }

    println!("  {:<8} {:<50} Reference", "ID", "Name");
    println!("  {}", "─".repeat(100));
    for test in &selected {
        println!("  {:<8} {:<50} {}", test.id, test.name, test.reference);
    }
    println!();
    println!("  {} tests", selected.len());
}

#[allow(clippy::too_many_arguments)]
async fn cmd_self_test(
    section: Option<String>,
    tag: Option<String>,
    test: Option<String>,
    fail_fast: bool,
    dry_run: bool,
    report: Option<std::path::PathBuf>,
    format: &str,
    verbose: bool,
) {
    // Start the in-process server
    let server = match InProcessServer::start().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to start self-test server: {e}");
            std::process::exit(1);
        }
    };

    // Build the test context
    let mut ctx = match server.build_context().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to build test context: {e}");
            std::process::exit(1);
        }
    };

    // Build registry and runner
    let mut registry = TestRegistry::new();
    tests::register_all(&mut registry);
    let runner = TestRunner::new(registry);

    let config = RunConfig {
        filter: TestFilter {
            section,
            tag,
            test_id: test,
            ..Default::default()
        },
        fail_fast,
        dry_run,
        ..Default::default()
    };

    // Run the tests
    let run = runner.run(&mut ctx, &config).await;

    // Output results
    match format {
        "json" => {
            println!("{}", json::to_json_string(&run).unwrap());
        }
        _ => {
            terminal::print_test_run(&run, verbose);
        }
    }

    // Save report if requested
    if let Some(path) = report {
        if let Err(e) = json::save_json(&run, &path) {
            eprintln!("Failed to save report: {e}");
        } else {
            println!("Report saved to {}", path.display());
        }
    }

    // Exit with appropriate code
    if run.summary.failed > 0 || run.summary.errors > 0 {
        std::process::exit(1);
    }
}
