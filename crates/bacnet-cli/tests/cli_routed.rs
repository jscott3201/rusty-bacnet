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

use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::loopback::LoopbackTransport;
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};
use support::{Process, boxed, command, failure, run};

/// The lab device's network and instance.
const DNET: u16 = 2;
const INSTANCE: u32 = 1234;

/// The router, the lab device behind it, and the router's B/IP port, which
/// the OS picks and the started router reports.
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

/// Boxed, as the starts in it are: every test awaits it in its own poll
/// frame, and the router's start runs under it (#2656).
fn site_with_config(
    database: bacnet_objects::database::ObjectDatabase,
    config: bacnet_server::server::ServerConfig,
) -> Pin<Box<impl Future<Output = (BACnetRouter, BACnetServer<LoopbackTransport>, u16)>>> {
    boxed(move || start_site(database, config))
}

async fn start_site(
    database: bacnet_objects::database::ObjectDatabase,
    config: bacnet_server::server::ServerConfig,
) -> (BACnetRouter, BACnetServer<LoopbackTransport>, u16) {
    let config = service_config(config);
    let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
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
    let StartedRouter { router, .. } = boxed(|| BACnetRouter::start(ports, RouterOptions::new()))
        .await
        .expect("the router binds its ports");
    let (_, port) = decode_bip_mac(router.ports()[0].local_mac()).unwrap();
    let server = boxed(|| BACnetServer::start(config, database, station))
        .await
        .unwrap();
    (router, server, port)
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
    let whois_router = |directed: bool, extra: &'static [&'static str]| async move {
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
        if directed {
            args.extend_from_slice(&["--via", &bbmd]);
        }
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
    assert_eq!(whois_router(false, &[]).await, expected);
    assert_eq!(whois_router(true, &[]).await, expected);
    assert_eq!(whois_router(false, &["--dnet", "2"]).await, expected);
    assert_eq!(whois_router(true, &["--dnet", "2"]).await, expected);
    assert_eq!(whois_router(false, &["--dnet", "77"]).await, json!([]));
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// `routing-table` (#2612) lists both of the router's networks, each with
/// its port and no port information, and a station that never answers is an
/// error that says so rather than an empty table.
#[tokio::test]
async fn routing_table_lists_the_routers_networks_and_silence_is_an_error() {
    let (mut router, mut server, port) = site().await;
    let routing_table = |target: String, wait: &'static str| async move {
        let bbmd = format!("127.0.0.1:{port}");
        run([
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--foreign-bbmd",
            &bbmd,
            "--json",
            "routing-table",
            &target,
            "--wait",
            wait,
        ])
        .await
    };
    let output = routing_table(format!("127.0.0.1:{port}"), "2").await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(table["router"], json!(format!("127.0.0.1:{port}")));
    assert_eq!(table["truncated"], json!(false));
    let mut networks: Vec<u64> = table["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            assert_eq!(entry["port_info"], Value::Null, "{entry}");
            assert!(entry["port_id"].is_u64(), "{entry}");
            entry["network"].as_u64().unwrap()
        })
        .collect();
    networks.sort_unstable();
    assert_eq!(networks, [1, u64::from(DNET)]);

    // Nothing listens on the discard port: no table, and the error says
    // the router did not answer.
    let output = routing_table("127.0.0.1:9".into(), "1").await;
    failure(&output, "sent no routing table within 1s");

    // A wait the clock can't hold is refused by the argument parser.
    let output = routing_table("127.0.0.1:9".into(), "99999999999").await;
    failure(&output, "99999999999");
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
