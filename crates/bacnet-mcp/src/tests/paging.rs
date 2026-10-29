//! The device list paged by instance (#2116): `bacnet://devices` names the
//! next page in `next`, and `bacnet://devices/after/{instance}` holds it.

use std::time::Duration;

use serde_json::{Value, json};

use super::{ALIASES, LAB, LAB_MAC, Lab};
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
            assert_eq!(first["next"], json!("bacnet://devices/after/302"));
            let second = lab.agent.read("bacnet://devices/after/302").await.unwrap();
            assert_eq!(instances(&second), [303, 304, 1201]);
            assert_eq!(second["total"], json!(10), "the whole list's count");
            assert_eq!(second["next"], json!("bacnet://devices/after/1201"));
        }
        let pages = follow(&mut lab, "bacnet://devices").await;
        let lengths: Vec<usize> = pages.iter().map(Vec::len).collect();
        assert_eq!(lengths, sizes, "page {page}");
        assert_eq!(pages.concat(), all, "page {page}");
        // A cursor need not be a listed instance, and one past the end
        // is an empty last page.
        let some = lab.agent.read("bacnet://devices/after/1000").await.unwrap();
        assert_eq!(instances(&some)[0], u64::from(LAB));
        let none = lab
            .agent
            .read("bacnet://devices/after/4194303")
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
    for instance in [100, 1000] {
        lab.client.add_device(instance, &LAB_MAC).await.unwrap();
    }
    let next = first["next"].as_str().unwrap().to_string();
    let rest = follow(&mut lab, &next).await;
    assert_eq!(rest[0], [303, 304, 1000]);
    let mut seen = instances(&first);
    seen.extend(rest.concat());
    let mut expected = lab_devices();
    expected.insert(5, 1000);
    assert_eq!(seen, expected, "each listed once, 100 behind the cursor");
    let last = lab.agent.read("bacnet://devices/after/1204").await.unwrap();
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
        "bacnet://devices/after/4194304",
        "bacnet://devices/after/+1",
        "bacnet://devices/after/",
        "bacnet://devices/after/1/2",
    ] {
        let error = lab.agent.read(bad).await.unwrap_err();
        assert_eq!(error["code"], json!(-32602), "{bad}: {error}");
        assert_eq!(error["data"]["error"], json!("invalid-argument"), "{bad}");
        assert_eq!(error["data"]["field"], json!("uri"), "{bad}");
    }
    assert_eq!(lab.sends().len(), before);
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
        .find(|t| t["uriTemplate"] == json!("bacnet://devices/after/{instance}"))
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
    for uri in ["bacnet://devices", "bacnet://devices/after/5"] {
        let error = agent.read(uri).await.unwrap_err();
        assert_eq!(error["data"]["reason"], json!("passive"), "{uri}: {error}");
    }
    let (_, accepted) = agent
        .listen(&["bacnet://devices/after/5", "bacnet://session"])
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
    asked.extend((0..MAX_DEVICE_PAGES + 4).map(|n| format!("bacnet://devices/after/{n}")));
    // Another spelling of a page held counts as one more.
    asked.push("bacnet://devices/after/00".to_string());
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    let (_, accepted) = lab.agent.listen(&asked).await;
    let expected: Vec<&str> = asked[..MAX_DEVICE_PAGES + 1].to_vec();
    assert_eq!(accepted, expected);
    lab.client.clear_devices().await;
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
    let uris = ["bacnet://devices", "bacnet://devices/after/302"];
    let (_, accepted) = lab.agent.listen(&uris).await;
    assert_eq!(accepted, uris);
    lab.client.clear_devices().await;
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
