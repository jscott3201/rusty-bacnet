//! Writes, reverts and watches by address over MCP (#2560): the
//! confirmation is bound to the address it previewed, a revert is checked
//! against the policy by its entry's address and never sends a Who-Is, and
//! the watch list reads who claims an instance from the discovery table.

use std::time::Duration;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_tool::safety::ledger::{FILE, Ledger};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};

use super::known::{i_am, i_am_from, until};
use super::writes::{answered, call, one, slot, writes};
use super::{ALIASES, LAB, LAB_MAC, Lab, alias_mac};
use crate::Policy;
use crate::policy::ArmMode;

/// AO:1's priority slot `n` on the alias device `alias`.
async fn alias_slot(lab: &Lab, alias: u32, n: u32) -> PropertyValue {
    let index = usize::try_from(alias - ALIASES.start).unwrap();
    let db = lab.aliases[index].database().read().await;
    let ao1 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    db.get(&ao1)
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(n))
        .unwrap()
}

/// A ledger of this server's principal's entries, each AO:1 slot 12 set
/// to 5 (empty before) on `device`, at `address` or, with none, as a
/// release before #2560 wrote it.
fn ledger(entries: &[(u64, u32, Option<&str>)]) -> String {
    let entries: Vec<Value> = entries
        .iter()
        .map(|(id, device, address)| {
            let mut entry = json!({
                "id": id, "run": "4242-1763000000000", "at": "2026-11-13T10:00:00.000Z",
                "device": device, "object_type": 1, "instance": 1, "property": 85,
                "index": null, "priority": 12, "commandable": true, "before": null,
                "written": { "bytes": "4440a00000", "text": "5" },
                "state": "applied", "kept": false, "seq": id,
                "front_end": "mcp", "principal": "tester",
            });
            if let Some(address) = address {
                entry["address"] = json!(address);
            }
            entry
        })
        .collect();
    json!({ "version": 1, "entries": entries }).to_string()
}

/// Device 1201 moves between the preview and the confirmation: its
/// address announces another instance, and an alias's address announces
/// 1201. The retry resolves 1201 again, to the alias, and its preview
/// there differs from the confirmed one by its address alone (the alias
/// holds the same objects and values), so it is refused as stale. Nothing
/// is written at either address. The form names the address.
#[tokio::test(start_paused = true)]
async fn a_device_that_moves_before_the_confirmation_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
    lab.agent.initialize().await;
    let args = one(json!(42.5), 12);
    let first = call(&mut lab, "bacnet_write", args.clone(), json!({})).await;
    let message = first["inputRequests"]["confirm"]["params"]["message"]
        .as_str()
        .unwrap();
    assert!(
        message.contains("to device 1201 at 10.0.0.2:47808:"),
        "{message}"
    );

    let alias = ALIASES.start;
    lab.inject(&i_am(9999), &LAB_MAC).await;
    lab.inject(&i_am(LAB), &alias_mac(alias)).await;
    let moved = |lab: &Lab| lab.mcp.ctx().known_at(LAB).map(|at| at.to_string());
    until(
        &lab,
        |lab| moved(lab).as_deref() == Some("10.0.0.3:47808"),
        "1201 is at the alias's address",
    )
    .await;
    let confirm = answered(&first["requestState"], "accept", true);
    let reply = call(&mut lab, "bacnet_write", args, confirm).await;
    let body = &reply["structuredContent"];
    assert_eq!(body["reason"], json!("stale"), "{reply}");
    assert!(
        body["message"].as_str().unwrap().contains("address"),
        "{body}"
    );
    assert_eq!(slot(&lab, 12).await, PropertyValue::Null);
    assert_eq!(
        alias_slot(&lab, alias, 12).await,
        PropertyValue::Null,
        "not written at the new address"
    );
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    lab.stop().await;
}

/// A write to a routed station is kept with its address; with the
/// policy's networks narrowed to the local one, its revert is refused for
/// the address's network, with nothing sent.
#[tokio::test(start_paused = true)]
async fn a_revert_to_a_network_the_policy_leaves_out_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let held = ledger(&[(1, 300, Some("2001:0x0A"))]);
    std::fs::write(dir.path().join(FILE), held).unwrap();
    let mut policy = writes(dir.path(), ArmMode::Elicit);
    policy.networks = Some([0].into());
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let reply = call(
        &mut lab,
        "bacnet_revert",
        json!({ "entries": [1] }),
        json!({}),
    )
    .await;
    let body = &reply["structuredContent"];
    assert_eq!(body["reason"], json!("network-not-allowed"), "{reply}");
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(Ledger::open(dir.path()).unwrap().entries().len(), 1);
    lab.stop().await;
}

/// `bacnet_revert` never sends a Who-Is: an entry at a routed station the
/// discovery table has no row for, and an entry from before addresses whose
/// instance no row claims, are each reported not reverted, saying why, with
/// nothing sent at all.
#[tokio::test(start_paused = true)]
async fn a_revert_sends_no_who_is_to_find_its_device() {
    let dir = tempfile::tempdir().unwrap();
    let held = ledger(&[(1, 4000, Some("2002:0x07")), (2, 4001, None)]);
    std::fs::write(dir.path().join(FILE), held).unwrap();
    let mut lab = Lab::start(writes(dir.path(), ArmMode::Elicit)).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let reply = call(&mut lab, "bacnet_revert", json!({ "all": true }), json!({})).await;
    assert_eq!(reply["isError"], json!(true), "{reply}");
    let results = reply["structuredContent"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{reply}");
    for result in results {
        assert_eq!(result["applied"], json!(false), "{result}");
        let why = result["outcome"]["why"].as_str().unwrap();
        assert!(why.contains("sends no Who-Is"), "{result}");
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.sends().len(), before, "no Who-Is, nothing sent");
    assert_eq!(Ledger::open(dir.path()).unwrap().entries().len(), 2);
    lab.stop().await;
}

/// The watch list's check of who claims an instance reads the discovery
/// table, not the server's copy of it, which can lag: with a second claim
/// in the table that the copy hasn't heard of yet, a watch on the lab
/// device by its address is refused as ambiguous, naming both claimants.
#[tokio::test(start_paused = true)]
async fn a_watch_reads_the_claims_from_the_table() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let elsewhere = NpduAddress {
        network: 2001,
        mac_address: MacAddr::from_slice(&[0x33]),
    };
    lab.inject(&i_am_from(LAB, Some(elsewhere)), &LAB_MAC).await;
    let claims = |lab: &Lab| {
        let known = lab.mcp.ctx().known.lock().unwrap();
        known.values().filter(|instance| **instance == LAB).count()
    };
    until(&lab, |lab| claims(lab) == 2, "both claims are known").await;
    // The copy lags: it hasn't heard of the routed claim.
    lab.mcp
        .ctx()
        .known
        .lock()
        .unwrap()
        .retain(|address, _| address.network() == 0);
    let points = json!({ "points": [{ "device": "10.0.0.2:47808", "object": "ai:1" }] });
    let watch = lab.agent.call("bacnet_watch", points).await;
    let error = &watch["structuredContent"]["points"][0]["error"];
    assert_eq!(error["reason"], json!("ambiguous-device"), "{watch}");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.starts_with("device 1201 is claimed by 10.0.0.2:47808 and 2001:0x33"),
        "{message}"
    );
    lab.stop().await;
}
