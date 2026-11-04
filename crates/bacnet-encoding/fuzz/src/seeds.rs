//! The seed inputs in `seeds/<target>/`: every file replayed through its
//! target's check, and the generated ones built here.
//!
//! A seed whose name is a 40-digit hexadecimal hash was harvested once from
//! a unit test's octets and is kept as found. Every other seed is built
//! below, mostly by encoding a value through the codec it seeds, and
//! [`generated_seeds_are_saved`] holds the files to what this module
//! builds: `SEEDS_WRITE=1 cargo test` writes them again. A minimized failing
//! input saved under `seeds/<target>/` is replayed like any other seed.

use std::fs;
use std::path::{Path, PathBuf};

use bacnet_encoding::apdu::{
    AbortPdu, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu, RejectPdu, SegmentAck, SimpleAck,
    UnconfirmedRequest, encode_apdu,
};
use bacnet_encoding::primitives::{encode_property_value, encode_timestamp_choice};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
    RejectReason, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use bytes::{Bytes, BytesMut};

mod constructed;
mod service;

/// One generated seed: its target, its file name and its octets.
pub(crate) struct Seed {
    pub(crate) target: &'static str,
    pub(crate) name: String,
    pub(crate) octets: Vec<u8>,
}

impl Seed {
    pub(crate) fn new(target: &'static str, name: impl Into<String>, octets: Vec<u8>) -> Self {
        Self {
            target,
            name: name.into(),
            octets,
        }
    }

    fn path(&self) -> PathBuf {
        directory(self.target).join(&self.name)
    }
}

fn directory(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("seeds")
        .join(target)
}

/// Every generated seed.
fn generated() -> Vec<Seed> {
    let mut seeds = apdu_seeds();
    seeds.extend(value_seeds());
    seeds.extend(constructed::seeds());
    seeds.extend(service::seeds());
    seeds
}

/// The selector of `label` in a codec table, followed by `body`.
pub(crate) fn selected(table: &[&str], label: &str, body: &[u8]) -> Vec<u8> {
    let index = table
        .iter()
        .position(|&name| name == label)
        .unwrap_or_else(|| panic!("no codec {label}"));
    let mut octets = vec![u8::try_from(index).expect("a selector octet")];
    octets.extend_from_slice(body);
    octets
}

/// `encode`'s output, from an empty buffer.
pub(crate) fn encoded(encode: impl FnOnce(&mut BytesMut)) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode(&mut buf);
    buf.to_vec()
}

pub(crate) fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).expect("an instance within 22 bits")
}

pub(crate) fn date() -> Date {
    Date {
        year: 126,
        month: 10,
        day: 30,
        day_of_week: 5,
    }
}

pub(crate) fn time() -> Time {
    Time {
        hour: 13,
        minute: 45,
        second: 7,
        hundredths: 50,
    }
}

fn apdu(apdu: &Apdu) -> Vec<u8> {
    encoded(|buf| encode_apdu(buf, apdu).expect("a valid APDU"))
}

/// ReadProperty of Present_Value, and its ACK, as service parameters.
fn read_property() -> (Bytes, Bytes) {
    let object = object(ObjectType::ANALOG_INPUT, 3);
    let request = ReadPropertyRequest {
        object_identifier: object,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let ack = ReadPropertyACK {
        object_identifier: object,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: encoded(|buf| {
            encode_property_value(buf, &PropertyValue::Real(21.5)).unwrap()
        }),
    };
    (
        encoded(|buf| request.encode(buf)).into(),
        encoded(|buf| ack.encode(buf).unwrap()).into(),
    )
}

/// One APDU of every PDU type, segmented headers included, and a run of
/// segments laid out as the reassembly check reads them.
fn apdu_seeds() -> Vec<Seed> {
    let (request, ack) = read_property();
    let confirmed = |segmented: bool, sequence: u8, more: bool| {
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented,
            more_follows: more,
            segmented_response_accepted: true,
            max_segments: Some(16),
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: segmented.then_some(sequence),
            proposed_window_size: segmented.then_some(4),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: request.clone(),
        })
    };
    let complex = |segmented: bool, sequence: u8, more: bool| {
        Apdu::ComplexAck(ComplexAck {
            segmented,
            more_follows: more,
            invoke_id: 7,
            sequence_number: segmented.then_some(sequence),
            proposed_window_size: segmented.then_some(4),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: ack.clone(),
        })
    };
    let pdus = [
        ("confirmed-request", confirmed(false, 0, false)),
        ("confirmed-request-segmented", confirmed(true, 3, true)),
        (
            "unconfirmed-who-is",
            Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: UnconfirmedServiceChoice::WHO_IS,
                service_request: Bytes::from_static(&[0x09, 0x01, 0x1A, 0x03, 0xE8]),
            }),
        ),
        (
            "simple-ack",
            Apdu::SimpleAck(SimpleAck {
                invoke_id: 9,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
        ),
        ("complex-ack", complex(false, 0, false)),
        ("complex-ack-segmented", complex(true, 1, true)),
        (
            "segment-ack-negative-server",
            Apdu::SegmentAck(SegmentAck {
                negative_ack: true,
                sent_by_server: true,
                invoke_id: 7,
                sequence_number: 2,
                actual_window_size: 4,
            }),
        ),
        (
            "error",
            Apdu::Error(ErrorPdu {
                invoke_id: 7,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
                error_data: Bytes::new(),
            }),
        ),
        (
            "reject",
            Apdu::Reject(RejectPdu {
                invoke_id: 7,
                reject_reason: RejectReason::INVALID_TAG,
            }),
        ),
        (
            "abort-server",
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id: 7,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            }),
        ),
    ];
    let mut seeds: Vec<Seed> = pdus
        .iter()
        .map(|(name, pdu)| Seed::new("apdu_decode", *name, apdu(pdu)))
        .collect();
    // Three segments of a reply, the middle one sent twice: a count octet
    // (three, less one), then each APDU behind its length.
    let mut layout = vec![2];
    for sequence in [0, 1, 1, 2] {
        let pdu = apdu(&complex(true, sequence, sequence < 2));
        layout.push(u8::try_from(pdu.len()).expect("a short segment"));
        layout.extend_from_slice(&pdu);
    }
    seeds.push(Seed::new(
        "apdu_decode",
        "reassembly-three-segments",
        layout,
    ));
    seeds
}

/// A run of every application type, with strings past the 253-octet edge
/// of a tag's short length forms; context tags nested past the 32 the walk
/// takes; and each timestamp form.
fn value_seeds() -> Vec<Seed> {
    let values = PropertyValue::List(vec![
        PropertyValue::Null,
        PropertyValue::Boolean(true),
        PropertyValue::Unsigned(0x1_0000_0000),
        PropertyValue::Signed(-32_769),
        PropertyValue::Real(f32::NAN),
        PropertyValue::Double(-0.0),
        PropertyValue::OctetString(vec![0xA5; 300]),
        PropertyValue::CharacterString("x".repeat(254)),
        PropertyValue::BitString {
            unused_bits: 3,
            data: vec![0xF8],
        },
        PropertyValue::Enumerated(70_000),
        PropertyValue::Date(date()),
        PropertyValue::Time(time()),
        PropertyValue::ObjectIdentifier(object(ObjectType::DEVICE, 4_194_302)),
    ]);
    let mut nested = vec![0x0E; 40];
    nested.push(0x21);
    nested.push(0x05);
    nested.extend(vec![0x0F; 40]);
    let mut seeds = vec![
        Seed::new(
            "value_decode",
            "every-application-type",
            encoded(|buf| encode_property_value(buf, &values).unwrap()),
        ),
        Seed::new("value_decode", "nested-40-opening-tags", nested),
    ];
    for (name, stamp) in [
        ("timestamp-time", BACnetTimeStamp::Time(time())),
        (
            "timestamp-sequence",
            BACnetTimeStamp::SequenceNumber(65_535),
        ),
        (
            "timestamp-date-time",
            BACnetTimeStamp::DateTime {
                date: date(),
                time: time(),
            },
        ),
    ] {
        let octets = encoded(|buf| encode_timestamp_choice(buf, &stamp).unwrap());
        seeds.push(Seed::new("value_decode", name, octets));
    }
    seeds
}

/// The saved files hold exactly the generated seeds (with `SEEDS_WRITE`
/// set, writes them first), and each generated service and constructed
/// seed decodes, so it reaches past its codec's parser.
#[test]
fn generated_seeds_are_saved() {
    let write = std::env::var_os("SEEDS_WRITE").is_some();
    for seed in generated() {
        let path = seed.path();
        if write {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &seed.octets).unwrap();
        }
        let saved = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; SEEDS_WRITE=1 cargo test writes it",
                path.display()
            )
        });
        assert_eq!(saved, seed.octets, "{} is stale", path.display());
        let (selector, rest) = crate::select(&seed.octets);
        match seed.target {
            "service_decode" => assert!(
                crate::service::decodes(usize::from(selector), rest),
                "{} doesn't decode",
                seed.name
            ),
            "constructed_decode" => assert!(
                crate::constructed::decodes(usize::from(selector), rest),
                "{} doesn't decode",
                seed.name
            ),
            _ => {}
        }
    }
}

/// Every seed file, generated or not, runs its target's check clean.
#[test]
fn every_seed_passes() {
    for (target, check) in [
        ("apdu_decode", crate::apdu::check as fn(&[u8])),
        ("value_decode", crate::value::check),
        ("constructed_decode", crate::constructed::check),
        ("service_decode", crate::service::check),
    ] {
        let mut replayed = 0;
        for entry in fs::read_dir(directory(target)).expect("the seeds directory") {
            let path = entry.expect("a directory entry").path();
            check(&fs::read(&path).expect("a seed input"));
            replayed += 1;
        }
        assert!(replayed > 0, "no seeds for {target}");
    }
}

/// Every codec of both tables has a seed.
#[test]
fn every_codec_has_a_seed() {
    for (target, count) in [
        ("constructed_decode", crate::constructed::CODECS.len()),
        ("service_decode", crate::service::CODECS.len()),
    ] {
        let mut seeded = vec![false; count];
        for entry in fs::read_dir(directory(target)).expect("the seeds directory") {
            let octets = fs::read(entry.unwrap().path()).unwrap();
            if let Some(&selector) = octets.first() {
                seeded[usize::from(selector) % count] = true;
            }
        }
        let missing: Vec<usize> = (0..count).filter(|&index| !seeded[index]).collect();
        assert!(
            missing.is_empty(),
            "{target}: no seed for selectors {missing:?}"
        );
    }
}
