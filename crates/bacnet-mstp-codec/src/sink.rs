//! Byte sinks for [`Frame::encode_to_sink`](crate::Frame::encode_to_sink).

use core::fmt;

/// A destination for encoded frame octets.
///
/// [`Frame::encode_to_sink`](crate::Frame::encode_to_sink) asks for
/// [`remaining_capacity`](FrameSink::remaining_capacity) first. A sink that
/// reports one is never handed a frame that does not fit, so the fixed sinks in
/// this crate never hold part of a frame. A sink that reports `None` can grow,
/// and is written without that check: if one of its writes fails after an
/// earlier one succeeded, the earlier octets stay in it, and the caller has to
/// discard them.
pub trait FrameSink {
    /// Octets of room left, or `None` when the sink grows as needed.
    fn remaining_capacity(&self) -> Option<usize>;

    /// Append `bytes`, or fail without appending any of them.
    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>;
}

/// A [`FrameSink`] had no room for a write.
///
/// [`Frame::encode_to_sink`](crate::Frame::encode_to_sink) reports it as
/// [`FrameError::BufferExhausted`](crate::FrameError::BufferExhausted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkFull {
    /// Octets the write needed.
    pub needed: usize,
    /// Room the sink had.
    pub available: usize,
}

impl fmt::Display for SinkFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sink full: need {}, have {}",
            self.needed, self.available
        )
    }
}

impl core::error::Error for SinkFull {}

/// Writes at the front of the slice and advances it past what was written,
/// so after a successful encode the slice holds the unused tail.
impl FrameSink for &mut [u8] {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        if self.len() < bytes.len() {
            return Err(SinkFull {
                needed: bytes.len(),
                available: self.len(),
            });
        }
        let (head, tail) = core::mem::take(self).split_at_mut(bytes.len());
        head.copy_from_slice(bytes);
        *self = tail;
        Ok(())
    }
}

#[cfg(feature = "heapless")]
impl<const N: usize> FrameSink for heapless::Vec<u8, N> {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(N - self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        let available = N - self.len();
        self.extend_from_slice(bytes).map_err(|_| SinkFull {
            needed: bytes.len(),
            available,
        })
    }
}

#[cfg(feature = "alloc")]
impl FrameSink for alloc::vec::Vec<u8> {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(feature = "bytes")]
impl FrameSink for bytes::BytesMut {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}
