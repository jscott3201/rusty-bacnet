//! One-shot decode of arbitrary octets.
//!
//! Oracle: `decode_frame_in_place` accepts exactly the inputs that start
//! with a whole valid frame by `mstp_codec_fuzz::decode`, which checks the
//! layout, both CRCs or the CRC-32K, and the COBS encoding without the
//! codec. It reports that frame's length, fields and decoded data, and
//! changes the input only by decoding an extended frame's data over its
//! Encoded Data field. `decode_frame` gives the same results without
//! changing anything, except that a good extended header stops it with
//! `ExtendedFrame`, whatever follows the header.
//!
//! A refusal past the preamble names the first check that failed: the
//! header CRC, then source 255, then the Length against its type's bounds,
//! then a reserved COBS type (Clause 9.5.8 and the codec's docs), then
//! (for `decode_frame`, only on a standard type) completeness, then the
//! data's check.
//!
//! A frame it accepts re-encodes to the same octets, unless the encoder's
//! stricter checks refuse it (data on a type defined without any, or a
//! proprietary frame too short for its vendor identifier) or the input
//! encodes an extended frame in a form Clause 9.10.3 accepts but a strict
//! encoder never sends; then it re-encodes to the harness's own encoding.

#![no_main]

use bacnet_mstp_codec::{decode_frame, decode_frame_in_place, FrameError};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, Header, HEADER_END, MAX_DATA};

/// The error the first failing check gives, for input that starts with a
/// preamble and has no whole valid frame.
fn expected_error(input: &[u8]) -> FrameError {
    let header = Header::read(input);
    let Header::Good {
        frame_type,
        source,
        length,
        ..
    } = header
    else {
        return match header {
            Header::Incomplete => FrameError::TooShort {
                needed: HEADER_END,
                available: input.len(),
            },
            _ => FrameError::HeaderCrcMismatch,
        };
    };
    if source == 0xFF {
        return FrameError::BroadcastSource;
    }
    if !header.passes() {
        return if wire::is_cobs(frame_type) {
            FrameError::InvalidCobsLength { length }
        } else {
            FrameError::Oversize {
                length,
                maximum: MAX_DATA,
            }
        };
    }
    if !header.decodable() {
        return FrameError::UnsupportedCobsFrame {
            raw_type: frame_type,
        };
    }
    if input.len() < wire::wire_length(length) {
        return FrameError::Truncated {
            length,
            available: input.len() - HEADER_END,
        };
    }
    FrameError::DataCrcMismatch { length }
}

fuzz_target!(
    init: wire::self_test(),
    |input: &[u8]| {
        let decoded = decode_frame(input);
        let mut copy = input.to_vec();
        let in_place = decode_frame_in_place(&mut copy).map(|(frame, used)| {
            (frame.header, frame.data.to_vec(), used)
        });
        // A good extended header is as far as decode_frame looks.
        let header = Header::read(input);
        let extended_header = input.starts_with(&wire::PREAMBLE)
            && header.decodable()
            && matches!(header, Header::Good { frame_type, .. } if wire::is_extended(frame_type));
        if extended_header {
            assert_eq!(
                decoded,
                Err(FrameError::ExtendedFrame {
                    raw_type: input[2]
                })
            );
        }
        let Some((total, data)) = wire::decode(input) else {
            let error = in_place.expect_err("accepted an invalid frame");
            assert_eq!(copy, input, "a refused decode changed its input");
            if !extended_header {
                assert_eq!(decoded, Err(error));
            }
            if input.len() >= 2 && input.starts_with(&wire::PREAMBLE) {
                assert_eq!(error, expected_error(input));
            }
            return;
        };
        let raw = input[2];
        let (header, decoded_data, consumed) = in_place.expect("refused a valid frame");
        assert_eq!(consumed, total);
        assert_eq!(header.frame_type.to_raw(), raw);
        assert_eq!(header.destination, input[3]);
        assert_eq!(header.source, input[4]);
        assert_eq!(decoded_data, data);
        if wire::is_extended(raw) {
            assert_eq!(decoded, Err(FrameError::ExtendedFrame { raw_type: raw }));
            // The header stays, the data replaces the Encoded Data from the
            // octet after it, and nothing past the frame changes.
            assert_eq!(copy[..HEADER_END], input[..HEADER_END]);
            assert_eq!(copy[HEADER_END..HEADER_END + data.len()], data[..]);
            assert_eq!(copy[total..], input[total..]);
        } else {
            let (frame, used) = decoded.expect("refused a valid standard frame");
            assert_eq!((frame.header, frame.data, used), (header, &data[..], total));
            assert_eq!(copy, input, "decoding a standard frame changed its input");
        }

        let frame = bacnet_mstp_codec::Frame { header, data: &data };
        let mut output = [0u8; wire::MAX_COBS_WIRE];
        match frame.encode_into(&mut output) {
            Ok(written) => {
                let strict = wire::encode(raw, input[3], input[4], &data);
                assert_eq!(output[..written], strict[..]);
                if !wire::is_extended(raw) {
                    assert_eq!(strict[..], input[..total]);
                }
            }
            Err(FrameError::UnexpectedData { raw_type, length }) => {
                assert!(wire::never_carries_data(raw_type) && length != 0)
            }
            Err(FrameError::MissingVendorId { raw_type, length }) => {
                assert!(wire::is_proprietary(raw_type) && length < 2)
            }
            Err(error) => panic!("a decoded frame failed to encode: {error:?}"),
        }
    }
);
