//! The fixed header of the APDUs that carry service parameters, read and
//! written without an allocator (#1614): an Unconfirmed-Request, an
//! unsegmented Confirmed-Request or ComplexACK, and an Error (Clauses
//! 20.1.2, 20.1.3, 20.1.5 and 20.1.7); and the two answers that are all
//! header, a Reject and an Abort (Clauses 20.1.8 and 20.1.9), so a device
//! without a heap can refuse what it doesn't serve (#1771).
//!
//! [`ApduHeader::decode`] splits an APDU into its header and the service
//! parameters after it, borrowed in place; [`ApduHeader::encode_to_sink`]
//! and [`ApduHeader::encode_with_parameters_to_sink`] write the same octets
//! the owned `encode_apdu` does. A segmented Confirmed-Request or ComplexACK
//! is refused with [`DecodingKind::Unsupported`]: reassembling one needs
//! storage this profile doesn't assume. A device that can't take segmented
//! requests answers one with an Abort (Clause 5.4.5.1). A SimpleACK or a
//! SegmentACK is refused the same way.
//!
//! ```
//! use bacnet_encoding::apdu::header::{ApduHeader, MaxApdu, MaxSegments};
//! use bacnet_types::enums::ConfirmedServiceChoice;
//!
//! // A ReadProperty request: invoke ID 42, segmented replies accepted, any
//! // number of segments, APDUs up to 1476 octets.
//! let apdu = [0x02, 0x05, 0x2A, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55];
//! let (header, parameters) = ApduHeader::decode(&apdu)?;
//! let ApduHeader::ConfirmedRequest(request) = header else { unreachable!() };
//! assert_eq!(request.invoke_id, 42);
//! assert_eq!(request.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
//! assert_eq!(request.max_apdu.octets(), 1476);
//! assert_eq!(request.max_segments, MaxSegments::UNSPECIFIED);
//! assert_eq!(parameters, &apdu[4..]);
//!
//! // The ComplexACK that answers it, written in front of its parameters.
//! let mut storage = [0; 16];
//! let mut tail = &mut storage[..];
//! let ack = ApduHeader::ComplexAck { invoke_id: 42, service_choice: request.service_choice };
//! let written = ack.encode_with_parameters_to_sink(&mut tail, &[0x0C])?;
//! assert_eq!(storage[..written], [0x30, 0x2A, 0x0C, 0x0C]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, PduType, RejectReason, UnconfirmedServiceChoice,
};
use bacnet_types::error::{DecodeError, DecodingKind, Error};

use crate::sink::{EncodeSink, SinkFull};

/// The max-segments-accepted field of a Confirmed-Request (Clause 20.1.2.4):
/// three bits naming 2, 4, 8, 16, 32 or 64 segments, more than 64, or an
/// unspecified number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaxSegments(u8);

impl MaxSegments {
    /// B'000': the number of segments is unspecified.
    pub const UNSPECIFIED: Self = Self(0);
    /// B'111': more than 64 segments.
    pub const MORE_THAN_64: Self = Self(7);

    /// The field for a configured capacity: unspecified for `None`, and a
    /// count rounded down to the largest one the field names, so it never
    /// promises more than configured (65 and up are "more than 64"). `None`
    /// for a capacity of 0 or 1 segments, which the field can't express:
    /// decline segmented replies instead.
    pub const fn from_capacity(capacity: Option<u8>) -> Option<Self> {
        Some(Self(match capacity {
            None => 0,
            Some(0..=1) => return None,
            Some(2..=3) => 1,
            Some(4..=7) => 2,
            Some(8..=15) => 3,
            Some(16..=31) => 4,
            Some(32..=63) => 5,
            Some(64) => 6,
            Some(65..=u8::MAX) => 7,
        }))
    }

    /// The field read from the low three bits of `bits`.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & 0x07)
    }

    /// The field's three bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The segments the field names: `None` when unspecified, and 255 for
    /// more than 64, as the owned decoder reports them.
    pub const fn count(self) -> Option<u8> {
        match self.0 {
            0 => None,
            7 => Some(255),
            bits => Some(1 << bits),
        }
    }
}

/// The max-APDU-length-accepted field of a Confirmed-Request (Clause
/// 20.1.2.5): one of six sizes, 50 (MinimumMessageSize), 128, 206, 480, 1024
/// and 1476 octets. The other ten codes are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaxApdu(u8);

/// The octets each [`MaxApdu`] code names.
const MAX_APDU_OCTETS: [u16; 6] = [50, 128, 206, 480, 1024, 1476];

impl MaxApdu {
    /// Up to 50 octets, MinimumMessageSize.
    pub const OCTETS_50: Self = Self(0);
    /// Up to 128 octets.
    pub const OCTETS_128: Self = Self(1);
    /// Up to 206 octets.
    pub const OCTETS_206: Self = Self(2);
    /// Up to 480 octets.
    pub const OCTETS_480: Self = Self(3);
    /// Up to 1024 octets.
    pub const OCTETS_1024: Self = Self(4);
    /// Up to 1476 octets, or more.
    pub const OCTETS_1476: Self = Self(5);

    /// The field naming exactly `octets`, or `None` for a size it doesn't
    /// name.
    pub const fn from_octets(octets: u16) -> Option<Self> {
        let mut code = 0;
        while code < MAX_APDU_OCTETS.len() {
            if MAX_APDU_OCTETS[code] == octets {
                return Some(Self(code as u8));
            }
            code += 1;
        }
        None
    }

    /// The largest size the field names that is at most `capacity` octets;
    /// `None` below 50.
    pub const fn at_or_below(capacity: u32) -> Option<Self> {
        let mut code = MAX_APDU_OCTETS.len();
        while code > 0 {
            code -= 1;
            if MAX_APDU_OCTETS[code] as u32 <= capacity {
                return Some(Self(code as u8));
            }
        }
        None
    }

    /// The field read from the low four bits of `bits`; `None` for a
    /// reserved code.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        let code = bits & 0x0F;
        if (code as usize) < MAX_APDU_OCTETS.len() {
            Some(Self(code))
        } else {
            None
        }
    }

    /// The field's four bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The size the field names, in octets.
    pub const fn octets(self) -> u16 {
        MAX_APDU_OCTETS[self.0 as usize]
    }
}

/// The header of an unsegmented Confirmed-Request (Clause 20.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfirmedRequestHeader {
    /// Whether the requester takes a segmented reply.
    pub segmented_response_accepted: bool,
    /// The most segments the requester takes in a reply.
    pub max_segments: MaxSegments,
    /// The largest APDU the requester takes.
    pub max_apdu: MaxApdu,
    /// The identifier the reply echoes.
    pub invoke_id: u8,
    /// The service requested.
    pub service_choice: ConfirmedServiceChoice,
}

/// The header of an APDU that carries service parameters, or the whole of
/// a Reject or an Abort, as the allocation-free profile reads and writes it.
///
/// More PDU types may join it, so a `match` on it needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApduHeader {
    /// An unsegmented Confirmed-Request: four octets.
    ConfirmedRequest(ConfirmedRequestHeader),
    /// An Unconfirmed-Request for this service: two octets.
    UnconfirmedRequest(UnconfirmedServiceChoice),
    /// An unsegmented ComplexACK: three octets.
    ComplexAck {
        /// The invoke ID of the request it answers.
        invoke_id: u8,
        /// The service it answers.
        service_choice: ConfirmedServiceChoice,
    },
    /// The header of an Error: three octets, followed by the error
    /// parameters (for most services, the application-tagged error class
    /// and code).
    Error {
        /// The invoke ID of the request it answers.
        invoke_id: u8,
        /// The service that failed.
        service_choice: ConfirmedServiceChoice,
    },
    /// A whole Reject: three octets, with no parameters after them.
    Reject {
        /// The invoke ID of the request it rejects.
        invoke_id: u8,
        /// Why.
        reason: RejectReason,
    },
    /// A whole Abort: three octets, with no parameters after them.
    Abort {
        /// Whether the server sent it (the SRV bit), rather than the client.
        server: bool,
        /// Which transaction it ends, by invoke ID.
        invoke_id: u8,
        /// Why.
        reason: AbortReason,
    },
}

/// The longest header this module writes.
const MAX_HEADER: usize = 4;

impl ApduHeader {
    /// Split `apdu` into its header and the service parameters after it.
    ///
    /// Refuses, at offset 0 unless noted: an empty APDU; one whose header is
    /// cut short ([`Error::BufferTooShort`]); a reserved max-APDU code (at
    /// offset 1), segmented or not; a segmented Confirmed-Request or
    /// ComplexACK, a SimpleACK and a SegmentACK, with
    /// [`DecodingKind::Unsupported`]; and a PDU type nibble past 7. The
    /// more-follows bit of an unsegmented PDU is ignored, as Clause 20.1.2.2
    /// asks, and so are the reserved bits of an Error, a Reject or an Abort.
    /// A Reject or an Abort is the whole PDU: what follows its three octets
    /// is returned as the parameters, for the caller to refuse.
    pub fn decode(apdu: &[u8]) -> Result<(Self, &[u8]), Error> {
        let Some(&first) = apdu.first() else {
            return Err(Error::decoding(0, "APDU data is empty"));
        };
        let pdu_type = PduType::from_raw(first >> 4);
        let segmented = first & 0x08 != 0;
        if pdu_type == PduType::CONFIRMED_REQUEST {
            let [_, fields, invoke_id, service, ..] = *apdu else {
                return Err(Error::buffer_too_short(4, apdu.len()));
            };
            // A reserved size code is malformed whether or not the request is
            // segmented, as the owned decoder judges it.
            let Some(max_apdu) = MaxApdu::from_bits(fields) else {
                return Err(DecodeError::new(
                    DecodingKind::InvalidEncoding,
                    1,
                    "reserved max-APDU-length field value {}",
                )
                .with_value(u64::from(fields & 0x0F))
                .into());
            };
            if segmented {
                return Err(segmented_refusal("segmented ConfirmedRequest"));
            }
            let header = ConfirmedRequestHeader {
                segmented_response_accepted: first & 0x02 != 0,
                max_segments: MaxSegments::from_bits(fields >> 4),
                max_apdu,
                invoke_id,
                service_choice: ConfirmedServiceChoice::from_raw(service),
            };
            Ok((Self::ConfirmedRequest(header), &apdu[4..]))
        } else if pdu_type == PduType::UNCONFIRMED_REQUEST {
            let [_, service, ..] = *apdu else {
                return Err(Error::buffer_too_short(2, apdu.len()));
            };
            let service = UnconfirmedServiceChoice::from_raw(service);
            Ok((Self::UnconfirmedRequest(service), &apdu[2..]))
        } else if pdu_type == PduType::COMPLEX_ACK {
            let [_, invoke_id, service, ..] = *apdu else {
                return Err(Error::buffer_too_short(3, apdu.len()));
            };
            if segmented {
                return Err(segmented_refusal("segmented ComplexAck"));
            }
            let service_choice = ConfirmedServiceChoice::from_raw(service);
            Ok((
                Self::ComplexAck {
                    invoke_id,
                    service_choice,
                },
                &apdu[3..],
            ))
        } else if pdu_type == PduType::ERROR
            || pdu_type == PduType::REJECT
            || pdu_type == PduType::ABORT
        {
            let [_, invoke_id, third, ..] = *apdu else {
                return Err(Error::buffer_too_short(3, apdu.len()));
            };
            let header = if pdu_type == PduType::ERROR {
                Self::Error {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::from_raw(third),
                }
            } else if pdu_type == PduType::REJECT {
                Self::Reject {
                    invoke_id,
                    reason: RejectReason::from_raw(third),
                }
            } else {
                Self::Abort {
                    server: first & 0x01 != 0,
                    invoke_id,
                    reason: AbortReason::from_raw(third),
                }
            };
            Ok((header, &apdu[3..]))
        } else if first >> 4 <= 7 {
            Err(DecodeError::new(
                DecodingKind::Unsupported,
                0,
                "PDU type {} carries no service parameters this header reads",
            )
            .with_value(u64::from(first >> 4))
            .into())
        } else {
            Err(DecodeError::new(
                DecodingKind::InvalidEncoding,
                0,
                "unknown PDU type nibble: {:#x}",
            )
            .with_value(u64::from(first >> 4))
            .into())
        }
    }

    /// The header's length in octets: 4 for a Confirmed-Request, 2 for an
    /// Unconfirmed-Request and 3 for the rest.
    pub const fn encoded_len(&self) -> usize {
        match self {
            Self::ConfirmedRequest(_) => 4,
            Self::UnconfirmedRequest(_) => 2,
            Self::ComplexAck { .. }
            | Self::Error { .. }
            | Self::Reject { .. }
            | Self::Abort { .. } => 3,
        }
    }

    /// The header's octets, in a buffer whose first
    /// [`encoded_len`](Self::encoded_len) octets hold them.
    fn octets(&self) -> [u8; MAX_HEADER] {
        match *self {
            Self::ConfirmedRequest(header) => [
                (PduType::CONFIRMED_REQUEST.to_raw() << 4)
                    | (u8::from(header.segmented_response_accepted) << 1),
                (header.max_segments.bits() << 4) | header.max_apdu.bits(),
                header.invoke_id,
                header.service_choice.to_raw(),
            ],
            Self::UnconfirmedRequest(service) => [
                PduType::UNCONFIRMED_REQUEST.to_raw() << 4,
                service.to_raw(),
                0,
                0,
            ],
            Self::ComplexAck {
                invoke_id,
                service_choice,
            } => [
                PduType::COMPLEX_ACK.to_raw() << 4,
                invoke_id,
                service_choice.to_raw(),
                0,
            ],
            Self::Error {
                invoke_id,
                service_choice,
            } => [
                PduType::ERROR.to_raw() << 4,
                invoke_id,
                service_choice.to_raw(),
                0,
            ],
            Self::Reject { invoke_id, reason } => {
                [PduType::REJECT.to_raw() << 4, invoke_id, reason.to_raw(), 0]
            }
            Self::Abort {
                server,
                invoke_id,
                reason,
            } => [
                (PduType::ABORT.to_raw() << 4) | u8::from(server),
                invoke_id,
                reason.to_raw(),
                0,
            ],
        }
    }

    /// Append the header in one write; returns the octets appended.
    pub fn encode_to_sink<S: EncodeSink + ?Sized>(&self, sink: &mut S) -> Result<usize, SinkFull> {
        self.encode_with_parameters_to_sink(sink, &[])
    }

    /// Append the header and the service `parameters` after it as one value
    /// ([`EncodeSink::write_parts`]); returns the octets appended.
    pub fn encode_with_parameters_to_sink<S: EncodeSink + ?Sized>(
        &self,
        sink: &mut S,
        parameters: &[u8],
    ) -> Result<usize, SinkFull> {
        let octets = self.octets();
        let header = &octets[..self.encoded_len()];
        sink.write_parts(&[header, parameters])?;
        Ok(header.len() + parameters.len())
    }
}

/// The refusal of a segmented PDU, which this header doesn't reassemble.
#[cold]
fn segmented_refusal(label: &'static str) -> Error {
    DecodeError::new(DecodingKind::Unsupported, 0, label)
        .within("APDU header")
        .into()
}

#[cfg(test)]
mod tests;
