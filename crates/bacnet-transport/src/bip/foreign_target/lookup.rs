//! FD_BBMD_Address as a host name on B/IP (Clause 12.56.37, #1852). When
//! to look the name up, and how, is [`crate::bbmd_lookup`]'s policy, which
//! B/IPv6 shares; this is what an answer means here: the device moves to
//! the first IPv4 address it can register with, as an activated address
//! would move it.
//!
//! Lock order, as in the parent: the target's lock, then the node's.

use std::net::{IpAddr, SocketAddrV4};
use std::sync::Weak;
use std::sync::atomic::Ordering;

use tracing::{debug, info};

use super::super::foreign_device::ForeignDeviceConfig;
use super::super::node::NodeHandle;
use super::{ForeignTarget, TargetState};
use crate::bbmd_lookup::{BbmdLookupFailure, BbmdName, Ended, Failed, Lookup, NameTarget, choose};

impl ForeignTarget {
    /// This target, as its lookups report to it.
    fn name_target(&self) -> Weak<dyn NameTarget> {
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
            config.bbmd_ip = *bbmd.ip();
            config.bbmd_port = bbmd.port();
            config
        };
        let usable = |bbmd| {
            let config = at(bbmd);
            config.interval().is_ok_and(|interval| {
                config.check_bbmd(self.broadcast, self.interface).is_ok()
                    && node.check_retarget(config.engine(interval)).is_ok()
            })
        };
        let ip = |bbmd: SocketAddrV4| IpAddr::V4(*bbmd.ip());
        let chosen = choose(&candidates, state.config.bbmd(), usable, ip)?;
        Ok(chosen.map(at))
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
                return;
            }
        };
        state.lookups.succeeded(&name);
        let Some(config) = moved_to else {
            return;
        };
        let old = state.config.bbmd();
        // `choose` checked the move as a retarget would, under this lock.
        let moved = config
            .interval()
            .and_then(|interval| node.retarget(config.engine(interval)));
        if let Err(error) = moved {
            debug!(%error, "The node refused the address the BBMD's host name resolved to");
            return;
        }
        info!(host = %name.host(), bbmd = %config.bbmd(), "The BBMD's host name now resolves elsewhere; registering there");
        state.config = config;
        drop(state);
        if self.release_previous.load(Ordering::Relaxed) {
            node.release_previous_bbmd(old);
        }
    }
}
