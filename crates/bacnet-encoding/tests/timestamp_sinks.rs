//! Independent timestamp wire literals and whole-call output refusal behavior.

// These compare with the BytesMut encoders, which need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::primitives::{encode_timestamp, encode_timestamp_choice};
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_types::error::Error;
use bacnet_types::primitives::BACnetTimeStamp;
use bytes::BytesMut;

#[path = "timestamp_sinks/cases.rs"]
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
    assert_eq!(&sink[..3], &PREFIX, "{}", case.name);
    assert_eq!(&sink[3..], &case.expected, "{}", case.name);
    let mut legacy = BytesMut::from(&PREFIX[..]);
    match case.outer {
        None => encode_timestamp_choice(&mut legacy, &case.timestamp).unwrap(),
        Some(tag) => encode_timestamp(&mut legacy, tag, &case.timestamp).unwrap(),
    }
    assert_eq!(&legacy[..], &sink, "{}", case.name);
}

#[test]
fn independent_literal_layouts_and_legacy_signatures() {
    let _: fn(&mut BytesMut, &BACnetTimeStamp) -> Result<(), Error> = encode_timestamp_choice;
    let _: fn(&mut BytesMut, u8, &BACnetTimeStamp) -> Result<(), Error> = encode_timestamp;
    let cases = wire_cases();
    assert_eq!(cases.len(), 54);
    assert_eq!(
        cases
            .iter()
            .filter(|c| c.outer.is_none())
            .map(|c| c.expected.len())
            .max(),
        Some(12)
    );
    assert_eq!(cases.iter().map(|c| c.expected.len()).max(), Some(16));
    for case in &cases {
        assert_literal(case);
    }
}

#[test]
fn compatibility_inputs_keep_unchecked_fields_and_tag_255() {
    let cases = compatibility_cases();
    assert_eq!(cases.len(), 23);
    for case in &cases {
        assert_literal(case);
    }
}

#[test]
fn every_slice_capacity_preserves_prefix_cursor_and_unused_tail() {
    for case in all_cases() {
        for available in 0..=case.expected.len() + 1 {
            let mut storage = [0xA5; 24];
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
        for available in 0..=case.expected.len() + 1 {
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
fn refusal_of_a_later_call_keeps_complete_earlier_timestamps() {
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
fn bounded_adapters_retain_repeated_timestamps_when_the_next_call_is_full() {
    let cases = wire_cases();
    let sequence: Vec<_> = ["time-zero/bare", "datetime/outer-15", "sequence-256/bare"]
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
    assert_eq!(expected.len(), 27);
    let mut storage = [0xA5; 32];
    storage[..3].copy_from_slice(&PREFIX);
    {
        let mut tail = &mut storage[3..27];
        for case in &sequence {
            assert_eq!(case.encode(&mut tail), Ok(case.expected.len()));
        }
        assert!(tail.is_empty());
        assert_eq!(
            sequence[0].encode(&mut tail),
            Err(SinkFull {
                needed: 5,
                available: 0
            })
        );
        assert!(tail.is_empty());
    }
    assert_eq!(&storage[..27], &expected);
    assert_eq!(&storage[27..], &[0xA5; 5]);
    #[cfg(feature = "heapless")]
    {
        let mut bounded = heapless::Vec::<u8, 27>::new();
        bounded.extend_from_slice(&PREFIX).unwrap();
        for case in &sequence {
            assert_eq!(case.encode(&mut bounded), Ok(case.expected.len()));
        }
        assert_eq!(
            sequence[0].encode(&mut bounded),
            Err(SinkFull {
                needed: 5,
                available: 0
            })
        );
        assert_eq!(&bounded[..], &expected);
    }
}
