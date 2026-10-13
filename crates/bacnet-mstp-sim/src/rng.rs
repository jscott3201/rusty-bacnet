//! A tiny seeded generator, so a scenario replays exactly from its seed.

/// A xorshift64* generator seeded through splitmix64.
///
/// Statistical quality is beside the point: scenarios need cheap,
/// reproducible choices (noise octets, fault times, station sets), and the
/// same seed must give the same run on every host.
#[derive(Debug, Clone)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// A generator for `seed`. Every seed, zero included, gives a usable
    /// non-zero state.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Self {
            state: if z == 0 { 0x2545_F491_4F6C_DD1D } else { z },
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound`, or 0 when `bound` is 0. The modulo bias is
    /// irrelevant at the small bounds scenarios use.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// A value in `low..=high`.
    pub fn range(&mut self, low: u64, high: u64) -> u64 {
        assert!(low <= high, "empty range {low}..={high}");
        low + self.below(high - low + 1)
    }

    /// True with probability `percent` / 100.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// One random octet.
    pub fn byte(&mut self) -> u8 {
        self.next_u64().to_le_bytes()[0]
    }

    /// `count` random octets.
    pub fn bytes(&mut self, count: usize) -> Vec<u8> {
        (0..count).map(|_| self.byte()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::SimRng;

    #[test]
    fn same_seed_same_sequence_and_seeds_differ() {
        let first: Vec<u64> = {
            let mut rng = SimRng::new(7);
            (0..8).map(|_| rng.next_u64()).collect()
        };
        let again: Vec<u64> = {
            let mut rng = SimRng::new(7);
            (0..8).map(|_| rng.next_u64()).collect()
        };
        let other: Vec<u64> = {
            let mut rng = SimRng::new(8);
            (0..8).map(|_| rng.next_u64()).collect()
        };
        assert_eq!(first, again);
        assert_ne!(first, other);
        let mut zero = SimRng::new(0);
        assert_ne!(zero.next_u64(), 0);
    }

    #[test]
    fn range_stays_inside_its_bounds() {
        let mut rng = SimRng::new(1);
        for _ in 0..1_000 {
            let value = rng.range(3, 9);
            assert!((3..=9).contains(&value));
        }
        assert_eq!(rng.below(0), 0);
    }
}
