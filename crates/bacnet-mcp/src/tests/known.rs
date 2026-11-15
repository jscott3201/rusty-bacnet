//! The server's known devices follow the client's table, one device per
//! address, and a call reads the device the policy checked (#2560).

use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use serde_json::{Value, json};

use super::{ALIASES, LAB, LAB_MAC, Lab, alias_mac};
use crate::Policy;
use crate::context::{Call, DeviceArg};

/// An I-Am for `instance`, as a device on the local network sends it.
pub(super) fn i_am(instance: u32) -> BytesMut {
    i_am_from(instance, None)
}

/// An I-Am for `instance`, from the station a router names as `source`
/// when there is one.
pub(super) fn i_am_from(instance: u32, source: Option<NpduAddress>) -> BytesMut {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut service_request);
    let mut payload = BytesMut::new();
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    });
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    let message = Npdu {
        source,
        payload: payload.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &message).unwrap();
    npdu
}

/// Wait until `holds` is true of the lab, on the paused clock.
pub(super) async fn until(lab: &Lab, holds: impl Fn(&Lab) -> bool, what: &str) {
    for _ in 0..1_000 {
        if holds(lab) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("never: {what}");
}

/// An address announcing another instance takes its old one out of the
/// devices the server knows: the client reports the old one lost.
#[tokio::test(start_paused = true)]
async fn an_address_announcing_another_instance_forgets_the_old_one() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let alias = ALIASES.start;
    let knows = |lab: &Lab, instance| lab.mcp.ctx().reach_known(instance);
    until(&lab, |lab| knows(lab, alias), "the alias is known").await;

    lab.inject(&i_am(7), &alias_mac(alias)).await;
    until(
        &lab,
        |lab| knows(lab, 7) && !knows(lab, alias),
        "7 replaces the alias",
    )
    .await;
    assert!(lab.low.get_device(alias).await.unwrap().is_none());
    lab.stop().await;
}

/// A call reads the device the policy let it reach, and no other: while a
/// read of the lab device times out, a station on a network the policy
/// leaves out announces the lab's instance, and the call doesn't follow
/// the instance there.
#[tokio::test(start_paused = true)]
async fn a_read_stays_on_the_device_the_policy_checked() {
    let policy = Policy {
        networks: Some([0].into()),
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    lab.go_silent_after(0);
    let before = lab.sends().len();
    let arguments =
        json!({ "device": LAB, "reads": [{ "object": "ai:1", "properties": ["present-value"] }] });
    let call = lab
        .agent
        .post(
            "tools/call",
            json!({ "name": "bacnet_read", "arguments": arguments }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let elsewhere = NpduAddress {
        network: 2001,
        mac_address: MacAddr::from_slice(&[0x33]),
    };
    lab.inject(&i_am_from(LAB, Some(elsewhere)), &LAB_MAC).await;
    for _ in 0..100 {
        if lab.low.get_device_at(2001, &[0x33]).await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        lab.low.get_device_at(2001, &[0x33]).await.is_some(),
        "the other claim is in the table"
    );
    let answer = lab.agent.answer(call).await;
    let result = &answer["result"];
    assert_eq!(result["isError"], json!(true), "{answer}");
    assert_eq!(
        result["structuredContent"]["error"],
        json!("timeout"),
        "{answer}"
    );
    assert_eq!(lab.sends().len() - before, 1, "one request, to the lab");
    lab.stop().await;
}

/// Once the client knows its own network's number, `number:instance`
/// names a device on the local network, as the client's handles key it:
/// the lab device answers as `7:1201` on network 7.
#[tokio::test(start_paused = true)]
async fn the_own_network_number_names_the_local_network() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let arguments = json!({ "device": format!("7:{LAB}") });
    let result = lab
        .agent
        .call("bacnet_describe_device", arguments.clone())
        .await;
    assert_eq!(result["isError"], json!(true), "not yet: {result}");
    // A router on the link broadcasts this network's number, configured.
    let says = bytes::Bytes::from_static(&[1, 0x80, 0x13, 0, 7, 1]);
    let inbox = lab.inbox.inbound().expect("started");
    let from = MacAddr::from_slice(&LAB_MAC);
    let frame = ReceivedNpdu::unverified(says, from, true, Vec::new(), None);
    inbox.send(frame).await.unwrap();
    until(&lab, |lab| lab.low.network_number() == Some(7), "learned").await;
    let result = lab.agent.call("bacnet_describe_device", arguments).await;
    assert_ne!(result["isError"], json!(true), "{result}");
    lab.stop().await;
}

/// A request the link never sent fails the tool call with the link's own
/// error, where a read once came back as rows of error text (#2610):
/// `bacnet_describe_device` and `bacnet_read` both say why, and send no
/// further request for the same refusal.
#[tokio::test(start_paused = true)]
async fn a_request_never_sent_fails_the_call_with_its_error() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    lab.refuse_sends();
    let device = json!({ "device": LAB.to_string() });
    let read = json!({ "device": LAB.to_string(), "reads": [{ "object": "ai:1" }] });
    for (tool, arguments) in [("bacnet_describe_device", device), ("bacnet_read", read)] {
        let before = lab.sends().len();
        let result = lab.agent.call(tool, arguments).await;
        assert_eq!(result["isError"], json!(true), "{tool}: {result}");
        let text = result.to_string();
        assert!(text.contains("the link refused the send"), "{tool}: {text}");
        let tried = lab.sends().len() - before;
        assert!(tried <= 2, "{tool}: {tried} sends for one refusal");
    }
    lab.stop().await;
}

/// Two networks hold one instance (#2560): the lab device on this one, and
/// a station on network 2001 behind it that announces the same instance.
/// The server knows them as two devices; the instance alone names neither,
/// `2001:instance` names the routed one and is refused by a policy that
/// leaves its network out, and the local one is reached, and watched, by
/// its address. Nothing is sent to resolve any of them.
#[tokio::test(start_paused = true)]
async fn two_networks_holding_one_instance_are_two_devices() {
    let policy = Policy {
        networks: Some([0].into()),
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
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
    assert!(
        !lab.mcp.ctx().reach_known(LAB),
        "an instance alone names neither"
    );
    let before = lab.sends().len();

    let read = |device: Value| json!({ "device": device, "reads": [{ "object": "ai:1" }] });
    // Refused as ambiguous, as a resource is, naming the target once.
    let by_instance = lab.agent.call("bacnet_read", read(json!(LAB))).await;
    let body = &by_instance["structuredContent"];
    assert_eq!(body["reason"], json!("ambiguous-device"), "{by_instance}");
    let message = body["message"].as_str().unwrap_or_default();
    assert_eq!(
        message, "device 1201 is claimed by 10.0.0.2:47808 and 2001:0x33; name one by its address",
        "{by_instance}"
    );
    let routed = lab
        .agent
        .call("bacnet_read", read(json!(format!("2001:{LAB}"))))
        .await;
    let routed = &routed["structuredContent"];
    assert_eq!(routed["reason"], json!("network-not-allowed"), "{routed}");
    assert_eq!(lab.sends().len(), before, "nothing sent to resolve them");

    let local = lab
        .agent
        .call("bacnet_read", read(json!("10.0.0.2:47808")))
        .await;
    assert_ne!(local["isError"], json!(true), "{local}");
    assert_eq!(local["structuredContent"]["device"], json!(LAB), "{local}");
    // The watch hub keys points by address (#2560), so the local one by
    // its address is watched while the instance is contested.
    let points = json!({ "points": [{ "device": "10.0.0.2:47808", "object": "ai:1" }] });
    let watch = lab.agent.call("bacnet_watch", points).await;
    let point = &watch["structuredContent"]["points"][0];
    assert!(point.get("error").is_none(), "{watch}");
    assert_eq!(point["watched"], json!(true), "{watch}");
    assert_eq!(point["address"], json!("10.0.0.2:47808"), "{watch}");
    assert_eq!(point["uri_ambiguous"], json!(true), "{watch}");
    lab.stop().await;
}

/// With both networks allowed, `2001:instance` resolves within its
/// network to the routed station, and the local one is still its own
/// device: each call is pinned to the address it named.
#[tokio::test(start_paused = true)]
async fn a_scoped_instance_names_the_claim_on_its_network() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let elsewhere = NpduAddress {
        network: 2001,
        mac_address: MacAddr::from_slice(&[0x33]),
    };
    lab.inject(&i_am_from(LAB, Some(elsewhere)), &LAB_MAC).await;
    let routed = |lab: &Lab| {
        let known = lab.mcp.ctx().known.lock().unwrap();
        known
            .keys()
            .any(|address| address.to_string() == "2001:0x33")
    };
    until(&lab, routed, "the routed claim is known").await;
    let ctx = lab.mcp.ctx();
    let call = Call::new(Duration::from_secs(5), ctx.account());
    let routed = ctx
        .device(&call, &DeviceArg::Text(format!("2001:{LAB}")))
        .await
        .map_err(|e| e.json())
        .unwrap();
    assert_eq!(routed.address.to_string(), "2001:0x33");
    assert_eq!((routed.instance, routed.network), (LAB, 2001));
    assert_eq!(routed.handle.address(), routed.address, "pinned there");
    let local = ctx
        .device(&call, &DeviceArg::Text("10.0.0.2:47808".into()))
        .await
        .map_err(|e| e.json())
        .unwrap();
    assert_eq!(local.address.to_string(), "10.0.0.2:47808");
    assert_eq!(local.instance, LAB);
    let missing = ctx
        .device(&call, &DeviceArg::Text(format!("2002:{LAB}")))
        .await
        .map(|_| ())
        .unwrap_err()
        .json();
    assert_eq!(missing["target"], json!(format!("2002:{LAB}")), "{missing}");
    drop((routed, local));
    lab.stop().await;
}
