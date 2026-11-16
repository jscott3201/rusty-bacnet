//! Property reads and writes, ranges, log pages and the generic requests.

use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest, ReadRangeValidation};
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::{BACnetAddress, PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};

use super::*;
use crate::client::station::Station;
use crate::log_reader::LogCursor;

/// 21.0 as an application-tagged REAL.
const REAL_21: [u8; 5] = [0x44, 0x41, 0xA8, 0x00, 0x00];

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn rp_ack(object: ObjectIdentifier) -> Answer {
    Answer::Complex(octets(|buf| {
        ReadPropertyACK {
            object_identifier: object,
            property_identifier: PV,
            property_array_index: None,
            property_value: REAL_21.to_vec(),
        }
        .encode(buf)
        .unwrap()
    }))
}

#[tokio::test]
async fn read_property() {
    let ack = rp_ack(ai1());
    pin!("rp/local", ack.clone(), |c| c.read_property(
        PEER,
        ai1(),
        PV,
        None
    ));
    pin!("rp/local", ack.clone(), |c| c.read_property(
        &c.resolve_device(LOCAL).await.unwrap(),
        ai1(),
        PV,
        None
    ));
    pin!("rp/routed", ack.clone(), |c| c.read_property(
        &c.resolve_device(REMOTE).await.unwrap(),
        ai1(),
        PV,
        None
    ));
    let routed = Station::routed(ROUTER, DNET, DADR);
    pin!("rp/routed", ack, |c| c.read_property(
        &routed,
        ai1(),
        PV,
        None
    ));
}

/// An answer naming another object fails the same check every way.
#[tokio::test]
async fn read_property_answer_is_checked() {
    let other = rp_ack(oid(ObjectType::ANALOG_INPUT, 2));
    pin!("rp-mismatch/local", other.clone(), |c| c.read_property(
        PEER,
        ai1(),
        PV,
        None
    ));
    let routed = Station::routed(ROUTER, DNET, DADR);
    pin!("rp-mismatch/routed", other, |c| c.read_property(
        &routed,
        ai1(),
        PV,
        None
    ));
}

fn rpm_specs() -> Vec<ReadAccessSpecification> {
    vec![ReadAccessSpecification {
        object_identifier: ai1(),
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PV,
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                property_array_index: Some(3),
            },
        ],
    }]
}

#[tokio::test]
async fn read_property_multiple() {
    let ack = Answer::Complex(octets(|buf| {
        ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ai1(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PV,
                    property_array_index: None,
                    property_value: Some(REAL_21.to_vec()),
                    error: None,
                }],
            }],
        }
        .encode(buf)
        .unwrap()
    }));
    pin!("rpm/local", ack.clone(), |c| c
        .read_property_multiple(PEER, rpm_specs()));
    pin!("rpm/local", ack.clone(), |c| c.read_property_multiple(
        &c.resolve_device(LOCAL).await.unwrap(),
        rpm_specs()
    ));
    pin!("rpm/routed", ack, |c| c.read_property_multiple(
        &c.resolve_device(REMOTE).await.unwrap(),
        rpm_specs()
    ));
}

/// 72.5 as an application-tagged REAL.
const REAL_72_5: [u8; 5] = [0x44, 0x42, 0x91, 0x00, 0x00];

#[tokio::test]
async fn write_property() {
    let ao1 = oid(ObjectType::ANALOG_OUTPUT, 1);
    let value = || REAL_72_5.to_vec();
    pin!("wp/local", Answer::Simple, |c| c.write_property(
        PEER,
        ao1,
        PV,
        None,
        value(),
        Some(8)
    ));
    pin!("wp/local", Answer::Simple, |c| c.write_property(
        &c.resolve_device(LOCAL).await.unwrap(),
        ao1,
        PV,
        None,
        value(),
        Some(8)
    ));
    pin!("wp/routed", Answer::Simple, |c| c.write_property(
        &c.resolve_device(REMOTE).await.unwrap(),
        ao1,
        PV,
        None,
        value(),
        Some(8)
    ));
    pin!("wp-error/local", Answer::Error, |c| c.write_property(
        PEER,
        ao1,
        PV,
        Some(2),
        value(),
        None
    ));
}

fn wpm_specs() -> Vec<WriteAccessSpecification> {
    vec![WriteAccessSpecification {
        object_identifier: oid(ObjectType::ANALOG_VALUE, 3),
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PV,
            property_array_index: None,
            value: REAL_72_5.to_vec(),
            priority: Some(10),
        }],
    }]
}

#[tokio::test]
async fn write_property_multiple() {
    pin!("wpm/local", Answer::Simple, |c| c
        .write_property_multiple(PEER, wpm_specs()));
    pin!("wpm/local", Answer::Simple, |c| c.write_property_multiple(
        &c.resolve_device(LOCAL).await.unwrap(),
        wpm_specs()
    ));
    pin!("wpm/routed", Answer::Simple, |c| c.write_property_multiple(
        &c.resolve_device(REMOTE).await.unwrap(),
        wpm_specs()
    ));
}

fn trend_log() -> ObjectIdentifier {
    oid(ObjectType::TREND_LOG, 1)
}

fn range_request() -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: trend_log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 2,
        }),
    }
}

/// A ReadRange-ACK of `items` empty-list items for the trend log.
fn range_ack(items: u32) -> Answer {
    Answer::Complex(octets(|buf| {
        ReadRangeAck {
            object_identifier: trend_log(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: (true, true, false),
            item_count: items,
            item_data: (0..items).flat_map(|_| [0x0E, 0x0F]).collect(),
            first_sequence_number: None,
        }
        .encode(buf)
    }))
}

#[tokio::test]
async fn read_range() {
    let range = || range_request().range;
    let log = trend_log();
    let buffer = PropertyIdentifier::LOG_BUFFER;
    pin!("rr/local", range_ack(2), |c| c.read_range(
        PEER,
        log,
        buffer,
        None,
        range()
    ));
    pin!("rr/local", range_ack(2), |c| c.read_range(
        &c.resolve_device(LOCAL).await.unwrap(),
        log,
        buffer,
        None,
        range()
    ));
    pin!("rr/routed", range_ack(2), |c| c.read_range(
        &c.resolve_device(REMOTE).await.unwrap(),
        log,
        buffer,
        None,
        range()
    ));
}

/// Three items for a request of two: refused strictly, kept leniently with
/// the rule it broke, every way.
#[tokio::test]
async fn read_range_answer_is_checked() {
    let request = range_request();
    let strict = ReadRangeValidation::Strict;
    let lenient = ReadRangeValidation::Lenient;
    pin!("rr-strict/local", range_ack(3), |c| c
        .read_range_with(PEER, &request, strict));
    pin!("rr-lenient/local", range_ack(3), |c| c
        .read_range_with(PEER, &request, lenient));
    pin!("rr-strict/routed", range_ack(3), |c| c.read_range_with(
        &c.resolve_device(REMOTE).await.unwrap(),
        &request,
        strict
    ));
    pin!("rr-lenient/routed", range_ack(3), |c| c.read_range_with(
        &c.resolve_device(REMOTE).await.unwrap(),
        &request,
        lenient
    ));
}

/// A page from a sequence number starts with its ReadRange; the device's
/// error ends it.
#[tokio::test]
async fn read_log_page() {
    let log = trend_log();
    let from = LogCursor::Sequence(5);
    pin!("log/local", Answer::Error, |c| c
        .read_log_page(PEER, log, from, 2));
    pin!("log/local", Answer::Error, |c| c.read_log_page(
        &c.resolve_device(LOCAL).await.unwrap(),
        log,
        from,
        2
    ));
    pin!("log/routed", Answer::Error, |c| c.read_log_page(
        &c.resolve_device(REMOTE).await.unwrap(),
        log,
        from,
        2
    ));
}

fn station(network: u16, mac: &[u8]) -> BACnetAddress {
    BACnetAddress {
        network_number: network,
        mac_address: MacAddr::from_slice(mac),
    }
}

#[tokio::test]
async fn generic_requests() {
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    let data = [0x0C, 0x00, 0x00, 0x00, 0x01];
    pin!("generic/local", Answer::Simple, |c| c
        .confirmed_request(PEER, wp, &data));
    pin!("generic/local", Answer::Simple, |c| c.confirmed_request(
        &station(0, PEER),
        wp,
        &data
    ));
    let routed = Station::routed(ROUTER, DNET, DADR);
    pin!("generic/routed", Answer::Simple, |c| c
        .confirmed_request(&routed, wp, &data));
    pin!("generic/routed", Answer::Simple, |c| c.confirmed_request(
        &station(DNET, DADR),
        wp,
        &data
    ));

    let ts = UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;
    let data = [0xA4, 0x7E, 0x0B, 0x0F, 0x07, 0xB4, 0x0C, 0x00, 0x00, 0x00];
    pin!("ugeneric/local", |c| c.unconfirmed_request(PEER, ts, &data));
    pin!("ugeneric/local", |c| c.unconfirmed_request(
        &station(0, PEER),
        ts,
        &data
    ));
    pin!("ugeneric/routed", |c| c.unconfirmed_request(
        &station(DNET, DADR),
        ts,
        &data
    ));
}
