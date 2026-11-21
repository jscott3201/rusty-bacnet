//! Placement by address when the station-addressed Who-Is goes unanswered
//! (#2599, decision 2026-11-20): one Who-Is to the station's network
//! follows, only an I-Am from the station's own address answers it, and a
//! station found that way is asked on its network first from then on. An
//! MS/TP slave behind a slave proxy is the case: the proxy answers a Who-Is
//! its own device receives, for its slaves (Clause 16.10.2).
//!
//! The client's link is scripted: a router at station 3 answers each Who-Is
//! the client sends to network 5 as `answer` says, with I-Ams carrying
//! SNET 5 and the answering station's MAC there as SADR.

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::discover_tests::who_is_frames;
use super::test_bus::{client_on, mac};
use crate::OpsError;
use crate::client::{Client, Scope};

/// The station the tests look up: MAC 0x0A on network 5.
const STATION: u8 = 0x0A;
/// Another station on network 5.
const OTHER: u8 = 0x0B;

/// Which Who-Is a scripted network 5 answers, and from which station.
#[derive(Clone, Copy)]
enum Answer {
    /// The station answers a Who-Is addressed to it, as most do.
    Directed,
    /// Only a Who-Is to the network is answered for the station, as a
    /// slave proxy answers for its slave.
    NetworkOnly,
    /// Nothing answers.
    Silent,
    /// Only another station answers, and only the network-wide Who-Is.
    Other,
}

/// An I-Am for device `instance` from `sadr` on network 5, as the router
/// at station 3 passes it on.
fn routed_i_am(sadr: u8, instance: u32) -> ReceivedNpdu {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
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
    let mut npdu = BytesMut::new();
    let frame = Npdu {
        source: Some(NpduAddress {
            network: 5,
            mac_address: MacAddr::from_slice(&[sadr]),
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &frame).unwrap();
    ReceivedNpdu {
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&mac(3)),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        direct_response: None,
        reply_tx: None,
    }
}

/// A client at station 1 whose Who-Is to network 5 is answered as `answer`
/// says, and its sends.
async fn scripted(answer: Answer) -> (Client<TestTransport>, SendLog) {
    scripted_by(Arc::new(Mutex::new(answer))).await
}

/// [`scripted`], with the answer read at each send, so a test can change it.
async fn scripted_by(answer: Arc<Mutex<Answer>>) -> (Client<TestTransport>, SendLog) {
    let (inbox, rx) = mpsc::channel(16);
    let port = TestTransport::builder()
        .local_mac(&mac(1))
        .inbound(rx)
        .on_send(move |frame: SentFrame| {
            let npdu = frame.decode_npdu();
            let asked = npdu.destination.filter(|to| to.network == 5);
            let is_who_is = matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS);
            if let (Some(to), true) = (asked, is_who_is) {
                let directed = !to.mac_address.is_empty();
                let answer = *answer.lock().unwrap();
                let from = match (answer, directed) {
                    (Answer::Directed, true) | (Answer::NetworkOnly, false) => Some(STATION),
                    (Answer::Other, false) => Some(OTHER),
                    _ => None,
                };
                if let Some(from) = from {
                    inbox
                        .try_send(routed_i_am(from, 500 + u32::from(from)))
                        .unwrap();
                }
            }
            async { Ok(()) }
        })
        .build();
    client_on(port).await
}

/// Where each Who-Is in `log` went: `Some(mac)` for one addressed to that
/// station on network 5, `None` for one to the whole network. Cleared.
fn asked(log: &SendLog) -> Vec<Option<Vec<u8>>> {
    let asked = who_is_frames(log)
        .iter()
        .map(|frame| {
            let to = frame.decode_npdu().destination.expect("a remote Who-Is");
            assert_eq!(to.network, 5);
            (!to.mac_address.is_empty()).then(|| to.mac_address.to_vec())
        })
        .collect();
    log.clear();
    asked
}

const TARGET: &str = "5:0x0a";

/// A station that answers the Who-Is addressed to it needs no second one.
/// That Who-Is, with no router known yet, is a link broadcast whose NPDU
/// names the station: DNET 5, DLEN 1, DADR 0x0A, hop count 255 and no
/// SNET (Clause 6.5.3: a broadcast DA while the router is unknown).
#[tokio::test(start_paused = true)]
async fn a_station_that_answers_is_asked_once() {
    let (client, sent) = scripted(Answer::Directed).await;
    let device = client.device(TARGET).await.unwrap();
    let [frame] = &who_is_frames(&sent)[..] else {
        panic!("one Who-Is")
    };
    assert!(frame.broadcast);
    assert_eq!(
        frame.npdu[..],
        [0x01, 0x20, 0x00, 0x05, 0x01, STATION, 0xFF, 0x10, 0x08],
        "the directed Who-Is's NPDU and APDU"
    );
    assert_eq!(device.instance(), Some(500 + u32::from(STATION)));
    assert_eq!(asked(&sent), [Some(vec![STATION])]);
    assert!(!client.unanswered_recently(&device.address()));
}

/// A proxied slave answers only the Who-Is to its network: found on the
/// retry, and its next lookup asks the network first, once.
#[tokio::test(start_paused = true)]
async fn a_proxied_slave_is_found_on_the_retry_and_asked_on_its_network_next() {
    let (client, sent) = scripted(Answer::NetworkOnly).await;
    let device = client.device(TARGET).await.unwrap();
    assert_eq!(device.instance(), Some(500 + u32::from(STATION)));
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
    drop(device);
    client.low_level().clear_devices().await;
    let again = client.device(TARGET).await.unwrap();
    assert_eq!(again.instance(), Some(500 + u32::from(STATION)));
    assert_eq!(asked(&sent), [None], "the network form first, and alone");
    assert!(!client.unanswered_recently(&again.address()));
}

/// A proxied slave whose one I-Am is lost stays a network-form station: it
/// is unanswered for the window, and the lookup after the window asks its
/// network first and finds it at once (review of #2694).
#[tokio::test(start_paused = true)]
async fn a_lost_answer_keeps_a_proxied_slave_on_the_network_form() {
    let answer = Arc::new(Mutex::new(Answer::NetworkOnly));
    let (client, sent) = scripted_by(answer.clone()).await;
    client.device(TARGET).await.unwrap();
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
    client.low_level().clear_devices().await;
    *answer.lock().unwrap() = Answer::Silent;
    client.device(TARGET).await.map(drop).unwrap_err();
    assert_eq!(asked(&sent), [None], "still the network form, and alone");
    let station = crate::DeviceAddress::new(5, &[STATION]).unwrap();
    assert!(client.unanswered_recently(&station));
    tokio::time::sleep(super::MISS_MEMORY).await;
    assert!(!client.unanswered_recently(&station));
    *answer.lock().unwrap() = Answer::NetworkOnly;
    let again = client.device(TARGET).await.unwrap();
    assert_eq!(again.instance(), Some(500 + u32::from(STATION)));
    assert_eq!(
        asked(&sent),
        [None],
        "the network form first after the window"
    );
}

/// A silent station is asked exactly twice, then is unknown, said to have
/// been asked on its network, and remembered as unanswered for a while.
#[tokio::test(start_paused = true)]
async fn a_silent_station_is_asked_twice_then_unknown() {
    let (client, sent) = scripted(Answer::Silent).await;
    let error = client.device(TARGET).await.map(drop).unwrap_err();
    assert!(
        matches!(&error, OpsError::UnknownDevice { target, asked: Some(Scope::Network(5)) }
            if target.eq_ignore_ascii_case(TARGET)),
        "{error:?}"
    );
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
    let station = crate::DeviceAddress::new(5, &[STATION]).unwrap();
    assert!(client.unanswered_recently(&station));
    tokio::time::sleep(super::MISS_MEMORY).await;
    assert!(!client.unanswered_recently(&station));
    // A silent miss changes no later lookup's order.
    client.device(TARGET).await.map(drop).unwrap_err();
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
}

/// Another station's I-Am to the network-wide Who-Is is no answer for this
/// one, though the discovery table keeps it.
#[tokio::test(start_paused = true)]
async fn another_stations_answer_to_the_retry_is_not_taken() {
    let (client, sent) = scripted(Answer::Other).await;
    let error = client.device(TARGET).await.map(drop).unwrap_err();
    assert!(matches!(error, OpsError::UnknownDevice { .. }), "{error:?}");
    assert_eq!(asked(&sent), [Some(vec![STATION]), None]);
    let rows = client.low_level().discovered_devices().await;
    assert!(
        rows.iter().any(|row| row.source_network == Some(5)
            && row.source_address.as_deref() == Some(&[OTHER][..])),
        "{rows:?}"
    );
    // The station itself is a silent miss, not a network-only one.
    let station = crate::DeviceAddress::new(5, &[STATION]).unwrap();
    assert!(client.unanswered_recently(&station));
}
