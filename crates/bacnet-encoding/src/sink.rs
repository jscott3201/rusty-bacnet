//! Append-only output for the sink-based encoders.
//!
//! The tag-header functions in [`crate::tags`], the bounded scalar, timestamp
//! and string encoders in [`crate::primitives`], the bounded references in
//! [`crate::constructed`], the APDU header writer in [`crate::apdu::header`]
//! and the allocation-free services in bacnet-services support these sinks.
//! Other encoder families still use their `BytesMut` buffers. The sink
//! contract and the slice and `heapless` adapters need no allocator.

use core::fmt;

/// Storage that accepts a complete byte slice or leaves its output unchanged.
///
/// A write appends after earlier output. On failure, neither the output bytes
/// nor the output position may change. Implementers must uphold that contract;
/// it is not a partial-I/O interface.
///
/// The tag-header, bounded scalar, timestamp and reference encoders check
/// advertised capacity, then write their complete output once. A
/// variable-length value, such as a string with its header, is written with
/// [`write_parts`](Self::write_parts), which is atomic for the whole value.
/// Neither promises rollback across several encoder calls: a custom sink can
/// refuse a later write even after a total-capacity check, leaving earlier
/// writes in place.
///
/// Implemented for `&mut [u8]`, and `heapless::Vec<u8, N>` with the `heapless`
/// feature, and with `alloc` for `alloc::vec::Vec<u8>` and `bytes::BytesMut`.
/// Growable adapters may allocate and retain their ordinary
/// allocation-failure behavior.
///
/// ```
/// use bacnet_encoding::sink::EncodeSink;
///
/// let mut storage = [0xA5; 4];
/// let mut tail = &mut storage[..];
/// tail.write(&[1, 2])?;
/// assert_eq!(tail.remaining_capacity(), Some(2));
/// assert_eq!(tail.write(&[3, 4, 5]).unwrap_err().available, 2);
/// tail.write(&[3, 4])?;
/// assert!(tail.is_empty());
/// assert_eq!(storage, [1, 2, 3, 4]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub trait EncodeSink {
    /// Remaining append space, or `None` when no fixed bound is advertised.
    ///
    /// Growable containers return `None`, not their current allocation's spare
    /// capacity. A sink may still reject a write; the encoder passes through
    /// its [`SinkFull`] without changing the error.
    fn remaining_capacity(&self) -> Option<usize>;

    /// Append all of `bytes`, or fail without changing output bytes or position.
    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>;

    /// Append every slice of `parts`, in order, as one value, or fail without
    /// changing output bytes or position.
    ///
    /// The provided method adds up the parts' lengths, refuses a total that
    /// overflows `usize` (with `needed` set to `usize::MAX`) or exceeds
    /// [`remaining_capacity`](Self::remaining_capacity), and only then calls
    /// [`write`](Self::write) once per part. That is atomic for a sink whose
    /// `write` never refuses an append its advertised capacity covers, and
    /// whose capacity, when it advertises none, is unbounded. Every adapter
    /// this crate ships is such a sink.
    ///
    /// **A sink that can refuse despite advertising enough room**, such as
    /// one with a quota it doesn't advertise or one backed by fallible
    /// storage, **must override this method**, and either stage the parts or
    /// undo the ones it wrote. Otherwise a refused later part leaves the
    /// earlier ones written.
    ///
    /// ```
    /// use bacnet_encoding::sink::EncodeSink;
    ///
    /// let mut storage = [0; 4];
    /// let mut tail = &mut storage[..];
    /// // Five octets don't fit: nothing is written.
    /// let refused = tail.write_parts(&[&[1, 2], &[3, 4, 5]]).unwrap_err();
    /// assert_eq!((refused.needed, refused.available), (5, 4));
    /// assert_eq!(tail.len(), 4);
    /// tail.write_parts(&[&[1], &[], &[2, 3]])?;
    /// assert_eq!(storage, [1, 2, 3, 0]);
    /// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
    /// ```
    fn write_parts(&mut self, parts: &[&[u8]]) -> Result<(), SinkFull> {
        let available = self.remaining_capacity();
        let mut needed = 0usize;
        for part in parts {
            needed = needed.checked_add(part.len()).ok_or(SinkFull {
                needed: usize::MAX,
                available: available.unwrap_or(usize::MAX),
            })?;
        }
        if let Some(available) = available {
            if needed > available {
                return Err(SinkFull { needed, available });
            }
        }
        for part in parts {
            self.write(part)?;
        }
        Ok(())
    }
}

/// A byte append did not fit its sink.
///
/// This error holds no allocated message. It does not describe allocation
/// failure in a growable container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkFull {
    /// Number of bytes in the refused append.
    pub needed: usize,
    /// Available append space when it was refused.
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

/// Writes at the front and advances the slice to its unused tail.
/// A failed write leaves both the slice and its backing bytes unchanged.
impl EncodeSink for &mut [u8] {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        if bytes.len() > self.len() {
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
impl<const N: usize> EncodeSink for heapless::Vec<u8, N> {
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
impl EncodeSink for alloc::vec::Vec<u8> {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl EncodeSink for bytes::BytesMut {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
