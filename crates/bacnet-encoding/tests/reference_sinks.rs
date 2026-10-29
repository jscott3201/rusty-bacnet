//! Independent reference wire literals and whole-call output refusal behavior.

// These compare with the BytesMut encoders, which need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::constructed::*;
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, PropertyReference,
};
use bytes::BytesMut;

#[path = "reference_sinks/cases.rs"]
mod cases;
use cases::{Case, all_cases, compatibility_cases, wire_cases};

const PREFIX: [u8; 3] = [0x12, 0x34, 0x56];
const REFUSAL: SinkFull = SinkFull {
    needed: 901,
    available: 902,
};

fn assert_literal(case: &Case) {
    let mut sink = Vec::from(PREFIX);
    assert_eq!(
        case.encode(&mut sink),
        Ok(case.expected.len()),
        "{}",
        case.name
    );
    assert_eq!(&sink[..3], &PREFIX);
    assert_eq!(&sink[3..], &case.expected, "{}", case.name);
    let mut legacy = BytesMut::from(&PREFIX[..]);
    case.encode_legacy(&mut legacy);
    assert_eq!(&legacy[..], &sink, "{}", case.name);
}

#[test]
fn independent_literal_layouts_and_legacy_signatures() {
    let _: fn(&mut BytesMut, &PropertyReference) = encode_property_reference;
    let _: fn(&mut BytesMut, &BACnetObjectPropertyReference) = encode_object_property_reference;
    let _: fn(&mut BytesMut, &BACnetObjectPropertyReference) = encode_setpoint_reference;
    let _: fn(&mut BytesMut, &BACnetDeviceObjectPropertyReference) =
        encode_device_object_property_reference;
    let _: fn(&mut BytesMut, &BACnetDeviceObjectReference) = encode_device_object_reference;
    let cases = wire_cases();
    assert_eq!(cases.len(), 17);
    for case in &cases {
        assert_literal(case);
    }
}

#[test]
fn raw_unsigned_widths_and_identifier_compatibility_keep_their_bytes() {
    let cases = compatibility_cases();
    assert_eq!(cases.len(), 365);
    for case in &cases {
        assert_literal(case);
    }
    for bound in [10, 15, 17, 20] {
        assert_eq!(
            cases
                .iter()
                .filter(|c| c.bound() == bound)
                .map(|c| c.expected.len())
                .max(),
            Some(bound)
        );
    }
}

#[test]
fn every_slice_capacity_preserves_prefix_cursor_and_unused_tail() {
    for case in all_cases() {
        for available in 0..=case.bound() + 1 {
            let mut storage = [0xA5; 32];
            storage[..3].copy_from_slice(&PREFIX);
            let before = storage;
            let success;
            {
                let mut tail = &mut storage[3..3 + available];
                let pointer = tail.as_ptr();
                let result = case.encode(&mut tail);
                success = available >= case.expected.len();
                if success {
                    assert_eq!(result, Ok(case.expected.len()), "{}", case.name);
                    assert_eq!(tail.len(), available - case.expected.len());
                    assert_eq!(tail.as_ptr(), pointer.wrapping_add(case.expected.len()));
                } else {
                    assert_eq!(
                        result,
                        Err(SinkFull {
                            needed: case.expected.len(),
                            available
                        }),
                        "{}",
                        case.name
                    );
                    assert_eq!(tail.len(), available);
                    assert_eq!(tail.as_ptr(), pointer);
                }
            }
            if success {
                assert_eq!(&storage[..3], &PREFIX);
                assert_eq!(
                    &storage[3..3 + case.expected.len()],
                    &case.expected,
                    "{}",
                    case.name
                );
                assert!(
                    storage[3 + case.expected.len()..]
                        .iter()
                        .all(|b| *b == 0xA5)
                );
            } else {
                assert_eq!(storage, before, "{}", case.name);
            }
        }
    }
}

#[test]
fn growable_and_trait_object_sinks_append_repeated_values() {
    let cases = all_cases();
    let mut vector = Vec::from(PREFIX);
    let mut bytes = BytesMut::from(&PREFIX[..]);
    let mut expected = Vec::from(PREFIX);
    for case in &cases {
        assert_eq!(
            case.encode(&mut vector as &mut dyn EncodeSink),
            Ok(case.expected.len())
        );
        assert_eq!(
            case.encode(&mut bytes as &mut dyn EncodeSink),
            Ok(case.expected.len())
        );
        expected.extend_from_slice(&case.expected);
        assert_eq!(vector, expected);
        assert_eq!(&bytes[..], &expected);
    }
}

#[cfg(feature = "heapless")]
#[test]
fn every_heapless_capacity_preserves_length_and_prior_values() {
    for case in all_cases() {
        for available in 0..=case.bound() + 1 {
            let mut output = heapless::Vec::<u8, 64>::new();
            output.resize(64 - available, 0xA5).unwrap();
            let before = output.clone();
            let result = case.encode(&mut output as &mut dyn EncodeSink);
            if available >= case.expected.len() {
                assert_eq!(result, Ok(case.expected.len()), "{}", case.name);
                assert_eq!(&output[..before.len()], &before[..]);
                assert_eq!(&output[before.len()..], &case.expected, "{}", case.name);
            } else {
                assert_eq!(
                    result,
                    Err(SinkFull {
                        needed: case.expected.len(),
                        available
                    }),
                    "{}",
                    case.name
                );
                assert_eq!(output, before, "{}", case.name);
            }
        }
    }
}

struct ObservedSink {
    bytes: [u8; 64],
    used: usize,
    advertised: Option<usize>,
    writes: usize,
    successful_writes: usize,
}

impl ObservedSink {
    fn new(advertised: Option<usize>, successful_writes: usize) -> Self {
        let mut bytes = [0xA5; 64];
        bytes[..3].copy_from_slice(&PREFIX);
        Self {
            bytes,
            used: 3,
            advertised,
            writes: 0,
            successful_writes,
        }
    }
}

impl EncodeSink for ObservedSink {
    fn remaining_capacity(&self) -> Option<usize> {
        self.advertised
    }
    fn write(&mut self, value: &[u8]) -> Result<(), SinkFull> {
        self.writes += 1;
        if self.writes > self.successful_writes {
            return Err(REFUSAL);
        }
        self.bytes[self.used..self.used + value.len()].copy_from_slice(value);
        self.used += value.len();
        Ok(())
    }
}

#[test]
fn total_capacity_preflight_and_custom_refusal_leave_storage_unchanged() {
    for case in all_cases() {
        for available in [0, case.expected.len() - 1] {
            let mut sink = ObservedSink::new(Some(available), 0);
            let before = sink.bytes;
            assert_eq!(
                case.encode(&mut sink),
                Err(SinkFull {
                    needed: case.expected.len(),
                    available
                })
            );
            assert_eq!(sink.writes, 0);
            assert_eq!(sink.used, 3);
            assert_eq!(sink.bytes, before);
        }
        for advertised in [None, Some(case.expected.len()), Some(usize::MAX)] {
            let mut sink = ObservedSink::new(advertised, 0);
            let before = sink.bytes;
            assert_eq!(case.encode(&mut sink), Err(REFUSAL), "{}", case.name);
            assert_eq!(sink.writes, 1);
            assert_eq!(sink.used, 3);
            assert_eq!(sink.bytes, before);
        }
    }
}

#[test]
fn one_whole_write_succeeds_when_the_sink_would_refuse_a_second() {
    for case in all_cases() {
        for advertised in [None, Some(case.expected.len()), Some(usize::MAX)] {
            let mut sink = ObservedSink::new(advertised, 1);
            assert_eq!(
                case.encode(&mut sink as &mut dyn EncodeSink),
                Ok(case.expected.len()),
                "{}",
                case.name
            );
            assert_eq!(sink.writes, 1, "{}", case.name);
            assert_eq!(sink.used, 3 + case.expected.len());
            assert_eq!(&sink.bytes[..3], &PREFIX);
            assert_eq!(&sink.bytes[3..sink.used], &case.expected, "{}", case.name);
            assert!(sink.bytes[sink.used..].iter().all(|b| *b == 0xA5));
        }
    }
}

#[test]
fn refusal_of_a_later_call_keeps_complete_earlier_references() {
    for case in all_cases() {
        let mut sink = ObservedSink::new(Some(usize::MAX), 2);
        assert_eq!(case.encode(&mut sink), Ok(case.expected.len()));
        assert_eq!(case.encode(&mut sink), Ok(case.expected.len()));
        assert_eq!(
            &sink.bytes[3..sink.used],
            [case.expected.clone(), case.expected.clone()].concat()
        );
        let before = sink.bytes;
        let used = sink.used;
        assert_eq!(case.encode(&mut sink), Err(REFUSAL));
        assert_eq!(sink.writes, 3);
        assert_eq!(sink.used, used);
        assert_eq!(sink.bytes, before);
    }
}

#[test]
fn bounded_adapters_retain_repeated_references_when_the_next_call_is_full() {
    let cases = wire_cases();
    let sequence: Vec<_> = [
        "present/property",
        "present/object-property",
        "present/setpoint",
        "present/device-property-remote",
        "remote/device-object",
    ]
    .iter()
    .map(|name| cases.iter().find(|case| case.name == *name).unwrap())
    .collect();
    let expected: Vec<_> = PREFIX
        .into_iter()
        .chain(
            sequence
                .iter()
                .flat_map(|case| case.expected.iter().copied()),
        )
        .collect();
    assert_eq!(expected.len(), 43);
    let mut storage = [0xA5; 48];
    storage[..3].copy_from_slice(&PREFIX);
    {
        let mut tail = &mut storage[3..43];
        for case in &sequence {
            assert_eq!(
                case.encode(&mut tail as &mut dyn EncodeSink),
                Ok(case.expected.len())
            );
        }
        assert!(tail.is_empty());
        assert_eq!(
            sequence[0].encode(&mut tail),
            Err(SinkFull {
                needed: 2,
                available: 0
            })
        );
        assert!(tail.is_empty());
    }
    assert_eq!(&storage[..43], &expected);
    assert_eq!(&storage[43..], &[0xA5; 5]);
    #[cfg(feature = "heapless")]
    {
        let mut bounded = heapless::Vec::<u8, 43>::new();
        bounded.extend_from_slice(&PREFIX).unwrap();
        for case in &sequence {
            assert_eq!(case.encode(&mut bounded), Ok(case.expected.len()));
        }
        assert_eq!(
            sequence[0].encode(&mut bounded),
            Err(SinkFull {
                needed: 2,
                available: 0
            })
        );
        assert_eq!(&bounded[..], &expected);
    }
}
