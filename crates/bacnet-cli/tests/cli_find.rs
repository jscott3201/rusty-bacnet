//! `bacnet find` (#960): the Who-Has goes to a loopback BBMD as a foreign
//! device's broadcast, the BBMD's own device answers, and its I-Have comes
//! back forwarded and is printed.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_types::enums::EngineeringUnits;
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};
use support::run;

const INSTANCE: u32 = 3301;

async fn bbmd() -> BACnetServer<BipTransport> {
    let mut db = lab_database(INSTANCE);
    db.add(Box::new(
        AnalogInputObject::new(7, "Supply Air Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap(),
    ))
    .unwrap();
    BACnetServer::bip_builder()
        .device(lab_identity(INSTANCE))
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap()
}

/// `find` for `name`, given as the positional NAME, or with `--name`
/// when `flag` is set (#1872).
async fn find(server: &BACnetServer<BipTransport>, name: &str, flag: bool) -> Value {
    let mac = server.local_mac();
    let bbmd = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    let mut args = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--foreign-bbmd",
        &bbmd,
        "--json",
        "find",
    ];
    if flag {
        args.push("--name");
    }
    args.extend([name, "--wait", "2"]);
    let output = run(args).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn find_prints_the_device_and_object_that_answer() {
    let mut server = bbmd().await;
    assert_eq!(
        find(&server, "Supply Air Temp", false).await,
        json!([{ "device": INSTANCE, "object": "ANALOG_INPUT:7", "name": "Supply Air Temp" }])
    );
    // Nothing has this name: an empty array once the wait is over.
    assert_eq!(find(&server, "Return Air Temp", true).await, json!([]));
    server.stop().await.unwrap();
}
