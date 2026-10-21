//! `--foreign-bbmd` (#938): one-shot commands register with a loopback BBMD
//! first, then reach its network through it.
#[allow(dead_code)]
mod support;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use serde_json::Value;
use std::net::Ipv4Addr;
use support::{failure, run};

/// A BBMD on loopback, whose own device is `instance`; it takes foreign
/// devices when `accept` is set.
async fn bbmd(instance: u32, accept: bool) -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(instance, 0)
                .expect("valid Device identity")
                .with_name(format!("bbmd-{instance}")),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    BACnetServer::bip_builder()
        .device(bacnet_objects::DeviceIdentity::new(instance, 0).unwrap())
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: accept.then(ForeignDevicePolicy::default),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap()
}

fn address(server: &BACnetServer<BipTransport>) -> String {
    let mac = server.local_mac();
    format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]))
}

/// The Who-Is goes to the BBMD as a foreign device's broadcast; the BBMD's
/// own device answers, and its I-Am comes back forwarded.
#[tokio::test]
async fn discover_reaches_a_device_through_the_bbmd() {
    let mut server = bbmd(3201, true).await;
    let output = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--foreign-bbmd",
        &address(&server),
        "--ttl",
        "60",
        "--renewal",
        "20",
        "--json",
        "discover",
        "3201-3201",
        "--wait",
        "1",
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let devices: Value = serde_json::from_slice(&output.stdout).unwrap();
    let instances: Vec<_> = devices
        .as_array()
        .unwrap()
        .iter()
        .map(|device| device["instance"].as_u64().unwrap())
        .collect();
    assert_eq!(instances, [3201]);
    let fdt = server
        .transport()
        .unwrap()
        .fdt_counters()
        .unwrap()
        .registrations_accepted;
    assert_eq!(fdt, 1);
    server.stop().await.unwrap();
}

/// A refused registration stops a one-shot command before it runs, and the
/// error names the BBMD's result code.
#[tokio::test]
async fn a_refused_registration_fails_the_command() {
    let mut server = bbmd(3202, false).await;
    let output = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--foreign-bbmd",
        &address(&server),
        "--json",
        "discover",
        "--wait",
        "0",
    ])
    .await;
    failure(
        &output,
        "refused the foreign-device registration (REGISTER_FOREIGN_DEVICE_NAK)",
    );
    server.stop().await.unwrap();
}

/// A BBMD that never answers fails the command once the transport gives the
/// attempt up, before the CLI's own backstop.
#[tokio::test]
async fn a_silent_bbmd_fails_the_command() {
    let silent = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let output = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--foreign-bbmd",
        &silent.local_addr().unwrap().to_string(),
        "--json",
        "discover",
        "--wait",
        "0",
    ])
    .await;
    failure(&output, "did not answer the foreign-device registration");
}

#[tokio::test]
async fn the_flags_need_b_ip_and_each_other() {
    let output = run(["--renewal", "20", "discover"]).await;
    failure(&output, "--foreign-bbmd");
    let output = run(["--ipv6", "--foreign-bbmd", "127.0.0.1", "discover"]).await;
    failure(&output, "--foreign-bbmd requires BACnet/IP");
    let output = run(["--foreign-bbmd", "127.0.0.1:0", "discover"]).await;
    failure(&output, "is not a BBMD a device can register with");
}
