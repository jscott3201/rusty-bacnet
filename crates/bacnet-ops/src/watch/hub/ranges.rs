//! Which ranges of subscriber process identifiers a client's hubs hold
//! (#2185, #2561).
//!
//! Each hub on a client takes a range of its own, so no two hubs alive at
//! once ever share an identifier, and neither's cancel can end the other's
//! subscription. A hub takes the lowest free range when it is made and
//! gives it back when it is gone: its last clone dropped and every one of
//! its tasks ended, cancels included, so a range is never handed out while
//! a cancel of the hub that had it may still go. The client's own hub keeps
//! its range for the client's life. With every range held, no further hub
//! is made ([`WatchError::Hubs`](crate::watch::WatchError::Hubs)).

use super::PROCESS_RANGES;

/// Words of 64 bits that hold one bit a range.
const WORDS: usize = (PROCESS_RANGES as usize).div_ceil(64);

/// The ranges held, one bit each.
#[derive(Debug, Default)]
pub(crate) struct Ranges {
    held: [u64; WORDS],
}

impl Ranges {
    /// The lowest range no hub holds, now held; `None` when all are.
    pub(crate) fn take(&mut self) -> Option<u32> {
        for (at, word) in self.held.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = word.trailing_ones();
                let range = at as u32 * 64 + bit;
                if range >= PROCESS_RANGES {
                    return None;
                }
                *word |= 1 << bit;
                return Some(range);
            }
        }
        None
    }

    /// Give `range` back.
    pub(crate) fn release(&mut self, range: u32) {
        if let Some(word) = self.held.get_mut(range as usize / 64) {
            *word &= !(1 << (range % 64));
        }
    }

    /// How many ranges are held.
    #[cfg(test)]
    pub(crate) fn held(&self) -> u32 {
        self.held.iter().map(|word| word.count_ones()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lowest_free_range_is_taken_and_a_released_one_comes_back() {
        let mut ranges = Ranges::default();
        let taken: Vec<u32> = (0..PROCESS_RANGES)
            .map(|_| ranges.take().unwrap())
            .collect();
        assert_eq!(taken, (0..PROCESS_RANGES).collect::<Vec<_>>());
        assert_eq!(ranges.take(), None, "every range is held");
        ranges.release(70);
        ranges.release(3);
        assert_eq!(ranges.held(), PROCESS_RANGES - 2);
        assert_eq!(ranges.take(), Some(3));
        assert_eq!(ranges.take(), Some(70));
        assert_eq!(ranges.take(), None);
    }
}
