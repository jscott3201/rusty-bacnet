//! The lookups of a BBMD host name, which a foreign device's registration
//! target keeps under its own lock: when to look the name up, the one
//! resolver call running, and the task that runs it. Each transport's
//! target decides what an answer means for it ([`NameTarget`]).
//!
//! Lock order, as in each target: the target's lock, then its node's. The
//! resolver runs on a task of its own, with no lock held, and a [`Lookup`]
//! is started only once the target's lock is released, since its report
//! takes that lock.

use std::fmt::Display;
use std::io;
use std::net::IpAddr;
use std::sync::{Arc, Weak};

use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::{ATTEMPT_LOOKUP_INTERVAL, BbmdLookupFailure, BbmdName, BbmdResolver, LOOKUP_TIMEOUT};

/// A foreign device's registration target whose BBMD may be known by host
/// name: B/IP's or B/IPv6's.
pub(crate) trait NameTarget: Send + Sync + 'static {
    /// A lookup started for `generation` ended with `answer`. Free the
    /// slot with [`Lookups::ended`], under the target's lock, first.
    fn looked_up(&self, generation: u64, answer: Result<Vec<IpAddr>, Failed>);
}

/// A failed lookup, with the resolver's error for the log.
pub(crate) struct Failed {
    pub(crate) failure: BbmdLookupFailure,
    error: Option<io::Error>,
}

impl Failed {
    /// The resolver's error, when it gave one.
    #[cfg(feature = "ipv6")]
    pub(crate) fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }
}

impl From<BbmdLookupFailure> for Failed {
    fn from(failure: BbmdLookupFailure) -> Self {
        Self {
            failure,
            error: None,
        }
    }
}

/// What a lookup that ended was for.
pub(crate) enum Ended {
    /// An earlier value of the row: drop the answer, and start this lookup
    /// of the name in effect now, if one waits, once the lock is released.
    Stale(Option<Lookup>),
    /// The name in effect.
    Current(BbmdName),
}

/// A target's lookup bookkeeping, under its lock.
pub(crate) struct Lookups {
    resolver: Arc<dyn BbmdResolver>,
    /// FD_BBMD_Address as a host name, when it names one.
    name: Option<BbmdName>,
    /// Counts activations of the BBMD row: a lookup started for an earlier
    /// one is dropped when it ends.
    generation: u64,
    /// The generation of the resolver call running, if any: the only one.
    running: Option<u64>,
    /// A name was activated while a call ran: look it up when that ends.
    rerun: bool,
    /// When the newest lookup started, for [`ATTEMPT_LOOKUP_INTERVAL`].
    last_started: Option<Instant>,
    /// The newest lookup failed: a run of failures warns once.
    failing: bool,
}

impl Lookups {
    /// Lookups with `resolver`, of `name` if the BBMD is configured by one.
    pub(crate) fn new(resolver: Arc<dyn BbmdResolver>, name: Option<BbmdName>) -> Self {
        Self {
            resolver,
            name,
            generation: 0,
            running: None,
            rerun: false,
            last_started: None,
            failing: false,
        }
    }

    pub(crate) fn set_resolver(&mut self, resolver: Arc<dyn BbmdResolver>) {
        self.resolver = resolver;
    }

    /// The host name FD_BBMD_Address holds, if it holds one.
    pub(crate) fn name(&self) -> Option<&BbmdName> {
        self.name.as_ref()
    }

    /// Whether a resolver call runs.
    pub(crate) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// A node starts: its first registration attempt looks the name up
    /// again, whenever the last lookup was. A call still running carries on
    /// as the only one: every started lookup frees its slot when it ends,
    /// answered or not ([`Lookup`]'s drop), and its answer then serves the
    /// new node.
    pub(crate) fn attached(&mut self) {
        self.last_started = None;
    }

    /// The BBMD row was activated as `name`, or as an address (`None`): a
    /// lookup started for an earlier value no longer counts, and a name is
    /// looked up now, or as soon as the call running ends.
    pub(crate) fn renamed(
        &mut self,
        name: Option<BbmdName>,
        target: &Weak<dyn NameTarget>,
    ) -> Option<Lookup> {
        self.generation = self.generation.wrapping_add(1);
        self.rerun = false;
        self.failing = false;
        self.name = name;
        self.lookup(true, target)
    }

    /// A lookup of the name in effect, unless there is none, or a resolver
    /// call runs already (an activation then waits for it to end), or,
    /// unless an activation asks (`activated`) or waits, the last lookup
    /// started less than [`ATTEMPT_LOOKUP_INTERVAL`] ago. A lookup returned
    /// holds the slot: start it once the lock is released.
    #[must_use = "a lookup holds the resolver slot until it is started"]
    pub(crate) fn lookup(
        &mut self,
        activated: bool,
        target: &Weak<dyn NameTarget>,
    ) -> Option<Lookup> {
        let host = self.name.as_ref()?.host().to_owned();
        if self.running.is_some() {
            self.rerun |= activated;
            return None;
        }
        let now = Instant::now();
        let recent = self
            .last_started
            .is_some_and(|started| now < started + ATTEMPT_LOOKUP_INTERVAL);
        if recent && !activated && !self.rerun {
            return None;
        }
        self.rerun = false;
        self.running = Some(self.generation);
        self.last_started = Some(now);
        Some(Lookup {
            target: target.clone(),
            resolver: Arc::clone(&self.resolver),
            host,
            generation: self.generation,
            armed: false,
            started: false,
        })
    }

    /// A lookup started for `generation` ended, `abandoned` or with an
    /// answer, and with it the resolver call. One for an earlier activation
    /// is [`Ended::Stale`]: a name activated meanwhile is looked up now,
    /// while the transport is `running`, unless this one was abandoned,
    /// perhaps with the runtime: then the next attempt, or the next start,
    /// does it.
    pub(crate) fn ended(
        &mut self,
        generation: u64,
        abandoned: bool,
        running: bool,
        target: &Weak<dyn NameTarget>,
    ) -> Ended {
        if self.running == Some(generation) {
            self.running = None;
        }
        match (&self.name, self.generation == generation) {
            (Some(name), true) => Ended::Current(name.clone()),
            _ => {
                debug!("A BBMD host-name lookup ended after the row changed; dropped");
                let rerun = self.rerun && !abandoned && running;
                Ended::Stale(rerun.then(|| self.lookup(true, target)).flatten())
            }
        }
    }

    /// The name resolved to an address the device can register with: a run
    /// of failures ends.
    pub(crate) fn succeeded(&mut self, name: &BbmdName) {
        if std::mem::take(&mut self.failing) {
            info!(host = %name.host(), "The BBMD's host name resolves again");
        }
    }

    /// Log a failed lookup of `name`, the device staying at `bbmd`, or,
    /// with no address yet (`None`, a configured name), waiting for one:
    /// the first of a run at warn, the rest at debug.
    pub(crate) fn failed<A: Display>(&mut self, name: &BbmdName, bbmd: Option<A>, failed: &Failed) {
        let host = name.host();
        let failure = failed.failure;
        let error = failed.error.as_ref().map(ToString::to_string);
        let repeat = std::mem::replace(&mut self.failing, true);
        let Some(bbmd) = bbmd else {
            if repeat {
                debug!(%host, ?failure, ?error, "The BBMD's host name still doesn't resolve; trying again in 30 s");
            } else {
                warn!(%host, ?failure, ?error, "The BBMD's host name doesn't resolve to a usable address; no BBMD to register with yet, trying again in 30 s");
            }
            return;
        };
        if failure == BbmdLookupFailure::Abandoned {
            if repeat {
                debug!(%host, %bbmd, "A lookup of the BBMD's host name ended without an answer again");
            } else {
                warn!(%host, %bbmd, "A lookup of the BBMD's host name ended without an answer; registering with the last address it had");
            }
        } else if repeat {
            debug!(%host, ?failure, ?error, %bbmd, "The BBMD's host name still doesn't resolve to a usable address");
        } else {
            warn!(%host, ?failure, ?error, %bbmd, "The BBMD's host name doesn't resolve to a usable address; registering with the last address it had");
        }
    }
}

/// What `resolver` answers for `host`, given up after [`LOOKUP_TIMEOUT`]:
/// a [`Lookup`]'s call.
async fn resolve(resolver: &dyn BbmdResolver, host: &str) -> Result<Vec<IpAddr>, Failed> {
    match tokio::time::timeout(LOOKUP_TIMEOUT, resolver.resolve(host)).await {
        Ok(Ok(addresses)) => Ok(addresses),
        Ok(Err(error)) => Err(Failed {
            failure: BbmdLookupFailure::Error(error.kind()),
            error: Some(error),
        }),
        Err(_) => Err(BbmdLookupFailure::TimedOut.into()),
    }
}

/// Where a lookup's `answers`, the ones of the transport's family in the
/// order given, put the BBMD now at `current` (`None` when it has no
/// address yet): `None` to stay, while it is still among them, or else the
/// first one `usable` takes. Failing that,
/// [`BbmdLookupFailure::Unusable`] with the first of them, or
/// [`BbmdLookupFailure::NoAddress`] when there are none.
pub(crate) fn choose<A: Copy + PartialEq>(
    answers: &[A],
    current: Option<A>,
    usable: impl Fn(A) -> bool,
    ip: impl Fn(A) -> IpAddr,
) -> Result<Option<A>, BbmdLookupFailure> {
    if current.is_some_and(|current| answers.contains(&current)) {
        return Ok(None);
    }
    if let Some(found) = answers.iter().copied().find(|answer| usable(*answer)) {
        return Ok(Some(found));
    }
    Err(answers
        .first()
        .map_or(BbmdLookupFailure::NoAddress, |first| {
            BbmdLookupFailure::Unusable(ip(*first))
        }))
}

/// A lookup to start once the target's lock is released. Once started it
/// always reports to the target, which frees the resolver slot: with its
/// answer, or, dropped without one (its resolver panicked, or its task was
/// dropped with the runtime), as [`BbmdLookupFailure::Abandoned`].
///
/// Making one takes the slot, so every one made must be started: one
/// dropped unstarted would hold the slot until the next activation. It is
/// made under the lock and started once the lock is released, since its
/// report takes the lock.
#[must_use = "a lookup holds the resolver slot until it is started"]
pub(crate) struct Lookup {
    target: Weak<dyn NameTarget>,
    resolver: Arc<dyn BbmdResolver>,
    host: String,
    generation: u64,
    /// Started and not reported yet.
    armed: bool,
    /// Started at all, for the check on drop.
    started: bool,
}

impl Lookup {
    /// Run it on a task of its own. With no runtime to run it on, it fails
    /// at once.
    pub(crate) fn start(mut self) {
        self.armed = true;
        self.started = true;
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(self.run());
            }
            Err(error) => {
                let failed = Failed {
                    failure: BbmdLookupFailure::Error(io::ErrorKind::Other),
                    error: Some(io::Error::other(error)),
                };
                self.end(Err(failed));
            }
        }
    }

    /// Run it here, as a B/IPv6 start on `::` does (#2365): its generation
    /// and answer, which the caller reports to [`Lookups::ended`] under its
    /// lock. Dropped before it answers, it ends as abandoned, as a started
    /// one does, so the slot is freed either way.
    #[cfg(feature = "ipv6")]
    pub(crate) async fn in_place(mut self) -> (u64, Result<Vec<IpAddr>, Failed>) {
        self.armed = true;
        self.started = true;
        let answer = resolve(&*self.resolver, &self.host).await;
        self.armed = false;
        (self.generation, answer)
    }

    async fn run(mut self) {
        let answer = resolve(&*self.resolver, &self.host).await;
        self.end(answer);
    }

    /// Report `answer`, once.
    fn end(&mut self, answer: Result<Vec<IpAddr>, Failed>) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        if let Some(target) = self.target.upgrade() {
            target.looked_up(self.generation, answer);
        }
    }
}

impl Drop for Lookup {
    /// A started lookup that never answered still ends: unwinding from a
    /// panicking resolver, or dropped with its task. No lock is held where
    /// a started one can drop.
    fn drop(&mut self) {
        debug_assert!(
            self.started || std::thread::panicking(),
            "a BBMD lookup was made and never started, so it holds the resolver slot"
        );
        self.end(Err(BbmdLookupFailure::Abandoned.into()));
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::{BbmdLookupFailure, choose};

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 0, 2, last)
    }

    #[test]
    fn the_device_stays_while_its_address_answers_and_else_takes_the_first_usable() {
        let pick = |answers: &[Ipv4Addr], usable: &[Ipv4Addr]| {
            choose(
                answers,
                Some(ip(1)),
                |answer| usable.contains(&answer),
                IpAddr::V4,
            )
        };
        assert_eq!(pick(&[ip(3), ip(1)], &[ip(3)]), Ok(None));
        assert_eq!(
            pick(&[ip(2), ip(3), ip(4)], &[ip(3), ip(4)]),
            Ok(Some(ip(3)))
        );
        assert_eq!(
            pick(&[ip(2), ip(3)], &[]),
            Err(BbmdLookupFailure::Unusable(ip(2).into()))
        );
        assert_eq!(pick(&[], &[]), Err(BbmdLookupFailure::NoAddress));
        // With no address yet, the device never stays.
        let first = choose(&[ip(1)], None, |_| true, IpAddr::V4);
        assert_eq!(first, Ok(Some(ip(1))));
    }
}
