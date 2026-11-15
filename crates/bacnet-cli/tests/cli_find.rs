//! `bacnet find` (#960): the Who-Has goes to a loopback BBMD as a foreign
//! device's broadcast, the BBMD's own device answers, and its I-Have comes
//! back forwarded and is printed. A miss on this network alone says so
//! (#2596).
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
/// when `flag` is set (#1872), with `more` flags: its JSON, and what it
/// said on stderr.
async fn find(
    server: &BACnetServer<BipTransport>,
    name: &str,
    flag: bool,
    more: &[&str],
) -> (Value, String) {
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
    args.extend(more);
    let output = run(args).await;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    (serde_json::from_slice(&output.stdout).unwrap(), stderr)
}

/// What `find` says on stderr when a search of this network alone found
/// nothing (#2596).
const LOCAL_MISS: &str = "only this network was asked";

/// With no scope flag the Who-Has is a local broadcast, which the BBMD
/// broadcasts on its own network, where its device hears it and answers.
#[tokio::test]
async fn find_prints_the_device_and_object_that_answer() {
    let mut server = bbmd().await;
    let (found, said) = find(&server, "Supply Air Temp", false, &[]).await;
    let mac = server.local_mac();
    let address = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    assert_eq!(
        found,
        json!([{ "device": INSTANCE, "object": "ANALOG_INPUT:7", "name": "Supply Air Temp",
                 "address": address }])
    );
    assert!(!said.contains(LOCAL_MISS), "{said}");
    server.stop().await.unwrap();
}

/// Nothing has this name: an empty array once the wait is over, and exit
/// 0 as for an empty discovery. Asked on this network alone, stderr says
/// so and names --dnet and --global, while the JSON on stdout stays data;
/// asked on every network, it doesn't.
#[tokio::test]
async fn a_miss_on_this_network_alone_says_how_to_widen_it() {
    let mut server = bbmd().await;
    let (found, said) = find(&server, "Return Air Temp", true, &[]).await;
    assert_eq!(found, json!([]));
    assert!(said.contains(LOCAL_MISS), "{said}");
    assert!(
        said.contains("--dnet") && said.contains("--global"),
        "{said}"
    );
    let (found, said) = find(&server, "Return Air Temp", false, &["--global"]).await;
    assert_eq!(found, json!([]));
    assert!(!said.contains(LOCAL_MISS), "{said}");
    server.stop().await.unwrap();
}
