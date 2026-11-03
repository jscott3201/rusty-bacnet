//! FD_BBMD_Address as a host name on B/IP (Clause 12.56.37, #1852). When
//! to look the name up, and how, is [`crate::bbmd_lookup`]'s policy, which
//! B/IPv6 shares; this is what an answer means here: the device moves to
//! the first IPv4 address it can register with, and with
//! [`BipTransport::set_release_previous_bbmd`](super::super::BipTransport::set_release_previous_bbmd)
//! on, the node releases the BBMD it leaves (#2307).
//!
//! Lock order, as in the parent: the target's lock, then the node's.

use std::net::{IpAddr, SocketAddrV4};
use std::sync::Weak;

use tracing::{debug, info};

use super::super::ForeignBbmd;
use super::super::foreign_device::ForeignDeviceConfig;
use super::super::node::NodeHandle;
use super::{ForeignTarget, TargetState};
use crate::bbmd_lookup::{
    ATTEMPT_LOOKUP_INTERVAL, BbmdLookupFailure, BbmdName, Ended, Failed, Lookup, NameTarget, choose,
};

impl ForeignTarget {
    /// This target, as its lookups report to it.
    pub(super) fn name_target(&self) -> Weak<dyn NameTarget> {
        self.me.clone()
    }

    /// The BBMD row was activated as `name`, or as an address (`None`):
    /// the lookup to start once the lock is released, if any.
    pub(super) fn renamed(
        &self,
        state: &mut TargetState,
        name: Option<BbmdName>,
    ) -> Option<Lookup> {
        state.lookups.renamed(name, &self.name_target())
    }

    /// The node is sending an automatic registration attempt, to the
    /// address in use: look the BBMD's name up alongside it, if it has one
    /// and the last lookup started 30 seconds ago or more (#1852).
    pub(super) fn attempting(&self) {
        let lookup = self.lock().lookups.lookup(false, &self.name_target());
        if let Some(lookup) = lookup {
            lookup.start();
        }
    }

    /// Where `answers` put the BBMD: `None` to stay with the address in
    /// use, which is still one of them, or else the registration at the
    /// first IPv4 address this device can register with.
    fn choose(
        &self,
        state: &TargetState,
        node: &NodeHandle,
        name: &BbmdName,
        answers: &[IpAddr],
    ) -> Result<Option<ForeignDeviceConfig>, Failed> {
        let candidates: Vec<SocketAddrV4> = answers
            .iter()
            .filter_map(|ip| match ip {
                IpAddr::V4(ip) => Some(SocketAddrV4::new(*ip, name.port())),
                IpAddr::V6(_) => None,
            })
            .collect();
        let at = |bbmd: SocketAddrV4| {
            let mut config = state.config.clone();
            config.bbmd = ForeignBbmd::Address(bbmd);
            config
        };
        let usable = |bbmd| {
            let config = at(bbmd);
            self.engine(&config).is_ok_and(|foreign| {
                config.check_bbmd(self.broadcast, self.interface).is_ok()
                    && node.check_retarget(foreign).is_ok()
            })
        };
        let ip = |bbmd: SocketAddrV4| IpAddr::V4(*bbmd.ip());
        let chosen = choose(&candidates, state.config.bbmd(), usable, ip)?;
        Ok(chosen.map(at))
    }
}

impl ForeignTarget {
    /// While the BBMD's address isn't known, a lookup that failed is tried
    /// again once [`ATTEMPT_LOOKUP_INTERVAL`] has passed from the newest
    /// failure: with no registration attempt to go alongside, nothing else
    /// would (#2308). Each failure replaces the retry waiting, so a failure
    /// in its window, after a write or a restart, still leads to the next
    /// lookup. With no runtime, the next start retries.
    fn retry_later(&self, state: &mut TargetState) {
        Self::cancel_retry(state);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let target = self.me.clone();
        let task = runtime.spawn(async move {
            tokio::time::sleep(ATTEMPT_LOOKUP_INTERVAL).await;
            if let Some(target) = target.upgrade() {
                target.retry();
            }
        });
        state.retry = Some(task.abort_handle());
    }

    /// Cancel the retry waiting, if any.
    pub(super) fn cancel_retry(state: &mut TargetState) {
        if let Some(retry) = state.retry.take() {
            retry.abort();
        }
    }

    /// The retry [`retry_later`](Self::retry_later) set up: look the name
    /// up, if the transport still runs and still waits for an address. It
    /// comes a full interval after the newest failure, so it doesn't wait
    /// for the floor between attempt-driven lookups; one already running
    /// sets up the next retry if it fails.
    fn retry(&self) {
        let mut state = self.lock();
        state.retry = None;
        let waiting = state.config.bbmd().is_none();
        if Self::running(&state).is_none() || !waiting || state.lookups.is_running() {
            return;
        }
        let lookup = state.lookups.lookup(true, &self.name_target());
        drop(state);
        if let Some(lookup) = lookup {
            lookup.start();
        }
    }
}

impl NameTarget for ForeignTarget {
    /// One for an earlier activation, or one that ends with the transport
    /// stopped, changes and counts nothing: a start looks the name up
    /// again.
    fn looked_up(&self, generation: u64, answer: Result<Vec<IpAddr>, Failed>) {
        let mut state = self.lock();
        let node = Self::running(&state);
        let abandoned = answer
            .as_ref()
            .is_err_and(|failed| failed.failure == BbmdLookupFailure::Abandoned);
        let target = self.name_target();
        let name = match state
            .lookups
            .ended(generation, abandoned, node.is_some(), &target)
        {
            Ended::Current(name) => name,
            Ended::Stale(rerun) => {
                drop(state);
                if let Some(lookup) = rerun {
                    lookup.start();
                }
                return;
            }
        };
        let Some(node) = node else {
            return;
        };
        let verdict = answer.and_then(|addresses| self.choose(&state, &node, &name, &addresses));
        node.record()
            .bbmd_lookup_ended(verdict.as_ref().err().map(|failed| failed.failure));
        let moved_to = match verdict {
            Ok(moved_to) => moved_to,
            Err(failed) => {
                let bbmd = state.config.bbmd();
                state.lookups.failed(&name, bbmd, &failed);
                // With no address yet, no registration attempt comes to
                // look the name up again alongside (#2308).
                if state.config.bbmd().is_none() {
                    self.retry_later(&mut state);
                }
                return;
            }
        };
        state.lookups.succeeded(&name);
        let Some(config) = moved_to else {
            return;
        };
        // `choose` checked the move as a retarget would, under this lock.
        // The node releases the BBMD it leaves when asked to (#2307).
        let moved = self
            .engine(&config)
            .and_then(|foreign| node.retarget(foreign));
        if let Err(error) = moved {
            debug!(%error, "The node refused the address the BBMD's host name resolved to");
            return;
        }
        info!(host = %name.host(), bbmd = %config.bbmd, "The BBMD's host name resolves to a new address; registering there");
        state.config = config;
    }
}
