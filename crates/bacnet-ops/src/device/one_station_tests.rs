//! Stations at an address the one-station mark lifts (#2738), named in the
//! target grammar: a handle made from `5:0xFF:one-station` sends a
//! confirmed request through its router to that DNET and DADR, where the
//! same text without the suffix is refused naming it.

use std::sync::Arc;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_encoding::apdu::{Apdu, ErrorPdu, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::{ErrorClass, ErrorCode, Segmentation};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::*;
use crate::limits::RequestLimits;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

/// Every confirmed request answered with an Error PDU, from the station it
/// was addressed to, through the router.
fn answer(frame: &SentFrame) -> Option<ReceivedNpdu> {
    if frame.broadcast {
        return None;
    }
    let npdu = frame.decode_npdu();
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) else {
        return None;
    };
    let error = Apdu::Error(ErrorPdu {
        invoke_id: request.invoke_id,
        service_choice: request.service_choice,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        error_data: Bytes::new(),
    });
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &error).unwrap();
    let back = Npdu {
        source: npdu.destination.map(|to| NpduAddress {
            network: to.network,
            mac_address: to.mac_address,
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &back).unwrap();
    Some(ReceivedNpdu::unverified(
        buf.freeze(),
        frame.mac.clone(),
        false,
        Vec::new(),
        None,
    ))
}

/// A client whose table holds `stations` behind the router, which answers
/// every request for them.
async fn site(stations: &[(u16, &[u8])]) -> (Client<TestTransport>, SendLog) {
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
    let low = BACnetClient::start(config, transport).await.unwrap();
    for (at, (network, mac)) in stations.iter().enumerate() {
        low.add_routed_device(RoutedDeviceConfig {
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
    (
        Client::new(Arc::new(low), Arc::new(RequestLimits::new(0))),
        sent,
    )
}

/// Each marked text names its station: the handle is pinned there, shows
/// the suffix, and a read goes through the router with the station's DNET
/// and DADR, which the unmarked client would have refused as a broadcast.
#[tokio::test(start_paused = true)]
async fn a_one_station_target_reads_through_the_router() {
    let (client, sent) = site(&[(5, &[0xFF]), (9, &[0x00, 0x12]), (5, &[0x05])]).await;
    let sites: [(&str, u16, &[u8]); 2] = [
        ("5:0xFF:one-station", 5, &[0xFF]),
        ("9:0x0012:one-station", 9, &[0x00, 0x12]),
    ];
    for (text, network, mac) in sites {
        let device = client.device(text).await.unwrap();
        assert_eq!(device.address().to_string(), text);
        assert_eq!(
            device.address(),
            DeviceAddress::marked(network, mac).unwrap()
        );
        assert!(device.address().is_one_station());
        sent.clear();
        let error = device.read("ai:1 pv").await.unwrap_err();
        assert!(
            matches!(error, OpsError::Refused { .. }),
            "{text}: {error:?}"
        );
        let unicasts = sent.unicasts();
        assert_eq!(unicasts.len(), 1, "{text}: one ReadProperty");
        let to = unicasts[0].decode_npdu().destination.unwrap();
        assert_eq!(
            unicasts[0].mac.to_vec(),
            ROUTER,
            "{text}: through the router"
        );
        assert_eq!(
            (to.network, to.mac_address.to_vec()),
            (network, mac.to_vec()),
            "{text}"
        );
        // Without the suffix the text is refused, naming it, and sends nothing.
        let bare = text.trim_end_matches(":one-station");
        sent.clear();
        let refused = client.device(bare).await.err().unwrap();
        let OpsError::Parse { reason, .. } = &refused else {
            panic!("{bare}: {refused:?}");
        };
        assert!(reason.contains(text), "{bare}: {reason}");
        assert!(sent.is_empty(), "{bare}: nothing sent");
    }
    // An ordinary station still reads as before, and the suffix there is
    // refused with its reason.
    let station = client.device("5:0x05").await.unwrap();
    assert_eq!(station.address().to_string(), "5:0x05");
    assert!(!station.address().is_one_station());
    let error = client.device("5:0x05:one-station").await.err().unwrap();
    assert!(error.to_string().contains("lifts nothing"), "{error}");
    let error = client.device("5:1234:one-station").await.err().unwrap();
    assert!(
        error.to_string().contains("not a device instance"),
        "{error}"
    );
}
