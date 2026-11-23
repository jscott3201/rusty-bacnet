//! Stations at an address the one-station mark lifts (#2738), named in the
//! target grammar: a handle made from `5:0xFF:one-station` sends a
//! confirmed request through its router to that DNET and DADR, where the
//! same text without the suffix is refused naming it.

use std::sync::Arc;

use bacnet_client::client::{BACnetClient, ClientConfig, Station};
use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_encoding::apdu::{Apdu, ErrorPdu, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, ObjectType, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::*;
use crate::limits::RequestLimits;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

/// One datagram from the router carrying `apdu`, from the station at
/// `network` and `mac`.
fn from_station(apdu: &Apdu, network: u16, mac: &[u8]) -> ReceivedNpdu {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let back = Npdu {
        source: Some(NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &back).unwrap();
    ReceivedNpdu::unverified(
        buf.freeze(),
        MacAddr::from_slice(&ROUTER),
        false,
        Vec::new(),
        None,
    )
}

/// An I-Am for `instance`.
fn i_am(instance: u32) -> Apdu {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut service_request);
    Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    })
}

/// The answers to `frame`. A Who-Is addressed to a station is answered by
/// its I-Am (instance 100) from the station, and, when the station is
/// 5/FF, also by one nobody asked, from 5/0012 (instance 101), both
/// through the router. A confirmed request is answered with an Error PDU
/// from the station it was addressed to. Anything else goes unanswered.
fn answer(frame: &SentFrame) -> Vec<ReceivedNpdu> {
    let npdu = frame.decode_npdu();
    let Some(to) = npdu.destination.clone() else {
        return Vec::new();
    };
    let station = (to.network, to.mac_address.to_vec());
    match decode_apdu(npdu.payload) {
        Ok(Apdu::UnconfirmedRequest(who_is))
            if who_is.service_choice == UnconfirmedServiceChoice::WHO_IS =>
        {
            let mut heard = vec![from_station(&i_am(100), station.0, &station.1)];
            if station == (5, vec![0xFF]) {
                heard.push(from_station(&i_am(101), 5, &[0x00, 0x12]));
            }
            heard
        }
        Ok(Apdu::ConfirmedRequest(request)) if !frame.broadcast => {
            let error = Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
                error_class: ErrorClass::OBJECT,
                error_code: ErrorCode::UNKNOWN_OBJECT,
                error_data: Bytes::new(),
            });
            vec![from_station(&error, station.0, &station.1)]
        }
        _ => Vec::new(),
    }
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
                for reply in answer(&frame) {
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

/// A station the table has no row for yet is asked for with a Who-Is
/// addressed to it, and the marked text is no obstacle to that: the
/// station answers, the handle is pinned there, and a read goes through the
/// router to its DNET and DADR (#2738).
#[tokio::test(start_paused = true)]
async fn a_one_station_target_the_table_lacks_is_asked_for() {
    let (client, sent) = site(&[]).await;
    let device = client.device("5:0xFF:one-station").await.unwrap();
    assert_eq!(device.address().to_string(), "5:0xFF:one-station");
    assert_eq!(device.instance(), Some(100));
    let asked = sent.frames();
    assert!(
        asked.iter().any(|frame| {
            frame
                .decode_npdu()
                .destination
                .is_some_and(|to| (to.network, to.mac_address.to_vec()) == (5, vec![0xFF]))
        }),
        "a Who-Is addressed to 5/FF went out"
    );
    sent.clear();
    let error = device.read("ai:1 pv").await.unwrap_err();
    assert!(matches!(error, OpsError::Refused { .. }), "{error:?}");
    let unicasts = sent.unicasts();
    let to = unicasts[0].decode_npdu().destination.unwrap();
    assert_eq!((to.network, to.mac_address.to_vec()), (5, vec![0xFF]));
}

/// A discovery directed at a station the caller marked lists the answer
/// from exactly that station under the marked address, and no other
/// answer from a lifted MAC (the mark is the caller's word, not the
/// I-Am's); the same scope unmarked is refused before anything is sent.
#[tokio::test(start_paused = true)]
async fn a_directed_discovery_lists_only_the_marked_station() {
    use crate::client::Discover;
    let (client, sent) = site(&[]).await;
    let marked = Station::remote(5, &[0xFF]).one_station();
    let found = client
        .discover(Discover::directed(marked).wait(Duration::from_secs(1)))
        .await
        .unwrap();
    let listed: Vec<_> = found
        .iter()
        .map(|found| (found.address.to_string(), found.instance))
        .collect();
    assert_eq!(listed, [("5:0xFF:one-station".to_string(), 100)]);
    sent.clear();
    let refused = client
        .discover(Discover::directed(Station::remote(5, &[0xFF])))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("5:0xFF:one-station"),
        "{refused}"
    );
    assert!(sent.is_empty());
}
