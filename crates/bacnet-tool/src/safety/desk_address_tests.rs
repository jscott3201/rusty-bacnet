//! The desk by address (#2560), against a scripted device on the shared
//! test transport: a write is read, gated and sent at the address it was
//! planned at, even once another device claims its instance; a route that
//! isn't there sends nothing; a ledger written before addresses loads and
//! reverts through its instance; and a revert after a restart goes to the
//! entry's address with no Who-Is.

use std::time::Duration;

use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::{Apdu, ComplexAck, SimpleAck, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::sync::mpsc;

use super::*;
use crate::safety::Tier;

const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
/// Where device 7 answers.
const A: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];
/// Another device that claims instance 7 too.
const B: [u8; 6] = [10, 0, 0, 3, 0xBA, 0xC0];
const DEVICE: u32 = 7;

struct Armed;

impl Caller for Armed {
    fn level(&self) -> Level {
        Level::Armed { dangerous: false }
    }
    fn refusal(&self) -> Option<String> {
        None
    }
}

fn context() -> Context {
    Context {
        profile: Some("lab".into()),
        transport: "BIP",
        local: "10.0.0.1:47808".into(),
    }
}

fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

fn at(mac: &[u8]) -> DeviceAddress {
    DeviceAddress::local(mac).unwrap()
}

/// Present_Value 5 at priority 12 on AO:1 of device 7 at `address`.
fn spec(address: DeviceAddress) -> WriteSpec {
    WriteSpec {
        address,
        device: DEVICE,
        object: ao1(),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        value: Some("5".into()),
        priority: Some(12),
    }
}

/// AO:1's pre-images as the scripted device reads them: 1.0, every slot
/// empty, a relinquish default of 0.0, in service.
fn ao1_ack() -> ReadPropertyMultipleACK {
    let element = |property, value: Vec<u8>| ReadResultElement {
        property_identifier: property,
        property_array_index: None,
        property_value: Some(value),
        error: None,
    };
    ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: ao1(),
            list_of_results: vec![
                element(
                    PropertyIdentifier::PRESENT_VALUE,
                    vec![0x44, 0x3F, 0x80, 0, 0],
                ),
                element(PropertyIdentifier::PRIORITY_ARRAY, vec![0x00; 16]),
                element(
                    PropertyIdentifier::RELINQUISH_DEFAULT,
                    vec![0x44, 0, 0, 0, 0],
                ),
                element(PropertyIdentifier::OUT_OF_SERVICE, vec![0x10]),
            ],
        }],
    }
}

/// The scripted device's answer to `frame`, from the MAC it went to: AO:1's
/// pre-images for a ReadPropertyMultiple, a SimpleAck for anything else
/// confirmed, nothing for a broadcast.
fn answer(frame: &SentFrame) -> Option<ReceivedNpdu> {
    if frame.broadcast {
        return None;
    }
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(frame.decode_npdu().payload) else {
        return None;
    };
    let reply = if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE {
        let mut ack = BytesMut::new();
        ao1_ack().encode(&mut ack).unwrap();
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: ack.freeze(),
        })
    } else {
        Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        })
    };
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &reply).unwrap();
    let mut npdu = BytesMut::new();
    let back = Npdu {
        payload: payload.freeze(),
        ..Npdu::default()
    };
    encode_npdu(&mut npdu, &back).unwrap();
    let source = frame.mac.clone();
    Some(ReceivedNpdu::unverified(
        npdu.freeze(),
        source,
        false,
        Vec::new(),
        None,
    ))
}

struct Lab {
    client: BACnetClient<TestTransport>,
    sent: SendLog,
    rate: RequestLimits,
}

/// A client whose every unicast is answered by the scripted device.
async fn lab() -> Lab {
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
    Lab {
        client,
        sent,
        rate: RequestLimits::new(0),
    }
}

/// The confirmed service each unicast carried, and where it went.
fn requests(sent: &SendLog) -> Vec<(ConfirmedServiceChoice, Vec<u8>)> {
    sent.unicasts()
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request) => Some((request.service_choice, frame.mac.to_vec())),
            _ => None,
        })
        .collect()
}

/// The write is planned at A. Before it is confirmed, B claims instance 7
/// too, so the instance names no one device: the re-read, the turn and the
/// write still go to A alone, and the entry keeps A.
#[tokio::test(start_paused = true)]
async fn a_write_goes_to_the_address_it_was_planned_at() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let preview = desk
        .prepare(
            &lab.client,
            &lab.rate,
            &Armed,
            &[spec(at(&A))],
            false,
            false,
        )
        .await
        .unwrap();
    assert_eq!(preview.address, at(&A));
    assert_eq!(preview.gate, Gate::Allowed(Tier::T1));

    lab.client.add_device(DEVICE, &B).await.unwrap();
    let contested = lab.client.get_device(DEVICE).await;
    assert!(
        matches!(contested, Err(Error::AmbiguousDevice { .. })),
        "{contested:?}"
    );

    let done = desk
        .execute(&lab.client, &lab.rate, &Armed, &preview, false)
        .await;
    assert_eq!(done[0].outcome, Outcome::Ack, "{done:?}");
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    assert_eq!(
        requests(&lab.sent),
        [(rpm, A.to_vec()), (rpm, A.to_vec()), (wp, A.to_vec())],
        "planned, read again and written at A, never at B"
    );
    let entry = &desk.entries()[0];
    assert_eq!(
        (entry.device, entry.address.as_ref()),
        (DEVICE, Some(&at(&A)))
    );
}

/// The write waits at its address's gate, the one the reads of that address
/// take: while another holds the turn there, nothing goes.
#[tokio::test(start_paused = true)]
async fn a_write_waits_at_its_addresss_gate() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let preview = desk
        .prepare(
            &lab.client,
            &lab.rate,
            &Armed,
            &[spec(at(&A))],
            false,
            false,
        )
        .await
        .unwrap();
    let held = lab.rate.turn(&at(&A)).await;
    let sent = lab.sent.clone();
    let release = async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let before = requests(&sent).len();
        drop(held);
        before
    };
    let (done, before) = tokio::join!(
        desk.commit(&lab.client, &lab.rate, &Armed, &preview),
        release
    );
    assert_eq!(
        before, 1,
        "only the pre-image read went while the turn was held"
    );
    assert_eq!(done[0].outcome, Outcome::Ack);
    assert_eq!(requests(&lab.sent).len(), 2);
}

/// A routed address the discovery table has no row for has no route: the
/// write is reported not sent, settled as not applied, and nothing goes out.
#[tokio::test(start_paused = true)]
async fn a_write_with_no_route_is_not_sent() {
    let lab = lab().await;
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let routed = DeviceAddress::new(2, &[0x05]).unwrap();
    let preview = desk
        .prepare(
            &lab.client,
            &lab.rate,
            &Armed,
            &[spec(at(&A))],
            false,
            false,
        )
        .await
        .unwrap();
    let preview = Preview {
        address: routed.clone(),
        items: preview
            .items
            .into_iter()
            .map(|mut item| {
                item.spec.address = routed.clone();
                item
            })
            .collect(),
        ..preview
    };
    lab.sent.clear();
    let done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
    assert!(
        matches!(&done[0].outcome, Outcome::NotSent(why) if why.contains("2:0x05")),
        "{:?}",
        done[0].outcome
    );
    assert!(desk.entries().is_empty(), "settled as not applied");
    assert!(lab.sent.is_empty(), "nothing went out");
}

/// A ledger as a release before #2560 saved it: an entry with the device's
/// instance and no address, field for field as that release's `Entry`
/// serialised (an empty slot written at 12).
const LEDGER_BEFORE_ADDRESSES: &str = r#"{
  "version": 1,
  "entries": [
    {
      "id": 1,
      "run": "4242-1763000000000",
      "at": "2026-11-13T10:00:00.000Z",
      "device": 7,
      "object_type": 1,
      "instance": 1,
      "property": 85,
      "index": null,
      "priority": 12,
      "commandable": true,
      "before": null,
      "written": {
        "bytes": "4440a00000",
        "text": "5"
      },
      "state": "applied",
      "kept": false,
      "seq": 1
    }
  ]
}"#;

/// The old ledger loads. Its entry has no address, so the revert finds the
/// device through its instance, as before: with the table holding it, no
/// Who-Is goes and the slot is relinquished at its one address; with the
/// table empty, a Who-Is for the instance goes first.
#[tokio::test(start_paused = true)]
async fn a_ledger_from_before_addresses_loads_and_reverts_by_instance() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(super::super::ledger::FILE),
        LEDGER_BEFORE_ADDRESSES,
    )
    .unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let entry = &desk.entries()[0];
    assert_eq!((entry.device, entry.address.as_ref()), (DEVICE, None));
    assert_eq!(entry.label(), "device 7 analog-output 1 present-value @12");

    // Nobody claims 7 yet: one Who-Is for it, then not sent.
    let lab = lab().await;
    let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
    assert!(matches!(done[0].1, Outcome::NotSent(_)), "{done:?}");
    let broadcasts = lab.sent.broadcasts();
    assert_eq!(broadcasts.len(), 1);
    assert!(matches!(
        broadcasts[0].apdu(),
        Apdu::UnconfirmedRequest(ref who) if who.service_choice == UnconfirmedServiceChoice::WHO_IS
    ));
    assert!(lab.sent.unicasts().is_empty());

    // Known at A: relinquished there, no Who-Is.
    lab.sent.clear();
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
    assert_eq!(done[0].1, Outcome::Ack, "{done:?}");
    assert!(lab.sent.broadcasts().is_empty());
    let frames = lab.sent.unicasts();
    let [write] = frames.as_slice() else {
        panic!("{frames:?}");
    };
    assert_eq!(write.mac.as_slice(), A);
    assert!(desk.entries().is_empty(), "undone");
}

/// After a restart the discovery table is empty. An entry this release
/// wrote keeps its address: a local one is reverted there with no Who-Is,
/// and a routed one with no row asks only its own network for the
/// instance, then is reported not sent.
#[tokio::test(start_paused = true)]
async fn a_revert_after_a_restart_goes_to_the_entrys_address() {
    let dir = tempfile::tempdir().unwrap();
    {
        let lab = lab().await;
        lab.client.add_device(DEVICE, &A).await.unwrap();
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let preview = desk
            .prepare(
                &lab.client,
                &lab.rate,
                &Armed,
                &[spec(at(&A))],
                false,
                false,
            )
            .await
            .unwrap();
        let done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
        assert_eq!(done[0].outcome, Outcome::Ack);
    }
    let text = std::fs::read_to_string(dir.path().join(super::super::ledger::FILE)).unwrap();
    assert!(text.contains("\"address\": \"10.0.0.2:47808\""), "{text}");

    let lab = lab().await;
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
    assert_eq!(done[0].1, Outcome::Ack, "{done:?}");
    assert!(lab.sent.broadcasts().is_empty(), "no Who-Is");
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    assert_eq!(requests(&lab.sent), [(wp, A.to_vec())]);

    // A routed entry: one Who-Is, to its network, and nothing else.
    let routed = DeviceAddress::new(2, &[0x05]).unwrap();
    let text = text.replace("\"10.0.0.2:47808\"", "\"2:0x05\"");
    std::fs::write(dir.path().join(super::super::ledger::FILE), text).unwrap();
    drop(desk);
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    assert_eq!(desk.entries()[0].address, Some(routed));
    lab.sent.clear();
    let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
    assert!(
        matches!(&done[0].1, Outcome::NotSent(why) if why.contains("2:0x05")),
        "{done:?}"
    );
    let broadcasts = lab.sent.broadcasts();
    let [who_is] = broadcasts.as_slice() else {
        panic!("{broadcasts:?}");
    };
    let to = who_is.decode_npdu().destination.unwrap();
    assert_eq!(to.network, 2);
    assert!(lab.sent.unicasts().is_empty());
}
