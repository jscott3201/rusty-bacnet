//! An allocation-free codec for BACnet MS/TP frames (Clause 9): the standard
//! frames and the two extended data frames.
//!
//! The crate covers the wire format and nothing above it: frame types, the
//! header and data CRCs, the COBS encoding and CRC-32K of extended frames,
//! one-shot encode and decode, and an incremental decoder for a serial byte
//! stream. Token passing, timers and line control belong to the layer that
//! uses it.
//!
//! The default build never allocates. [`Frame`] borrows its data,
//! [`Frame::encode_into`] writes into caller storage, and [`StreamDecoder`]
//! keeps its receive state in a buffer the caller lends it. Only the optional
//! `alloc` and `bytes` sinks grow on the heap.
//!
//! # Extended frames
//!
//! Types 32 and 33 ([`FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY`] and
//! [`FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY`]) carry NPDUs too long
//! for a standard frame (Clauses 9.3.9, 9.3.10).
//! On the wire their data is COBS-encoded with a CRC-32K, so no preamble can
//! appear inside them (Clause 9.10), and the header's Length counts the two
//! encoded fields less two, which lets a receiver without COBS skip them by
//! count (Clause 9.3). [`Frame::data`] is always the decoded data: encoding
//! takes it plain, and decoding hands it back plain. Decoding an extended
//! frame writes its data over its encoded form, so it takes writable input:
//! [`decode_frame_in_place`], or a [`StreamDecoder`], which does it in its
//! own storage. [`decode_frame`] borrows read-only input and refuses an
//! extended frame with [`FrameError::ExtendedFrame`] as soon as its header
//! checks out.
//!
//! The other COBS-encoded types, 34-127, are reserved: both directions refuse
//! them with [`FrameError::UnsupportedCobsFrame`], after the header's other
//! checks. The exception is a [`StreamDecoder`] built
//! [`with_reserved_cobs`](StreamDecoder::with_reserved_cobs), for a receiver
//! that supports COBS: it checks their CRC-32K as an extended frame's and
//! yields a good one as a frame of a type it doesn't know. Test_Request and
//! Test_Response stay standard frames.
//!
//! # Validation
//!
//! Decoding is a raw, link-level check. [`decode_frame`] verifies the
//! preamble, the header CRC, the header fields Clause 9.5.8 checks, then the
//! frame's data. The header checks run in this order: the source, which may
//! not be 255, the broadcast address no station may send from (Clause 9.3);
//! the Length, at most [`MAX_STANDARD_MPDU_DATA`] on a standard type and
//! within [`MIN_COBS_LENGTH`]..=[`MAX_COBS_LENGTH`] on a COBS-encoded one;
//! and last whether the codec supports the type. Decoding accepts every other
//! source, including the slave-only range 128-254: whether a station may use
//! its address in its role is for the receiver to judge. It also accepts data
//! on a frame type that never carries any, and a proprietary frame too short
//! for its vendor identifier, so a receiver can see what arrived.
//!
//! Encoding is strict. [`Frame::encode_into`] and [`Frame::encode_to_sink`]
//! apply the same header checks in the same order, then refuse data on the
//! four frame types defined without any (see [`FrameType::allows_data`]) and
//! a proprietary frame (128-255) with less than its 2-octet vendor identifier
//! (Clause 9.3.11). The reserved types 8-31 get the same CRC, source and
//! length checks as any frame in both directions, but the codec doesn't
//! police their content. An extended frame needs data, since an empty one
//! would fall below [`MIN_COBS_LENGTH`]; the BACnet range of 502 to
//! [`MAX_EXTENDED_MPDU_DATA`] octets for it is the sending layer's to keep.
//!
//! # Storage
//!
//! Each frame class has a constant for its longest frame on the wire, which
//! is the [`StreamDecoder`] storage it takes (plus room for the slices pushed
//! into it):
//!
//! - [`MAX_STANDARD_FRAME_LENGTH`] (511): every standard frame.
//! - [`MAX_EXTENDED_FRAME_LENGTH`] (1516): every extended frame that carries
//!   a BACnet NPDU, at most [`MAX_EXTENDED_MPDU_DATA`] octets.
//! - [`MAX_COBS_FRAME_LENGTH`] (2053): every frame whose header passes, a
//!   COBS-encoded one at the largest Length included.
//!
//! A frame longer than the storage is reported once and skipped; see
//! [`StreamDecoder`].
//!
//! # Features
//!
//! No feature needs `std`. Most add a [`FrameSink`] implementation:
//!
//! - `alloc`: for `alloc::vec::Vec<u8>`.
//! - `bytes`: for `bytes::BytesMut`; implies `alloc`.
//! - `heapless`: for `heapless::Vec<u8, N>`.
//! - `crc32k-table`: works out the CRC-32K with a 256-entry table, about
//!   1 KiB more flash than the default 16-entry one and half the lookups (see
//!   [`crc::crc32k`]).
//!
//! # Example
//!
//! ```
//! use bacnet_mstp_codec::{decode_frame, Frame, FrameType, MAX_STANDARD_FRAME_LENGTH};
//!
//! let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0x10, 0x05, &[0x01, 0x00]);
//! let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
//! let length = frame.encode_into(&mut wire)?;
//!
//! let (decoded, consumed) = decode_frame(&wire[..length])?;
//! assert_eq!(decoded, frame);
//! assert_eq!(consumed, length);
//! # Ok::<(), bacnet_mstp_codec::FrameError>(())
//! ```

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, clippy::must_use_candidate)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod cobs;
pub mod crc;
mod decode;
mod error;
mod frame;
mod sink;
mod stream;

pub use decode::{decode_frame, decode_frame_in_place, find_preamble};
pub use error::FrameError;
pub use frame::{Frame, FrameHeader, FrameType};
pub use sink::{FrameSink, SinkFull};
pub use stream::{DecodeEvent, StreamDecoder, StreamPhase};

/// The two preamble octets that start every frame.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];
/// Header length after the preamble: type, destination, source, the two
/// length octets and the header CRC.
pub const HEADER_LENGTH: usize = 6;
/// Largest data field a standard (non-COBS) frame can carry.
pub const MAX_STANDARD_MPDU_DATA: usize = 501;
/// Highest station address a master may use. Slaves may also use 128-254.
pub const MAX_MASTER: u8 = 127;
/// The broadcast address. It is valid only as a destination.
pub const BROADCAST_MAC: u8 = 0xFF;
/// Longest standard frame on the wire: preamble, header, the largest data
/// field and its CRC.
pub const MAX_STANDARD_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_STANDARD_MPDU_DATA + 2;
/// Smallest Length a COBS-encoded frame's header may declare: one data
/// octet takes two encoded, the CRC field always takes five, and the Length
/// leaves out two (Clause 9.5.3, Nmin_COBS_length).
pub const MIN_COBS_LENGTH: usize = 5;
/// Largest Length a COBS-encoded frame's header may declare (Clause 9.5.3,
/// Nmax_COBS_length).
pub const MAX_COBS_LENGTH: usize = 2043;
/// Largest NPDU an extended frame carries (Clauses 9.3.9, 9.3.10).
pub const MAX_EXTENDED_MPDU_DATA: usize = 1497;
/// Longest extended frame carrying a BACnet NPDU: preamble, header, the
/// largest NPDU with COBS's worst-case overhead of one octet per 254, and the
/// Encoded CRC-32K (Clause 9.5.2 sizes a receive buffer the same way).
pub const MAX_EXTENDED_FRAME_LENGTH: usize = PREAMBLE.len()
    + HEADER_LENGTH
    + MAX_EXTENDED_MPDU_DATA
    + MAX_EXTENDED_MPDU_DATA.div_ceil(254)
    + cobs::ENCODED_CRC_LENGTH;
/// Longest frame whose header passes validation: a COBS-encoded frame of
/// the largest Length.
pub const MAX_COBS_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_COBS_LENGTH + cobs::LENGTH_ADJUSTMENT;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
