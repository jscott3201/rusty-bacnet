//! Caller-buffered incremental decoding.

use crate::{decode_frame, Frame, FrameError, PREAMBLE};

/// Result of feeding one or more octets to [`StreamDecoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeEvent<'a> {
    /// The input does not yet contain a complete frame.
    NeedMore,
    /// A validated frame is available. It remains borrowed until the next push.
    Frame {
        /// Validated frame borrowing decoder storage.
        frame: Frame<'a>,
        /// Number of input octets occupied by the frame.
        consumed: usize,
    },
    /// Non-frame input was discarded while looking for the next preamble.
    ///
    /// This is synchronization noise, not a received-invalid-frame event in
    /// the Clause 9 receive state machine.
    Invalid {
        /// Number of synchronization-noise octets discarded.
        discard: usize,
    },
    /// A candidate frame was malformed and discarded.
    Malformed {
        /// Number of candidate octets discarded while resynchronizing.
        discard: usize,
        /// Validation failure for the malformed candidate.
        error: FrameError,
    },
    /// The caller-owned receive buffer could not accept another octet.
    Error(FrameError),
}

/// Receive phase represented by the decoder's retained bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPhase {
    /// Searching for the first preamble octet; retained bytes are only noise.
    Sync,
    /// The first preamble octet has been recognized.
    Preamble,
    /// Both preamble octets are present and the fixed header is incomplete.
    Header,
    /// A valid fixed header is present and frame data remains incomplete.
    Data,
}

/// Incremental decoder backed by a caller-owned receive buffer.
pub struct StreamDecoder<'a> {
    storage: &'a mut [u8],
    length: usize,
    pending: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalEvent {
    NeedMore,
    Frame { consumed: usize },
    Invalid { discard: usize },
    Malformed { discard: usize, error: FrameError },
}

impl<'a> StreamDecoder<'a> {
    /// Create an empty decoder over `storage`.
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self {
            storage,
            length: 0,
            pending: 0,
        }
    }

    /// Number of buffered octets, including a frame awaiting the next push.
    pub const fn buffered_len(&self) -> usize {
        self.length
    }

    /// Capacity of the caller-owned receive buffer.
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return the semantic receive phase, excluding retained resync noise.
    pub fn phase(&self) -> StreamPhase {
        if self.pending != 0 || self.length == 0 || self.storage[0] != PREAMBLE[0] {
            return StreamPhase::Sync;
        }
        if self.length == 1 {
            return StreamPhase::Preamble;
        }
        if self.storage[1] != PREAMBLE[1] {
            return StreamPhase::Sync;
        }
        if self.length < 8 {
            StreamPhase::Header
        } else {
            StreamPhase::Data
        }
    }

    /// Clear all buffered input and pending frame state.
    pub fn reset(&mut self) {
        self.length = 0;
        self.pending = 0;
    }

    /// Feed one octet and return the first resulting decode event.
    pub fn push(&mut self, byte: u8) -> DecodeEvent<'_> {
        self.push_slice(core::slice::from_ref(&byte))
    }

    /// Feed a DMA or serial slice and return the first resulting event.
    pub fn push_slice(&mut self, input: &[u8]) -> DecodeEvent<'_> {
        self.commit_pending();
        if input.len() > self.storage.len().saturating_sub(self.length) {
            return DecodeEvent::Error(FrameError::BufferExhausted {
                needed: self.length + input.len(),
                available: self.storage.len(),
            });
        }
        let end = self.length + input.len();
        self.storage[self.length..end].copy_from_slice(input);
        self.length = end;
        match self.try_decode() {
            InternalEvent::NeedMore => DecodeEvent::NeedMore,
            InternalEvent::Frame { consumed } => self.frame_event(consumed),
            InternalEvent::Invalid { discard } => DecodeEvent::Invalid { discard },
            InternalEvent::Malformed { discard, error } => {
                DecodeEvent::Malformed { discard, error }
            }
        }
    }

    fn commit_pending(&mut self) {
        if self.pending == 0 {
            return;
        }
        let consumed = self.pending.min(self.length);
        let remaining = self.length - consumed;
        self.storage.copy_within(consumed..self.length, 0);
        self.length = remaining;
        self.pending = 0;
    }

    fn frame_event(&self, consumed: usize) -> DecodeEvent<'_> {
        match decode_frame(&self.storage[..self.length]) {
            Ok((frame, _)) => DecodeEvent::Frame { frame, consumed },
            Err(error) => DecodeEvent::Error(error),
        }
    }

    fn try_decode(&mut self) -> InternalEvent {
        if self.length == 0 {
            return InternalEvent::NeedMore;
        }
        if self.storage[0] != PREAMBLE[0] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        if self.length < 2 {
            return InternalEvent::NeedMore;
        }
        if self.storage[1] != PREAMBLE[1] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        match decode_frame(&self.storage[..self.length]) {
            Ok((_, consumed)) => {
                self.pending = consumed;
                InternalEvent::Frame { consumed }
            }
            Err(FrameError::TooShort { .. } | FrameError::Truncated { .. }) => {
                InternalEvent::NeedMore
            }
            Err(error @ FrameError::DataCrcMismatch { data_length }) => {
                let discard = if self.length >= 8 {
                    8 + data_length + 2
                } else {
                    1
                };
                self.discard(discard.min(self.length));
                InternalEvent::Malformed { discard, error }
            }
            Err(error) => {
                self.discard(1);
                InternalEvent::Malformed { discard: 1, error }
            }
        }
    }

    fn discard(&mut self, count: usize) {
        let count = count.min(self.length);
        let remaining = self.length - count;
        self.storage.copy_within(count..self.length, 0);
        self.length = remaining;
    }
}
