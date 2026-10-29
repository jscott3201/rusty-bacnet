//! One-shot decoding, and the checks the stream decoder shares with it.

use crate::cobs;
use crate::crc::{crc8_valid, crc16_valid};
use crate::frame::{DATA_OFFSET, check_header_fields, wire_length};
use crate::{Frame, FrameError, FrameHeader, FrameType, PREAMBLE};

/// The header of a candidate that passed the header CRC and the header
/// checks of Clause 9.5.8, with a type the codec decodes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GoodHeader {
    pub(crate) header: FrameHeader,
    /// The Length field.
    pub(crate) length: usize,
}

/// A whole valid frame at the start of some input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValidFrame {
    pub(crate) header: FrameHeader,
    /// Octets of decoded data.
    pub(crate) data_length: usize,
    /// Its wire length.
    pub(crate) consumed: usize,
}

/// Check the preamble and the header at the start of `input`, in the order
/// [`decode_frame`] lists. With `reserved_cobs`, a reserved COBS type
/// (34-127) whose header passes is accepted, so that its encoded fields are
/// checked as an extended frame's are.
pub(crate) fn check_header(input: &[u8], reserved_cobs: bool) -> Result<GoodHeader, FrameError> {
    if input.len() < DATA_OFFSET {
        return Err(FrameError::TooShort {
            needed: DATA_OFFSET,
            available: input.len(),
        });
    }
    if input[..PREAMBLE.len()] != PREAMBLE {
        return Err(FrameError::InvalidPreamble {
            first: input[0],
            second: input[1],
        });
    }
    if !crc8_valid(&input[PREAMBLE.len()..DATA_OFFSET]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let frame_type = FrameType::from_raw(input[2]);
    let (destination, source) = (input[3], input[4]);
    let length = usize::from(u16::from_be_bytes([input[5], input[6]]));
    match check_header_fields(frame_type, source, length) {
        Err(FrameError::UnsupportedCobsFrame { .. }) if reserved_cobs => {}
        result => result?,
    }
    Ok(GoodHeader {
        header: FrameHeader::new(frame_type, destination, source),
        length,
    })
}

/// Check the whole frame at the start of `input` without changing it; see
/// [`check_header`] for `reserved_cobs`.
pub(crate) fn check_frame(input: &[u8], reserved_cobs: bool) -> Result<ValidFrame, FrameError> {
    let GoodHeader { header, length } = check_header(input, reserved_cobs)?;
    let consumed = wire_length(length);
    if input.len() < consumed {
        return Err(FrameError::Truncated {
            length,
            available: input.len() - DATA_OFFSET,
        });
    }
    let body = &input[DATA_OFFSET..consumed];
    let data_length = if header.frame_type.is_cobs() {
        cobs::check_fields(body).ok_or(FrameError::DataCrcMismatch { length })?
    } else {
        if length != 0 && !crc16_valid(body) {
            return Err(FrameError::DataCrcMismatch { length });
        }
        length
    };
    Ok(ValidFrame {
        header,
        data_length,
        consumed,
    })
}

/// Decode the COBS-encoded frame that [`check_frame`] accepted at the start
/// of `input` in place, so its data starts where its Encoded Data field did.
pub(crate) fn decode_extended_in_place(input: &mut [u8], valid: &ValidFrame) {
    if valid.header.frame_type.is_cobs() {
        cobs::decode_data_in_place(&mut input[DATA_OFFSET..valid.consumed]);
    }
}

/// Decode one frame from `input`, which must start with the preamble, and
/// borrow its data from `input`.
///
/// Returns the frame and the number of octets it occupies; anything after
/// them is ignored. The checks, in order: preamble, header CRC, source 255,
/// the Length (at most 501 octets on a standard type, within
/// [`MIN_COBS_LENGTH`](crate::MIN_COBS_LENGTH)..=[`MAX_COBS_LENGTH`](crate::MAX_COBS_LENGTH)
/// on a COBS type), a COBS type other than 32 or 33, completeness, then the
/// data CRC. The [crate docs](crate) explain what is deliberately not
/// checked.
///
/// An extended frame's data is COBS-encoded on the wire, so it can't be
/// borrowed: a good extended header stops the checks there, with
/// [`FrameError::ExtendedFrame`], before anything after the header is
/// looked at. [`decode_frame_in_place`] decodes the frame and judges the
/// rest of it.
///
/// ```
/// use bacnet_mstp_codec::{decode_frame, FrameError, FrameType};
///
/// // Data Not Expecting Reply from slave 200 to station 1, data 01 00.
/// let wire = [0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF, 0x01, 0x00, 0x9F, 0x16];
/// let (frame, consumed) = decode_frame(&wire)?;
/// assert_eq!(frame.header.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
/// assert_eq!((frame.header.destination, frame.header.source), (1, 200));
/// assert_eq!(frame.data, [0x01, 0x00]);
/// assert_eq!(consumed, wire.len());
///
/// // Without its data CRC the frame is incomplete.
/// assert!(matches!(decode_frame(&wire[..10]), Err(FrameError::Truncated { .. })));
/// # Ok::<(), FrameError>(())
/// ```
pub fn decode_frame(input: &[u8]) -> Result<(Frame<'_>, usize), FrameError> {
    let frame_type = check_header(input, false)?.header.frame_type;
    if frame_type.is_cobs() {
        return Err(FrameError::ExtendedFrame {
            raw_type: frame_type.to_raw(),
        });
    }
    let valid = check_frame(input, false)?;
    let frame = Frame {
        header: valid.header,
        data: &input[DATA_OFFSET..DATA_OFFSET + valid.data_length],
    };
    Ok((frame, valid.consumed))
}

/// Decode one frame of any type the codec supports from `input`, which must
/// start with the preamble.
///
/// The checks and their order are those of [`decode_frame`], and so are the
/// results for a standard frame, whose data is borrowed and `input` left as
/// it was. An extended frame then goes on to completeness, the CRC-32K and
/// the COBS encoding of both fields; its data is decoded over its Encoded
/// Data field, so it starts just after the header, and the rest of the
/// frame's octets are left unspecified. On an error `input` is unchanged.
///
/// ```
/// use bacnet_mstp_codec::{decode_frame, decode_frame_in_place, Frame, FrameError, FrameType};
///
/// let npdu = [0x01, 0x00, 0x10, 0x08];
/// let frame = Frame::new(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, 0xFF, 0x02, &npdu);
/// let mut wire = [0u8; 32];
/// let length = frame.encode_into(&mut wire)?;
///
/// assert_eq!(decode_frame(&wire[..length]), Err(FrameError::ExtendedFrame { raw_type: 0x21 }));
/// let (decoded, consumed) = decode_frame_in_place(&mut wire[..length])?;
/// assert_eq!((decoded, consumed), (frame, length));
/// # Ok::<(), FrameError>(())
/// ```
pub fn decode_frame_in_place(input: &mut [u8]) -> Result<(Frame<'_>, usize), FrameError> {
    let valid = check_frame(input, false)?;
    decode_extended_in_place(input, &valid);
    let frame = Frame {
        header: valid.header,
        data: &input[DATA_OFFSET..DATA_OFFSET + valid.data_length],
    };
    Ok((frame, valid.consumed))
}

/// Return the offset of the first preamble in `data`, if it holds one.
#[must_use]
pub fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|pair| pair == PREAMBLE)
}
