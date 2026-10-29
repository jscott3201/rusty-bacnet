//! Public tag-header sink contract and independently tabulated wire cases.

// These compare with the BytesMut encoders, which need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_encoding::tags::{
    TagClass, encode_closing_tag, encode_closing_tag_to_sink, encode_opening_tag,
    encode_opening_tag_to_sink, encode_tag, encode_tag_to_sink,
};
use bytes::BytesMut;

#[derive(Clone, Copy, Debug)]
enum Header {
    Tag(u8, TagClass, u32),
    Opening(u8),
    Closing(u8),
}

impl Header {
    fn encode(self, sink: &mut (impl EncodeSink + ?Sized)) -> Result<usize, SinkFull> {
        match self {
            Self::Tag(number, class, length) => encode_tag_to_sink(sink, number, class, length),
            Self::Opening(number) => encode_opening_tag_to_sink(sink, number),
            Self::Closing(number) => encode_closing_tag_to_sink(sink, number),
        }
    }

    fn hosted(self, output: &mut BytesMut) {
        match self {
            Self::Tag(number, class, length) => encode_tag(output, number, class, length),
            Self::Opening(number) => encode_opening_tag(output, number),
            Self::Closing(number) => encode_closing_tag(output, number),
        }
    }
}

use Header::{Closing, Opening, Tag};
use TagClass::{Application as App, Context as Ctx};

// Independently tabulated from ASHRAE 135-2020, 20.2.1.1–20.2.1.3.
// These exercise header layout, not whether a datatype accepts each content
// length. No expected bytes come from the encoder or decoder under test.
const GOLDENS: &[(Header, &[u8])] = &[
    (Tag(2, App, 0), &[0x20]),
    (Tag(2, App, 4), &[0x24]),
    (Tag(2, App, 5), &[0x25, 0x05]),
    (Tag(2, App, 253), &[0x25, 0xFD]),
    (Tag(2, App, 254), &[0x25, 0xFE, 0x00, 0xFE]),
    (Tag(2, App, 65535), &[0x25, 0xFE, 0xFF, 0xFF]),
    (Tag(2, App, 65536), &[0x25, 0xFF, 0x00, 0x01, 0x00, 0x00]),
    (Tag(2, App, u32::MAX), &[0x25, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    (Tag(2, Ctx, 0), &[0x28]),
    (Tag(2, Ctx, 4), &[0x2C]),
    (Tag(2, Ctx, 5), &[0x2D, 0x05]),
    (Tag(2, Ctx, 253), &[0x2D, 0xFD]),
    (Tag(2, Ctx, 254), &[0x2D, 0xFE, 0x00, 0xFE]),
    (Tag(2, Ctx, 65535), &[0x2D, 0xFE, 0xFF, 0xFF]),
    (Tag(2, Ctx, 65536), &[0x2D, 0xFF, 0x00, 0x01, 0x00, 0x00]),
    (Tag(2, Ctx, u32::MAX), &[0x2D, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    (Tag(15, App, 0), &[0xF0, 0x0F]),
    (Tag(15, App, 4), &[0xF4, 0x0F]),
    (Tag(15, App, 5), &[0xF5, 0x0F, 0x05]),
    (Tag(15, App, 253), &[0xF5, 0x0F, 0xFD]),
    (Tag(15, App, 254), &[0xF5, 0x0F, 0xFE, 0x00, 0xFE]),
    (Tag(15, App, 65535), &[0xF5, 0x0F, 0xFE, 0xFF, 0xFF]),
    (
        Tag(15, App, 65536),
        &[0xF5, 0x0F, 0xFF, 0x00, 0x01, 0x00, 0x00],
    ),
    (
        Tag(15, App, u32::MAX),
        &[0xF5, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ),
    (Tag(15, Ctx, 0), &[0xF8, 0x0F]),
    (Tag(15, Ctx, 4), &[0xFC, 0x0F]),
    (Tag(15, Ctx, 5), &[0xFD, 0x0F, 0x05]),
    (Tag(15, Ctx, 253), &[0xFD, 0x0F, 0xFD]),
    (Tag(15, Ctx, 254), &[0xFD, 0x0F, 0xFE, 0x00, 0xFE]),
    (Tag(15, Ctx, 65535), &[0xFD, 0x0F, 0xFE, 0xFF, 0xFF]),
    (
        Tag(15, Ctx, 65536),
        &[0xFD, 0x0F, 0xFF, 0x00, 0x01, 0x00, 0x00],
    ),
    (
        Tag(15, Ctx, u32::MAX),
        &[0xFD, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ),
    (Tag(0, App, 0), &[0x00]),
    (Tag(1, App, 0), &[0x10]),
    (Tag(1, App, 1), &[0x11]),
    (Tag(14, Ctx, 4), &[0xEC]),
    (
        Tag(254, Ctx, 65536),
        &[0xFD, 0xFE, 0xFF, 0x00, 0x01, 0x00, 0x00],
    ),
    (Opening(0), &[0x0E]),
    (Closing(0), &[0x0F]),
    (Opening(14), &[0xEE]),
    (Closing(14), &[0xEF]),
    (Opening(15), &[0xFE, 0x0F]),
    (Closing(15), &[0xFF, 0x0F]),
    (Opening(254), &[0xFE, 0xFE]),
    (Closing(254), &[0xFF, 0xFE]),
];

const PREFIX: [u8; 2] = [0xCA, 0xFE];

fn assert_growable(header: Header, expected: &[u8]) {
    let mut vector = PREFIX.to_vec();
    assert_eq!(vector.remaining_capacity(), None);
    assert_eq!(header.encode(&mut vector), Ok(expected.len()));
    assert_eq!(&vector[..PREFIX.len()], PREFIX);
    assert_eq!(&vector[PREFIX.len()..], expected, "{header:?}");
    assert_eq!(vector.remaining_capacity(), None);

    let mut bytes = BytesMut::from(&PREFIX[..]);
    assert_eq!(bytes.remaining_capacity(), None);
    assert_eq!(header.encode(&mut bytes), Ok(expected.len()));
    assert_eq!(&bytes[..], &vector[..], "{header:?}");
    assert_eq!(bytes.remaining_capacity(), None);

    let mut hosted = BytesMut::from(&PREFIX[..]);
    header.hosted(&mut hosted);
    assert_eq!(&hosted[..], &vector[..], "{header:?}");
}

#[test]
fn independently_tabulated_headers_append_to_vec_bytes_and_hosted_adapters() {
    for &(header, expected) in GOLDENS {
        assert_growable(header, expected);
    }
}

#[test]
fn slice_headers_preserve_output_and_cursor_for_every_short_capacity() {
    for &(header, expected) in GOLDENS {
        for available in 0..=expected.len() + 1 {
            let mut storage = [0xA5; 12];
            storage[..PREFIX.len()].copy_from_slice(&PREFIX);
            let before = storage;
            {
                let mut tail = &mut storage[PREFIX.len()..PREFIX.len() + available];
                let pointer = tail.as_ptr();
                assert_eq!(tail.remaining_capacity(), Some(available));
                let result = header.encode(&mut tail);
                if available < expected.len() {
                    assert_eq!(
                        result,
                        Err(SinkFull {
                            needed: expected.len(),
                            available
                        })
                    );
                    assert_eq!(tail.as_ptr(), pointer, "{header:?}, {available}");
                    assert_eq!(tail.len(), available);
                } else {
                    assert_eq!(result, Ok(expected.len()));
                    assert_eq!(tail.len(), available - expected.len());
                    assert_eq!(tail.remaining_capacity(), Some(available - expected.len()));
                }
            }
            if available < expected.len() {
                assert_eq!(storage, before, "{header:?}, {available}");
            } else {
                assert_eq!(&storage[..PREFIX.len()], PREFIX);
                assert_eq!(
                    &storage[PREFIX.len()..PREFIX.len() + expected.len()],
                    expected
                );
                assert_eq!(
                    &storage[PREFIX.len() + expected.len()..],
                    &before[PREFIX.len() + expected.len()..]
                );
            }
        }
    }
}

#[test]
fn slice_repeated_headers_preserve_the_previous_output_on_refusal() {
    let mut storage = [0xA5; 5];
    {
        let mut tail = &mut storage[..];
        assert_eq!(Opening(15).encode(&mut tail), Ok(2));
        assert_eq!(Tag(1, App, 1).encode(&mut tail), Ok(1));
        let pointer = tail.as_ptr();
        assert_eq!(
            Tag(254, Ctx, 65536).encode(&mut tail),
            Err(SinkFull {
                needed: 7,
                available: 2
            })
        );
        assert_eq!(tail.as_ptr(), pointer);
        assert_eq!(tail.len(), 2);
        assert_eq!(Closing(15).encode(&mut tail), Ok(2));
        assert!(tail.is_empty());
    }
    assert_eq!(storage, [0xFE, 0x0F, 0x11, 0xFF, 0x0F]);
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_headers_preserve_prefix_and_length_for_every_short_capacity() {
    for &(header, expected) in GOLDENS {
        for available in 0..=expected.len() + 1 {
            let mut output = heapless::Vec::<u8, 16>::new();
            output
                .extend_from_slice(&[0xA5; 16][..16 - available])
                .unwrap();
            let before = output.clone();
            let prefix = output.len();
            assert_eq!(output.remaining_capacity(), Some(available));
            let result = header.encode(&mut output);
            if available < expected.len() {
                assert_eq!(
                    result,
                    Err(SinkFull {
                        needed: expected.len(),
                        available
                    })
                );
                assert_eq!(output, before, "{header:?}, {available}");
            } else {
                assert_eq!(result, Ok(expected.len()));
                assert_eq!(&output[..prefix], &before[..]);
                assert_eq!(&output[prefix..], expected);
                assert_eq!(
                    output.remaining_capacity(),
                    Some(available - expected.len())
                );
            }
        }
    }
}

#[test]
fn slice_adapter_writes_are_atomic_even_without_an_encoder_preflight() {
    let mut storage = [0xA5; 3];
    {
        let mut tail = &mut storage[..];
        assert_eq!(tail.write(&[]), Ok(()));
        assert_eq!(tail.write(&PREFIX), Ok(()));
        let pointer = tail.as_ptr();
        assert_eq!(
            tail.write(&[7, 8]),
            Err(SinkFull {
                needed: 2,
                available: 1
            })
        );
        assert_eq!(tail.as_ptr(), pointer);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail.write(&[9]), Ok(()));
        assert_eq!(tail.write(&[]), Ok(()));
        assert_eq!(tail.remaining_capacity(), Some(0));
    }
    assert_eq!(storage, [0xCA, 0xFE, 9]);
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_adapter_writes_are_atomic_even_without_an_encoder_preflight() {
    let mut output = heapless::Vec::<u8, 3>::new();
    assert_eq!(output.write(&[]), Ok(()));
    assert_eq!(output.write(&PREFIX), Ok(()));
    assert_eq!(
        output.write(&[7, 8]),
        Err(SinkFull {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(&output[..], PREFIX);
    assert_eq!(output.write(&[9]), Ok(()));
    assert_eq!(output.write(&[]), Ok(()));
    assert_eq!(output.remaining_capacity(), Some(0));
    assert_eq!(&output[..], &[0xCA, 0xFE, 9]);
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
fn custom_sinks_receive_one_complete_header_and_their_refusal_is_unchanged() {
    for &(header, expected) in GOLDENS {
        for capacity in [None, Some(expected.len()), Some(expected.len() + 1)] {
            for rejection in [
                None,
                Some(SinkFull {
                    needed: expected.len(),
                    available: 0,
                }),
            ] {
                let mut sink = ObservedSink {
                    capacity,
                    rejection,
                    writes: Vec::new(),
                    output: PREFIX.to_vec(),
                };
                let erased: &mut dyn EncodeSink = &mut sink;
                assert_eq!(
                    header.encode(erased),
                    rejection.map_or(Ok(expected.len()), Err)
                );
                assert_eq!(sink.writes.len(), 1, "{header:?}");
                assert_eq!(sink.writes[0], expected, "{header:?}");
                assert_eq!(&sink.output[..PREFIX.len()], PREFIX);
                if rejection.is_some() {
                    assert_eq!(sink.output, PREFIX);
                } else {
                    assert_eq!(&sink.output[PREFIX.len()..], expected);
                }
            }
        }
    }
}

#[test]
fn known_short_capacity_never_calls_a_custom_sink() {
    for &(header, expected) in GOLDENS {
        for available in 0..expected.len() {
            let mut sink = ObservedSink {
                capacity: Some(available),
                rejection: None,
                writes: Vec::new(),
                output: PREFIX.to_vec(),
            };
            assert_eq!(
                header.encode(&mut sink),
                Err(SinkFull {
                    needed: expected.len(),
                    available
                })
            );
            assert!(sink.writes.is_empty());
            assert_eq!(sink.output, PREFIX);
        }
    }
}

#[test]
fn legacy_unvalidated_inputs_are_preserved_without_claiming_valid_datatypes() {
    for (header, expected) in [
        (
            Tag(255, Ctx, u32::MAX),
            &[0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF][..],
        ),
        (Tag(255, App, 0), &[0xF0, 0xFF][..]),
        (Opening(255), &[0xFE, 0xFF][..]),
        (Closing(255), &[0xFF, 0xFF][..]),
        (Tag(1, App, 2), &[0x12][..]),
        (Tag(1, App, 5), &[0x15, 0x05][..]),
        (Tag(0, App, 4), &[0x04][..]),
    ] {
        assert_growable(header, expected);
    }
}
