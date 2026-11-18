//! A server whose time-synchronization policy is off, or accepts no source,
//! does not list TimeSynchronization or UTCTimeSynchronization in the
//! Device's Protocol_Services_Supported (#2617; Clause 12.11.14 names what
//! the device executes, and Clauses 16.7 and 16.8 define the two services).
//! The policy is fixed when the server starts, so a read has nothing to
//! follow at run time. Reads are made on the wire, by ReadProperty, by
//! ReadPropertyMultiple and by ReadRange of a Group, over the shared test
//! transport on paused time.
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::group::GroupObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::ServiceSupported;
use std::collections::BTreeSet;

const DEVICE: u32 = 4321;
const PEER: [u8; 1] = [0x11];
const LOCAL: ServiceSupported = ServiceSupported::TIME_SYNCHRONIZATION;
const UTC: ServiceSupported = ServiceSupported::UTC_TIME_SYNCHRONIZATION;

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap()
}

fn off() -> TimeSyncPolicy {
    TimeSyncPolicy {
        enabled: false,
        ..TimeSyncPolicy::default()
    }
}

/// On, with every restriction the policy has: an allowlist of `sources`, a
/// zero step cap, rates, coalescing and a one-entry source table.
fn restricted(sources: Vec<TimeSyncSource>) -> TimeSyncPolicy {
    let rate = TimeSyncRateLimit {
        max_per_second: 1.0,
        burst_capacity: 1,
    };
    TimeSyncPolicy {
        enabled: true,
        source_restriction: Some(TimeSyncSourceRestriction::new(sources).unwrap()),
        max_step: Some(Duration::ZERO),
        per_source_rate: Some(rate),
        global_rate: Some(rate),
        coalesce_window: Duration::from_secs(1),
        global_coalesce_window: Duration::from_secs(1),
        max_sources: 1,
    }
}

struct Lab {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
}

fn group() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::GROUP, 7).unwrap()
}

/// A server whose database holds Group 7, whose one member is the Device's
/// Protocol_Services_Supported.
async fn lab(policy: TimeSyncPolicy, clock: bool) -> Lab {
    let identity = bacnet_objects::DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_name("time-sync-services");
    let mut group = GroupObject::new(group().instance_number(), "GRP-7").unwrap();
    group
        .add_member(ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
                property_array_index: None,
            }],
        })
        .unwrap();
    let db = build_database_with_extra(&identity, vec![Box::new(group)]).unwrap();
    let config = ServerConfig {
        device: identity,
        time_sync_policy: policy,
        ..ServerConfig::default()
    };
    let (transport, tx) = TestTransport::inbound(16);
    let sent = transport.sent();
    let server = if clock {
        BACnetServer::start(config, db, transport).await
    } else {
        BACnetServer::start_clockless(config, db, transport).await
    }
    .unwrap();
    Lab { server, tx, sent }
}

fn from_peer(invoke_id: u8, service: ConfirmedServiceChoice, body: &[u8]) -> ReceivedNpdu {
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            ..Default::default()
        },
    )
    .unwrap();
    encode_apdu(
        &mut npdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: Bytes::copy_from_slice(body),
        }),
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

/// The APDUs sent to the peer so far.
fn answers(lab: &Lab) -> Vec<Apdu> {
    lab.sent
        .frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == PEER)
        .map(|frame| {
            let npdu = decode_npdu(frame.npdu).unwrap();
            bacnet_encoding::apdu::decode_apdu(npdu.payload).unwrap()
        })
        .collect()
}

/// Send `body` and wait, a millisecond of paused time at a time, for the
/// ComplexAck that answers it.
async fn ack(lab: &Lab, service: ConfirmedServiceChoice, body: &BytesMut) -> Bytes {
    let seen = answers(lab).len();
    lab.tx.send(from_peer(1, service, body)).await.unwrap();
    for _ in 0..5_000 {
        if let Some(Apdu::ComplexAck(ack)) = answers(lab).into_iter().nth(seen) {
            return ack.service_ack;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("no answer to {service:?}");
}

/// The octets of a Protocol_Services_Supported value: 49 bits in seven
/// octets.
fn octets(value: &[u8]) -> Vec<u8> {
    let (value, end) = bacnet_encoding::primitives::decode_application_value(value, 0).unwrap();
    assert_eq!(
        end, 10,
        "tag, length, unused-bits octet and seven data octets"
    );
    match value {
        PropertyValue::BitString { unused_bits, data } => {
            assert_eq!((unused_bits, data.len()), (7, 7));
            data
        }
        other => panic!("{other:?}"),
    }
}

/// The Device's `property`, as ReadProperty serves it.
async fn device_property(lab: &Lab, property: PropertyIdentifier) -> PropertyValue {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut body);
    let service_ack = ack(lab, ConfirmedServiceChoice::READ_PROPERTY, &body).await;
    let value = ReadPropertyACK::decode(&service_ack)
        .unwrap()
        .property_value;
    bacnet_encoding::primitives::decode_application_value(&value, 0)
        .unwrap()
        .0
}

async fn by_read_property(lab: &Lab) -> Vec<u8> {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
        property_array_index: None,
    }
    .encode(&mut body);
    let service_ack = ack(lab, ConfirmedServiceChoice::READ_PROPERTY, &body).await;
    octets(
        &ReadPropertyACK::decode(&service_ack)
            .unwrap()
            .property_value,
    )
}

/// By ReadRange of the Group's Present_Value, whose one item is its member's
/// row.
async fn by_read_range(lab: &Lab) -> Vec<u8> {
    let mut body = BytesMut::new();
    ReadRangeRequest {
        object_identifier: group(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        range: None,
    }
    .encode(&mut body)
    .unwrap();
    let service_ack = ack(lab, ConfirmedServiceChoice::READ_RANGE, &body).await;
    let items = ReadRangeAck::decode(&service_ack).unwrap().item_data;
    let ack = ReadPropertyMultipleACK::decode(&items).unwrap();
    let [result] = ack.list_of_read_access_results.as_slice() else {
        panic!("one member: {ack:?}")
    };
    let [row] = result.list_of_results.as_slice() else {
        panic!("one row: {result:?}")
    };
    assert_eq!(result.object_identifier, device());
    octets(row.property_value.as_deref().unwrap())
}

/// By ReadPropertyMultiple, naming the property twice: both rows agree.
async fn by_rpm(lab: &Lab) -> Vec<u8> {
    let reference = PropertyReference {
        property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
        property_array_index: None,
    };
    let mut body = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![reference; 2],
        }],
    }
    .encode(&mut body)
    .unwrap();
    let service_ack = ack(lab, ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, &body).await;
    let ack = ReadPropertyMultipleACK::decode(&service_ack).unwrap();
    let rows = &ack.list_of_read_access_results[0].list_of_results;
    assert_eq!(rows.len(), 2);
    let first = octets(rows[0].property_value.as_deref().unwrap());
    assert_eq!(octets(rows[1].property_value.as_deref().unwrap()), first);
    first
}

/// The bit numbers a read lists.
fn listed(data: &[u8]) -> BTreeSet<u8> {
    let supported = ServicesSupported::from_bacnet(data);
    (0..49)
        .filter(|bit| supported.contains(ServiceSupported::from_raw(*bit)))
        .collect()
}

/// One row of [`the_time_sync_bits_follow_the_policy_on_the_wire`]: the
/// bits but the two time-synchronization ones, which are `expected`, after
/// ReadProperty, ReadPropertyMultiple and a Group's ReadRange agree. With a
/// clock the Device serves Local_Date and Local_Time whatever the policy.
async fn row(name: &str, policy: TimeSyncPolicy, clock: bool, expected: bool) -> BTreeSet<u8> {
    let lab = lab(policy, clock).await;
    let mut read = listed(&by_read_property(&lab).await);
    assert_eq!(
        listed(&by_rpm(&lab).await),
        read,
        "{name}: ReadProperty and ReadPropertyMultiple agree"
    );
    assert_eq!(
        listed(&by_read_range(&lab).await),
        read,
        "{name}: ReadProperty and a Group's ReadRange agree"
    );
    for service in [LOCAL, UTC] {
        assert_eq!(read.remove(&service.to_raw()), expected, "{name}");
    }
    assert!(read.contains(&ServiceSupported::READ_PROPERTY.to_raw()));
    if clock {
        assert!(matches!(
            device_property(&lab, PropertyIdentifier::LOCAL_DATE).await,
            PropertyValue::Date(_)
        ));
        assert!(matches!(
            device_property(&lab, PropertyIdentifier::LOCAL_TIME).await,
            PropertyValue::Time(_)
        ));
    }
    read
}

/// A clock with the default policy lists both services; a policy that is
/// off lists neither, with every other bit as the default's; a server with
/// no clock lists neither, as before (#834). An enabled policy with every
/// restriction keeps both while its allowlist names a source, since the
/// device executes the service for the peers it accepts, and lists neither
/// when the allowlist names nobody, since it then executes it for no peer.
#[tokio::test(start_paused = true)]
async fn the_time_sync_bits_follow_the_policy_on_the_wire() {
    let mut others: Option<BTreeSet<u8>> = None;
    let one = vec![TimeSyncSource::Direct(PEER.to_vec())];
    for (name, policy, clock, expected) in [
        ("default, clock", TimeSyncPolicy::default(), true, true),
        ("off, clock", off(), true, false),
        ("off, no clock", off(), false, false),
        ("default, no clock", TimeSyncPolicy::default(), false, false),
        ("allowlist of one, clock", restricted(one), true, true),
        (
            "allowlist of nobody, clock",
            restricted(vec![]),
            true,
            false,
        ),
    ] {
        let read = Box::pin(row(name, policy, clock, expected)).await;
        assert_eq!(
            *others.get_or_insert_with(|| read.clone()),
            read,
            "{name}: no other bit moves"
        );
    }
}

/// The local read and the PICS follow the same policy.
#[tokio::test(start_paused = true)]
async fn the_local_read_and_the_pics_follow_the_policy() {
    for (policy, expected) in [(TimeSyncPolicy::default(), true), (off(), false)] {
        let lab = lab(policy, true).await;
        let read = lab
            .server
            .read_local(
                &device(),
                PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
                None,
            )
            .await
            .unwrap();
        let PropertyValue::BitString { data, .. } = read else {
            panic!("{read:?}")
        };
        let supported = ServicesSupported::from_bacnet(&data);
        let pics = lab
            .server
            .generate_pics(&crate::pics::PicsConfig::default())
            .await;
        for (service, name) in [
            (LOCAL, "TimeSynchronization"),
            (UTC, "UTCTimeSynchronization"),
        ] {
            assert_eq!(supported.contains(service), expected, "{service:?}");
            let executed = pics
                .supported_services
                .iter()
                .find(|row| row.service_name == name)
                .is_some_and(|row| row.executor);
            assert_eq!(executed, expected, "PICS {name}");
        }
    }
}
