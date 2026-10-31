//! The server and bacnet-device-core's `Responder`, answering the same
//! requests from the same fixture, send the same APDU octets (#2178).
//!
//! One database is built twice, the server's way: a Device declaring the
//! executed services, an Analog Input, a Binary Value, a Multi-state Value
//! with its state texts and a Group of the first two, both reading one fixed
//! clock. A `BACnetServer` on the shared `TestTransport` serves one; a
//! `Responder` with the server's resolved identity and limits serves the
//! other. Each request goes to both, and the APDU of the frame the server
//! sends is compared with the `Responder`'s, octet for octet; silence
//! matches silence. The server serves its executor-owned Device properties
//! from its view, and the `Responder`, with no overlay, from the Device
//! object, so those rows may differ: [`DIFFER`] pins exactly which do, and
//! [`DIFFER_EXCHANGES`] the one other exchange that differs by design.
use super::*;
use crate::server::test_transport::{self, SendLog, TestTransport};
use bacnet_device_core::read::ReadPropertyMultipleBudget;
use bacnet_device_core::responder::{Inbound, Limits, Outcome, Responder};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::EXECUTED_SERVICES;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::group::GroupObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P, Segmentation};
use bacnet_types::primitives::{Date, Time};
use std::collections::BTreeSet;
use tokio::sync::mpsc;

const DEVICE: u32 = 4321;

/// The ReadProperty rows that differ by design, as (object type, property,
/// index): the executor-owned Device properties, which the server serves
/// from its view and the `Responder`, with no overlay, from the object.
///
/// The server answers Active_COV_Subscriptions from its subscription table,
/// which holds the one subscription the test makes; the Device object knows
/// of none. Every other Device row agrees, the identity and services
/// included: both databases declare the executed services from one
/// identity.
const DIFFER: &[(ObjectType, P, Option<u32>)] =
    &[(ObjectType::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS, None)];

/// The other exchanges that differ by design, by the name the test gives
/// them. The core segments neither way, so a reply longer than the
/// requester's max_reply is aborted with SEGMENTATION_NOT_SUPPORTED even when
/// the requester accepts a segmented response; the server sends the first
/// segment.
const DIFFER_EXCHANGES: &[&str] = &["reply over max_reply, segmented response accepted"];

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 10,
                day: 30,
                day_of_week: 5,
            },
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn identity() -> bacnet_objects::DeviceIdentity {
    bacnet_objects::DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_name("parity-device")
}

fn database(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let mut group = GroupObject::new(1, "Group-1").unwrap();
    for member in [
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::BINARY_VALUE, 1),
    ] {
        group
            .add_member(ReadAccessSpecification {
                object_identifier: member,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                }],
            })
            .unwrap();
    }
    let identity = identity.clone().with_services(EXECUTED_SERVICES);
    let mut db = build_database_with_extra(
        &identity,
        vec![Box::new(ai), Box::new(bv), Box::new(msv), Box::new(group)],
    )
    .unwrap();
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db
}

struct Parity {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    responder: Responder,
    db: ObjectDatabase,
    requests: u16,
}

impl Parity {
    async fn start() -> Self {
        Self::start_with(ReadPropertyMultipleBudget::default(), Segmentation::NONE).await
    }

    /// Both sides within `budget`'s result-row limit, the server's Device
    /// claiming `segmentation`; the core's claims none, as it must.
    async fn start_with(budget: ReadPropertyMultipleBudget, segmentation: Segmentation) -> Self {
        let identity = identity();
        let (transport, tx) = TestTransport::inbound(16);
        let sent = transport.sent();
        let served = identity.clone().with_segmentation(segmentation);
        let config = ServerConfig {
            device: served.clone(),
            read_property_multiple_budget: budget,
            ..ServerConfig::default()
        };
        let server = BACnetServer::start(config, database(&served), transport)
            .await
            .unwrap();
        // Start installs the system clock; serve the fixed one, as the
        // Responder's database does.
        server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        let limits = Limits::new(
            server
                .config
                .read_property_multiple_budget
                .max_result_elements,
        );
        let core_identity = server
            .config
            .device
            .clone()
            .with_segmentation(Segmentation::NONE);
        let responder = Responder::new(core_identity, limits).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        sent.clear();
        Self {
            server,
            tx,
            sent,
            responder,
            db: database(&identity),
            requests: 0,
        }
    }

    /// Deliver `apdu` to the server alone, from a source of its own, and
    /// return what it sent.
    async fn deliver(&mut self, apdu: &Bytes) -> Vec<test_transport::SentFrame> {
        self.requests += 1;
        let [high, low] = self.requests.to_be_bytes();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: apdu[0] >> 4 == 0,
                payload: apdu.clone(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&[10, 0, high, low, 0xBA, 0xC0]),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        // The paused clock moves only once every server task is idle.
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.sent.take()
    }

    /// Deliver `apdu` to both, each request from a source of its own, and
    /// return the APDU each answered with, `None` for silence.
    async fn exchange(&mut self, apdu: Bytes, max_reply: u16) -> (Option<Bytes>, Option<Bytes>) {
        let frames = self.deliver(&apdu).await;
        assert!(frames.len() <= 1, "{apdu:02X?} drew {frames:?}");
        let server = frames.first().map(|frame| frame.decode_npdu().payload);
        let mut out = BytesMut::new();
        let request = Inbound::new(apdu, max_reply);
        let core = match self
            .responder
            .handle(&mut self.db, request, &mut out)
            .outcome
        {
            Outcome::Reply | Outcome::Announce => Some(out.freeze()),
            Outcome::Silent => None,
            other => unreachable!("handle answers every request: {other:?}"),
        };
        (server, core)
    }

    /// [`exchange`](Self::exchange), requiring the same answer from both.
    async fn same(&mut self, what: &str, apdu: Bytes, max_reply: u16) -> Option<Apdu> {
        let (server, core) = self.exchange(apdu, max_reply).await;
        assert_eq!(server, core, "{what}: server, then core");
        server.map(|apdu| bacnet_encoding::apdu::decode_apdu(apdu).unwrap())
    }

    /// [`exchange`](Self::exchange) for one of the [`DIFFER_EXCHANGES`],
    /// requiring the answers to differ; the server's, then the core's.
    async fn differ(
        &mut self,
        what: &str,
        apdu: Bytes,
        max_reply: u16,
    ) -> (Option<Apdu>, Option<Apdu>) {
        assert!(DIFFER_EXCHANGES.contains(&what), "{what} isn't pinned");
        let (server, core) = self.exchange(apdu, max_reply).await;
        assert_ne!(server, core, "{what}: pinned, but the answers agree");
        let decode = |apdu: Option<Bytes>| {
            apdu.map(|apdu| bacnet_encoding::apdu::decode_apdu(apdu).unwrap())
        };
        (decode(server), decode(core))
    }
}

fn confirmed(invoke_id: u8, service: ConfirmedServiceChoice, body: &[u8], max: u16) -> Bytes {
    confirmed_with(invoke_id, service, body, max, false, false)
}

fn confirmed_with(
    invoke_id: u8,
    service: ConfirmedServiceChoice,
    body: &[u8],
    max_apdu_length: u16,
    segmented: bool,
    segmented_response_accepted: bool,
) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented,
            more_follows: segmented,
            segmented_response_accepted,
            max_segments: None,
            max_apdu_length,
            invoke_id,
            sequence_number: segmented.then_some(0),
            proposed_window_size: segmented.then_some(1),
            service_choice: service,
            service_request: Bytes::copy_from_slice(body),
        }),
    )
    .unwrap();
    apdu.freeze()
}

fn read_property(
    invoke_id: u8,
    object: ObjectIdentifier,
    property: P,
    index: Option<u32>,
) -> Bytes {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut body);
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        &body,
        1476,
    )
}

fn unconfirmed(service: UnconfirmedServiceChoice, body: BytesMut) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: service,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    apdu.freeze()
}

fn who_is(range: Option<(u32, u32)>) -> Bytes {
    let mut body = BytesMut::new();
    WhoIsRequest {
        range: range.map(|(low, high)| DeviceInstanceRange::new(low, high).unwrap()),
    }
    .encode(&mut body);
    unconfirmed(UnconfirmedServiceChoice::WHO_IS, body)
}

fn who_has(object: WhoHasObject) -> Bytes {
    let mut body = BytesMut::new();
    WhoHasRequest {
        range: None,
        object,
    }
    .encode(&mut body)
    .unwrap();
    unconfirmed(UnconfirmedServiceChoice::WHO_HAS, body)
}

fn error_of(reply: Option<Apdu>) -> (ErrorClass, ErrorCode) {
    match reply {
        Some(Apdu::Error(error)) => (error.error_class, error.error_code),
        other => panic!("expected an Error PDU, got {other:?}"),
    }
}

/// Every ReadProperty row: each property of each object, the Device's
/// server-owned lists included, with no index, index 0 and index 1.
fn rows(db: &ObjectDatabase) -> Vec<(ObjectIdentifier, P, Option<u32>)> {
    let mut rows = Vec::new();
    for (object, stored) in db.iter_objects() {
        let mut properties: Vec<P> = stored.property_list().to_vec();
        properties.push(P::PROPERTY_LIST);
        if object.object_type() == ObjectType::DEVICE {
            properties.extend([
                P::PROTOCOL_SERVICES_SUPPORTED,
                P::ACTIVE_COV_SUBSCRIPTIONS,
                P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
                P::DEVICE_ADDRESS_BINDING,
            ]);
        }
        properties.sort_by_key(|property| property.to_raw());
        properties.dedup();
        for property in properties {
            for index in [None, Some(0), Some(1)] {
                rows.push((object, property, index));
            }
        }
    }
    rows.sort_by_key(|(object, property, index)| {
        (
            object.object_type().to_raw(),
            object.instance_number(),
            property.to_raw(),
            *index,
        )
    });
    rows
}

#[tokio::test(start_paused = true)]
async fn read_property_rows_match_but_the_pinned_device_rows() {
    let mut parity = Parity::start().await;
    // One live subscription, so the server's Active_COV_Subscriptions has
    // an entry the Device object lacks.
    let mut body = BytesMut::new();
    bacnet_services::cov::SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(0),
    }
    .encode(&mut body)
    .unwrap();
    let subscribe = confirmed(0, ConfirmedServiceChoice::SUBSCRIBE_COV, &body, 1476);
    let sent = parity.deliver(&subscribe).await;
    assert!(
        sent.iter()
            .any(|frame| matches!(frame.apdu(), Apdu::SimpleAck(_))),
        "{sent:?}"
    );
    let mut differ = BTreeSet::new();
    let rows = rows(&parity.db);
    // Five objects, each with no fewer than ten properties.
    assert!(rows.len() > 5 * 10 * 3, "{} rows", rows.len());
    for (n, (object, property, index)) in rows.into_iter().enumerate() {
        let apdu = read_property(n as u8, object, property, index);
        let (server, core) = parity.exchange(apdu, 1476).await;
        assert!(
            server.is_some(),
            "{object:?} {property:?} {index:?} drew no answer"
        );
        if server != core {
            differ.insert((object.object_type().to_raw(), property.to_raw(), index));
        }
    }
    let pinned: BTreeSet<_> = DIFFER
        .iter()
        .map(|&(object_type, property, index)| (object_type.to_raw(), property.to_raw(), index))
        .collect();
    let named = |set: &BTreeSet<(u32, u32, Option<u32>)>| -> Vec<String> {
        set.iter()
            .map(|&(t, p, i)| format!("{:?} {:?} {i:?}", ObjectType::from_raw(t), P::from_raw(p)))
            .collect()
    };
    assert_eq!(
        named(&differ),
        named(&pinned),
        "differing rows, then pinned rows"
    );
    parity.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn refusals_and_discovery_match() {
    let mut parity = Parity::start().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let read = |object, property, index| read_property(1, object, property, index);

    let unknown_object = read(oid(ObjectType::ANALOG_INPUT, 99), P::PRESENT_VALUE, None);
    let reply = parity.same("unknown object", unknown_object, 1476).await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
    );

    let unknown_property = read(ai, P::from_raw(9999), None);
    let reply = parity
        .same("unknown property", unknown_property, 1476)
        .await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );

    let not_an_array = read(ai, P::PRESENT_VALUE, Some(1));
    let reply = parity
        .same("index on a non-array", not_an_array, 1476)
        .await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );

    // The object identifier, then nothing: no property identifier.
    let malformed = confirmed(
        2,
        ConfirmedServiceChoice::READ_PROPERTY,
        &[0x0C, 0x00, 0x00, 0x00, 0x01],
        1476,
    );
    let reply = parity.same("malformed parameters", malformed, 1476).await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");

    let unknown_service = confirmed(3, ConfirmedServiceChoice::from_raw(60), &[], 1476);
    let reply = parity.same("unknown service", unknown_service, 1476).await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");

    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ai,
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut body);
    let segmented = confirmed_with(
        4,
        ConfirmedServiceChoice::READ_PROPERTY,
        &body,
        1476,
        true,
        false,
    );
    let reply = parity.same("segmented request", segmented, 1476).await;
    assert!(matches!(reply, Some(Apdu::Abort(_))), "{reply:?}");

    // The Device's Property_List takes more than the 50 octets the
    // requester takes.
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::DEVICE, DEVICE),
        property_identifier: P::PROPERTY_LIST,
        property_array_index: None,
    }
    .encode(&mut body);
    let oversize = confirmed(5, ConfirmedServiceChoice::READ_PROPERTY, &body, 50);
    let reply = parity.same("reply over max_reply", oversize, 50).await;
    assert!(matches!(reply, Some(Apdu::Abort(_))), "{reply:?}");

    let reply = parity.same("Who-Is, unbounded", who_is(None), 1476).await;
    assert!(
        matches!(reply, Some(Apdu::UnconfirmedRequest(_))),
        "{reply:?}"
    );
    let in_range = who_is(Some((DEVICE - 1, DEVICE + 1)));
    assert!(
        parity
            .same("Who-Is in range", in_range, 1476)
            .await
            .is_some()
    );
    let out_of_range = who_is(Some((1, 100)));
    assert!(
        parity
            .same("Who-Is out of range", out_of_range, 1476)
            .await
            .is_none()
    );

    let by_identifier = who_has(WhoHasObject::Identifier(oid(ObjectType::BINARY_VALUE, 1)));
    assert!(
        parity
            .same("Who-Has by identifier", by_identifier, 1476)
            .await
            .is_some()
    );
    let by_name = who_has(WhoHasObject::Name("MSV-1".into()));
    assert!(
        parity
            .same("Who-Has by name", by_name, 1476)
            .await
            .is_some()
    );
    let missing = who_has(WhoHasObject::Name("nowhere".into()));
    assert!(
        parity
            .same("Who-Has, no such object", missing, 1476)
            .await
            .is_none()
    );
    parity.server.stop().await.unwrap();
}

/// The edges: the work limit, both wildcard instances, malformed discovery,
/// APDUs that don't decode, and the one exchange pinned to differ.
#[tokio::test(start_paused = true)]
async fn edge_rows_match_and_the_segmented_reply_differs() {
    let mut parity = Parity::start().await;
    let read = |object, property| read_property(1, object, property, None);

    // The Device wildcard reads the configured Device on both sides.
    let wildcard = read(oid(ObjectType::DEVICE, 4_194_303), P::OBJECT_IDENTIFIER);
    let reply = parity.same("Device wildcard", wildcard, 1476).await;
    assert!(matches!(reply, Some(Apdu::ComplexAck(_))), "{reply:?}");
    // No port is registered, so the Network_Port wildcard names no object.
    let port = read(oid(ObjectType::NETWORK_PORT, 4_194_303), P::OBJECT_NAME);
    let reply = parity.same("Network_Port wildcard", port, 1476).await;
    assert_eq!(
        error_of(reply),
        (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
    );

    // A low limit with no high one; a Who-Has with no object.
    let who_is = unconfirmed(
        UnconfirmedServiceChoice::WHO_IS,
        BytesMut::from(&[0x09, 0x01][..]),
    );
    assert!(
        parity
            .same("malformed Who-Is", who_is, 1476)
            .await
            .is_none()
    );
    let who_has = unconfirmed(
        UnconfirmedServiceChoice::WHO_HAS,
        BytesMut::from(&[0xFF][..]),
    );
    assert!(
        parity
            .same("malformed Who-Has", who_has, 1476)
            .await
            .is_none()
    );

    // Cut off before the service choice, and a segmented one before its
    // sequence number: neither decodes, and both sides drop it.
    for (what, truncated) in [
        ("truncated confirmed request", &[0x00, 0x05, 6][..]),
        ("truncated segmented request", &[0x08, 0x05, 6][..]),
    ] {
        let apdu = Bytes::copy_from_slice(truncated);
        assert!(parity.same(what, apdu, 1476).await.is_none());
    }

    parity.server.stop().await.unwrap();

    // The requester takes a segmented response, and the server's Device
    // segments; only the server sends one.
    let mut parity =
        Parity::start_with(ReadPropertyMultipleBudget::default(), Segmentation::BOTH).await;
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::DEVICE, DEVICE),
        property_identifier: P::PROPERTY_LIST,
        property_array_index: None,
    }
    .encode(&mut body);
    let accepts = confirmed_with(
        7,
        ConfirmedServiceChoice::READ_PROPERTY,
        &body,
        50,
        false,
        true,
    );
    let (server, core) = parity
        .differ(
            "reply over max_reply, segmented response accepted",
            accepts,
            50,
        )
        .await;
    assert!(
        matches!(&server, Some(Apdu::ComplexAck(ack)) if ack.segmented),
        "{server:?}"
    );
    assert!(
        matches!(&core, Some(Apdu::Abort(abort))
            if abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED),
        "{core:?}"
    );
    parity.server.stop().await.unwrap();

    // A Group's Present_Value is its own row and two member rows: past a
    // limit of two, both sides abort with OUT_OF_RESOURCES.
    let mut parity = Parity::start_with(
        ReadPropertyMultipleBudget {
            max_result_elements: 2,
            ..ReadPropertyMultipleBudget::default()
        },
        Segmentation::NONE,
    )
    .await;
    let group = read(oid(ObjectType::GROUP, 1), P::PRESENT_VALUE);
    let reply = parity.same("Group past the work limit", group, 1476).await;
    assert!(
        matches!(&reply, Some(Apdu::Abort(abort))
            if abort.abort_reason == AbortReason::OUT_OF_RESOURCES),
        "{reply:?}"
    );
    parity.server.stop().await.unwrap();
}
