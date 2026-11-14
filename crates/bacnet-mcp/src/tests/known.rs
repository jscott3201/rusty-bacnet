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
use serde_json::json;

use super::{ALIASES, LAB, LAB_MAC, Lab, alias_mac};
use crate::Policy;

/// An I-Am for `instance`, as a device on the local network sends it.
fn i_am(instance: u32) -> BytesMut {
    i_am_from(instance, None)
}

/// An I-Am for `instance`, from the station a router names as `source`
/// when there is one.
fn i_am_from(instance: u32, source: Option<NpduAddress>) -> BytesMut {
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
async fn until(lab: &Lab, holds: impl Fn(&Lab) -> bool, what: &str) {
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
