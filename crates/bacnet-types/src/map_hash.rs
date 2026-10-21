//! A cheap hasher for small maps a program fills itself (#1713).
//!
//! SipHash, the standard map's hasher, guards a map an attacker can fill
//! with colliding keys. Some of this stack's per-request maps can't be
//! filled that way: the client's transaction table holds only the peers the
//! client sent requests to, since an answer from the network only looks a
//! key up, and the server's admission table holds at most one entry per
//! request it is running, which its in-flight limit caps. Such a map can
//! hash with the multiply-and-rotate scheme rustc's own maps use, at a
//! fraction of SipHash's cost. Don't use it for a map remote input can grow.

use core::hash::{BuildHasherDefault, Hasher};

/// The odd multiplier rustc-hash 2 uses.
const K: u64 = 0xf135_7aea_2e62_a9c5;

/// Multiply-and-rotate hasher over 64-bit words. See the module
/// documentation for the maps it suits.
#[derive(Debug, Default, Clone, Copy)]
pub struct MapHasher(u64);

impl MapHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = self.0.wrapping_add(word).wrapping_mul(K);
    }
}

impl Hasher for MapHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            // The length keeps a short tail distinct from zero padding.
            self.add(u64::from_le_bytes(word) ^ ((rest.len() as u64) << 56));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // The map takes its control bits from the top of the hash, where a
        // multiply mixes least; the rotation brings mixed bits up there.
        self.0.rotate_left(26)
    }
}

/// Builds a [`MapHasher`]: the `S` of a `HashMap<K, V, S>`.
pub type BuildMapHasher = BuildHasherDefault<MapHasher>;

// The keys are `MacAddr`, which needs `alloc`.
#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::MacAddr;
    use core::hash::BuildHasher;
    use std::collections::HashMap;

    fn hash(key: &(MacAddr, u8)) -> u64 {
        BuildMapHasher::default().hash_one(key)
    }

    #[test]
    fn distinct_peers_and_invoke_ids_hash_apart() {
        let macs: [&[u8]; 6] = [
            &[1],
            &[1, 0],
            &[0, 1],
            &[192, 168, 1, 10, 0xba, 0xc0],
            &[192, 168, 1, 11, 0xba, 0xc0],
            &[0xff, 0x52, 0, 5, 1, 3, 9],
        ];
        let mut seen = std::collections::HashSet::new();
        for mac in macs {
            for invoke in [0u8, 1, 255] {
                assert!(seen.insert(hash(&(MacAddr::from_slice(mac), invoke))));
            }
        }
    }

    #[test]
    fn a_map_finds_what_it_holds() {
        let mut map: HashMap<_, _, BuildMapHasher> = HashMap::default();
        for station in 0..=255u8 {
            for invoke in 0..4u8 {
                map.insert(
                    (MacAddr::from_slice(&[station]), invoke),
                    u16::from(station),
                );
            }
        }
        assert_eq!(map.len(), 1024);
        assert_eq!(map.get(&(MacAddr::from_slice(&[77]), 3)), Some(&77));
        assert!(!map.contains_key(&(MacAddr::from_slice(&[77, 0]), 3)));
    }
}
