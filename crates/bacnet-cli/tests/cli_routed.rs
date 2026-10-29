//! Routed one-shot targets (#963): `bacnet read <dnet>:<instance>` and
//! `<dnet>:0x<mac>` reach the lab device behind an in-process
//! `BACnetRouter`, with the same JSON as a local read.
//!
//! The router's B/IP port is a loopback BBMD the CLI registers with as a
//! foreign device, so its Who-Is for network 2 reaches the router on a host
//! with nothing else on the link. The lab device is station 5 on network 2,
//! an in-memory link.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_transport::loopback::LoopbackTransport;
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};
use support::{Process, command, failure, run};

/// The lab device's network and instance.
const DNET: u16 = 2;
const INSTANCE: u32 = 1234;

/// The router, the lab device behind it, and the router's B/IP port.
///
/// The transport binds its own socket, so the pick lets go of the port
/// before the router takes it, and another process can take it in
/// between: a start that fails is tried again on a fresh port.
async fn site() -> (BACnetRouter, BACnetServer<LoopbackTransport>, u16) {
    site_with_database(read_services::database()).await
}

async fn site_with_database(
    database: bacnet_objects::database::ObjectDatabase,
) -> (BACnetRouter, BACnetServer<LoopbackTransport>, u16) {
    site_with_config(
        database,
        bacnet_server::server::ServerConfig {
            device: lab_identity(INSTANCE),
            ..Default::default()
        },
    )
    .await
}

async fn site_with_config(
    database: bacnet_objects::database::ObjectDatabase,
    config: bacnet_server::server::ServerConfig,
) -> (BACnetRouter, BACnetServer<LoopbackTransport>, u16) {
    let config = service_config(config);
    for _ in 0..8 {
        let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
        bip.set_mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..BbmdOptions::default()
        }));
        let (router_end, station) = LoopbackTransport::pair(vec![0x01], vec![0x05]);
        let ports = vec![
            RouterPort {
                transport: AnyTransport::Bip(Box::new(bip)),
                network_number: 1,
            },
            RouterPort {
                transport: AnyTransport::Loopback(router_end),
                network_number: DNET,
            },
        ];
        if let Ok(StartedRouter { router, .. }) =
            BACnetRouter::start(ports, RouterOptions::new()).await
        {
            let server = BACnetServer::start(config, database, station)
                .await
                .unwrap();
            return (router, server, port);
        }
    }
    panic!("no loopback UDP port the router could bind");
}

fn service_config(
    mut config: bacnet_server::server::ServerConfig,
) -> bacnet_server::server::ServerConfig {
    // Each service command starts with an empty discovery cache, even if the
    // OS reuses the preceding child's UDP source. Keep service assertions
    // independent of duplicate suppression; discovery.rs tests that policy.
    // Preserve admission limits, rate limits and directed replies.
    config.discovery_policy.coalesce_window = std::time::Duration::ZERO;
    config
}

/// `bacnet read` through the router's BBMD, waiting `wait_ms` for a routed
/// device to answer.
async fn read(port: u16, target: &str, wait_ms: &str) -> std::process::Output {
    let mut cmd = command();
    cmd.env("BACNET_CLI_ROUTED_WAIT_MS", wait_ms).args([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--foreign-bbmd",
        &format!("127.0.0.1:{port}"),
        "--json",
        "read",
        target,
        "analog-input:2",
        "present-value",
    ]);
    Process::spawn(&mut cmd).output().await
}

#[tokio::test]
async fn a_routed_read_reaches_the_device_behind_the_router() {
    let (mut router, mut server, port) = site().await;
    for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
        let output = read(port, &target, "3000").await;
        assert!(
            output.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        // The same shape a local read prints.
        assert_eq!(
            value,
            json!({
                "object": "ANALOG_INPUT:2",
                "property": "PRESENT_VALUE",
                "value": "51",
                "decoded": 51.0
            }),
            "{target}"
        );
    }
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// `whois-router` (#960) lists the router, at its B/IP address, with the
/// network behind it; asking about a network nobody reaches lists none.
#[tokio::test]
async fn whois_router_lists_the_router_and_its_network() {
    let (mut router, mut server, port) = site().await;
    let whois_router = |extra: &'static [&'static str]| async move {
        let bbmd = format!("127.0.0.1:{port}");
        let mut args = vec![
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--foreign-bbmd",
            &bbmd,
            "--json",
            "whois-router",
            "--wait",
            "2",
        ];
        args.extend_from_slice(extra);
        let output = run(args).await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let expected = json!([{ "router": format!("127.0.0.1:{port}"), "networks": [DNET] }]);
    assert_eq!(whois_router(&[]).await, expected);
    assert_eq!(whois_router(&["--dnet", "2"]).await, expected);
    assert_eq!(whois_router(&["--dnet", "77"]).await, json!([]));
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// A routed target that nothing answers fails with the network named, and
/// a command with no routed client call refuses a routed target.
#[tokio::test]
async fn routed_targets_fail_clearly() {
    let (mut router, mut server, port) = site().await;
    let output = read(port, &format!("{DNET}:{}", INSTANCE + 1), "300").await;
    failure(&output, "did not answer a Who-Is on network 2");
    let output = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--json",
        "bdt",
        &format!("{DNET}:{INSTANCE}"),
    ])
    .await;
    failure(&output, "is a routed target");
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[path = "cli_routed/read_services.rs"]
mod read_services;

#[path = "cli_routed/file_fixture.rs"]
mod file_fixture;
#[path = "cli_routed/file_services.rs"]
mod file_services;

#[path = "cli_routed/device_admin.rs"]
mod device_admin;

#[path = "cli_routed/object_alarm.rs"]
mod object_alarm;

#[path = "cli_routed/discovery.rs"]
mod discovery;
