//! Errors an encoder or a segmenter returns without an allocated message
//! (#1802): [`EncodeError`], which an encoder or validator returns as
//! [`Error::Encode`](super::Error::Encode), and [`SegmentationError`], which
//! splitting a payload or reassembling its segments returns as
//! [`Error::Segmentation`](super::Error::Segmentation).
//!
//! Like [`DecodeError`] they carry static labels and their numbers, and build
//! their text only when displayed, so a build that never shows one links no
//! formatting code for it. An [`EncodeError`]'s label takes the placeholders
//! the [`decode`](super::decode) module lists.

use core::fmt;

use super::decode::write_label;
use super::DecodeError;
use crate::enums::Segmentation;

/// A value an encoder or validator refused to put on the wire, without an
/// allocated message (#1802).
///
/// It displays as `encoding error: {context}: {label}`, leaving out
/// `{context}: ` when the context is empty and with the label's placeholders
/// filled from [`values`](Self::values). One that restates a decoder's
/// refusal of octets the encoder was handed, such as a captured raw value
/// (see [`from_decode`](Self::from_decode)), adds that refusal's offset:
/// `encoding error: decoding error at offset {offset}: {context}: {label}`.
///
/// ```
/// use bacnet_types::error::{EncodeError, Error};
///
/// let refused = EncodeError::new("mac-address of {} octets exceeds the {}-octet limit")
///     .within("BACnetAddress")
///     .with_values(19, 18);
/// assert_eq!(
///     Error::from(refused).to_string(),
///     "encoding error: BACnetAddress: mac-address of 19 octets exceeds the 18-octet limit"
/// );
/// ```
///
/// It is `#[non_exhaustive]`: build one with [`new`](Self::new) and the
/// builder methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct EncodeError {
    /// The production or member being encoded, such as `"BACnetNameValue"`;
    /// empty when the encoder names none.
    pub context: &'static str,
    /// What is wrong, such as `"exceeds {} property values"`.
    pub label: &'static str,
    /// The numbers the label's placeholders stand for, in order; a slot with
    /// no placeholder is 0. They are 32 bits wide, so the error fits in
    /// [`Error`](super::Error) without widening it: a number restated from a
    /// decoder's refusal past `u32::MAX` reads as `u32::MAX`.
    pub values: [u32; 2],
    /// The offset, within the octets the encoder was handed, of the fault a
    /// decoder found there; `None` when the refusal is the encoder's own. An
    /// offset past `u32::MAX` reads as `u32::MAX`.
    pub decode_offset: Option<u32>,
}

/// `value`, or `u32::MAX` when it doesn't fit.
fn saturate(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl EncodeError {
    /// A refusal described by `label`.
    pub const fn new(label: &'static str) -> Self {
        Self {
            context: "",
            label,
            values: [0; 2],
            decode_offset: None,
        }
    }

    /// The refusal of octets an encoder was handed, which `refused` found
    /// malformed: its context, label, numbers and offset. A list item index
    /// the decoder named is not kept.
    pub fn from_decode(refused: &DecodeError) -> Self {
        Self::restating(refused).with_offset(refused.offset)
    }

    /// `refused`'s context, label and numbers, without its offset.
    pub(super) fn restating(refused: &DecodeError) -> Self {
        Self {
            context: refused.context,
            label: refused.label,
            values: [saturate(refused.values[0]), saturate(refused.values[1])],
            decode_offset: None,
        }
    }

    const fn with_offset(mut self, offset: usize) -> Self {
        self.decode_offset = Some(if offset > u32::MAX as usize {
            u32::MAX
        } else {
            offset as u32
        });
        self
    }

    /// The same refusal, met encoding `context`.
    #[must_use]
    pub const fn within(mut self, context: &'static str) -> Self {
        self.context = context;
        self
    }

    /// The same refusal, with the number the label's one placeholder stands
    /// for.
    #[must_use]
    pub const fn with_value(mut self, value: u32) -> Self {
        self.values = [value, 0];
        self
    }

    /// The same refusal, with the numbers the label's two placeholders stand
    /// for.
    #[must_use]
    pub const fn with_values(mut self, first: u32, second: u32) -> Self {
        self.values = [first, second];
        self
    }

    /// The text without the `encoding error: ` prefix.
    pub fn message(&self) -> impl fmt::Display + '_ {
        EncodeText(self)
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("encoding error: ")?;
        fmt::Display::fmt(&self.message(), f)
    }
}

impl core::error::Error for EncodeError {}

/// [`EncodeError::message`].
struct EncodeText<'a>(&'a EncodeError);

impl fmt::Display for EncodeText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = self.0;
        if let Some(offset) = error.decode_offset {
            write!(f, "decoding error at offset {offset}: ")?;
        }
        if !error.context.is_empty() {
            f.write_str(error.context)?;
            f.write_str(": ")?;
        }
        let [first, second] = error.values;
        write_label(f, error.label, [first.into(), second.into()])
    }
}

/// Why a payload could not be split into segments, or its segments put back
/// together, or a segmented request sent (#1802).
///
/// It displays as `segmentation error: ` and the variant's text, built when
/// displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SegmentationError {
    /// A segment size of 0: a segment still costs its PDU header, so a peer
    /// whose maximum APDU can't hold that header can't take one.
    ZeroSegmentSize,
    /// The payload needs more than the 256 segments a sequence number can
    /// count.
    TooManySegments {
        /// Segments the payload needs.
        segments: usize,
    },
    /// A received segment larger than the receiver takes.
    SegmentTooLarge {
        /// The segment's length in octets.
        size: usize,
        /// The largest segment the receiver takes.
        max: usize,
    },
    /// A reassembly asked for more than 256 segments.
    TotalTooLarge {
        /// The segment count asked for.
        total: usize,
    },
    /// A segment due in the reassembly was never received.
    MissingSegment {
        /// The zero-based sequence number of the missing segment.
        index: usize,
        /// The segment count asked for.
        total: usize,
    },
    /// A request needs more segments than the peer said it accepts.
    PeerSegmentLimit {
        /// Segments the request needs.
        segments: usize,
        /// The most the peer accepts.
        max: u32,
    },
    /// A request needs segmenting and the peer advertised that it can't
    /// receive segmented requests.
    PeerCannotReceive {
        /// The segmentation support the peer advertised.
        capability: Segmentation,
    },
    /// A segmented request's octets, or its count of segments, passed the
    /// receiver's cap.
    PayloadCapacityExceeded,
    /// Counting a segmented request's octets overflowed.
    PayloadAccountingOverflow,
    /// The peer kept asking for the same window again with negative
    /// SegmentACKs past the sender's retry limit.
    NegativeAckRetriesExhausted,
    /// An endpoint operation's acknowledgement is longer than the endpoint's
    /// maximum APDU; endpoint operations don't reassemble segments.
    EndpointAckTooLong,
    /// An endpoint request too long for one APDU; the endpoint requester
    /// doesn't segment.
    EndpointRequestTooLong,
}

impl fmt::Display for SegmentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroSegmentSize => {
                f.write_str("payload cannot be segmented with max segment size 0")
            }
            Self::TooManySegments { segments } => {
                write!(f, "payload requires {segments} segments, maximum is 256")
            }
            Self::SegmentTooLarge { size, max } => {
                write!(f, "segment size {size} exceeds maximum {max}")
            }
            Self::TotalTooLarge { total } => write!(
                f,
                "total_segments {total} exceeds maximum BACnet value (256)"
            ),
            Self::MissingSegment { index, total } => {
                write!(f, "missing segment {index} of {total}")
            }
            Self::PeerSegmentLimit { segments, max } => write!(
                f,
                "request requires {segments} segments but remote accepts at most {max}"
            ),
            Self::PeerCannotReceive { capability } => write!(
                f,
                "request requires segmentation but the peer advertised {capability:?} \
                 and cannot receive segmented requests"
            ),
            Self::PayloadCapacityExceeded => f.write_str("request payload capacity exceeded"),
            Self::PayloadAccountingOverflow => f.write_str("request payload accounting overflow"),
            Self::NegativeAckRetriesExhausted => {
                f.write_str("too many negative SegmentAck retransmissions")
            }
            Self::EndpointAckTooLong => {
                f.write_str("endpoint operation ACK exceeds configured max APDU")
            }
            Self::EndpointRequestTooLong => {
                f.write_str("endpoint requester supports only unsegmented requests")
            }
        }
    }
}

impl core::error::Error for SegmentationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{DecodingKind, Error};

    #[test]
    fn encode_errors_display_their_label() {
        assert_eq!(
            Error::from(EncodeError::new("{} cannot be ALL").with_value(8)).to_string(),
            "encoding error: 8 cannot be ALL"
        );
        let decoded = DecodeError::new(DecodingKind::InvalidTag, 3, "expected tag [{}]")
            .within("opaque value")
            .with_value(2);
        let refused = EncodeError::from_decode(&decoded);
        assert_eq!(
            refused.to_string(),
            "encoding error: decoding error at offset 3: opaque value: expected tag [2]"
        );
        // The same text the owned form printed from the decoder's display.
        assert_eq!(refused.to_string(), format!("encoding error: {decoded}"));
    }

    #[test]
    fn segmentation_errors_keep_their_numbers() {
        let refused = Error::Segmentation(SegmentationError::MissingSegment { index: 2, total: 5 });
        assert_eq!(
            refused.to_string(),
            "segmentation error: missing segment 2 of 5"
        );
        assert_eq!(
            SegmentationError::PeerCannotReceive {
                capability: Segmentation::TRANSMIT
            }
            .to_string(),
            "request requires segmentation but the peer advertised Segmentation::TRANSMIT \
             and cannot receive segmented requests"
        );
    }
}
