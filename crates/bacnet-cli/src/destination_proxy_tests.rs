//! A routed MAC target the discovery table doesn't hold (#2599, decision
//! 2026-11-20): a Who-Is addressed to the station first, then one Who-Is to
//! its network if nothing answers, where only the station's own address
//! counts, as a slave proxy answers for its slave (Clause 16.10.2).

use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bytes::BytesMut;
use tokio::sync::mpsc;

use super::*;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
const STATION: u8 = 0x0A;
const OTHER: u8 = 0x0B;
const WAIT: Duration = Duration::from_secs(3);

/// Which Who-Is to network 5 the scripted router answers, and for whom.
#[derive(Clone, Copy)]
enum Answer {
    Directed,
    NetworkOnly,
    Other,
}

/// An I-Am for device `500 + sadr` from `sadr` on network 5, through the
/// router.
fn routed_i_am(sadr: u8) -> ReceivedNpdu {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 500 + u32::from(sadr))
            .unwrap(),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 42,
    }
    .encode(&mut service);
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let frame = Npdu {
        source: Some(NpduAddress {
            network: 5,
            mac_address: MacAddr::from_slice(&[sadr]),
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut npdu = BytesMut::new();
    encode_npdu(&mut npdu, &frame).unwrap();
    ReceivedNpdu::unverified(
        npdu.freeze(),
        MacAddr::from_slice(&ROUTER),
        false,
        Vec::new(),
        None,
    )
}

async fn scripted(answer: Answer) -> (BACnetClient<TestTransport>, SendLog) {
    let (inbox, inbound) = mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(inbound)
        .on_send(move |frame: SentFrame| {
            let to = frame.decode_npdu().destination.filter(|to| to.network == 5);
            let from = to.and_then(|to| match (answer, to.mac_address.is_empty()) {
                (Answer::Directed, false) | (Answer::NetworkOnly, true) => Some(STATION),
                (Answer::Other, true) => Some(OTHER),
                _ => None,
            });
            if let Some(from) = from {
                inbox.try_send(routed_i_am(from)).unwrap();
            }
            async { Ok(()) }
        })
        .build();
    let sent = transport.sent();
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    (client, sent)
}

/// Where each Who-Is went: the station's MAC, or `None` for its network.
fn asked(sent: &SendLog) -> Vec<Option<Vec<u8>>> {
    sent.frames()
        .iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS)
        })
        .map(|frame| {
            let to = frame.decode_npdu().destination.unwrap();
            (!to.mac_address.is_empty()).then(|| to.mac_address.to_vec())
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_station_that_answers_is_asked_once() {
    let (client, sent) = scripted(Answer::Directed).await;
    let found = resolve(&client, "5:0x0a", WAIT).await.unwrap();
    assert_eq!(found.instance, Some(500 + u32::from(STATION)));
    assert_eq!(asked(&sent), [Some(vec![STATION])]);
}

#[tokio::test(start_paused = true)]
async fn a_proxied_slave_is_found_on_its_networks_who_is() {
    let (client, sent) = scripted(Answer::NetworkOnly).await;
    let found = resolve(&client, "5:0x0a", WAIT).await.unwrap();
    assert_eq!(found.instance, Some(500 + u32::from(STATION)));
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
}

/// Another station's answer to the network's Who-Is is no answer for this
/// one: two Who-Is, then a miss naming both.
#[tokio::test(start_paused = true)]
async fn another_stations_answer_is_not_taken() {
    let (client, sent) = scripted(Answer::Other).await;
    let error = resolve(&client, "5:0x0a", WAIT)
        .await
        .map(drop)
        .unwrap_err();
    let each = WAIT.as_millis();
    assert!(
        error.contains(&format!(
            "answered neither a Who-Is addressed to it nor one on network 5, \
             waiting {each} ms for each ({} ms in all)",
            each * 2
        )),
        "{error}"
    );
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
}
