//! Who-Is and Who-Has answers hold no database guard across their send
//! (#2199). A held send must not keep a database writer, or the readers
//! queued behind it, waiting.

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest as UnconfirmedRequestPdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, UnconfirmedServiceChoice};

use super::*;
use crate::server::test_transport::{TestTransport, TestTransportHandle};

const PEER: [u8; 4] = [0x0A, 0x00, 0x00, 0x02];
/// Long enough that only a write stuck behind the reply's guard reaches it;
/// the paused clock jumps there at once when nothing else can run.
const STUCK: Duration = Duration::from_secs(30);

fn received(service_choice: UnconfirmedServiceChoice, service_request: Bytes) -> ReceivedNpdu {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice,
            service_request,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn who_is() -> ReceivedNpdu {
    let mut request = BytesMut::new();
    WhoIsRequest {
        range: DeviceInstanceRange::from_limits(None, None).unwrap(),
    }
    .encode(&mut request);
    received(UnconfirmedServiceChoice::WHO_IS, request.freeze())
}

fn who_has() -> ReceivedNpdu {
    let mut request = BytesMut::new();
    WhoHasRequest {
        range: DeviceInstanceRange::from_limits(None, None).unwrap(),
        object: WhoHasObject::Name("Zone Temp".into()),
    }
    .encode(&mut request)
    .unwrap();
    received(UnconfirmedServiceChoice::WHO_HAS, request.freeze())
}

struct Harness {
    server: BACnetServer<TestTransport>,
    link: TestTransportHandle,
    tx: mpsc::Sender<ReceivedNpdu>,
}

async fn harness() -> Harness {
    let (tx, rx) = mpsc::channel(16);
    let transport = TestTransport::builder()
        .local_mac(&[0x0A, 0x00, 0x00, 0x01])
        .inbound(rx)
        .build();
    let link = transport.handle();
    let identity = bacnet_objects::DeviceIdentity::new(1234, 42).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("TestDevice"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    // No throttling or coalescing: every request here draws its answer.
    let policy = DiscoveryPolicy {
        max_responses_per_sec_global: 1_000,
        max_responses_per_sec_per_source: 1_000,
        global_burst_capacity: 1_000,
        source_burst_capacity: 1_000,
        coalesce_window: Duration::ZERO,
        prefer_directed_responses: true,
        ..Default::default()
    };
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .discovery_policy(policy)
        .build()
        .await
        .unwrap();
    Harness { server, link, tx }
}

fn answers(server: &BACnetServer<TestTransport>) -> u64 {
    let counters = server.discovery_counters();
    counters.i_am_sent + counters.i_have_sent
}

async fn answered(server: &BACnetServer<TestTransport>, count: u64) {
    tokio::time::timeout(STUCK, async {
        while answers(server) < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the answer should be sent");
}

/// Answer `request` once freely, for reference, then again with the link
/// holding the send: a database write must finish while the second answer is
/// held, and that answer must match the first byte for byte.
async fn write_finishes_while_the_answer_is_held(request: fn() -> ReceivedNpdu) {
    let mut h = harness().await;
    let sent = h.link.sent();
    h.tx.send(request()).await.unwrap();
    answered(&h.server, 1).await;
    let reference = sent.frame(0).npdu;

    h.link.block_next_send();
    h.tx.send(request()).await.unwrap();
    tokio::time::timeout(STUCK, h.link.wait_blocked())
        .await
        .expect("the second answer should reach the link");
    let mut db = tokio::time::timeout(STUCK, h.server.database().write())
        .await
        .expect("a database write must not wait for the held answer");
    db.add(Box::new(
        AnalogValueObject::new(9, "Added", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    drop(db);
    assert_eq!(answers(&h.server), 1, "the second answer is still held");

    h.link.release_sends(1);
    answered(&h.server, 2).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent.frame(1).npdu, reference);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_held_i_am_send_does_not_hold_the_database() {
    write_finishes_while_the_answer_is_held(who_is).await;
}

#[tokio::test(start_paused = true)]
async fn a_held_i_have_send_does_not_hold_the_database() {
    write_finishes_while_the_answer_is_held(who_has).await;
}
