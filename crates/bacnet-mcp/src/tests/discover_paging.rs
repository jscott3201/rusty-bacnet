//! `bacnet_discover` paged by row (#2606): a later page starts after the
//! last row listed (an instance at an address), not at a position in the
//! table, so rows found or lost between two calls repeat and skip none
//! of the others. `bacnet_find_object` pages by instance, which is its
//! row key, as it never searches an instance two addresses claim.

use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_types::enums::Segmentation;
use serde_json::{Value, json};

use super::{LAB_MAC, Lab};
use crate::Policy;
use crate::policy::Caps;

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

/// The structured content of a successful call.
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], json!(false), "{result}");
    &result["structuredContent"]
}

/// One page of the whole table, as known: nothing sent.
async fn page(lab: &mut Lab, cursor: Option<&Value>) -> Value {
    let mut args = json!({ "send": false, "scope": "global" });
    if let Some(cursor) = cursor {
        args["cursor"] = cursor.clone();
    }
    ok(&lab.agent.call("bacnet_discover", args).await).clone()
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

/// The routed station at `mac` on network 2001 (behind the lab device)
/// announces `instance`: the row there now holds it.
async fn routed(lab: &Lab, instance: u32, mac: u8) {
    lab.low
        .add_routed_device(RoutedDeviceConfig {
            instance,
            router_mac: LAB_MAC.to_vec(),
            remote_network: 2001,
            remote_mac: vec![mac],
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
}

/// A device found below the cursor between two pages (a late I-Am to the
/// first page's Who-Is) repeats nothing: the next page starts after 302.
#[tokio::test(start_paused = true)]
async fn a_row_found_below_the_cursor_repeats_nothing() {
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    assert_eq!(instances(&first), [300, 301, 302]);
    lab.low
        .add_device(100, &[10, 0, 0, 100, 0xBA, 0xC0])
        .await
        .unwrap();
    let second = page(&mut lab, Some(&first["next_cursor"])).await;
    assert_eq!(instances(&second), [303, 304, 1201], "{second}");
    assert_eq!(second["total"], json!(11));
    lab.stop().await;
}

/// A row lost below the cursor between two pages (its station announces
/// another instance) skips nothing: the next page still starts at 303.
#[tokio::test(start_paused = true)]
async fn a_row_lost_below_the_cursor_skips_nothing() {
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    assert_eq!(instances(&first), [300, 301, 302]);
    // Network 2001, MAC 0x0a was 300; it now announces 5000.
    routed(&lab, 5000, 10).await;
    let second = page(&mut lab, Some(&first["next_cursor"])).await;
    assert_eq!(instances(&second), [303, 304, 1201], "{second}");
    lab.stop().await;
}

/// A local station at 10.0.0.`host` announces `instance`.
async fn local(lab: &Lab, instance: u32, host: u8) {
    lab.low
        .add_device(instance, &[10, 0, 0, host, 0xBA, 0xC0])
        .await
        .unwrap();
}

/// A row as `(instance, network, address)`.
fn rows(body: &Value) -> Vec<(u64, u64, String)> {
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

/// Every page from the first, following its cursors: each row listed.
async fn walk(lab: &mut Lab, first: &Value) -> Vec<(u64, u64, String)> {
    let mut seen = rows(first);
    let mut cursor = first["next_cursor"].clone();
    while !cursor.is_null() {
        let body = page(lab, Some(&cursor)).await;
        seen.extend(rows(&body));
        cursor = body["next_cursor"].clone();
    }
    seen
}

/// A walk of six pages with rows found below and above the cursor, and
/// lost below and above it, at every boundary: each row there for the
/// whole walk comes once, in order; each row found ahead comes once; no
/// row found behind, and no row lost ahead, comes at all.
#[tokio::test(start_paused = true)]
async fn a_walk_with_rows_found_and_lost_at_each_boundary_lists_each_once() {
    let mut lab = Lab::start(paged(2)).await;
    lab.agent.initialize().await;
    let mut seen = Vec::new();
    let mut cursor = None;
    for boundary in 0.. {
        let body = page(&mut lab, cursor.as_ref()).await;
        seen.extend(instances(&body));
        if body["next_cursor"].is_null() {
            assert_eq!(body["truncated"], json!(false), "{body}");
            break;
        }
        cursor = Some(body["next_cursor"].clone());
        match boundary {
            0 => {
                // Behind the cursor (301): one found, and 300 lost (its
                // station announces 2, behind too); 1500 found ahead.
                local(&lab, 1, 101).await;
                routed(&lab, 2, 10).await;
                local(&lab, 1500, 150).await;
            }
            1 => {
                // Behind 303: one found, 302 lost; 1501 found ahead, and
                // 1203 lost ahead (its station announces 5).
                local(&lab, 3, 102).await;
                routed(&lab, 4, 12).await;
                local(&lab, 1501, 151).await;
                lab.low
                    .add_device(5, &super::alias_mac(1203))
                    .await
                    .unwrap();
            }
            2 => {
                // Behind 1201: one found, 1201 itself lost; 1502 ahead.
                local(&lab, 6, 103).await;
                lab.low.add_device(7, &LAB_MAC).await.unwrap();
                local(&lab, 1502, 152).await;
            }
            _ => {}
        }
    }
    assert_eq!(
        seen,
        [
            300, 301, 302, 303, 304, 1201, 1202, 1204, 1205, 1500, 1501, 1502
        ]
    );
    lab.stop().await;
}

/// The row a page ended on can leave the table before the next page (its
/// station announces another instance): the next page starts at the row
/// after where it stood.
#[tokio::test(start_paused = true)]
async fn the_boundary_row_leaving_loses_nothing() {
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    assert_eq!(instances(&first), [300, 301, 302]);
    // Network 2001, MAC 0x0c was 302, the boundary row; it now announces
    // 777, ahead of the cursor.
    routed(&lab, 777, 12).await;
    let second = page(&mut lab, Some(&first["next_cursor"])).await;
    assert_eq!(instances(&second), [303, 304, 777], "{second}");
    lab.stop().await;
}

/// A device that moves address between pages: ahead of the cursor, it
/// comes once, at its new address. The boundary row's own device comes
/// again only when its new address sorts after the old one: once a key.
#[tokio::test(start_paused = true)]
async fn a_device_moving_address_between_pages_comes_once_a_key() {
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    assert_eq!(instances(&first), [300, 301, 302]);
    // 1203 is heard at 10.0.0.160, and its old station announces 9.
    lab.low
        .add_device(9, &super::alias_mac(1203))
        .await
        .unwrap();
    local(&lab, 1203, 160).await;
    // 302, the boundary row, moves from MAC 0x0c to 0x05 on its network,
    // behind its old key, and 0x0c announces 8: 302 isn't listed again.
    routed(&lab, 8, 12).await;
    routed(&lab, 302, 5).await;
    let seen = walk(&mut lab, &first).await;
    let at = |instance| {
        seen.iter()
            .filter(|(i, ..)| *i == instance)
            .map(|(_, _, address)| address.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(at(1203), ["10.0.0.160:47808"], "{seen:?}");
    assert_eq!(at(302), ["2001:0x0C"], "{seen:?}");
    let listed: Vec<u64> = seen.iter().map(|(i, ..)| *i).collect();
    assert_eq!(
        listed,
        [300, 301, 302, 303, 304, 1201, 1202, 1203, 1204, 1205]
    );
    lab.stop().await;

    // Moving to 0x20 instead sorts after the old key: listed at each.
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    routed(&lab, 8, 12).await;
    routed(&lab, 302, 32).await;
    let second = page(&mut lab, Some(&first["next_cursor"])).await;
    assert_eq!(rows(&second)[0].2, "2001:0x20", "{second}");
    assert_eq!(instances(&second), [302, 303, 304]);
    lab.stop().await;
}

/// Two addresses claiming one instance are two rows: a page boundary
/// between them lists each once.
#[tokio::test(start_paused = true)]
async fn a_boundary_between_two_claimants_lists_each_once() {
    let mut lab = Lab::start(paged(6)).await;
    lab.agent.initialize().await;
    local(&lab, 1000, 100).await;
    local(&lab, 1000, 101).await;
    let first = page(&mut lab, None).await;
    assert_eq!(
        rows(&first).last().unwrap(),
        &(1000, 0, "10.0.0.100:47808".to_string())
    );
    let second = page(&mut lab, Some(&first["next_cursor"])).await;
    assert_eq!(
        rows(&second)[0],
        (1000, 0, "10.0.0.101:47808".to_string()),
        "{second}"
    );
    assert_eq!(instances(&second)[1..], [1201, 1202, 1203, 1204, 1205]);
    // A page that ends on the table's last row, full, is the last page.
    assert_eq!(second["truncated"], json!(false), "{second}");
    assert!(second["next_cursor"].is_null(), "{second}");
    lab.stop().await;
}

/// A cursor this server signed whose row key is shorter than a row's
/// instance and network, or longer than the longest row (24 octets), is
/// refused as an unusable cursor; the longest row's key is taken.
#[tokio::test(start_paused = true)]
async fn a_signed_row_key_of_the_wrong_length_is_refused() {
    let mut lab = Lab::start(paged(3)).await;
    lab.agent.initialize().await;
    let Value::Object(args) = json!({ "send": false, "scope": "global" }) else {
        unreachable!()
    };
    let mut longest = vec![0, 0x3F, 0xFF, 0xFF, 0xFF, 0xFF];
    longest.extend([0xAB; 18]);
    let mut too_long = longest.clone();
    too_long.push(0);
    for (key, usable) in [
        (vec![0; 5], false),
        (too_long, false),
        (vec![0; 6], true),
        (longest, true),
    ] {
        let cursor = crate::tools::next_after(lab.mcp.ctx(), "bacnet_discover", &args, &key);
        let result = lab
            .agent
            .call(
                "bacnet_discover",
                json!({ "send": false, "scope": "global", "cursor": cursor }),
            )
            .await;
        if usable {
            ok(&result);
            continue;
        }
        let error = failed(&result);
        assert_eq!(
            error["field"],
            json!("cursor"),
            "{} octets: {error}",
            key.len()
        );
        assert_eq!(
            error["message"],
            json!("not a usable cursor: malformed"),
            "{} octets",
            key.len()
        );
    }
    lab.stop().await;
}

/// The error of a failed call.
fn failed(result: &Value) -> &Value {
    assert_eq!(result["isError"], json!(true), "{result}");
    &result["structuredContent"]
}

/// A cursor forged, cut short, of another tool, or for other arguments is
/// refused as any unusable cursor is, with nothing sent.
#[tokio::test(start_paused = true)]
async fn a_bad_cursor_is_refused() {
    let policy = Policy {
        caps: Caps {
            devices: 3,
            objects: 1,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let first = page(&mut lab, None).await;
    let token = first["next_cursor"].as_str().unwrap().to_string();
    let mut forged = token.clone();
    forged.replace_range(0..2, "00");
    let listed = lab
        .agent
        .call("bacnet_list_objects", json!({ "device": "1201" }))
        .await;
    let foreign = ok(&listed)["next_cursor"].clone();
    assert!(foreign.is_string(), "{listed}");
    let before = lab.sends().len();
    for (cursor, scope) in [
        (json!(forged), "global"),
        (json!(&token[..token.len() / 2]), "global"),
        (json!(&token[..token.len() - 2]), "global"),
        (foreign, "global"),
        (json!(token), "local"),
    ] {
        let result = lab
            .agent
            .call(
                "bacnet_discover",
                json!({ "send": false, "scope": scope, "cursor": cursor }),
            )
            .await;
        let error = failed(&result);
        assert_eq!(error["error"], json!("invalid-argument"), "{error}");
        assert_eq!(error["field"], json!("cursor"), "{error}");
        let message = error["message"].as_str().unwrap();
        assert!(message.starts_with("not a usable cursor: "), "{message}");
    }
    assert_eq!(lab.sends().len(), before, "nothing sent");
    lab.stop().await;
}

/// The first page sends the Who-Is; a later page sends nothing and says
/// so.
#[tokio::test(start_paused = true)]
async fn only_the_first_page_sends() {
    let mut lab = Lab::start(paged(2)).await;
    lab.agent.initialize().await;
    let args = json!({ "range": "1000-1999", "wait_s": 1 });
    let first = ok(&lab.agent.call("bacnet_discover", args.clone()).await).clone();
    assert_eq!(first["sent"], json!(true), "{first}");
    let sent = lab.sends().len();
    assert!(sent > 0);
    let mut seen = instances(&first);
    let mut cursor = first["next_cursor"].clone();
    while !cursor.is_null() {
        let mut call = args.clone();
        call["cursor"] = cursor;
        let body = ok(&lab.agent.call("bacnet_discover", call).await).clone();
        assert_eq!(body["sent"], json!(false), "{body}");
        seen.extend(instances(&body));
        cursor = body["next_cursor"].clone();
    }
    assert_eq!(seen, [1201, 1202, 1203, 1204, 1205]);
    assert_eq!(lab.sends().len(), sent, "no Who-Is after the first page");
    lab.stop().await;
}

/// One page of `bacnet_find_object`: its matches as `device object`, and
/// the skipped devices' errors.
async fn find_page(
    lab: &mut Lab,
    args: &Value,
    cursor: &Value,
) -> (Vec<String>, Vec<Value>, Value) {
    let mut call = args.clone();
    if !cursor.is_null() {
        call["cursor"] = cursor.clone();
    }
    let body = ok(&lab.agent.call("bacnet_find_object", call).await).clone();
    let matches = body["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|found| format!("{} {}", found["device"], found["object"].as_str().unwrap()))
        .collect();
    let skipped = body["skipped"].as_array().unwrap().clone();
    (matches, skipped, body["next_cursor"].clone())
}

/// `bacnet_find_object` pages by instance alone, which is its row key
/// (#2606): an instance two addresses claim is skipped whole, each row
/// naming both, and never searched, so the cursor's index always counts
/// one device's candidates. One that gains a second claimant after a page
/// ended inside its list is not searched on from the cursor at either.
#[tokio::test(start_paused = true)]
async fn find_skips_an_instance_claimed_twice_whole() {
    let policy = Policy {
        caps: Caps {
            objects: 1,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    // 1202 answers at its own station and at 10.0.0.150.
    local(&lab, 1202, 150).await;
    let args = json!({ "types": ["ai"], "range": "1201-1203" });
    let mut matches = Vec::new();
    let mut skipped = Vec::new();
    let mut cursor = Value::Null;
    for _ in 0..20 {
        let (found, refused, next) = find_page(&mut lab, &args, &cursor).await;
        matches.extend(found);
        skipped.extend(refused);
        cursor = next;
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(
        matches,
        [
            "1201 analog-input:1",
            "1201 analog-input:3",
            "1203 analog-input:1",
            "1203 analog-input:3"
        ]
    );
    assert_eq!(skipped.len(), 2, "each claimant's row: {skipped:?}");
    for row in &skipped {
        assert_eq!(row["device"], json!(1202));
        assert_eq!(row["error"]["reason"], json!("ambiguous-device"), "{row}");
        let message = row["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("10.0.0.150:47808") && message.contains("10.0.0.3:47808"),
            "{message}"
        );
    }

    // A page ends inside 1201's candidates; then 10.0.0.151 claims 1201.
    let (found, _, cursor) = find_page(&mut lab, &args, &Value::Null).await;
    assert_eq!(found, ["1201 analog-input:1"]);
    local(&lab, 1201, 151).await;
    let (found, refused, _) = find_page(&mut lab, &args, &cursor).await;
    assert_eq!(found, ["1203 analog-input:1"]);
    let devices: Vec<&Value> = refused.iter().map(|row| &row["device"]).collect();
    assert_eq!(
        devices,
        [&json!(1201), &json!(1201), &json!(1202), &json!(1202)]
    );
    lab.stop().await;
}
