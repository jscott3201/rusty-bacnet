//! The device list paged by row (#2116, #2589): `bacnet://devices` names the
//! next page in `next`, and `bacnet://devices/after/{instance}/{network}/{mac}`
//! holds it. A row is an instance at an address, and several addresses can
//! claim one instance.

use std::time::Duration;

use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_types::enums::Segmentation;
use serde_json::{Value, json};

use super::{ALIASES, LAB, Lab};
use crate::Policy;
use crate::listen::MAX_DEVICE_PAGES;
use crate::policy::Caps;

const UPDATED: &str = "notifications/resources/updated";

/// The lab's devices: the routed 300 to 304, the lab device and its aliases.
fn lab_devices() -> Vec<u64> {
    (300..305)
        .chain(LAB..LAB + 1)
        .chain(ALIASES)
        .map(u64::from)
        .collect()
}

/// A policy whose device page holds `page` devices.
fn paged(page: usize) -> Policy {
    Policy {
        caps: Caps {
            devices: page,
            ..Caps::default()
        },
        ..Policy::default()
    }
}

/// The instances on one page.
fn instances(body: &Value) -> Vec<u64> {
    body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["instance"].as_u64().unwrap())
        .collect()
}

/// Read `uri` and every page its `next` leads to: each page's instances.
async fn follow(lab: &mut Lab, uri: &str) -> Vec<Vec<u64>> {
    let mut pages = Vec::new();
    let mut uri = uri.to_string();
    loop {
        let body = lab.agent.read(&uri).await.unwrap();
        assert_eq!(body["uri"], json!(uri));
        pages.push(instances(&body));
        match body.get("next") {
            Some(next) => {
                assert_eq!(body["truncated"], json!(true), "{body}");
                uri = next.as_str().unwrap().to_string();
            }
            None => {
                assert_eq!(body["truncated"], json!(false), "{body}");
                return pages;
            }
        }
    }
}

/// Following `next` from the first page lists every device once, in
/// instance order, whatever the page size; the last page has no `next`,
/// even when it is exactly full.
#[tokio::test(start_paused = true)]
async fn next_leads_through_every_device_once() {
    let all = lab_devices();
    for (page, sizes) in [
        (3, vec![3, 3, 3, 1]),
        (5, vec![5, 5]),
        (10, vec![10]),
        (200, vec![10]),
    ] {
        let mut lab = Lab::start(paged(page)).await;
        let first = lab.agent.read("bacnet://devices").await.unwrap();
        assert_eq!(first["total"], json!(10), "{first}");
        if page == 3 {
            assert_eq!(first["next"], json!("bacnet://devices/after/302/2001/0c"));
            let second = lab
                .agent
                .read("bacnet://devices/after/302/2001/0c")
                .await
                .unwrap();
            assert_eq!(instances(&second), [303, 304, 1201]);
            assert_eq!(second["total"], json!(10), "the whole list's count");
            assert_eq!(
                second["next"],
                json!("bacnet://devices/after/1201/0/0a000002bac0")
            );
        }
        let pages = follow(&mut lab, "bacnet://devices").await;
        let lengths: Vec<usize> = pages.iter().map(Vec::len).collect();
        assert_eq!(lengths, sizes, "page {page}");
        assert_eq!(pages.concat(), all, "page {page}");
        // A cursor need not be a listed instance, and one past the end
        // is an empty last page.
        let some = lab
            .agent
            .read("bacnet://devices/after/1000/0/00")
            .await
            .unwrap();
        assert_eq!(instances(&some)[0], u64::from(LAB));
        let none = lab
            .agent
            .read("bacnet://devices/after/4194303/65535/00")
            .await
            .unwrap();
        assert_eq!(none["devices"], json!([]));
        assert_eq!(none["truncated"], json!(false));
        assert!(none.get("next").is_none(), "{none}");
        lab.stop().await;
    }
}

/// Devices found between two page reads repeat and skip nothing listed:
/// one above the cursor lands on a later page, one below only in `total`.
#[tokio::test(start_paused = true)]
async fn a_device_found_between_pages_shifts_nothing() {
    let mut lab = Lab::start(paged(3)).await;
    let first = lab.agent.read("bacnet://devices").await.unwrap();
    assert_eq!(instances(&first), [300, 301, 302]);
    for (instance, host) in [(100, 100), (1000, 101)] {
        let mac = [10, 0, 0, host, 0xBA, 0xC0];
        lab.low.add_device(instance, &mac).await.unwrap();
    }
    let next = first["next"].as_str().unwrap().to_string();
    let rest = follow(&mut lab, &next).await;
    assert_eq!(rest[0], [303, 304, 1000]);
    let mut seen = instances(&first);
    seen.extend(rest.concat());
    let mut expected = lab_devices();
    expected.insert(5, 1000);
    assert_eq!(seen, expected, "each listed once, 100 behind the cursor");
    let last = lab
        .agent
        .read("bacnet://devices/after/1204/0/00")
        .await
        .unwrap();
    assert_eq!(last["total"], json!(12), "{last}");
    lab.stop().await;
}

/// A bad cursor is refused as any bad URI is, with nothing sent.
#[tokio::test(start_paused = true)]
async fn a_bad_cursor_is_refused_with_nothing_sent() {
    let mut lab = Lab::start(paged(3)).await;
    let before = lab.sends().len();
    for bad in [
        "bacnet://devices/after/abc",
        "bacnet://devices/after/5",
        "bacnet://devices/after/5/0",
        "bacnet://devices/after/4194304/0/00",
        "bacnet://devices/after/+1/0/00",
        "bacnet://devices/after//0/00",
        "bacnet://devices/after/1/65536/00",
        "bacnet://devices/after/1/-1/00",
        "bacnet://devices/after/1/x/00",
        "bacnet://devices/after/1//00",
        "bacnet://devices/after/1/0/0",
        "bacnet://devices/after/1/0/zz",
        "bacnet://devices/after/1/0/0a0b0c0d0e0f0a0b0c0d0e0f0a0b0c0d0e0f00",
        "bacnet://devices/after/1/0/00/",
        "bacnet://devices/after/1/0/00/9",
    ] {
        let error = lab.agent.read(bad).await.unwrap_err();
        assert_eq!(error["code"], json!(-32602), "{bad}: {error}");
        assert_eq!(error["data"]["error"], json!("invalid-argument"), "{bad}");
        assert_eq!(error["data"]["field"], json!("uri"), "{bad}");
    }
    assert_eq!(lab.sends().len(), before);
    lab.stop().await;
}

/// A device's row as `(instance, address)`.
fn rows(body: &Value) -> Vec<(u64, String)> {
    body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["instance"].as_u64().unwrap(),
                row["address"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Every page from `uri` on, each page's rows.
async fn follow_rows(lab: &mut Lab, uri: &str) -> Vec<Vec<(u64, String)>> {
    let mut pages = Vec::new();
    let mut uri = uri.to_string();
    loop {
        let body = lab.agent.read(&uri).await.unwrap();
        pages.push(rows(&body));
        match body.get("next") {
            Some(next) => uri = next.as_str().unwrap().to_string(),
            None => return pages,
        }
    }
}

/// Instance 1000 answers from `claimants` addresses, between the routed
/// 300 to 304 and the lab device (#2589).
async fn claim_1000(lab: &Lab, claimants: u8) {
    for host in 0..claimants {
        let mac = [10, 0, 0, 100 + host, 0xBA, 0xC0];
        lab.low.add_device(1000, &mac).await.unwrap();
    }
}

/// A page boundary between claimants of one instance loses none of them:
/// every row appears once across the pages, in the order an unpaged list
/// holds them, whichever claimant the boundary follows (#2589).
#[tokio::test(start_paused = true)]
async fn a_shared_instance_on_a_page_boundary_is_listed_whole() {
    // Rows before 1000: the five routed devices; the claimants then follow.
    for (claimants, page, first_page_claimants) in [(2, 6, 1), (3, 6, 1), (3, 7, 2), (3, 8, 3)] {
        let mut lab = Lab::start(paged(200)).await;
        claim_1000(&lab, claimants).await;
        let whole = follow_rows(&mut lab, "bacnet://devices").await;
        assert_eq!(whole.len(), 1, "one page");
        let whole = whole.concat();
        let ones: Vec<&(u64, String)> = whole.iter().filter(|(i, _)| *i == 1000).collect();
        assert_eq!(ones.len(), usize::from(claimants), "{whole:?}");
        lab.stop().await;

        let mut lab = Lab::start(paged(page)).await;
        claim_1000(&lab, claimants).await;
        let pages = follow_rows(&mut lab, "bacnet://devices").await;
        let on_first = pages[0].iter().filter(|(i, _)| *i == 1000).count();
        assert_eq!(
            on_first, first_page_claimants,
            "the boundary falls inside 1000"
        );
        if usize::from(claimants) > first_page_claimants {
            assert_eq!(pages[1][0].0, 1000, "the next page starts on a claimant");
        }
        assert_eq!(pages.concat(), whole, "{claimants} claimants, page {page}");
        lab.stop().await;
    }
}

/// A row as `(instance, network, address)`.
fn rows_on(body: &Value) -> Vec<(u64, u64, String)> {
    body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["instance"].as_u64().unwrap(),
                row["network"].as_u64().unwrap(),
                row["address"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Instance 1000 at one MAC, local and routed on networks 2002 and 2003
/// (#2589): three rows that differ only in network.
async fn claim_1000_on_networks(lab: &Lab) {
    let mac = [10, 0, 0, 100, 0xBA, 0xC0];
    lab.low.add_device(1000, &mac).await.unwrap();
    for remote_network in [2002, 2003] {
        lab.low
            .add_routed_device(RoutedDeviceConfig {
                instance: 1000,
                router_mac: super::LAB_MAC.to_vec(),
                remote_network,
                remote_mac: mac.to_vec(),
                max_apdu_length: 480,
                segmentation_supported: Segmentation::NO_SEGMENTATION,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
    }
}

/// Rows of one instance and MAC that differ only in network are told apart
/// by the cursor: a boundary between the local row and a routed one, or
/// between two networks, loses and repeats none.
#[tokio::test(start_paused = true)]
async fn rows_differing_only_in_network_are_listed_whole() {
    let mut lab = Lab::start(paged(200)).await;
    claim_1000_on_networks(&lab).await;
    let whole = rows_on(&lab.agent.read("bacnet://devices").await.unwrap());
    let of_1000: Vec<u64> = whole
        .iter()
        .filter(|(i, ..)| *i == 1000)
        .map(|(_, network, _)| *network)
        .collect();
    assert_eq!(of_1000, [0, 2002, 2003], "{whole:?}");
    lab.stop().await;
    // Five routed rows come first: page 6 ends on the local row, 7 between
    // the networks.
    for page in [6, 7] {
        let mut lab = Lab::start(paged(page)).await;
        claim_1000_on_networks(&lab).await;
        let mut seen = Vec::new();
        let mut uri = "bacnet://devices".to_string();
        let mut pages = 0;
        loop {
            let body = lab.agent.read(&uri).await.unwrap();
            seen.extend(rows_on(&body));
            pages += 1;
            match body.get("next") {
                Some(next) => uri = next.as_str().unwrap().to_string(),
                None => break,
            }
        }
        assert!(pages > 1);
        assert_eq!(seen, whole, "page {page}");
        lab.stop().await;
    }
}

/// The cursor's row can leave the table between pages (its address
/// announces another instance): the next page still starts at the row after
/// where it stood.
#[tokio::test(start_paused = true)]
async fn the_cursor_row_leaving_the_table_loses_nothing() {
    let mut lab = Lab::start(paged(3)).await;
    let first = lab.agent.read("bacnet://devices").await.unwrap();
    assert_eq!(instances(&first), [300, 301, 302]);
    let next = first["next"].as_str().unwrap().to_string();
    // Network 2001, MAC 0x0c was 302; it now announces 777.
    lab.low
        .add_routed_device(RoutedDeviceConfig {
            instance: 777,
            router_mac: super::LAB_MAC.to_vec(),
            remote_network: 2001,
            remote_mac: vec![12],
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    let rest = follow(&mut lab, &next).await.concat();
    let mut expected: Vec<u64> = lab_devices().split_off(3);
    expected.insert(2, 777);
    assert_eq!(rest, expected);
    lab.stop().await;
}

/// The template is listed beside the others, named from identifiers.
#[tokio::test(start_paused = true)]
async fn the_page_template_is_listed() {
    let mut lab = Lab::start(Policy::default()).await;
    let listed = lab
        .agent
        .modern("resources/templates/list", json!({}))
        .await;
    let templates = listed["result"]["resourceTemplates"].as_array().unwrap();
    let page = templates
        .iter()
        .find(|t| t["uriTemplate"] == json!("bacnet://devices/after/{instance}/{network}/{mac}"))
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(page["name"], json!("devices-after"));
    assert_eq!(page["mimeType"], json!("application/json"));
    lab.stop().await;
}

/// A passive server lists no template and refuses a page, as it refuses
/// the first one.
#[tokio::test(start_paused = true)]
async fn a_passive_server_offers_no_page() {
    use bacnet_transport::bip::BipTransport;
    let policy = Policy {
        passive: true,
        ..Policy::default()
    };
    let (mut agent, serving) = super::Agent::connect(
        crate::McpServer::<BipTransport>::new(None, policy, "tester").unwrap(),
    );
    let templates = agent.modern("resources/templates/list", json!({})).await;
    assert_eq!(templates["result"]["resourceTemplates"], json!([]));
    for uri in ["bacnet://devices", "bacnet://devices/after/5/0/00"] {
        let error = agent.read(uri).await.unwrap_err();
        assert_eq!(error["data"]["reason"], json!("passive"), "{uri}: {error}");
    }
    let (_, accepted) = agent
        .listen(&["bacnet://devices/after/5/0/00", "bacnet://session"])
        .await;
    assert_eq!(accepted, ["bacnet://session"]);
    drop(agent);
    serving.await.unwrap();
}

/// A stream asking for more device-list pages than it may follow is
/// honoured for the first [`MAX_DEVICE_PAGES`] (counted by spelling), and
/// one change to the list announces only those.
#[tokio::test(start_paused = true)]
async fn a_stream_follows_at_most_the_page_cap() {
    let mut lab = Lab::start(paged(3)).await;
    let mut asked = vec![
        "bacnet://session".to_string(),
        "bacnet://devices".to_string(),
    ];
    asked.extend((0..MAX_DEVICE_PAGES + 4).map(|n| format!("bacnet://devices/after/{n}/0/00")));
    // Another spelling of a page held counts as one more.
    asked.push("bacnet://devices/after/00/0/00".to_string());
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    let (_, accepted) = lab.agent.listen(&asked).await;
    let expected: Vec<&str> = asked[..MAX_DEVICE_PAGES + 1].to_vec();
    assert_eq!(accepted, expected);
    lab.low.clear_devices().await;
    lab.server.broadcast_i_am().await.unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    let mut told: Vec<&str> = updates
        .iter()
        .map(|u| u["params"]["uri"].as_str().unwrap())
        .collect();
    told.sort_unstable();
    let mut pages = expected[1..].to_vec();
    pages.sort_unstable();
    assert_eq!(told, pages, "one update per followed page");
    lab.stop().await;
}

/// A stream following the first page and a later one is told of both
/// when devices arrive, once each within the second.
#[tokio::test(start_paused = true)]
async fn every_followed_page_is_announced_when_a_device_arrives() {
    let mut lab = Lab::start(paged(3)).await;
    let uris = ["bacnet://devices", "bacnet://devices/after/302/2001/0c"];
    let (_, accepted) = lab.agent.listen(&uris).await;
    assert_eq!(accepted, uris);
    lab.low.clear_devices().await;
    for _ in 0..3 {
        lab.server.broadcast_i_am().await.unwrap();
    }
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    let mut told: Vec<&str> = updates
        .iter()
        .map(|u| u["params"]["uri"].as_str().unwrap())
        .collect();
    told.sort_unstable();
    assert_eq!(told, uris, "{updates:?}");
    lab.stop().await;
}
