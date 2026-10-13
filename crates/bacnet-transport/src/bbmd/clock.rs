//! Maps the host's monotonic clock onto `bacnet-bip-core`'s timeline.

use std::time::{Duration, Instant};

use bacnet_bip_core as bip_core;

/// Maps `std::time::Instant` to and from the core's `Instant`, nanoseconds
/// on a `u64`. `epoch` maps to the middle of that range, 2^63 ns, so an
/// instant up to about 292 years either side of it maps exactly, both ways:
/// a registration backdated before the BBMD started keeps its time.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CoreClock {
    epoch: Instant,
}

/// Where `epoch` sits on the core's timeline.
const MIDPOINT: u64 = 1 << 63;

impl CoreClock {
    /// A clock whose midpoint is `epoch`.
    pub(crate) fn new(epoch: Instant) -> Self {
        Self { epoch }
    }

    /// `at` on the core's timeline.
    ///
    /// The difference is always taken from the later instant. On Windows
    /// `std` reports an earlier instant less than one counter tick (100 ns)
    /// away as no time at all, which would map an instant just before the
    /// epoch onto the epoch itself.
    pub(crate) fn to_core(self, at: Instant) -> bip_core::Instant {
        // Either side saturates some 292 years from the epoch.
        let midpoint = bip_core::Instant::from_nanos(MIDPOINT);
        if at >= self.epoch {
            midpoint + bip_core::Duration::saturating_from_core(at.duration_since(self.epoch))
        } else {
            midpoint - bip_core::Duration::saturating_from_core(self.epoch.duration_since(at))
        }
    }

    /// The host instant that `at` on the core's timeline stands for, or
    /// `epoch` for one the host clock can't represent.
    pub(crate) fn to_std(self, at: bip_core::Instant) -> Instant {
        let nanos = at.as_nanos();
        let mapped = if nanos >= MIDPOINT {
            self.epoch
                .checked_add(Duration::from_nanos(nanos - MIDPOINT))
        } else {
            self.epoch
                .checked_sub(Duration::from_nanos(MIDPOINT - nanos))
        };
        mapped.unwrap_or(self.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The expected values come from the instants the host clock produced,
    /// so the test holds at any clock resolution: Windows' is 100 ns.
    #[test]
    fn instants_either_side_of_the_epoch_map_exactly_both_ways() {
        let epoch = Instant::now();
        let clock = CoreClock::new(epoch);
        assert_eq!(clock.to_core(epoch).as_nanos(), MIDPOINT);
        for offset in [
            Duration::from_nanos(1),
            Duration::from_nanos(100),
            Duration::from_nanos(123_456_789),
            Duration::from_secs(91),
        ] {
            let later = epoch + offset;
            assert_eq!(
                clock.to_core(later).as_nanos(),
                MIDPOINT + u64::try_from(later.duration_since(epoch).as_nanos()).unwrap()
            );
            assert_eq!(clock.to_std(clock.to_core(later)), later);
            if let Some(earlier) = epoch.checked_sub(offset) {
                assert_eq!(
                    clock.to_core(earlier).as_nanos(),
                    MIDPOINT - u64::try_from(epoch.duration_since(earlier).as_nanos()).unwrap()
                );
                assert_eq!(clock.to_std(clock.to_core(earlier)), earlier);
                assert!(clock.to_core(earlier) <= clock.to_core(epoch));
            }
        }
    }
}
