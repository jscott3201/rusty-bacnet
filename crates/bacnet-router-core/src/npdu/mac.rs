//! [`Mac`]: a link-layer address stored inline.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::MAX_MAC_LEN;

/// A link-layer address of up to [`MAX_MAC_LEN`] octets, stored inline, so
/// a route or a pending reply holds one without an allocator.
///
/// It is empty for a directly attached network's route, which has no next
/// router. Two `Mac`s are equal when their octets are.
#[derive(Clone, Copy)]
pub struct Mac {
    len: u8,
    octets: [u8; MAX_MAC_LEN],
}

/// A MAC longer than [`MAX_MAC_LEN`] octets was offered to [`Mac`].
///
/// It is `#[non_exhaustive]`: only the crate builds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MacTooLong {
    /// The length offered.
    pub length: usize,
}

impl fmt::Display for MacTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a MAC of {} octets is longer than {MAX_MAC_LEN}",
            self.length
        )
    }
}

impl core::error::Error for MacTooLong {}

impl Mac {
    /// The empty MAC.
    pub const EMPTY: Self = Self {
        len: 0,
        octets: [0; MAX_MAC_LEN],
    };

    /// A copy of `octets`, or [`MacTooLong`] when there are more than
    /// [`MAX_MAC_LEN`] of them.
    pub const fn from_slice(octets: &[u8]) -> Result<Self, MacTooLong> {
        if octets.len() > MAX_MAC_LEN {
            return Err(MacTooLong {
                length: octets.len(),
            });
        }
        let mut mac = Self::EMPTY;
        let mut at = 0;
        while at < octets.len() {
            mac.octets[at] = octets[at];
            at += 1;
        }
        mac.len = octets.len() as u8;
        Ok(mac)
    }

    /// A MAC of `L` octets, `L` checked against [`MAX_MAC_LEN`] when the
    /// program compiles.
    #[must_use]
    pub const fn from_array<const L: usize>(octets: [u8; L]) -> Self {
        const { assert!(L <= MAX_MAC_LEN, "a Mac holds at most MAX_MAC_LEN octets") };
        match Self::from_slice(&octets) {
            Ok(mac) => mac,
            Err(_) => Self::EMPTY,
        }
    }

    /// The octets.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.octets[..usize::from(self.len)]
    }

    /// How many octets there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for Mac {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl PartialEq for Mac {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Mac {}

impl PartialEq<[u8]> for Mac {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl Hash for Mac {
    fn hash<S: Hasher>(&self, state: &mut S) {
        self.as_slice().hash(state);
    }
}

impl fmt::Debug for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mac({:02x?})", self.as_slice())
    }
}

impl AsRef<[u8]> for Mac {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl TryFrom<&[u8]> for Mac {
    type Error = MacTooLong;

    fn try_from(octets: &[u8]) -> Result<Self, MacTooLong> {
        Self::from_slice(octets)
    }
}
