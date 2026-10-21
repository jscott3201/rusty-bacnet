//! [`HostClock`]: a host's `std::time::Instant` on the cores' timeline.

use std::time::Instant as StdInstant;

use crate::{Duration, Instant};

/// Where a [`HostClock::centred`] origin sits on the timeline: 2^63 ns.
const MIDPOINT: Instant = Instant::from_nanos(1 << 63);

/// Maps a host's monotonic clock, as `std::time::Instant`, onto the
/// nanosecond timeline the cores take, and back. Both ways are exact for
/// every instant the timeline holds, so a timer set at the host instant of
/// a core's deadline never wakes before that deadline is due.
///
/// The clock fixes one host instant, its origin, to one point of the
/// timeline:
///
/// - [`HostClock::new`] puts the origin at [`Instant::ZERO`]: the usual
///   choice, with the origin taken as the host starts its cores. A host
///   instant before the origin maps to [`Instant::ZERO`].
/// - [`HostClock::centred`] puts it at 2^63 ns, the middle of the timeline,
///   so instants up to about 292 years either side of it map exactly. A
///   host that also hands the cores times from before it started (a
///   restored or backdated record, say) takes this one.
///
/// [`now`](Self::now) reads the host clock through a function the host
/// chooses, `std::time::Instant::now` unless [`reading`](Self::reading)
/// names another. An async runtime whose time can be paused in tests (Tokio's
/// `tokio::time::Instant::now().into_std()`, say) passes its own, so the
/// cores' deadlines follow the runtime's time.
///
/// ```
/// use bacnet_time::{Duration, HostClock, Instant};
///
/// let origin = std::time::Instant::now();
/// let clock = HostClock::new(origin);
/// let later = origin + std::time::Duration::from_millis(250);
/// assert_eq!(clock.instant(later), Instant::from_millis(250));
/// assert_eq!(clock.to_std(Instant::from_millis(250)), Some(later));
///
/// // Centred: an instant before the origin keeps its distance from it.
/// let clock = HostClock::centred(later);
/// assert_eq!(clock.instant(later) - clock.instant(origin), Duration::from_millis(250));
/// assert_eq!(clock.to_std(clock.instant(origin)), Some(origin));
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
#[derive(Debug, Clone, Copy)]
pub struct HostClock {
    /// The host instant at `zero`.
    origin: StdInstant,
    /// The point of the timeline `origin` maps to.
    zero: Instant,
    /// What [`now`](Self::now) reads.
    source: fn() -> StdInstant,
}

impl HostClock {
    /// A clock that maps `origin` to [`Instant::ZERO`] and reads
    /// `std::time::Instant::now`. Host instants before `origin` map to
    /// [`Instant::ZERO`], and those more than about 584 years after it to
    /// [`Instant::MAX`].
    #[must_use]
    pub fn new(origin: StdInstant) -> Self {
        Self {
            origin,
            zero: Instant::ZERO,
            source: StdInstant::now,
        }
    }

    /// A clock that maps `origin` to the middle of the timeline, 2^63 ns,
    /// and reads `std::time::Instant::now`: host instants up to about 292
    /// years either side of `origin` map exactly, and those further away
    /// saturate at [`Instant::ZERO`] or [`Instant::MAX`].
    #[must_use]
    pub fn centred(origin: StdInstant) -> Self {
        Self {
            origin,
            zero: MIDPOINT,
            source: StdInstant::now,
        }
    }

    /// This clock, with [`now`](Self::now) reading `now` instead. `now`
    /// must give instants on the same monotonic clock as the origin.
    #[must_use]
    pub fn reading(self, now: fn() -> StdInstant) -> Self {
        Self {
            source: now,
            ..self
        }
    }

    /// The host instant this clock is built on.
    #[must_use]
    pub fn origin(&self) -> StdInstant {
        self.origin
    }

    /// The timeline's time now, from the clock's source.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.instant((self.source)())
    }

    /// `at` on the timeline, saturating as the constructor says.
    ///
    /// The difference is taken from the later of `at` and the origin. On
    /// Windows, `std` reports an earlier instant less than one counter tick
    /// (100 ns) away as no time at all, which would put an instant just
    /// before the origin on the origin itself.
    #[must_use]
    pub fn instant(&self, at: StdInstant) -> Instant {
        if at >= self.origin {
            self.zero + Duration::saturating_from_core(at.duration_since(self.origin))
        } else {
            self.zero - Duration::saturating_from_core(self.origin.duration_since(at))
        }
    }

    /// The host instant that `at` on the timeline stands for, or `None` when
    /// the host's clock can't represent it. A host sleeping until a core's
    /// deadline takes `None` as no deadline: that far out, it never comes.
    #[must_use]
    pub fn to_std(&self, at: Instant) -> Option<StdInstant> {
        if at >= self.zero {
            let after = at.saturating_duration_since(self.zero);
            self.origin
                .checked_add(core::time::Duration::from_nanos(after.as_nanos()))
        } else {
            let before = self.zero.saturating_duration_since(at);
            self.origin
                .checked_sub(core::time::Duration::from_nanos(before.as_nanos()))
        }
    }
}
