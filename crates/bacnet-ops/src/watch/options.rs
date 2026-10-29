//! The hub's settings: lifetimes, polling, backoffs, what a point reads,
//! and what a front end gets per change.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::PropertyIdentifier;

use super::{Lifetime, POINT_PROPERTIES};
use crate::limits::TokenBucket;

/// The default shortest time between two polls of a device, the MCP
/// server's floor.
pub const MIN_POLL: Duration = Duration::from_secs(10);

/// The lowest [`WatchOptions::poll_floor`] a hub takes.
pub const POLL_FLOOR: Duration = Duration::from_secs(1);

/// Rows a point's snapshot holds at most: what it reads, the first read's
/// extras, and room for a device's spares.
pub const MAX_ROWS: usize = 32;

/// Samples a lease's stream holds at most.
pub const MAX_SAMPLES: usize = 1024;

/// A wait that doubles after each failure, from `first` up to `longest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// The first wait.
    pub first: Duration,
    /// The longest wait.
    pub longest: Duration,
}

impl Backoff {
    /// A wait that never grows.
    pub const fn fixed(wait: Duration) -> Self {
        Self {
            first: wait,
            longest: wait,
        }
    }

    /// The wait after `previous` (zero for the first failure).
    pub fn next(self, previous: Duration) -> Duration {
        if previous.is_zero() {
            self.first
        } else {
            (previous * 2).clamp(self.first, self.longest)
        }
    }

    fn validate(self, what: &str) -> Result<(), String> {
        if self.first.is_zero() || self.longest < self.first {
            return Err(format!(
                "{what}: the first wait must be above 0 and at most the longest"
            ));
        }
        Ok(())
    }
}

/// What a watch does when its device isn't in the discovery table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingDevice {
    /// Give the point up: [`State::Gone`](super::State::Gone).
    Fail,
    /// Report it unreachable and look again after the poll backoff.
    Poll,
}

/// Which properties a poll reads one by one once the device refused RPM
/// or failed a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// Every property the point reads.
    All,
    /// The watched one only.
    Watched,
}

/// What a point reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reads {
    /// The properties an object watch reads and polls; the first is the
    /// one its state texts name.
    pub object: Vec<PropertyIdentifier>,
    /// A property watch also reads Status_Flags.
    pub property_flags: bool,
    /// Properties only the first read adds, such as Units.
    pub first: Vec<PropertyIdentifier>,
    /// The first read asks for Object_Name too, and again without it when
    /// the name may have failed the read.
    pub name: bool,
    /// A binary or multi-state object's state texts follow the first read.
    pub state_texts: bool,
    /// What a poll reads without RPM.
    pub fallback: Fallback,
}

impl Reads {
    /// The MCP server's: an object's [`POINT_PROPERTIES`], a property
    /// alone, nothing more on the first read.
    pub fn points() -> Self {
        Self {
            object: POINT_PROPERTIES.to_vec(),
            property_flags: false,
            first: Vec::new(),
            name: false,
            state_texts: false,
            fallback: Fallback::All,
        }
    }

    /// A value with its Status_Flags, named, with its Units and state
    /// texts from the first read; polled for the value alone without RPM.
    /// An object watch follows Present_Value. The TUI's watch list reads
    /// this way.
    pub fn value() -> Self {
        Self {
            object: vec![
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
            ],
            property_flags: true,
            first: vec![PropertyIdentifier::UNITS],
            name: true,
            state_texts: true,
            fallback: Fallback::Watched,
        }
    }
}

/// The hub's settings.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Each subscription's lifetime.
    pub lifetime: Lifetime,
    /// How long before a lifetime ends the renewal goes. It covers a wait
    /// for the device's gate and the renewal's own request with retries
    /// (see [`renewal_margin`](super::renewal_margin)).
    pub renewal_margin: Duration,
    /// Least time between two polls of a device, all its polled points
    /// read together; at least [`poll_floor`](Self::poll_floor).
    pub poll_every: Duration,
    /// The shortest [`poll_every`](Self::poll_every) taken; at least
    /// [`POLL_FLOOR`].
    pub poll_floor: Duration,
    /// How long a device that stopped answering polls waits for the next.
    pub poll_backoff: Backoff,
    /// After a subscription failed for want of an answer (rather than a
    /// refusal), or a renewal failed, how long the point is polled before
    /// it is read and subscribed again; never sooner than
    /// [`poll_floor`](Self::poll_floor).
    pub retry_cov: Backoff,
    /// What a point does whose device isn't in the discovery table.
    pub missing: MissingDevice,
    /// What a point reads.
    pub reads: Reads,
    /// Changes to one point within this window make one signal; zero for
    /// a signal per change.
    pub coalesce: Duration,
    /// Samples a lease's stream ([`Lease::samples`](super::Lease::samples))
    /// holds at least before the oldest go (the channel rounds it up to a
    /// power of two).
    pub samples: usize,
    /// Points watched at once, those still cancelling included.
    pub max_points: usize,
    /// The hub's own rate, taken by each request it sends but cancels;
    /// `None` for none beyond the engine's limits.
    pub bucket: Option<Arc<TokenBucket>>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            lifetime: Lifetime::DEFAULT,
            renewal_margin: Duration::from_secs(60),
            poll_every: MIN_POLL,
            poll_floor: MIN_POLL,
            poll_backoff: Backoff {
                first: MIN_POLL,
                longest: Duration::from_secs(60),
            },
            retry_cov: Backoff::fixed(Duration::from_secs(300)),
            missing: MissingDevice::Poll,
            reads: Reads::points(),
            coalesce: Duration::from_secs(1),
            samples: 32,
            max_points: 256,
            bucket: None,
        }
    }
}

impl WatchOptions {
    /// Check the settings: polls no faster than the floor, a renewal
    /// margin inside the lifetime, backoffs that start above zero, room
    /// for one point and its rows.
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_floor < POLL_FLOOR {
            return Err(format!(
                "the poll floor is at least {} s",
                POLL_FLOOR.as_secs()
            ));
        }
        if self.poll_every < self.poll_floor {
            return Err(format!(
                "polls go at most every {} s",
                self.poll_floor.as_secs()
            ));
        }
        if self.renewal_margin.is_zero() || self.renewal_margin >= self.lifetime.duration() {
            return Err("the renewal margin must be inside the lifetime".into());
        }
        self.poll_backoff.validate("poll_backoff")?;
        self.retry_cov.validate("retry_cov")?;
        if self.retry_cov.first < self.poll_floor {
            return Err("retry_cov waits at least the poll floor".into());
        }
        if self.max_points == 0 {
            return Err("max_points must be at least 1".into());
        }
        if self.coalesce > Duration::from_secs(60) {
            return Err("coalesce must be a minute or less".into());
        }
        if !(1..=MAX_SAMPLES).contains(&self.samples) {
            return Err(format!("samples must be 1 to {MAX_SAMPLES}"));
        }
        let reads = &self.reads;
        if reads.object.is_empty() {
            return Err("an object watch must read a property".into());
        }
        // The extras: Object_Name, Status_Flags and two state texts.
        if reads.object.len() + reads.first.len() + 4 > MAX_ROWS {
            return Err(format!("a point reads at most {MAX_ROWS} rows"));
        }
        Ok(())
    }
}
