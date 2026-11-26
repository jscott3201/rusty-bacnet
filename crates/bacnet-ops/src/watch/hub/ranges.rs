//! Which ranges of subscriber process identifiers a client's hubs hold
//! (#2185, #2561).
//!
//! Each hub on a client takes a range of its own, so no two hubs alive at
//! once ever share an identifier, and neither's cancel can end the other's
//! subscription. A hub takes the next free range after the one taken last,
//! going round, and gives it back when it is gone: its last clone dropped
//! and every one of its tasks ended, cancels included, so a range is never
//! handed out while a cancel of the hub that had it may still go. Going
//! round, a range just given back is taken again only after every other
//! free one: a late cancel of the old hub that the network delivers after
//! a new hub's subscription of the same point (closed, then watched again)
//! names another process identifier. A client's pooled hubs (#2603) take and
//! give back theirs as any other hub does. With every range held, no further hub is made
//! ([`WatchError::Hubs`](crate::watch::WatchError::Hubs)).

use super::PROCESS_RANGES;

/// Words of 64 bits that hold one bit a range.
const WORDS: usize = (PROCESS_RANGES as usize).div_ceil(64);

/// The ranges held, one bit each, and where the next search starts.
#[derive(Debug, Default)]
pub(crate) struct Ranges {
    held: [u64; WORDS],
    /// The range after the one taken last.
    next: u32,
}

impl Ranges {
    fn is_held(&self, range: u32) -> bool {
        self.held[range as usize / 64] & (1 << (range % 64)) != 0
    }

    /// The first range no hub holds from the one after the range taken
    /// last, going round, now held; `None` when all are.
    pub(crate) fn take(&mut self) -> Option<u32> {
        let range = (0..PROCESS_RANGES)
            .map(|step| (self.next + step) % PROCESS_RANGES)
            .find(|range| !self.is_held(*range))?;
        self.held[range as usize / 64] |= 1 << (range % 64);
        self.next = (range + 1) % PROCESS_RANGES;
        Some(range)
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
    fn ranges_are_taken_going_round_and_a_released_one_comes_back_last() {
        let mut ranges = Ranges::default();
        let taken: Vec<u32> = (0..PROCESS_RANGES)
            .map(|_| ranges.take().unwrap())
            .collect();
        assert_eq!(taken, (0..PROCESS_RANGES).collect::<Vec<_>>());
        assert_eq!(ranges.take(), None, "every range is held");
        ranges.release(70);
        ranges.release(3);
        assert_eq!(ranges.held(), PROCESS_RANGES - 2);
        // Round from the start: 3, then 70.
        assert_eq!(ranges.take(), Some(3));
        ranges.release(3);
        // 3 was given back just now: 70 comes first.
        assert_eq!(ranges.take(), Some(70));
        assert_eq!(ranges.take(), Some(3));
        assert_eq!(ranges.take(), None);
    }
}
