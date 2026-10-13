//! Incremental decoding over caller-owned storage.

use core::fmt;

use crate::decode::{check_frame, decode_extended_in_place};
use crate::frame::{wire_length, DATA_OFFSET};
use crate::{Frame, FrameError, FrameHeader, PREAMBLE};

/// The outcome of one [`StreamDecoder::push_slice`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the event may be a frame, which the next push drops"]
pub enum DecodeEvent<'a> {
    /// Nothing complete and nothing to discard: push more octets.
    NeedMore,
    /// A valid frame. It borrows the decoder's storage and stays buffered
    /// until the next push, which removes it.
    Frame {
        /// The decoded frame.
        frame: Frame<'a>,
        /// The frame's wire length.
        consumed: usize,
    },
    /// Line noise was discarded while searching for a preamble: every
    /// buffered octet up to the next `0x55` among them, which might start
    /// one, in a single event. A run of noise that arrives over several
    /// pushes gives one event per push.
    ///
    /// This is not a received invalid frame in terms of the Clause 9.5.4
    /// receive state machine.
    Invalid {
        /// Octets discarded.
        discard: usize,
    },
    /// A candidate frame failed a check and was dropped.
    Malformed {
        /// Octets of the stream discarded, counted from the candidate's
        /// first octet.
        ///
        /// When its header passed every check, its declared length can be
        /// trusted, so the decoder drops the whole frame by count, as the
        /// receive state machine skips one (Clause 9.5.4.4): for a frame
        /// longer than the storage, a reserved COBS type, or data that failed
        /// its CRC. This is then the frame's wire length, and any of it that
        /// hasn't arrived yet is dropped as it comes, with no further event.
        /// For anything else, a bad header or one that never completes, it is
        /// the first preamble octet, and the octets after it are scanned
        /// again.
        discard: usize,
        /// Why the candidate was dropped.
        error: FrameError,
        /// The candidate's destination address once its header CRC has
        /// passed, including for a header refused afterwards (source 255, a
        /// Length out of range). `None` when the header CRC failed or the
        /// header never completed. See [`StreamDecoder`] for why a receiver
        /// needs it.
        destination: Option<u8>,
    },
    /// The pushed slice did not fit in the free storage. None of it was
    /// taken; see [`StreamDecoder`].
    Error(FrameError),
}

/// Receive phase represented by the decoder's retained bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamPhase {
    /// Searching for the first preamble octet; retained bytes are only noise.
    Sync,
    /// The first preamble octet has been recognized.
    Preamble,
    /// Both preamble octets are present and the fixed header is incomplete.
    Header,
    /// A valid fixed header is present and the rest of its frame hasn't all
    /// arrived, whether the decoder is buffering the frame or skipping it by
    /// count.
    Data,
}

/// Incremental frame decoder over a buffer the caller lends it.
///
/// Feed it received octets with [`push`](Self::push) or
/// [`push_slice`](Self::push_slice). Each call buffers the input and reports
/// at most one [`DecodeEvent`], so:
///
/// - A [`DecodeEvent::Frame`] borrows the storage. The frame stays buffered
///   until the next push, which commits it (drops it from the buffer) before
///   it reads new input. An empty push commits it too.
/// - After any event other than [`DecodeEvent::NeedMore`], more may already be
///   buffered, such as a second frame from the same slice or more noise. Call
///   `push_slice(&[])` until it returns `NeedMore` to drain it.
/// - A slice longer than the free storage is refused whole with
///   [`DecodeEvent::Error`] carrying [`FrameError::BufferExhausted`] (the
///   slice's length and the free room), and none of it is taken. Drain, then
///   push it again or in smaller pieces. Octets that finish a frame being
///   skipped by count need no room, and don't count toward the slice's
///   length.
///
/// An extended frame is decoded in place, so the frame's data is its decoded
/// NPDU, and the storage it was received in holds that from the octet after
/// its header.
///
/// # Storage and skipping
///
/// Size the storage for the largest frame you accept, plus room for the
/// slices you push: [`MAX_STANDARD_FRAME_LENGTH`](crate::MAX_STANDARD_FRAME_LENGTH)
/// for every standard frame, or
/// [`MAX_EXTENDED_FRAME_LENGTH`](crate::MAX_EXTENDED_FRAME_LENGTH) for
/// extended frames too (see the [crate docs](crate)). A candidate that could
/// never fit is reported once, as [`DecodeEvent::Malformed`] with
/// [`FrameError::FrameTooLong`], as soon as its header shows its size, and
/// is then skipped by count, so the decoder never wedges. Storage too small
/// for even a header reports a candidate at its first octet and drops that
/// octet.
///
/// Once a header passes its CRC and the header checks of Clause 9.5.8, its
/// Length can be trusted: a frame dropped after that point (too long for the
/// storage, a reserved COBS type, or data that fails its CRC) is skipped
/// whole, as the receive state machine skips one (Clause 9.5.4.4), and
/// nothing inside it can come out as a frame. A header that fails its CRC or
/// those checks says nothing reliable about the frame's length, so only its
/// first octet is dropped and the octets after it are scanned again. A
/// decoder built [`with_reserved_cobs`](Self::with_reserved_cobs) buffers a
/// reserved COBS type and checks it like an extended frame instead.
///
/// # Destinations
///
/// The decoder does not know which station it serves, so it reports every
/// candidate it drops, whatever the destination. A receive state machine
/// (Clause 9.5.4.4) only counts an oversized, unsupported or bad-data-CRC
/// frame as invalid when it is addressed to that station or broadcast, and
/// skips one meant for another station without complaint. Such a caller
/// should check the `destination` of a [`DecodeEvent::Malformed`] carrying
/// [`FrameError::FrameTooLong`], [`FrameError::UnsupportedCobsFrame`] or
/// [`FrameError::DataCrcMismatch`] before treating it as an invalid frame. A
/// bad header (a CRC failure, source 255, a Length out of range) counts
/// wherever it was going. Since the header checks come before the type
/// check, an unsupported COBS type always has a good header.
///
/// The decoder keeps no time. A receiver that aborts a partial frame after a
/// silence (Clause 9.5.4) calls [`reset`](Self::reset), which also ends a
/// skip.
///
/// ```
/// use bacnet_mstp_codec::{DecodeEvent, Frame, FrameType, StreamDecoder};
///
/// let mut wire = [0u8; 16];
/// Frame::new(FrameType::TOKEN, 1, 0, &[]).encode_into(&mut wire[..8])?;
/// Frame::new(FrameType::TOKEN, 2, 1, &[]).encode_into(&mut wire[8..])?;
///
/// let mut storage = [0u8; 64];
/// let mut decoder = StreamDecoder::new(&mut storage);
/// let mut destinations = Vec::new();
/// let mut event = decoder.push_slice(&wire);
/// loop {
///     match event {
///         DecodeEvent::NeedMore => break,
///         DecodeEvent::Frame { frame, .. } => destinations.push(frame.header.destination),
///         _ => {}
///     }
///     // Commit the frame, if any, and look for the next one.
///     event = decoder.push_slice(&[]);
/// }
/// assert_eq!(destinations, [1, 2]);
/// assert_eq!(decoder.buffered_len(), 0);
/// # Ok::<(), bacnet_mstp_codec::FrameError>(())
/// ```
pub struct StreamDecoder<'a> {
    storage: &'a mut [u8],
    /// Where the buffered octets begin. Octets the decoder is done with are
    /// dropped by moving this past them, not by shifting the rest down, and
    /// it goes back to 0 whenever nothing is buffered.
    start: usize,
    /// How many octets are buffered, from `start`.
    length: usize,
    /// Wire length of the frame the last push delivered, which the next
    /// push commits.
    pending: usize,
    /// Octets of a dropped frame still to come, which pushes drop by count.
    /// While it is nonzero nothing is buffered.
    skip: usize,
    /// Reserved COBS types are checked and yielded, not skipped; see
    /// [`Self::with_reserved_cobs`].
    reserved_cobs: bool,
}

/// The decoder's counts, phase and mode; the storage's contents are left out.
impl fmt::Debug for StreamDecoder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamDecoder")
            .field("capacity", &self.capacity())
            .field("buffered", &self.length)
            .field("pending", &self.pending)
            .field("skip", &self.skip)
            .field("phase", &self.phase())
            .field("reserved_cobs", &self.reserved_cobs)
            .finish()
    }
}

/// Offset of the destination octet: after the preamble and the type octet.
const DESTINATION_INDEX: usize = PREAMBLE.len() + 1;
/// Offset of the Length field's two octets, most significant first.
const LENGTH_INDEX: usize = PREAMBLE.len() + 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalEvent {
    NeedMore,
    Frame {
        header: FrameHeader,
        data_length: usize,
        consumed: usize,
    },
    Invalid {
        discard: usize,
    },
    Malformed {
        discard: usize,
        error: FrameError,
        destination: Option<u8>,
    },
}

impl<'a> StreamDecoder<'a> {
    /// Create an empty decoder over `storage`.
    #[must_use]
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self {
            storage,
            start: 0,
            length: 0,
            pending: 0,
            skip: 0,
            reserved_cobs: false,
        }
    }

    /// Check the reserved COBS-encoded types, 34-127, as a receiver that
    /// supports COBS-encoded frames does, rather than skip them.
    ///
    /// Such a receiver takes the encoded fields of every COBS type in, and
    /// a frame whose CRC-32K checks out is a valid frame of a type it
    /// doesn't know (Clause 9.5.4.9), for its state machine to ignore or
    /// treat as unexpected. So a reserved type whose header passes is
    /// buffered like an extended frame: when it fits the storage and its
    /// encoded fields check out, it comes out as a [`DecodeEvent::Frame`]
    /// with its type, which no [`FrameType`](crate::FrameType) constant
    /// names, and its decoded data, and otherwise it is dropped and skipped
    /// by count, with [`FrameError::DataCrcMismatch`] or
    /// [`FrameError::FrameTooLong`].
    /// [`FrameError::UnsupportedCobsFrame`] never comes out.
    ///
    /// A decoder from [`Self::new`] skips them by count as soon as their
    /// header passes, as a receiver without COBS support does, and reports
    /// [`FrameError::UnsupportedCobsFrame`].
    #[must_use]
    pub const fn with_reserved_cobs(mut self) -> Self {
        self.reserved_cobs = true;
        self
    }

    /// Whether the decoder checks reserved COBS types; see
    /// [`Self::with_reserved_cobs`].
    #[must_use]
    pub const fn checks_reserved_cobs(&self) -> bool {
        self.reserved_cobs
    }

    /// Number of buffered octets, including a frame awaiting the next push.
    /// Octets of a frame being skipped by count are never buffered.
    #[must_use]
    pub const fn buffered_len(&self) -> usize {
        self.length
    }

    /// Capacity of the caller-owned receive buffer.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return the semantic receive phase, excluding retained resync noise.
    #[must_use]
    pub fn phase(&self) -> StreamPhase {
        if self.skip != 0 {
            return StreamPhase::Data;
        }
        let buffered = self.buffered();
        if self.pending != 0 || buffered.first() != Some(&PREAMBLE[0]) {
            return StreamPhase::Sync;
        }
        if self.length == 1 {
            return StreamPhase::Preamble;
        }
        if buffered[1] != PREAMBLE[1] {
            return StreamPhase::Sync;
        }
        if self.length < DATA_OFFSET {
            StreamPhase::Header
        } else {
            StreamPhase::Data
        }
    }

    /// Clear all buffered input, the pending frame and any skip.
    pub fn reset(&mut self) {
        self.start = 0;
        self.length = 0;
        self.pending = 0;
        self.skip = 0;
    }

    /// Feed one octet; see [`push_slice`](Self::push_slice).
    pub fn push(&mut self, byte: u8) -> DecodeEvent<'_> {
        self.push_slice(core::slice::from_ref(&byte))
    }

    /// Commit a frame delivered by the previous push, take `input` (dropping
    /// what finishes a frame being skipped and buffering the rest), and
    /// report the first resulting event. The [type docs](Self) describe the
    /// protocol.
    pub fn push_slice(&mut self, input: &[u8]) -> DecodeEvent<'_> {
        self.commit_pending();
        let skipped = self.skip.min(input.len());
        let rest = &input[skipped..];
        let free = self.storage.len() - self.length;
        if rest.len() > free {
            return DecodeEvent::Error(FrameError::BufferExhausted {
                needed: rest.len(),
                available: free,
            });
        }
        self.skip -= skipped;
        if self.start + self.length + rest.len() > self.storage.len() {
            self.compact();
        }
        let end = self.start + self.length;
        self.storage[end..end + rest.len()].copy_from_slice(rest);
        self.length += rest.len();
        match self.try_decode() {
            InternalEvent::NeedMore => DecodeEvent::NeedMore,
            InternalEvent::Frame {
                header,
                data_length,
                consumed,
            } => DecodeEvent::Frame {
                frame: Frame {
                    header,
                    data: &self.buffered()[DATA_OFFSET..DATA_OFFSET + data_length],
                },
                consumed,
            },
            InternalEvent::Invalid { discard } => DecodeEvent::Invalid { discard },
            InternalEvent::Malformed {
                discard,
                error,
                destination,
            } => DecodeEvent::Malformed {
                discard,
                error,
                destination,
            },
        }
    }

    fn commit_pending(&mut self) {
        if self.pending == 0 {
            return;
        }
        self.discard(self.pending);
        self.pending = 0;
    }

    fn try_decode(&mut self) -> InternalEvent {
        let buffered = self.buffered();
        let Some(&first) = buffered.first() else {
            return InternalEvent::NeedMore;
        };
        let may_start_preamble =
            first == PREAMBLE[0] && buffered.get(1).is_none_or(|&second| second == PREAMBLE[1]);
        if !may_start_preamble {
            // Noise, up to the next octet that might start a preamble.
            let discard = buffered[1..]
                .iter()
                .position(|&octet| octet == PREAMBLE[0])
                .map_or(buffered.len(), |index| index + 1);
            self.discard(discard);
            return InternalEvent::Invalid { discard };
        }
        let error = match check_frame(buffered, self.reserved_cobs) {
            Ok(valid) => {
                let end = self.start + self.length;
                decode_extended_in_place(&mut self.storage[self.start..end], &valid);
                self.pending = valid.consumed;
                return InternalEvent::Frame {
                    header: valid.header,
                    data_length: valid.data_length,
                    consumed: valid.consumed,
                };
            }
            Err(error) => error,
        };
        // Errors past the header CRC come with the whole header buffered,
        // so its destination octet can be trusted. Every variant is listed,
        // so a new error has to say which side of the CRC it falls on.
        let destination = match error {
            FrameError::TooShort { .. }
            | FrameError::InvalidPreamble { .. }
            | FrameError::HeaderCrcMismatch => None,
            FrameError::BroadcastSource
            | FrameError::InvalidCobsLength { .. }
            | FrameError::Oversize { .. }
            | FrameError::UnsupportedCobsFrame { .. }
            | FrameError::Truncated { .. }
            | FrameError::DataCrcMismatch { .. } => Some(self.buffered()[DESTINATION_INDEX]),
            // check_frame never returns these: they come from decode_frame,
            // from encoding or from the stream decoder itself.
            FrameError::ExtendedFrame { .. }
            | FrameError::UnexpectedData { .. }
            | FrameError::MissingVendorId { .. }
            | FrameError::FrameTooLong { .. }
            | FrameError::BufferExhausted { .. } => None,
        };
        match error {
            FrameError::TooShort { needed, .. } => self.wait_for_header(needed),
            FrameError::Truncated { length, .. } => {
                let needed = wire_length(length);
                let capacity = self.storage.len();
                if needed <= capacity {
                    return InternalEvent::NeedMore;
                }
                self.skip_frame(
                    needed,
                    FrameError::FrameTooLong { needed, capacity },
                    destination,
                )
            }
            FrameError::UnsupportedCobsFrame { .. } => {
                let length = self.declared_length();
                self.skip_frame(wire_length(length), error, destination)
            }
            FrameError::DataCrcMismatch { length } => {
                self.skip_frame(wire_length(length), error, destination)
            }
            _ => {
                self.discard(1);
                InternalEvent::Malformed {
                    discard: 1,
                    error,
                    destination,
                }
            }
        }
    }

    /// Wait for the rest of a header, unless the storage can't hold the
    /// `needed` octets of one: then drop the first octet so the decoder
    /// resynchronizes instead of filling up for good.
    fn wait_for_header(&mut self, needed: usize) -> InternalEvent {
        let capacity = self.storage.len();
        if needed <= capacity {
            return InternalEvent::NeedMore;
        }
        self.discard(1);
        InternalEvent::Malformed {
            discard: 1,
            error: FrameError::FrameTooLong { needed, capacity },
            destination: None,
        }
    }

    /// The Length field of the header at the front, which is complete.
    fn declared_length(&self) -> usize {
        let buffered = self.buffered();
        usize::from(u16::from_be_bytes([
            buffered[LENGTH_INDEX],
            buffered[LENGTH_INDEX + 1],
        ]))
    }

    /// Drop the candidate at the front, whose good header declares `total`
    /// octets in all, by count: what is buffered of it now, and the rest as
    /// it arrives.
    fn skip_frame(
        &mut self,
        total: usize,
        error: FrameError,
        destination: Option<u8>,
    ) -> InternalEvent {
        if total <= self.length {
            self.discard(total);
        } else {
            self.skip = total - self.length;
            self.start = 0;
            self.length = 0;
        }
        InternalEvent::Malformed {
            discard: total,
            error,
            destination,
        }
    }

    /// The buffered octets.
    fn buffered(&self) -> &[u8] {
        &self.storage[self.start..self.start + self.length]
    }

    /// Drop the first `count` buffered octets, without moving the rest.
    fn discard(&mut self, count: usize) {
        let count = count.min(self.length);
        self.start += count;
        self.length -= count;
        if self.length == 0 {
            self.start = 0;
        }
    }

    /// Move the buffered octets to the front of the storage, to make room
    /// after them. They go a piece at a time, each piece no longer than the
    /// free octets in front of it, so no copy overlaps its source: a plain
    /// copy does each, not a `memmove`.
    fn compact(&mut self) {
        let gap = self.start;
        if gap == 0 {
            return;
        }
        let end = self.start + self.length;
        let mut from = gap;
        while from < end {
            let count = gap.min(end - from);
            let (front, back) = self.storage.split_at_mut(from);
            front[from - gap..from - gap + count].copy_from_slice(&back[..count]);
            from += count;
        }
        self.start = 0;
    }
}
