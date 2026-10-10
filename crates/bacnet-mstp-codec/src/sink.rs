//! Small storage adapters for encoded frames.

use crate::{crc16, crc8, Frame, FrameError, HEADER_LENGTH, MAX_STANDARD_MPDU_DATA, PREAMBLE};

/// A destination that can accept an encoded frame without allocating.
pub trait FrameSink {
    /// Remaining fixed capacity, or `None` when the sink can grow.
    fn remaining_capacity(&self) -> Option<usize>;

    /// Append bytes to this sink.
    fn write(&mut self, bytes: &[u8]) -> Result<(), FrameError>;

    /// Append one byte to this sink.
    fn push(&mut self, byte: u8) -> Result<(), FrameError> {
        self.write(core::slice::from_ref(&byte))
    }

    /// Append a byte slice to this sink.
    fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        self.write(bytes)
    }

    /// Encode `frame` into this sink.
    fn encode<'a>(&mut self, frame: Frame<'a>) -> Result<usize, FrameError> {
        let data_length = frame.data.len();
        let raw_type = frame.header.frame_type.to_raw();
        if frame.header.frame_type.is_cobs() {
            return Err(FrameError::EncodeUnsupportedCobsFrame { raw_type });
        }
        if data_length > MAX_STANDARD_MPDU_DATA {
            return Err(FrameError::EncodeOversize {
                length: data_length,
                maximum: MAX_STANDARD_MPDU_DATA,
            });
        }
        if frame.header.data_length != data_length {
            return Err(FrameError::HeaderLengthMismatch {
                declared: frame.header.data_length,
                actual: data_length,
            });
        }
        let length =
            PREAMBLE.len() + HEADER_LENGTH + data_length + if data_length == 0 { 0 } else { 2 };
        if let Some(available) = self.remaining_capacity() {
            if available < length {
                return Err(FrameError::BufferExhausted {
                    needed: length,
                    available,
                });
            }
        }

        self.write(&PREAMBLE)?;
        let header = [
            raw_type,
            frame.header.destination,
            frame.header.source,
            (data_length >> 8) as u8,
            data_length as u8,
        ];
        self.write(&header)?;
        self.write(core::slice::from_ref(&crc8(&header)))?;
        if data_length != 0 {
            self.write(frame.data)?;
            self.write(&crc16(frame.data).to_le_bytes())?;
        }
        Ok(length)
    }
}

/// Encode a frame into any [`FrameSink`].
pub fn encode_to_sink<'a, S: FrameSink>(
    sink: &mut S,
    frame: Frame<'a>,
) -> Result<usize, FrameError> {
    sink.encode(frame)
}

impl<'a> FrameSink for &'a mut [u8] {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        if self.len() < bytes.len() {
            return Err(FrameError::BufferExhausted {
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
        Some(N.saturating_sub(self.len()))
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        self.extend_from_slice(bytes)
            .map_err(|_| FrameError::BufferExhausted {
                needed: bytes.len(),
                available: N.saturating_sub(self.len()),
            })
    }
}

#[cfg(feature = "std")]
impl FrameSink for bytes::BytesMut {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}
