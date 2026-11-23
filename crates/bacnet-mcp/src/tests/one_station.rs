//! A station at an address the one-station mark lifts, named by the tools
//! in the target grammar (#2738): `5:0xFF:one-station` is read through
//! its router to that DNET and DADR, and the same text without the suffix
//! is refused naming it, with nothing sent.

use std::time::Duration;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;
use serde_json::{Value, json};

use super::known::i_am_from;
use super::{LAB, LAB_MAC, Lab};
use crate::Policy;

/// The destinations (network, MAC) of the routed requests the client sent
/// since the log was last cleared.
fn routed_to(lab: &Lab) -> Vec<(u16, Vec<u8>)> {
    lab.frames
        .unicasts()
        .iter()
        .filter_map(|frame| {
            let to = frame.decode_npdu().destination?;
            Some((to.network, to.mac_address.to_vec()))
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_one_station_address_is_read_through_its_router() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let sites: [(&str, u16, &[u8]); 2] = [
        ("5:0xFF:one-station", 5, &[0xFF]),
        ("9:0x0012:one-station", 9, &[0x00, 0x12]),
    ];
    for (text, network, mac) in sites {
        let source = NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        };
        lab.inject(&i_am_from(LAB, Some(source)), &LAB_MAC).await;
        for _ in 0..100 {
            if lab.low.get_device_at(network, mac).await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            lab.low.get_device_at(network, mac).await.is_some(),
            "{text}"
        );
        lab.frames.clear();
        let read = json!({ "device": text, "reads": [{ "object": "ai:1" }] });
        let answer = lab.agent.call("bacnet_read", read).await;
        // The lab's server answers nothing for a DNET it doesn't own, so
        // the call times out; what counts is where the request went.
        assert_eq!(
            routed_to(&lab),
            [(network, mac.to_vec())],
            "{text}: {answer}"
        );

        // Without the suffix: refused, naming the way to write it, and
        // nothing is sent.
        let bare = text.trim_end_matches(":one-station");
        let before = lab.sends().len();
        lab.frames.clear();
        let read = json!({ "device": bare, "reads": [{ "object": "ai:1" }] });
        let refused = lab.agent.call("bacnet_read", read).await;
        let body: &Value = &refused["structuredContent"];
        let message = body["message"].as_str().unwrap_or_default();
        assert_eq!(refused["isError"], json!(true), "{bare}: {refused}");
        assert!(message.contains(text), "{bare}: {refused}");
        assert!(message.contains("Ask the operator"), "{bare}: {refused}");
        assert!(message.contains("MS/TP"), "{bare}: {refused}");
        assert_eq!(lab.sends().len(), before, "{bare}: nothing sent");
        assert!(lab.frames.is_empty(), "{bare}: nothing sent");
    }
    // An instance whose only claimant on a network is at a lifted MAC is
    // named by that address, which the refusal shows, for the operator.
    let read = json!({ "device": "9:1201", "reads": [{ "object": "ai:1" }] });
    let refused = lab.agent.call("bacnet_read", read).await;
    let message = refused["structuredContent"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("answered from 9:0x0012"), "{refused}");
    assert!(message.contains("Ask the operator"), "{refused}");
    // The suffix where it lifts nothing is refused with its reason.
    let read = json!({ "device": "5:0x05:one-station", "reads": [{ "object": "ai:1" }] });
    let refused = lab.agent.call("bacnet_read", read).await;
    assert!(refused.to_string().contains("lifts nothing"), "{refused}");
    lab.stop().await;
}
