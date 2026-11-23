//! A routed target at an address the one-station mark lifts (#2738):
//! `5:0xFF:one-station` and `9:0x0012:one-station` resolve to their
//! stations and a confirmed request goes through the router to that DNET
//! and DADR; the same text without the suffix is refused, naming it, and
//! sends nothing.

use bacnet_client::client::ClientConfig;
use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ObjectType, Segmentation};
use tokio::sync::mpsc;

use super::routed_tests::{CLIENT, ROUTER, answer, refused, routed};
use super::*;

/// A client whose table holds `stations` behind the router, which answers
/// every request for them with the device's Error.
async fn site(stations: &[(u16, &[u8])]) -> (BACnetClient<TestTransport>, SendLog) {
    let (inbox, inbound) = mpsc::channel(32);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(inbound)
        .on_send(move |frame| {
            let inbox = inbox.clone();
            async move {
                if let Some(reply) = answer(&frame) {
                    let _ = inbox.send(reply).await;
                }
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 500,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    for (at, (network, mac)) in stations.iter().enumerate() {
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: 100 + u32::try_from(at).unwrap(),
                router_mac: ROUTER.to_vec(),
                remote_network: *network,
                remote_mac: mac.to_vec(),
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::NO_SEGMENTATION,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
    }
    (client, sent)
}

#[tokio::test(start_paused = true)]
async fn a_one_station_target_is_sent_to_its_dnet_and_dadr() {
    let wait = Duration::from_secs(1);
    let (mut client, sent) = site(&[(5, &[0xFF]), (9, &[0x00, 0x12])]).await;
    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let sites: [(&str, u16, &[u8]); 2] = [
        ("5:0xFF:one-station", 5, &[0xFF]),
        ("9:0x0012:one-station", 9, &[0x00, 0x12]),
    ];
    for (text, network, mac) in sites {
        let to = resolve(&client, text, wait).await.unwrap();
        // The table's row is the station: nothing is sent to find it, and
        // the address reads back as the text.
        assert!(sent.is_empty(), "{text}: resolving sent nothing");
        assert_eq!(to.address.to_string(), text);
        assert!(to.instance.is_some(), "{text}: the row's instance");
        let result = to.read_property(&client, object, pv, None).await;
        assert!(refused(result), "{text}: the device's own Error came back");
        let sends = routed(&sent);
        assert_eq!(sends.len(), 1, "{text}: {sends:?}");
        assert_eq!(sends[0].0, ROUTER.to_vec(), "{text}: through the router");
        assert_eq!(sends[0].1, Some((network, mac.to_vec())), "{text}");
        sent.clear();

        // Without the suffix the address is refused before anything is sent
        // or waited for, and the refusal names the way to write it.
        let bare = text.trim_end_matches(":one-station");
        let error = resolve(&client, bare, wait).await.unwrap_err();
        assert!(error.contains(text), "{bare}: {error}");
        assert!(error.contains("not a station"), "{bare}: {error}");
        assert!(sent.is_empty(), "{bare}: nothing sent");
    }
    // The suffix where it lifts nothing is refused with its reason.
    for (text, why) in [
        ("5:0x05:one-station", "an ordinary station"),
        ("5:0x0000:one-station", "whole domain"),
        ("5:1234:one-station", "not a device instance"),
    ] {
        let error = resolve(&client, text, wait).await.unwrap_err();
        assert!(error.contains(why), "{text}: {error}");
        assert!(sent.is_empty(), "{text}: nothing sent");
    }
    client.stop().await.unwrap();
}

/// A marked station on the client's own network lifts nothing: it is on
/// this link, so the target is refused whatever the table holds.
#[test]
fn a_one_station_target_on_the_own_network_is_refused() {
    let marked = Station::Mac {
        mac: vec![0xFF],
        one_station: true,
    };
    let error = marked.check(5, Some(5)).unwrap_err();
    assert!(error.contains("own network"), "{error}");
    assert!(error.contains("5:0xFF:one-station"), "{error}");
    assert!(marked.check(5, Some(6)).is_ok());
    assert!(marked.check(5, None).is_ok());
}

/// An instance whose only claimant answered from an address the mark
/// lifts says so and shows the form, instead of waiting for an answer; and
/// on the client's own network the unmarked refusal is the link's, with no
/// suffix to suggest, since it would lift nothing there (#2738).
#[tokio::test(start_paused = true)]
async fn the_refusals_say_how_to_name_the_station() {
    let (mut client, sent) = site(&[(5, &[0xFF])]).await;
    let error = resolve(&client, "5:100", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        error.contains("answered from 5:0xFF:one-station"),
        "{error}"
    );
    assert!(sent.is_empty(), "nothing sent, nothing waited for");
    client.stop().await.unwrap();

    let unmarked = Station::Mac {
        mac: vec![0x00, 0x12],
        one_station: false,
    };
    let remote = unmarked.check(9, Some(5)).unwrap_err();
    assert!(remote.contains("9:0x0012:one-station"), "{remote}");
    let own = unmarked.check(5, Some(5)).unwrap_err();
    assert!(own.contains("local network"), "{own}");
    assert!(!own.contains("one-station"), "{own}");
}
