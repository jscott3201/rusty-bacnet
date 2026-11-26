//! What an application sets of its watches ([`WatchSettings`]: the COV
//! lifetime, the poll interval, subscribe or poll only), the [`Lifetime`]
//! they take, and the pool of hubs a client's watches run on, one a set
//! of settings (#2603).
//!
//! A [`Client`](crate::Client) keeps default settings, which its connect
//! builder sets ([`BipConnect::watch_settings`](crate::client::BipConnect::watch_settings)),
//! and [`Client::watch`](crate::Client::watch) uses them;
//! [`Client::watch_with`](crate::Client::watch_with) takes others for one
//! watch. Watches with equal settings share one hub, so a point two of
//! them hold is one subscription; watches with other settings run on
//! another hub, in a range of subscriber process identifiers of its own,
//! so no hub ever mixes two lifetimes. A pooled hub lives while a watch on
//! it lives or one of its tasks runs (a cancel included), then gives its
//! range back to the client.

use std::num::NonZeroU32;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_types::error::Refusal;

use super::WatchError;
use super::hub::{
    Backoff, Hub, Inner, MIN_POLL, MissingDevice, POLL_FLOOR, Reads, WatchOptions, renewal_margin,
};
use super::stream::MAX_WATCHED;

/// A COV subscription's lifetime in seconds: never zero, so never the
/// subscription that doesn't lapse, and at most [`MAX`](Self::MAX), eight
/// hours, the longest every device is obliged to take (Clauses 13.14.1.1.4
/// and 13.15.1.1.4). A device that takes no longer may refuse a longer
/// one, and its point would then be polled for good
/// ([`Why::Refused`](super::Why::Refused)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lifetime(pub(in crate::watch) NonZeroU32);

impl Lifetime {
    /// Five minutes, a watch's default.
    pub const DEFAULT: Self = Self(NonZeroU32::new(300).expect("300 is not zero"));

    /// Eight hours (28,800 s), the longest lifetime taken.
    pub const MAX: Self = Self(NonZeroU32::new(28_800).expect("28,800 is not zero"));

    /// A lifetime of `secs`, 1 to 28,800.
    ///
    /// # Errors
    ///
    /// [`WatchError::Settings`] for 0, which would ask for a subscription
    /// that never lapses, and for more than [`MAX`](Self::MAX).
    pub fn new(secs: u32) -> Result<Self, WatchError> {
        NonZeroU32::new(secs)
            .filter(|secs| *secs <= Self::MAX.0)
            .map(Self)
            .ok_or_else(|| {
                WatchError::Settings(
                    Refusal::new("a COV lifetime is 1 to {} seconds, not {}")
                        .with_values(u64::from(Self::MAX.secs()), u64::from(secs)),
                )
            })
    }

    /// The lifetime in seconds, 1 to 28,800.
    pub fn secs(self) -> u32 {
        self.0.get()
    }

    pub(in crate::watch) fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.secs()))
    }
}

/// What a watch sets: each subscription's lifetime, the least time between
/// two polls of a device, and whether points are subscribed at all.
///
/// The default is a client's own unless its builder set others
/// ([`BipConnect::watch_settings`](crate::client::BipConnect::watch_settings)):
/// five-minute subscriptions, polls at most every 10 s, COV where the
/// device takes it. Equal settings share one hub
/// ([`Client::watch_with`](crate::Client::watch_with));
/// [`validate`](Self::validate) says whether a hub takes them.
///
/// ```
/// use std::time::Duration;
/// use bacnet_ops::watch::WatchSettings;
/// use bacnet_ops::watch::hub::Lifetime;
///
/// # fn main() -> Result<(), bacnet_ops::watch::WatchError> {
/// let settings = WatchSettings::default()
///     .with_lifetime(Lifetime::new(3600)?)
///     .with_poll_every(Duration::from_secs(30));
/// settings.validate()?;
/// let polled = WatchSettings::default().with_subscribe(false);
/// assert!(!polled.subscribe);
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct WatchSettings {
    /// Each COV subscription's lifetime, renewed before it ends.
    pub lifetime: Lifetime,
    /// The least time between two polls of a device, all its polled
    /// points read together: 1 s to an hour.
    pub poll_every: Duration,
    /// Subscribe each point for COV where its device takes it; `false`
    /// polls every point and sends no subscription
    /// ([`Why::PollOnly`](super::Why::PollOnly)).
    pub subscribe: bool,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            lifetime: Lifetime::DEFAULT,
            poll_every: MIN_POLL,
            subscribe: true,
        }
    }
}

impl WatchSettings {
    /// These settings with subscriptions of `lifetime`.
    #[must_use]
    pub fn with_lifetime(mut self, lifetime: Lifetime) -> Self {
        self.lifetime = lifetime;
        self
    }

    /// These settings with polls at most every `every`.
    #[must_use]
    pub fn with_poll_every(mut self, every: Duration) -> Self {
        self.poll_every = every;
        self
    }

    /// These settings subscribing points (`true`) or polling every one.
    #[must_use]
    pub fn with_subscribe(mut self, subscribe: bool) -> Self {
        self.subscribe = subscribe;
        self
    }

    /// Whether a hub takes these settings: what
    /// [`WatchOptions::validate`] says of the hub's settings they make.
    ///
    /// # Errors
    ///
    /// [`WatchError::Settings`]: a poll interval below 1 s or above an
    /// hour.
    pub fn validate(&self) -> Result<(), WatchError> {
        // The request budget moves only the renewal margin, which stays
        // inside the lifetime whatever it is.
        self.options(Duration::ZERO).validate()
    }

    /// The hub's settings these make for a client whose requests take up
    /// to `request_budget` with their retries: subscriptions of the
    /// lifetime, renewed early enough to cover that budget twice
    /// ([`renewal_margin`]); a device's polled points read together every
    /// `poll_every`, backing off to a minute (or `poll_every`, if longer)
    /// when it stops answering; a point whose subscription went unanswered,
    /// met a full table, or whose renewal failed polled for 5 minutes before
    /// it is subscribed again; each point read for its value with its
    /// Status_Flags, Units, name and state texts; every change signalled;
    /// at most [`MAX_WATCHED`] points.
    pub(crate) fn options(&self, request_budget: Duration) -> WatchOptions {
        let poll_every = self.poll_every;
        WatchOptions {
            lifetime: self.lifetime,
            renewal_margin: renewal_margin(self.lifetime.secs(), request_budget),
            poll_every,
            poll_floor: POLL_FLOOR,
            poll_backoff: Backoff {
                first: poll_every,
                longest: poll_every.max(Duration::from_secs(60)),
            },
            retry_cov: Backoff::fixed(Duration::from_secs(300)),
            missing: MissingDevice::Poll,
            reads: Reads::value(),
            coalesce: Duration::ZERO,
            max_points: MAX_WATCHED,
            subscribe: self.subscribe,
            ..WatchOptions::default()
        }
    }
}

/// The hubs a client's watches run on, one for each set of settings in
/// use: weak, since each hub holds the client. Pruned
/// as it is used. Only the client's pool lock guards it; that lock is
/// taken before the client's ranges, never the other way, and held across
/// no await.
pub(crate) struct Pool<T: TransportPort + 'static> {
    hubs: Vec<Pooled<T>>,
}

struct Pooled<T: TransportPort + 'static> {
    settings: WatchSettings,
    hub: Weak<Inner<T>>,
}

impl<T: TransportPort + 'static> Pool<T> {
    pub(crate) fn new() -> Self {
        Self { hubs: Vec::new() }
    }

    /// The live hub of `settings`, or the one `make` makes,
    /// kept for the next watch of the same.
    pub(crate) fn hub(
        &mut self,
        settings: &WatchSettings,
        make: impl FnOnce() -> Result<Hub<T>, WatchError>,
    ) -> Result<Hub<T>, WatchError> {
        self.hubs.retain(|pooled| pooled.hub.strong_count() > 0);
        let live = self
            .hubs
            .iter()
            .filter(|pooled| pooled.settings == *settings)
            .find_map(|pooled| pooled.hub.upgrade());
        if let Some(inner) = live {
            return Ok(Hub { inner });
        }
        let hub = make()?;
        self.hubs.push(Pooled {
            settings: settings.clone(),
            hub: Arc::downgrade(&hub.inner),
        });
        Ok(hub)
    }

    /// Every live hub, taken out of the pool: the next watch makes a new
    /// one.
    pub(crate) fn drain(&mut self) -> Vec<Hub<T>> {
        self.hubs
            .drain(..)
            .filter_map(|pooled| pooled.hub.upgrade())
            .map(|inner| Hub { inner })
            .collect()
    }

    /// How many hubs are alive.
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.hubs
            .iter()
            .filter(|pooled| pooled.hub.strong_count() > 0)
            .count()
    }
}
