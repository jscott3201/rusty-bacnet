//! Whole-scalar atomicity and independent wire vectors for all 22 entrypoints.

// These compare with the BytesMut encoders, which need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::primitives::{signed_len, unsigned_len};
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bytes::BytesMut;

#[path = "scalar_sinks/cases.rs"]
mod cases;
use cases::{all_cases, compatibility_cases, wire_cases, Case, Encoding, Form, Value};

const PREFIX: [u8; 2] = [0xCA, 0xFE];

fn assert_growable(case: &Case) {
    let mut vector = PREFIX.to_vec();
    let mut bytes = BytesMut::from(&PREFIX[..]);
    let mut hosted = BytesMut::from(&PREFIX[..]);
    assert_eq!(
        case.encoding.encode(&mut vector),
        Ok(case.expected.len()),
        "{case:?}"
    );
    assert_eq!(
        case.encoding.encode(&mut bytes),
        Ok(case.expected.len()),
        "{case:?}"
    );
    case.encoding.hosted(&mut hosted);
    let expected = [&PREFIX[..], &case.expected].concat();
    assert_eq!(vector, expected, "{case:?}");
    assert_eq!(&bytes[..], expected, "{case:?}");
    assert_eq!(&hosted[..], expected, "{case:?}");
    assert_eq!(vector.remaining_capacity(), None);
    assert_eq!(bytes.remaining_capacity(), None);
}

#[test]
fn scalar_wire_goldens_cover_minimum_widths_and_all_selected_entrypoints() {
    for case in wire_cases() {
        assert_growable(&case);
    }
    for &(value, expected) in cases::UNSIGNED {
        assert_eq!(unsigned_len(value) as usize, expected.len());
    }
    for &(value, expected) in cases::SIGNED {
        assert_eq!(signed_len(value) as usize, expected.len());
    }
}

#[test]
fn scalar_legacy_input_goldens_preserve_nonfinite_bits_unchecked_fields_and_tag_255() {
    for case in compatibility_cases() {
        assert_growable(&case);
    }
}

#[test]
fn scalar_slices_preserve_prefix_cursor_and_tail_at_every_short_capacity() {
    for case in all_cases() {
        for available in 0..=case.expected.len() + 1 {
            let mut storage = [0xA5; 16];
            storage[..PREFIX.len()].copy_from_slice(&PREFIX);
            let before = storage;
            {
                let mut tail = &mut storage[PREFIX.len()..PREFIX.len() + available];
                let pointer = tail.as_ptr();
                let result = case.encoding.encode(&mut tail);
                if available < case.expected.len() {
                    assert_eq!(
                        result,
                        Err(SinkFull {
                            needed: case.expected.len(),
                            available
                        }),
                        "{case:?}"
                    );
                    assert_eq!(tail.as_ptr(), pointer);
                    assert_eq!(tail.len(), available);
                } else {
                    assert_eq!(result, Ok(case.expected.len()), "{case:?}");
                    assert_eq!(tail.as_ptr(), pointer.wrapping_add(case.expected.len()));
                    assert_eq!(
                        tail.remaining_capacity(),
                        Some(available - case.expected.len())
                    );
                }
            }
            if available < case.expected.len() {
                assert_eq!(storage, before, "{case:?}");
            } else {
                assert_eq!(&storage[..PREFIX.len()], PREFIX);
                assert_eq!(
                    &storage[PREFIX.len()..PREFIX.len() + case.expected.len()],
                    case.expected,
                    "{case:?}"
                );
                assert_eq!(
                    &storage[PREFIX.len() + case.expected.len()..],
                    &before[PREFIX.len() + case.expected.len()..]
                );
            }
        }
    }
}

#[cfg(feature = "heapless")]
#[test]
fn scalar_heapless_preserves_prefix_and_length_at_every_short_capacity() {
    for case in all_cases() {
        for available in 0..=case.expected.len() + 1 {
            let mut output = heapless::Vec::<u8, 16>::new();
            output
                .extend_from_slice(&[0xA5; 16][..16 - available])
                .unwrap();
            let before = output.clone();
            let result = case.encoding.encode(&mut output);
            if available < case.expected.len() {
                assert_eq!(
                    result,
                    Err(SinkFull {
                        needed: case.expected.len(),
                        available
                    }),
                    "{case:?}"
                );
                assert_eq!(output, before);
            } else {
                assert_eq!(result, Ok(case.expected.len()), "{case:?}");
                assert_eq!(&output[..before.len()], &before[..]);
                assert_eq!(&output[before.len()..], case.expected);
                assert_eq!(
                    output.remaining_capacity(),
                    Some(available - case.expected.len())
                );
            }
        }
    }
}

struct ObservedSink {
    capacity: Option<usize>,
    rejection: Option<SinkFull>,
    writes: Vec<Vec<u8>>,
    output: Vec<u8>,
}
impl EncodeSink for ObservedSink {
    fn remaining_capacity(&self) -> Option<usize> {
        self.capacity
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.writes.push(bytes.to_vec());
        if let Some(error) = self.rejection {
            return Err(error);
        }
        self.output.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn scalar_preflight_uses_total_length_without_calling_a_known_short_sink() {
    for case in all_cases() {
        for available in 0..case.expected.len() {
            let mut sink = ObservedSink {
                capacity: Some(available),
                rejection: None,
                writes: Vec::new(),
                output: PREFIX.to_vec(),
            };
            assert_eq!(
                case.encoding.encode(&mut sink),
                Err(SinkFull {
                    needed: case.expected.len(),
                    available
                }),
                "{case:?}"
            );
            assert!(sink.writes.is_empty(), "{case:?}");
            assert_eq!(sink.output, PREFIX);
        }
    }
}

#[test]
fn scalar_unknown_or_sufficient_capacity_gets_one_whole_write_and_unchanged_error() {
    let custom = SinkFull {
        needed: 901,
        available: 907,
    };
    for case in all_cases() {
        for capacity in [None, Some(case.expected.len()), Some(usize::MAX)] {
            for rejection in [None, Some(custom)] {
                let mut sink = ObservedSink {
                    capacity,
                    rejection,
                    writes: Vec::new(),
                    output: PREFIX.to_vec(),
                };
                // The public generic bound also permits a dynamically sized trait object.
                let output: &mut dyn EncodeSink = &mut sink;
                let result = case.encoding.encode(output);
                assert_eq!(sink.writes, vec![case.expected.clone()], "{case:?}");
                if let Some(error) = rejection {
                    assert_eq!(result, Err(error), "{case:?}");
                    assert_eq!(sink.output, PREFIX);
                } else {
                    assert_eq!(result, Ok(case.expected.len()), "{case:?}");
                    assert_eq!(sink.output, [&PREFIX[..], &case.expected].concat());
                }
            }
        }
    }
}

struct RefuseSecondWrite {
    writes: usize,
    output: Vec<u8>,
}
impl EncodeSink for RefuseSecondWrite {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.writes += 1;
        if self.writes > 1 {
            return Err(SinkFull {
                needed: bytes.len(),
                available: 0,
            });
        }
        self.output.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn scalar_never_splits_header_and_payload_across_caller_writes() {
    for case in all_cases() {
        let mut sink = RefuseSecondWrite {
            writes: 0,
            output: PREFIX.to_vec(),
        };
        assert_eq!(
            case.encoding.encode(&mut sink),
            Ok(case.expected.len()),
            "{case:?}"
        );
        assert_eq!(sink.writes, 1);
        assert_eq!(sink.output, [&PREFIX[..], &case.expected].concat());
    }
}

#[test]
fn scalar_repeated_appends_leave_completed_values_and_cursor_intact_after_refusal() {
    let first = Encoding(Form::App, Value::Boolean(true));
    let refused = Encoding(Form::Context(254), Value::Double(0x8000000000000000));
    let last = Encoding(Form::Context(15), Value::Signed(-129));
    let mut storage = [0xA5; 7];
    {
        let mut tail = &mut storage[..];
        tail.write(&PREFIX).unwrap();
        assert_eq!(first.encode(&mut tail), Ok(1));
        let pointer = tail.as_ptr();
        assert_eq!(
            refused.encode(&mut tail),
            Err(SinkFull {
                needed: 11,
                available: 4
            })
        );
        assert_eq!(tail.as_ptr(), pointer);
        assert_eq!(tail.len(), 4);
        assert_eq!(last.encode(&mut tail), Ok(4));
        assert!(tail.is_empty());
    }
    assert_eq!(storage, [0xCA, 0xFE, 0x11, 0xFA, 0x0F, 0xFF, 0x7F]);
    let mut vector = PREFIX.to_vec();
    let mut bytes = BytesMut::from(&PREFIX[..]);
    let mut expected = PREFIX.to_vec();
    for case in all_cases() {
        assert_eq!(case.encoding.encode(&mut vector), Ok(case.expected.len()));
        assert_eq!(case.encoding.encode(&mut bytes), Ok(case.expected.len()));
        expected.extend_from_slice(&case.expected);
    }
    assert_eq!(vector, expected);
    assert_eq!(&bytes[..], expected);
    #[cfg(feature = "heapless")]
    {
        let mut output = heapless::Vec::<u8, 7>::new();
        output.extend_from_slice(&PREFIX).unwrap();
        assert_eq!(first.encode(&mut output), Ok(1));
        let before = output.clone();
        assert_eq!(
            refused.encode(&mut output),
            Err(SinkFull {
                needed: 11,
                available: 4
            })
        );
        assert_eq!(output, before);
        assert_eq!(last.encode(&mut output), Ok(4));
        assert_eq!(&output[..], storage);
    }
}
