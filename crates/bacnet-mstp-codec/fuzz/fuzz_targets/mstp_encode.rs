//! Encode an arbitrary frame through every sink and decode what each wrote.
//!
//! Oracle: the outcome the crate docs give, worked out here without the
//! codec. A frame the encode checks refuse gets that error from every sink,
//! and no sink changes. The checks, in order: source 255; the Length, which
//! for a COBS type (32-127) is the harness's own encoding of the data plus
//! the Encoded CRC-32K less two and must lie within 5..=2043, and for any
//! other type is the data length, at most 501; a reserved COBS type
//! (34-127); data on a type defined without any; a proprietary frame under
//! two octets. A frame that passes is the octets `mstp_codec_fuzz::encode`
//! builds from the layout, COBS-encoded with a CRC-32K for types 32 and 33,
//! in every sink with room for it; a fixed sink without room refuses it
//! with `BufferExhausted` and keeps its contents, which the target compares
//! before and after. Each successful encode decodes back to the frame,
//! ignoring octets after it: through `decode_frame` for a standard frame,
//! and through `decode_frame_in_place` for an extended one, which
//! `decode_frame` refuses.

#![no_main]

use arbitrary::Arbitrary;
use bacnet_mstp_codec::{
    decode_frame, decode_frame_in_place, Frame, FrameError, FrameSink, FrameType,
};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, SinkKnobs, MAX_COBS_LENGTH, MAX_DATA, MIN_COBS_LENGTH};

#[derive(Debug, Arbitrary)]
struct Input {
    frame_type: u8,
    destination: u8,
    source: u8,
    /// The data length: modulo 640 for a standard type, past the 501-octet
    /// limit too, and modulo 2100 for a COBS type, past its largest Length.
    length: u16,
    /// Repeated to fill the data field.
    pattern: Vec<u8>,
    sinks: SinkKnobs,
    /// Octets after the frame when it is decoded again.
    trailing: Vec<u8>,
}

/// The error the encode checks give, in the codec's order, or `None` when
/// the frame passes them.
fn refusal(raw: u8, source: u8, data: &[u8]) -> Option<FrameError> {
    let length = wire::length_field(raw, data);
    Some(if source == 0xFF {
        FrameError::BroadcastSource
    } else if wire::is_cobs(raw) && !(MIN_COBS_LENGTH..=MAX_COBS_LENGTH).contains(&length) {
        FrameError::InvalidCobsLength { length }
    } else if !wire::is_cobs(raw) && length > MAX_DATA {
        FrameError::Oversize {
            length,
            maximum: MAX_DATA,
        }
    } else if wire::is_cobs(raw) && !wire::is_extended(raw) {
        FrameError::UnsupportedCobsFrame { raw_type: raw }
    } else if length != 0 && wire::never_carries_data(raw) {
        FrameError::UnexpectedData {
            raw_type: raw,
            length,
        }
    } else if wire::is_proprietary(raw) && length < 2 {
        FrameError::MissingVendorId {
            raw_type: raw,
            length,
        }
    } else {
        return None;
    })
}

fuzz_target!(
    init: wire::self_test(),
    |input: Input| {
        let raw = input.frame_type;
        let modulus = if wire::is_cobs(raw) { 2100 } else { 640 };
        let data = wire::fill(usize::from(input.length) % modulus, &input.pattern);
        let frame = Frame::new(
            FrameType::from_raw(raw),
            input.destination,
            input.source,
            &data,
        );
        let wire_length = wire::wire_length(wire::length_field(raw, &data));
        let expected = match refusal(raw, input.source, &data) {
            Some(error) => Err(error),
            None => Ok(wire::encode(raw, input.destination, input.source, &data)),
        };

        let outputs = wire::check_sinks(
            &expected,
            wire_length,
            &input.sinks,
            |needed, available| FrameError::BufferExhausted { needed, available },
            |output| frame.encode_into(output),
            |sink: &mut dyn FrameSink| frame.encode_to_sink(sink),
        );

        for mut output in outputs {
            let length = output.len();
            output.extend_from_slice(&input.trailing);
            if wire::is_extended(raw) {
                assert_eq!(
                    decode_frame(&output),
                    Err(FrameError::ExtendedFrame { raw_type: raw })
                );
            } else {
                let (borrowed, consumed) = decode_frame(&output).expect("an encoded frame decodes");
                assert_eq!((borrowed.data, consumed), (&data[..], length));
            }
            let (decoded, consumed) =
                decode_frame_in_place(&mut output).expect("an encoded frame decodes");
            assert_eq!(consumed, length);
            assert_eq!(decoded.header.frame_type.to_raw(), raw);
            assert_eq!(decoded.header.frame_type, FrameType::from_raw(raw));
            assert_eq!(decoded.header.destination, input.destination);
            assert_eq!(decoded.header.source, input.source);
            assert_eq!(decoded.data, data);
            assert_eq!(decoded, frame);
        }
    }
);
