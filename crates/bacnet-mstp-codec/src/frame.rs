//! Frame types, headers, and encoding.

use crate::cobs;
use crate::crc::{crc8, crc16};
use crate::{
    BROADCAST_MAC, FrameError, FrameSink, HEADER_LENGTH, MAX_COBS_LENGTH, MAX_STANDARD_MPDU_DATA,
    MIN_COBS_LENGTH, PREAMBLE,
};

/// Octets before the data field: the preamble and the header with its CRC.
pub(crate) const DATA_OFFSET: usize = PREAMBLE.len() + HEADER_LENGTH;
/// What follows a non-empty data field: the two-octet data CRC of a standard
/// frame, or the two octets an extended frame's Length leaves out.
const TRAILER_LENGTH: usize = 2;

/// Wire length of a frame whose header declares `length`: the data and its
/// CRC for a standard frame, both encoded fields for an extended one, whose
/// Length counts them less two (Clause 9.3).
pub(crate) const fn wire_length(length: usize) -> usize {
    if length == 0 {
        DATA_OFFSET
    } else {
        DATA_OFFSET + length + TRAILER_LENGTH
    }
}

/// An open set of wire codes with named constants, the shape
/// `bacnet-bip-codec` gives `BvlcFunction` and `bacnet-types` its
/// enumerations: any raw value converts, and the names cover the values
/// the standard defines.
macro_rules! named_codes {
    (
        $(#[$meta:meta])*
        pub struct $Name:ident($inner:ty);
        $(
            $(#[$vmeta:meta])*
            const $VARIANT:ident = $val:expr_2021;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Name($inner);

        impl $Name {
            $(
                $(#[$vmeta])*
                #[doc = ""]
                #[doc = concat!("Wire value `", stringify!($val), "`.")]
                pub const $VARIANT: Self = Self($val);
            )*

            /// Every named constant, with its name, in the order declared.
            pub const ALL_NAMED: &[(&str, Self)] = &[
                $( (stringify!($VARIANT), Self($val)), )*
            ];

            /// Wrap a raw wire value, named or not.
            #[inline]
            #[must_use]
            pub const fn from_raw(value: $inner) -> Self {
                Self(value)
            }

            /// The raw wire value.
            #[inline]
            #[must_use]
            pub const fn to_raw(self) -> $inner {
                self.0
            }

            /// The constant's name, or `None` for a value the standard
            /// doesn't define.
            #[must_use]
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $( $val => Some(stringify!($VARIANT)), )*
                    _ => None,
                }
            }

            /// Whether the standard defines this value.
            #[must_use]
            pub const fn is_defined(self) -> bool {
                self.name().is_some()
            }
        }

        /// `Name::CONSTANT` for a named value, `Name(raw)` otherwise.
        impl core::fmt::Debug for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => write!(f, "{}::{}", stringify!($Name), name),
                    None => write!(f, "{}({})", stringify!($Name), self.0),
                }
            }
        }

        /// The constant's name, or the raw value in decimal.
        impl core::fmt::Display for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => f.write_str(name),
                    None => write!(f, "{}", self.0),
                }
            }
        }
    };
}

named_codes! {
    /// An MS/TP frame type: the first octet of the header after the
    /// preamble, which says what the frame is (Clause 9.3).
    ///
    /// The set is open, so a decoder hands on every octet as it arrived and
    /// a dispatcher matches the constants with a `_` arm for the rest. The
    /// constants are the ten types Clause 9.3 defines. Any other octet is
    /// reserved (8-31), reserved and COBS-encoded (34-127), or proprietary
    /// (128-255); [`FrameType::from_raw`] wraps it all the same, and two
    /// frame types are equal when their octets are.
    ///
    /// Frame types are numbered in decimal, as Clause 9.3 numbers them: the
    /// constants' wire values below, and `Debug` and `Display` for a type no
    /// constant names.
    ///
    /// ```
    /// use bacnet_mstp_codec::FrameType;
    ///
    /// assert_eq!(FrameType::from_raw(0), FrameType::TOKEN);
    /// assert_eq!(FrameType::TOKEN.name(), Some("TOKEN"));
    ///
    /// // A proprietary type has no name, but keeps its octet.
    /// let proprietary = FrameType::from_raw(128);
    /// assert!(proprietary.is_proprietary() && !proprietary.is_defined());
    /// assert_eq!(proprietary.to_raw(), 128);
    /// assert_eq!(format!("{proprietary:?}"), "FrameType(128)");
    /// assert_eq!(format!("{:?}", FrameType::TOKEN), "FrameType::TOKEN");
    /// ```
    pub struct FrameType(u8);

    /// Token: passes the right to send to the next master (Clause 9.3.1).
    const TOKEN = 0;
    /// Poll For Master: looks for a master at an address (Clause 9.3.2).
    const POLL_FOR_MASTER = 1;
    /// Reply To Poll For Master: a master answering a poll (Clause 9.3.3).
    const REPLY_TO_POLL_FOR_MASTER = 2;
    /// Test_Request: asks the receiver to send its data back (Clause
    /// 9.3.4).
    const TEST_REQUEST = 3;
    /// Test_Response: the answer to a Test_Request (Clause 9.3.5).
    const TEST_RESPONSE = 4;
    /// BACnet Data Expecting Reply: an NPDU whose sender waits for an
    /// answer (Clause 9.3.6).
    const BACNET_DATA_EXPECTING_REPLY = 5;
    /// BACnet Data Not Expecting Reply: an NPDU that wants no answer (Clause
    /// 9.3.7).
    const BACNET_DATA_NOT_EXPECTING_REPLY = 6;
    /// Reply Postponed: puts off the answer to a BACnet Data Expecting Reply
    /// (Clause 9.3.8).
    const REPLY_POSTPONED = 7;
    /// BACnet Extended Data Expecting Reply: an NPDU too long for a
    /// standard frame, COBS-encoded (Clause 9.3.9).
    const BACNET_EXTENDED_DATA_EXPECTING_REPLY = 32;
    /// BACnet Extended Data Not Expecting Reply: an NPDU too long for a
    /// standard frame, COBS-encoded (Clause 9.3.10).
    const BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY = 33;
}

impl FrameType {
    /// Whether a frame of this type may carry a data field.
    ///
    /// False only for the four types defined without data:
    /// [`TOKEN`](Self::TOKEN), [`POLL_FOR_MASTER`](Self::POLL_FOR_MASTER),
    /// [`REPLY_TO_POLL_FOR_MASTER`](Self::REPLY_TO_POLL_FOR_MASTER) and
    /// [`REPLY_POSTPONED`](Self::REPLY_POSTPONED) (Clauses 9.3.1-9.3.3, 9.3.8).
    /// Every other type may, including the reserved types 8-31, which the
    /// codec passes through without policing their content.
    #[must_use]
    pub const fn allows_data(self) -> bool {
        !matches!(self.to_raw(), 0 | 1 | 2 | 7)
    }

    /// Whether this type is in the COBS-encoded range, 32-127 (Clause 9.3).
    /// Its Length must fall within
    /// [`MIN_COBS_LENGTH`]..=[`MAX_COBS_LENGTH`] (Clause 9.5.8).
    #[must_use]
    pub const fn is_cobs(self) -> bool {
        let raw = self.to_raw();
        raw >= 32 && raw <= 127
    }

    /// Whether this is one of the two extended data types, 32 and 33: the
    /// COBS-encoded types the codec encodes and decodes, which carry BACnet
    /// NPDUs of 502 to [`MAX_EXTENDED_MPDU_DATA`](crate::MAX_EXTENDED_MPDU_DATA)
    /// octets (Clauses 9.3.9, 9.3.10). The rest of the COBS range, 34-127,
    /// is reserved.
    #[must_use]
    pub const fn is_extended_data(self) -> bool {
        matches!(self.to_raw(), 32 | 33)
    }

    /// Whether this type is in the vendor-proprietary range, 128-255. Such a
    /// frame's data opens with a 2-octet vendor identifier (Clause 9.3.11).
    #[must_use]
    pub const fn is_proprietary(self) -> bool {
        self.to_raw() >= 128
    }
}

/// The addressing fields of an MS/TP frame header.
///
/// The length field is not stored: it follows from the data that goes with
/// the header, [`Frame::data`], and the frame type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    /// Frame type.
    pub frame_type: FrameType,
    /// Destination station address; 255 is broadcast.
    pub destination: u8,
    /// Source station address.
    pub source: u8,
}

impl FrameHeader {
    /// Construct a header.
    #[must_use]
    pub const fn new(frame_type: FrameType, destination: u8, source: u8) -> Self {
        Self {
            frame_type,
            destination,
            source,
        }
    }
}

/// Check the header fields that both directions check, in the order they
/// check them (Clause 9.5.8): the source, then the Length against the bounds
/// of the type's class, then whether the codec supports a COBS type. So
/// [`FrameError::UnsupportedCobsFrame`] always comes with a good header.
pub(crate) fn check_header_fields(
    frame_type: FrameType,
    source: u8,
    length: usize,
) -> Result<(), FrameError> {
    if source == BROADCAST_MAC {
        return Err(FrameError::BroadcastSource);
    }
    if frame_type.is_cobs() {
        if !(MIN_COBS_LENGTH..=MAX_COBS_LENGTH).contains(&length) {
            return Err(FrameError::InvalidCobsLength { length });
        }
        if !frame_type.is_extended_data() {
            return Err(FrameError::UnsupportedCobsFrame {
                raw_type: frame_type.to_raw(),
            });
        }
    } else if length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::Oversize {
            length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    Ok(())
}

/// A frame whose data borrows from the caller or from decoder storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Frame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// The data field as the sender's client gave it: an NPDU, test data or
    /// proprietary content. An extended frame carries it COBS-encoded on the
    /// wire; here it is decoded.
    pub data: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Construct a frame.
    #[must_use]
    pub const fn new(frame_type: FrameType, destination: u8, source: u8, data: &'a [u8]) -> Self {
        Self {
            header: FrameHeader::new(frame_type, destination, source),
            data,
        }
    }

    /// Encode this frame at the start of `output` and return its wire length.
    ///
    /// The frame is checked first, as described in the [crate docs](crate):
    /// a source other than 255; for a standard type, at most
    /// [`MAX_STANDARD_MPDU_DATA`] octets of data, none on a type that never
    /// carries any, and a vendor identifier's two octets on a proprietary
    /// type; for a COBS type, a Length within the COBS bounds and one of the
    /// two extended data types. Nothing is written unless the frame passes
    /// and fits.
    ///
    /// ```
    /// use bacnet_mstp_codec::{Frame, FrameError, FrameType};
    ///
    /// let mut wire = [0u8; 8];
    /// let length = Frame::new(FrameType::TOKEN, 0x10, 0x05, &[]).encode_into(&mut wire)?;
    /// assert_eq!(wire[..length], [0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C]);
    ///
    /// // A Token never carries data.
    /// assert_eq!(
    ///     Frame::new(FrameType::TOKEN, 0x10, 0x05, &[1]).encode_into(&mut wire),
    ///     Err(FrameError::UnexpectedData { raw_type: 0, length: 1 }),
    /// );
    /// # Ok::<(), FrameError>(())
    /// ```
    pub fn encode_into(self, output: &mut [u8]) -> Result<usize, FrameError> {
        let mut sink = output;
        self.encode_to_sink(&mut sink)
    }

    /// Encode this frame into `sink` and return its wire length.
    ///
    /// The checks are those of [`Frame::encode_into`]. When the sink reports a
    /// fixed capacity, a frame that does not fit fails before anything is
    /// written. A growable sink is written directly, so if one of its writes
    /// fails, the octets written before it stay in the sink; see
    /// [`FrameSink`].
    pub fn encode_to_sink<S: FrameSink + ?Sized>(self, sink: &mut S) -> Result<usize, FrameError> {
        let length = self.checked_length()?;
        let total = wire_length(length);
        if let Some(available) = sink.remaining_capacity()
            && available < total
        {
            return Err(FrameError::BufferExhausted {
                needed: total,
                available,
            });
        }
        let fields = [
            self.header.frame_type.to_raw(),
            self.header.destination,
            self.header.source,
            (length >> 8) as u8,
            length as u8,
        ];
        let mut head = [0u8; DATA_OFFSET];
        head[..PREAMBLE.len()].copy_from_slice(&PREAMBLE);
        head[PREAMBLE.len()..DATA_OFFSET - 1].copy_from_slice(&fields);
        head[DATA_OFFSET - 1] = crc8(&fields);
        sink.write(&head)?;
        if self.header.frame_type.is_cobs() {
            cobs::write_fields(self.data, sink)?;
        } else if length != 0 {
            sink.write(self.data)?;
            sink.write(&crc16(self.data).to_le_bytes())?;
        }
        Ok(total)
    }

    /// Apply the encode checks and return the frame's Length field.
    fn checked_length(&self) -> Result<usize, FrameError> {
        let frame_type = self.header.frame_type;
        let length = if frame_type.is_cobs() {
            cobs::frame_length(self.data)
        } else {
            self.data.len()
        };
        check_header_fields(frame_type, self.header.source, length)?;
        if length != 0 && !frame_type.allows_data() {
            return Err(FrameError::UnexpectedData {
                raw_type: frame_type.to_raw(),
                length,
            });
        }
        if length < 2 && frame_type.is_proprietary() {
            return Err(FrameError::MissingVendorId {
                raw_type: frame_type.to_raw(),
                length,
            });
        }
        Ok(length)
    }
}
