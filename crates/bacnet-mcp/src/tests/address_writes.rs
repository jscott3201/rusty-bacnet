//! Writes, reverts and watches by address over MCP (#2560): the
//! confirmation is bound to the address it previewed, a revert is checked
//! against the policy where the desk finds its device (by the address in
//! its local form) and never sends a Who-Is, and the watch list reads who
//! claims an instance from the discovery table, one station per address.

use std::time::Duration;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_tool::safety::ledger::{FILE, Ledger};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};

use super::known::{i_am, i_am_from, until};
use super::writes::{answered, call, command, one, slot, writes};
use super::{ALIASES, LAB, LAB_MAC, Lab, alias_mac};
use crate::Policy;
use crate::policy::ArmMode;

/// Teach the client its network's number, as a router's
/// Network-Number-Is does.
async fn learn(lab: &Lab, network: u16) {
    let [hi, lo] = network.to_be_bytes();
    let says = bytes::Bytes::from(vec![1, 0x80, 0x13, hi, lo, 1]);
    let inbox = lab.inbox.inbound().expect("started");
    let from = MacAddr::from_slice(&LAB_MAC);
    let frame =
        bacnet_transport::port::ReceivedNpdu::unverified(says, from, true, Vec::new(), None);
    inbox.send(frame).await.unwrap();
    until(
        lab,
        |lab| lab.low.network_number() == Some(network),
        "learned",
    )
    .await;
}

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

/// Two devices sharing an instance, each named by its address, are
/// watched there (#2560): the hub keys points on the address, so neither
/// claim makes the other ambiguous, and each row carries its address.
#[tokio::test(start_paused = true)]
async fn a_watch_by_address_ignores_another_claim_to_the_instance() {
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
    let points = json!({ "points": [
        { "device": "10.0.0.2:47808", "object": "ai:1" },
        { "device": "2001:0x33", "object": "ai:1" },
    ] });
    let watch = lab.agent.call("bacnet_watch", points).await;
    let rows = &watch["structuredContent"]["points"];
    for (row, address) in [(&rows[0], "10.0.0.2:47808"), (&rows[1], "2001:0x33")] {
        assert!(row.get("error").is_none(), "{watch}");
        assert_eq!(row["watched"], json!(true), "{watch}");
        assert_eq!(row["device"], json!(LAB));
        // Each row says which device it is; the instance's URI names
        // neither, and the row says so.
        assert_eq!(row["address"], json!(address), "{watch}");
        assert_eq!(row["uri_ambiguous"], json!(true), "{watch}");
    }
    assert_eq!(rows[0]["uri"], rows[1]["uri"]);
    lab.stop().await;
}

/// An entry from before addresses for device 300, whose one claim is its
/// routed station on network 2001: with the policy's networks narrowed to
/// the local one, the revert finds it there and the policy refuses it at
/// that address. The entry is reported not reverted, saying why, with
/// nothing sent: the address checked is the one the desk would send to.
#[tokio::test(start_paused = true)]
async fn a_revert_of_an_old_entry_is_checked_where_its_device_is() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FILE), ledger(&[(1, 300, None)])).unwrap();
    let mut policy = writes(dir.path(), ArmMode::Elicit);
    policy.networks = Some([0].into());
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let before = lab.sends().len();
    let reply = call(&mut lab, "bacnet_revert", json!({ "all": true }), json!({})).await;
    let result = &reply["structuredContent"]["results"][0];
    assert_eq!(result["applied"], json!(false), "{reply}");
    assert_eq!(
        result["outcome"]["why"],
        json!("network 2001 is not in this server's [mcp] networks"),
        "{reply}"
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(Ledger::open(dir.path()).unwrap().entries().len(), 1);
    lab.stop().await;
}

/// An entry kept in routed form on the client's own network (written
/// before it learned the number) is the local station: with the networks
/// narrowed to the local one, its revert goes ahead, and the slot another
/// commander left aside is emptied as the entry says it was.
#[tokio::test(start_paused = true)]
async fn an_entry_in_routed_form_on_this_network_is_local_to_the_policy() {
    let dir = tempfile::tempdir().unwrap();
    let routed = "7:0x0A000002BAC0";
    std::fs::write(dir.path().join(FILE), ledger(&[(1, LAB, Some(routed))])).unwrap();
    let mut policy = writes(dir.path(), ArmMode::Elicit);
    policy.networks = Some([0].into());
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    learn(&lab, 7).await;
    command(&lab, 5.0, 12).await;
    let reply = call(
        &mut lab,
        "bacnet_revert",
        json!({ "entries": [1] }),
        json!({}),
    )
    .await;
    assert_eq!(reply["isError"], json!(false), "{reply}");
    assert_eq!(slot(&lab, 12).await, PropertyValue::Null, "reverted");
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    lab.stop().await;
}

/// A routed row on the client's own network that the table hasn't folded
/// into the local one yet (#2572) is the local station, not a second
/// claim: a watch on the lab device by its address is taken, and the
/// scoped instance names one device.
#[tokio::test(start_paused = true)]
async fn a_relayed_row_on_this_network_is_no_second_claim() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let relayed = NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&LAB_MAC),
    };
    lab.inject(&i_am_from(LAB, Some(relayed)), &LAB_MAC).await;
    let mut rows = 0;
    for _ in 0..100 {
        rows = lab
            .low
            .discovered_devices()
            .await
            .iter()
            .filter(|row| row.object_identifier.instance_number() == LAB)
            .count();
        if rows == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(rows, 2, "the local row and the relayed one");
    learn(&lab, 7).await;
    let points = json!({ "points": [{ "device": "10.0.0.2:47808", "object": "ai:1" }] });
    let watch = lab.agent.call("bacnet_watch", points).await;
    assert_ne!(watch["isError"], json!(true), "{watch}");
    let point = &watch["structuredContent"]["points"][0];
    assert!(point.get("error").is_none(), "{watch}");
    let read = json!({ "device": format!("7:{LAB}"), "reads": [{ "object": "ai:1" }] });
    let read = lab.agent.call("bacnet_read", read).await;
    assert_ne!(read["isError"], json!(true), "{read}");
    lab.stop().await;
}

/// MCP's revert hook checks the address the desk found: the policy's
/// networks there, and for an attached revert the address the person
/// confirmed. Nothing is checked for a caller that isn't reverting.
#[test]
fn an_mcp_revert_is_checked_at_the_address_the_desk_found() {
    use bacnet_tool::safety::desk::Caller;
    use bacnet_tool::safety::ledger::Entry;

    let policy = Policy {
        networks: Some([0].into()),
        ..Policy::default()
    };
    let writes = crate::writes::Writes::new(&policy.writes);
    let file: Value = serde_json::from_str(&ledger(&[(1, 300, None)])).unwrap();
    let entry: Entry = serde_json::from_value(file["entries"][0].clone()).unwrap();
    let here: bacnet_ops::DeviceAddress = "10.0.0.2:47808".parse().unwrap();
    let there: bacnet_ops::DeviceAddress = "10.0.0.3:47808".parse().unwrap();
    let routed: bacnet_ops::DeviceAddress = "2001:0x0A".parse().unwrap();
    let caller =
        || crate::writes::McpCaller::new(&writes, &policy.writes, None, Default::default());

    let stdio = caller().reverting(&policy, None);
    assert_eq!(stdio.revert_refusal(&entry, &here), None);
    assert_eq!(
        stdio.revert_refusal(&entry, &routed).as_deref(),
        Some("network 2001 is not in this server's [mcp] networks")
    );
    let attached = caller().reverting(&policy, Some(here.clone()));
    assert_eq!(attached.revert_refusal(&entry, &here), None);
    assert_eq!(
        attached.revert_refusal(&entry, &there).as_deref(),
        Some(
            "device 300 is at 10.0.0.3:47808 now, not at 10.0.0.2:47808 where its revert was confirmed"
        )
    );
    assert_eq!(
        caller().revert_refusal(&entry, &routed),
        None,
        "not reverting"
    );
}
