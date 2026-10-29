//! `bacnet_network_map` with `bbmd: true` against real BBMDs on loopback
//! (#2102): the survey follows the tables from the registration's BBMD or a
//! known device's address, reports what disagrees, stays within reach and
//! sends nothing but table reads.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::bbmd::{address_of, mac};
use bacnet_ops::engine::Engine;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{
    BipMode, BipTransport, ForeignDeviceConfig, ForeignRegistrationOutcome,
};
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer};
use serde_json::{Value, json};
use tokio::net::UdpSocket;

use super::Agent;
use crate::{McpServer, Policy};

const ONES: [u8; 4] = [255; 4];
const SUBNET: [u8; 4] = [255, 255, 255, 0];

/// BVLC functions: the two reads, and what must never go out.
const READ_BDT: u8 = 0x02;
const READ_FDT: u8 = 0x06;

fn policy() -> Policy {
    Policy {
        broad_discovery: true,
        ..Policy::default()
    }
}

/// A BBMD on loopback, its BDT set once it knows its own port.
async fn bbmd(foreign: bool) -> BipTransport {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    bbmd.enable_bbmd(Vec::new());
    if foreign {
        bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    }
    let _rx = bbmd.start().await.unwrap();
    bbmd
}

fn at(transport: &BipTransport) -> SocketAddrV4 {
    address_of(transport.local_mac()).unwrap()
}

fn set_bdt(bbmd: &BipTransport, rows: &[(SocketAddrV4, [u8; 4])]) {
    let rows = rows
        .iter()
        .map(|(address, mask)| BdtEntry {
            ip: address.ip().octets(),
            port: address.port(),
            broadcast_mask: *mask,
        })
        .collect();
    bbmd.bbmd_state()
        .unwrap()
        .lock()
        .unwrap()
        .set_bdt(rows)
        .unwrap();
}

/// A BBMD of another make on loopback: it answers Read-BDT with itself,
/// `peer` and `extra`, which no stack of ours would hold, and Read-FDT with
/// an empty table.
async fn hostile_bbmd(peer: SocketAddrV4, extra: &[SocketAddrV4]) -> SocketAddrV4 {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let std::net::SocketAddr::V4(own) = socket.local_addr().unwrap() else {
        unreachable!("bound to IPv4");
    };
    let mut bdt = Vec::new();
    for address in [own, peer].iter().chain(extra) {
        bdt.extend_from_slice(&address.ip().octets());
        bdt.extend_from_slice(&address.port().to_be_bytes());
        bdt.extend_from_slice(&ONES);
    }
    tokio::spawn(async move {
        let mut buf = [0; 64];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let reply = match buf[..n] {
                [0x81, READ_BDT, ..] => {
                    let len = u16::try_from(4 + bdt.len()).unwrap().to_be_bytes();
                    [&[0x81, 0x03, len[0], len[1]][..], &bdt].concat()
                }
                [0x81, READ_FDT, ..] => vec![0x81, 0x07, 0x00, 0x04],
                _ => continue,
            };
            let _ = socket.send_to(&reply, from).await;
        }
    });
    own
}

/// A server over `client`, under `policy`, and an agent connected to it.
fn serve(client: &Arc<BACnetClient<BipTransport>>, policy: Policy) -> Agent {
    let engine = Arc::new(Engine::new(
        Arc::clone(client),
        Arc::new(policy.request_limits()),
    ));
    let mcp = McpServer::new(Some(engine), policy, "tester").unwrap();
    Agent::connect(mcp).0
}

/// Every frame the client sent since `tap` subscribed: (BVLC function,
/// destination).
fn sent(tap: &mut FrameTapReceiver) -> Vec<(u8, SocketAddrV4)> {
    let mut out = Vec::new();
    while let Some(frame) = tap.try_recv() {
        if frame.direction != TapDirection::Outbound {
            continue;
        }
        let TapPeer::Udp(std::net::SocketAddr::V4(to)) = frame.peer else {
            panic!("not a B/IP peer: {:?}", frame.peer);
        };
        out.push((frame.bytes[1], to));
    }
    out.sort_unstable();
    out
}

fn problems(bbmd: &Value) -> Vec<(String, String)> {
    bbmd["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let who = f["bbmd"].as_str().or(f["from"].as_str()).unwrap_or("");
            (f["problem"].as_str().unwrap().to_string(), who.to_string())
        })
        .collect()
}

/// The lab of #2102: the server registers with BBMD A, whose BDT lists B,
/// a node that is no BBMD and an address that never answers; B's BDT lacks
/// A. One call reads them all (the silent one costing one timeout, not
/// stalling the rest) and names the one-way entry, the refused read, the
/// silence and a foreign device near expiry, sending only table reads.
#[tokio::test]
async fn a_missing_peer_entry_is_reported_and_only_reads_go_out() {
    let a = bbmd(true).await;
    let b = bbmd(false).await;
    let mut node = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _node_rx = node.start().await.unwrap();
    let silent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let std::net::SocketAddr::V4(silent_at) = silent.local_addr().unwrap() else {
        unreachable!("bound to IPv4");
    };
    let (a_at, b_at, node_at) = (at(&a), at(&b), at(&node));
    // A BBMD of another make whose table lists an unresolved host name
    // (all zeros), the limited broadcast and a multicast group besides A
    // and itself: listed, never read.
    let hostile_at = hostile_bbmd(
        a_at,
        &[
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
            SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808),
            SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 1), 47808),
        ],
    )
    .await;
    set_bdt(
        &a,
        &[
            (a_at, ONES),
            (b_at, ONES),
            (node_at, ONES),
            (silent_at, ONES),
            (hostile_at, ONES),
        ],
    );
    set_bdt(&b, &[(b_at, ONES)]);
    // A foreign device whose registration lapsed 25 s ago: 5 s of grace
    // left.
    let lapsed = SocketAddrV4::new(Ipv4Addr::new(10, 9, 9, 9), 47808);
    a.bbmd_state()
        .unwrap()
        .lock()
        .unwrap()
        .register_foreign_device_at(
            lapsed.ip().octets(),
            lapsed.port(),
            10,
            std::time::Instant::now() - Duration::from_secs(35),
        );
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: *a_at.ip(),
            bbmd_port: a_at.port(),
            ttl: 600,
            renewal_interval: Some(Duration::from_secs(300)),
        }))
        .build()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while client
            .transport()
            .bvlc_client_snapshot()
            .foreign_registration
            .last_outcome
            != Some(ForeignRegistrationOutcome::Accepted)
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("A accepts the registration");
    let client = Arc::new(client);
    let mut tap = client.transport().frame_tap().subscribe();
    let mut agent = serve(&client, policy());
    agent.initialize().await;

    let result = agent
        .call("bacnet_network_map", json!({ "bbmd": true }))
        .await;
    assert_eq!(result["isError"], json!(false), "{result}");
    let body = &result["structuredContent"];
    let report = &body["bbmd"];
    assert_eq!(report["seeds"], json!([a_at.to_string()]));
    assert_eq!(report["verdict"], json!("inconsistent"), "{report}");
    let mut found = problems(report);
    found.sort();
    let mut expected = vec![
        ("one-way-entry".to_string(), a_at.to_string()),
        ("read-not-supported".to_string(), node_at.to_string()),
        ("no-answer".to_string(), silent_at.to_string()),
        ("foreign-device-expiring".to_string(), a_at.to_string()),
        ("not-followed".to_string(), hostile_at.to_string()),
        ("not-followed".to_string(), hostile_at.to_string()),
        ("not-followed".to_string(), hostile_at.to_string()),
    ];
    expected.sort();
    assert_eq!(found, expected, "{report}");
    let one_way = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["problem"] == json!("one-way-entry"))
        .unwrap();
    assert_eq!(one_way["to"], json!(b_at.to_string()));
    let expiring = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["problem"] == json!("foreign-device-expiring"))
        .unwrap();
    assert_eq!(expiring["device"], json!(lapsed.to_string()));
    assert!(expiring["remaining_s"].as_u64().unwrap() < 30, "{expiring}");
    // A's FDT also holds this server's own registration, alive.
    let reasons: Vec<&Value> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["problem"] == json!("not-followed"))
        .map(|f| &f["reason"])
        .collect();
    assert_eq!(
        reasons,
        [
            &json!("unresolved"),
            &json!("broadcast"),
            &json!("multicast")
        ]
    );
    let a_fdt = &report["bbmds"][0]["fdt"];
    assert_eq!(a_fdt["count"], json!(2), "{a_fdt}");
    // Five BDT reads and three FDT reads (the BBMDs that gave a BDT), and
    // one timeout for the silent address, waited out beside the others.
    let cost = &body["cost"];
    assert_eq!(cost["requests"], json!(8), "{cost}");
    assert_eq!(cost["timeouts"], json!(1), "{cost}");
    assert!(cost["elapsed_ms"].as_u64().unwrap() < 9_000, "{cost}");
    // The wire: table reads only, no write, registration or delete.
    let mut reads = vec![
        (READ_BDT, a_at),
        (READ_BDT, b_at),
        (READ_BDT, node_at),
        (READ_BDT, silent_at),
        (READ_BDT, hostile_at),
        (READ_FDT, a_at),
        (READ_FDT, b_at),
        (READ_FDT, hostile_at),
    ];
    reads.sort_unstable();
    assert_eq!(sent(&mut tap), reads);
    super::schema::check_output(&mut agent, "bacnet_network_map", body).await;

    // A second survey within the minute is refused, and sends nothing.
    let again = agent
        .call("bacnet_network_map", json!({ "bbmd": true }))
        .await;
    assert_eq!(again["structuredContent"]["reason"], json!("rate-limited"));
    assert!(sent(&mut tap).is_empty());
}

/// A survey may start from a known device's B/IP address, and only a known
/// one; a mask one BBMD gives its peer and the peer gives itself
/// differently is named with both masks.
#[tokio::test]
async fn a_known_address_seeds_the_survey_and_mismatched_masks_are_named() {
    let a = bbmd(false).await;
    let b = bbmd(false).await;
    let (a_at, b_at) = (at(&a), at(&b));
    set_bdt(&a, &[(a_at, ONES), (b_at, ONES)]);
    set_bdt(&b, &[(a_at, ONES), (b_at, SUBNET)]);
    let client = Arc::new(
        BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .build()
            .await
            .unwrap(),
    );
    let mut tap = client.transport().frame_tap().subscribe();

    // Without broad discovery the survey is refused.
    let mut narrow = serve(&client, Policy::default());
    narrow.initialize().await;
    let refused = narrow
        .call("bacnet_network_map", json!({ "bbmd": true }))
        .await;
    assert_eq!(
        refused["structuredContent"]["reason"],
        json!("broad-discovery")
    );
    // Nor outside reach: the local network left out of [mcp] networks.
    let mut remote = serve(
        &client,
        Policy {
            networks: Some([5].into()),
            ..policy()
        },
    );
    remote.initialize().await;
    let refused = remote
        .call("bacnet_network_map", json!({ "bbmd": true }))
        .await;
    assert_eq!(
        refused["structuredContent"]["reason"],
        json!("network-not-allowed")
    );

    let mut agent = serve(&client, policy());
    agent.initialize().await;
    // No registration and nothing named: no BBMD to start from.
    let none = agent
        .call("bacnet_network_map", json!({ "bbmd": true }))
        .await;
    assert_eq!(none["structuredContent"]["reason"], json!("no-bbmd-known"));
    // An address the server doesn't know is no seed: nothing is scanned.
    let named = json!({ "bbmd": true, "bbmds": [a_at.to_string()] });
    let unknown = agent.call("bacnet_network_map", named.clone()).await;
    assert_eq!(
        unknown["structuredContent"]["reason"],
        json!("address-not-known"),
        "{unknown}"
    );
    // A named seed passes the same check as a listed peer: no broadcast and
    // not our own address, refused as an argument before anything is sent.
    let own = address_of(client.transport().local_mac()).unwrap();
    for (seed, reason) in [
        ("255.255.255.255:47808".to_string(), "broadcast"),
        (own.to_string(), "own-address"),
    ] {
        let refused = agent
            .call(
                "bacnet_network_map",
                json!({ "bbmd": true, "bbmds": [seed] }),
            )
            .await;
        let body = &refused["structuredContent"];
        assert_eq!(body["error"], json!("invalid-argument"), "{refused}");
        assert!(
            body["message"].as_str().unwrap().contains(reason),
            "{refused}"
        );
    }
    // A device the policy leaves out doesn't make its address known.
    client.add_device(77, &mac(a_at)).await.unwrap();
    let mut fenced = serve(
        &client,
        Policy {
            devices: Some(vec![(1, 2)]),
            ..policy()
        },
    );
    fenced.initialize().await;
    let outside = fenced.call("bacnet_network_map", named.clone()).await;
    assert_eq!(
        outside["structuredContent"]["reason"],
        json!("address-not-known")
    );
    assert!(sent(&mut tap).is_empty(), "every refusal sends nothing");

    // Once A is a discovered device, its address seeds the survey.
    let result = agent.call("bacnet_network_map", named).await;
    assert_eq!(result["isError"], json!(false), "{result}");
    let report = &result["structuredContent"]["bbmd"];
    assert_eq!(report["verdict"], json!("inconsistent"), "{report}");
    assert_eq!(
        problems(report),
        [("mismatched-masks".to_string(), b_at.to_string())]
    );
    assert_eq!(
        report["findings"][0]["masks"],
        json!(["255.255.255.0", "255.255.255.255"])
    );
    assert_eq!(
        report["bbmds"][1]["bdt"]["entries"][1],
        json!({ "address": b_at.to_string(), "mask": "255.255.255.0",
                "forward_to": "127.0.0.255" })
    );
    let wire = sent(&mut tap);
    assert!(
        wire.iter().all(|(f, _)| *f == READ_BDT || *f == READ_FDT),
        "{wire:?}"
    );
    assert_eq!(wire.len(), 4, "{wire:?}");
    // bbmds without bbmd is an argument error.
    let lone = agent
        .call("bacnet_network_map", json!({ "bbmds": [a_at.to_string()] }))
        .await;
    assert_eq!(
        lone["structuredContent"]["error"],
        json!("invalid-argument")
    );
}

/// Passive mode doesn't list the map, so a survey can't even be asked for.
#[tokio::test(start_paused = true)]
async fn a_passive_server_refuses_the_survey() {
    let policy = Policy {
        passive: true,
        ..policy()
    };
    let (mut agent, serving) =
        Agent::connect(McpServer::<BipTransport>::new(None, policy, "tester").unwrap());
    agent.initialize().await;
    let reply = agent
        .request(
            "tools/call",
            json!({ "name": "bacnet_network_map", "arguments": { "bbmd": true } }),
        )
        .await;
    assert_eq!(reply["error"]["code"], json!(-32602), "{reply}");
    drop(agent);
    serving.await.unwrap();
}
