//! Fixed-width command-priority filter used by Audit Reporter objects.

#[cfg(not(feature = "std"))]
use alloc::{vec, vec::Vec};

use crate::error::{Error, Result};

/// `BACnetPriorityFilter` — one bit for each command priority from 1 through 16.
///
/// The bit-position-preserving `u16` representation uses bit 0 for priority 1
/// and bit 15 for priority 16. BACnet encoding places those positions
/// most-significant-bit first in exactly two content octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BACnetPriorityFilter {
    bits: u16,
}

impl BACnetPriorityFilter {
    /// A filter that selects no command priorities.
    #[inline]
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// A filter that selects every command priority.
    #[inline]
    pub const fn all() -> Self {
        Self { bits: u16::MAX }
    }

    /// Construct from the bit-position-preserving representation.
    #[inline]
    pub const fn from_bits(bits: u16) -> Self {
        Self { bits }
    }

    /// Return the bit-position-preserving representation.
    #[inline]
    pub const fn bits(self) -> u16 {
        self.bits
    }

    /// Whether command `priority` is selected.
    ///
    /// Priorities outside 1 through 16 are never selected.
    #[inline]
    pub const fn contains(self, priority: u8) -> bool {
        match Self::priority_mask(priority) {
            Some(mask) => self.bits & mask != 0,
            None => false,
        }
    }

    /// Select or clear command `priority`.
    ///
    /// An out-of-range priority is rejected before the filter is mutated.
    pub fn set(&mut self, priority: u8, selected: bool) -> Result<()> {
        let Some(mask) = Self::priority_mask(priority) else {
            return Err(Error::OutOfRange(
                "BACnetPriorityFilter: priority must be in 1..=16".into(),
            ));
        };
        if selected {
            self.bits |= mask;
        } else {
            self.bits &= !mask;
        }
        Ok(())
    }

    const fn priority_mask(priority: u8) -> Option<u16> {
        if priority >= 1 && priority <= 16 {
            Some(1u16 << (priority - 1))
        } else {
            None
        }
    }

    /// Decode one fixed-width BACnet BIT STRING payload.
    ///
    /// The payload must contain exactly 16 meaningful bits: two content
    /// octets, zero unused bits, and no alternate-width representation.
    pub fn from_bacnet(unused_bits: u8, data: &[u8]) -> Result<Self> {
        if unused_bits != 0 {
            return Err(Error::decoding(
                0,
                "BACnetPriorityFilter: a 16-bit value must have zero unused bits",
            ));
        }
        if data.len() != 2 {
            return Err(Error::decoding(
                0,
                "BACnetPriorityFilter: payload must contain exactly two octets",
            ));
        }

        Ok(Self {
            bits: u16::from(data[0].reverse_bits()) | (u16::from(data[1].reverse_bits()) << 8),
        })
    }

    /// Encode as the canonical two-octet BACnet BIT STRING payload.
    pub fn to_bacnet(self) -> (u8, Vec<u8>) {
        (
            0,
            vec![
                (self.bits as u8).reverse_bits(),
                ((self.bits >> 8) as u8).reverse_bits(),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_priority_filter_construction_and_raw_round_trip() {
        assert_eq!(BACnetPriorityFilter::empty().bits(), 0);
        assert_eq!(BACnetPriorityFilter::all().bits(), u16::MAX);

        let raw = 0x8181;
        assert_eq!(BACnetPriorityFilter::from_bits(raw).bits(), raw);
    }

    #[test]
    fn audit_priority_filter_uses_msb_first_priority_positions() {
        for (priority, expected) in [
            (1, vec![0x80, 0x00]),
            (8, vec![0x01, 0x00]),
            (9, vec![0x00, 0x80]),
            (16, vec![0x00, 0x01]),
        ] {
            let mut filter = BACnetPriorityFilter::empty();
            filter.set(priority, true).unwrap();
            assert!(filter.contains(priority));
            assert_eq!(filter.to_bacnet(), (0, expected.clone()));
            assert_eq!(
                BACnetPriorityFilter::from_bacnet(0, &expected).unwrap(),
                filter
            );
        }

        assert_eq!(BACnetPriorityFilter::empty().to_bacnet(), (0, vec![0, 0]));
        assert_eq!(
            BACnetPriorityFilter::all().to_bacnet(),
            (0, vec![0xff, 0xff])
        );
    }

    #[test]
    fn audit_priority_filter_mutation_is_bounded_and_transactional() {
        let mut filter = BACnetPriorityFilter::from_bits(0x8001);
        filter.set(1, false).unwrap();
        filter.set(8, true).unwrap();
        assert!(!filter.contains(1));
        assert!(filter.contains(8));
        assert!(filter.contains(16));

        let before = filter;
        assert!(filter.set(0, true).is_err());
        assert_eq!(filter, before);
        assert!(filter.set(17, false).is_err());
        assert_eq!(filter, before);
        assert!(!filter.contains(0));
        assert!(!filter.contains(17));
    }

    #[test]
    fn audit_priority_filter_rejects_noncanonical_width_and_padding() {
        for (unused_bits, data) in [
            (1, &[0x80, 0x00][..]),
            (7, &[0x80, 0x00][..]),
            (8, &[0x80, 0x00][..]),
            (0, &[][..]),
            (0, &[0x80][..]),
            (0, &[0x80, 0x00, 0x00][..]),
        ] {
            assert!(BACnetPriorityFilter::from_bacnet(unused_bits, data).is_err());
        }
    }
}
