//! A [`Client`]'s watches (#2560, #2603): its default
//! [`WatchSettings`], and the hubs its watches run on, one for each set
//! of settings in use ([`Pool`](super::settings::Pool)).

use std::time::Duration;

use bacnet_transport::port::TransportPort;

use super::hub::{Hub, PROCESS_RANGES};
use super::{Point, Watch, WatchError, WatchSettings};
use crate::client::{Client, lock};
use crate::device::Device;
use crate::error::OpsError;
use crate::reading::IntoProperty;

impl<T: TransportPort + 'static> Client<T> {
    /// Watch `points`, from [`Device::point`]: a [`Watch`] streaming each
    /// point's [`Update`](super::Update)s, under this client's
    /// [`watch_settings`](Self::watch_settings). As
    /// [`watch_with`](Self::watch_with) with those.
    ///
    /// # Errors
    ///
    /// As [`watch_with`](Self::watch_with).
    pub fn watch(&self, points: impl IntoIterator<Item = Point>) -> Result<Watch, OpsError> {
        self.watch_with(points, &self.watch_settings)
    }

    /// Watch `points` under `settings`: a [`Watch`] streaming each point's
    /// [`Update`](super::Update)s.
    ///
    /// The points go to the client's hub for `settings`, made on first use
    /// and shared by every watch of equal settings while any lives: a point
    /// two such watches hold is one subscription. Each point is read once for
    /// its value, Status_Flags, Units, name and state texts, then, unless
    /// `settings` poll only, subscribed for its lifetime at a time, renewed
    /// before each lifetime ends; a point the device won't subscribe is
    /// polled with the device's other polled points, every `poll_every` at
    /// most (the [`hub`](super::hub) says the rest). Each renewal goes early
    /// enough to leave room for two requests with every retry on the
    /// low-level client's own timers ([`renewal_margin`](super::hub::renewal_margin)), at most
    /// half the lifetime. A device that
    /// refuses the COV service itself is remembered by its address, and
    /// its later points are polled without asking. A point named twice, or
    /// one station named two ways, is watched once.
    ///
    /// Each hub takes a range of subscriber process identifiers from the
    /// client's 256 and gives it back once its last watch has ended and its
    /// cancels are done. It works with the client's base settings,
    /// whichever clone calls: a following handle's device that stops
    /// answering is looked for with
    /// [`RESOLVE_WAIT`](crate::client::RESOLVE_WAIT), as by a clone
    /// [`Client::new`] made.
    ///
    /// # Errors
    ///
    /// [`OpsError::Watch`]: [`WatchError::Settings`] for settings a hub
    /// refuses ([`WatchSettings::validate`]), [`WatchError::Full`] when
    /// the points would take the hub past [`MAX_WATCHED`](super::MAX_WATCHED)
    /// (nothing is watched then, and nothing sent),
    /// [`WatchError::NoRuntime`] outside a Tokio runtime, which the hub's
    /// tasks need, [`WatchError::Hubs`] when a hub for new settings
    /// would be made while other hubs hold every range, and
    /// [`WatchError::Shut`] when the hub shuts down under the call
    /// ([`shutdown_watches`](Self::shutdown_watches)); a call after makes a new
    /// hub.
    pub fn watch_with(
        &self,
        points: impl IntoIterator<Item = Point>,
        settings: &WatchSettings,
    ) -> Result<Watch, OpsError> {
        settings.validate()?;
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(OpsError::Watch(WatchError::NoRuntime));
        }
        let hub = self.pooled_hub(settings)?;
        Ok(hub.stream(points)?)
    }

    /// The default settings of this clone's watches
    /// ([`watch`](Self::watch)): [`WatchSettings::default`] for
    /// [`Client::new`]'s, else what the connect builder or
    /// [`with_watch_settings`](Self::with_watch_settings) set.
    pub fn watch_settings(&self) -> &WatchSettings {
        &self.watch_settings
    }

    /// A clone of this client whose [`watch`](Self::watch)es take
    /// `settings`. The setting is the clone's alone, and its clones' and
    /// handles': the other clones keep theirs. Watches of other settings
    /// run on other hubs, so none mixes two lifetimes. Settings a hub
    /// refuses are refused when a watch is made.
    pub fn with_watch_settings(&self, settings: WatchSettings) -> Self {
        let mut clone = self.clone();
        clone.watch_settings = settings;
        clone
    }

    /// How long one confirmed request takes with every retry on the
    /// low-level client: its APDU timeout times one more than its retries.
    pub(crate) fn request_budget(&self) -> Duration {
        self.shared.request_budget
    }

    /// Shut down every hub the client's watches run on, as
    /// [`Hub::shutdown`] does: each watch on them ends at once (its `next`
    /// gives `None`), every point stops, and this waits up to `wait` for
    /// the cancels, all hubs at once. True when every cancel finished in
    /// time; the subscriptions of the others lapse with their lifetime. A
    /// watch made after makes a new hub.
    pub async fn shutdown_watches(&self, wait: Duration) -> bool {
        let hubs = lock(&self.shared.watch_pool).drain();
        let mut shutting = tokio::task::JoinSet::new();
        for hub in hubs {
            shutting.spawn(async move { hub.shutdown(wait).await });
        }
        let mut all = true;
        while let Some(done) = shutting.join_next().await {
            all &= done.unwrap_or(false);
        }
        all
    }

    /// The next range of subscriber process identifiers no live hub on
    /// this client holds, going round from the one taken last, now held
    /// until [`release_range`](Self::release_range); `None` when every one
    /// is held.
    pub(crate) fn take_range(&self) -> Option<u32> {
        lock(&self.shared.ranges).take()
    }

    /// Give back `range`: its hub is gone.
    pub(crate) fn release_range(&self, range: u32) {
        lock(&self.shared.ranges).release(range);
    }

    /// The live hub of `settings`, or a new one in the next free range over the client's base settings. Under
    /// the pool's lock, so two watches of new equal settings make one hub;
    /// the range is taken inside it (pool, then ranges).
    fn pooled_hub(&self, settings: &WatchSettings) -> Result<Hub<T>, WatchError> {
        let budget = self.request_budget();
        let mut pool = lock(&self.shared.watch_pool);
        pool.hub(settings, || {
            let range = self.take_range().ok_or(WatchError::Hubs {
                max: PROCESS_RANGES as usize,
            })?;
            Ok(Hub::in_range(
                self.base(),
                settings.options(budget),
                range,
                true,
            ))
        })
    }
}

impl From<WatchError> for OpsError {
    fn from(error: WatchError) -> Self {
        Self::Watch(error)
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// Watch `points` of this device, each in the grammar of
    /// [`point`](Self::point) (`"ai:1"`, `"ao:2 priority-array[8]"`) or a
    /// tuple, under its client's [`watch_settings`](Client::watch_settings):
    /// shorthand for [`Client::watch`] of each point. A following handle's
    /// points follow its device from address to address (#2795), a pinned
    /// one's stay at its address ([`point`](Self::point)).
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that doesn't name one property (nothing
    /// is watched then), and as [`Client::watch`].
    pub fn watch<P: IntoProperty>(
        &self,
        points: impl IntoIterator<Item = P>,
    ) -> Result<Watch, OpsError> {
        let settings = self.client().watch_settings().clone();
        self.watch_with(points, &settings)
    }

    /// Watch `points` of this device under `settings`: shorthand for
    /// [`Client::watch_with`] of each point.
    ///
    /// # Errors
    ///
    /// As [`watch`](Self::watch).
    pub fn watch_with<P: IntoProperty>(
        &self,
        points: impl IntoIterator<Item = P>,
        settings: &WatchSettings,
    ) -> Result<Watch, OpsError> {
        let points = points
            .into_iter()
            .map(|point| self.point(point))
            .collect::<Result<Vec<_>, _>>()?;
        self.client().watch_with(points, settings)
    }
}
