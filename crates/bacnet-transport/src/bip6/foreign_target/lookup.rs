//! FD_BBMD_Address as a host name on B/IPv6 (Clause 12.56.37; Table 12-71
//! footnote 13, #2308). When to look the name up, and how, is
//! [`crate::bbmd_lookup`]'s policy, shared with B/IP; this is what an
//! answer means here: the device moves to the first IPv6 address it can
//! register with, which a Network Port write would accept
//! ([`ForeignTarget::usable`]), and its re-registration task registers
//! there at once.

use std::net::{IpAddr, SocketAddrV6};
use std::sync::Weak;

use tracing::info;

use super::super::port::Bip6ForeignDeviceConfig;
use super::ForeignTarget;
use crate::bbmd_lookup::{BbmdLookupFailure, BbmdName, Ended, Failed, NameTarget, choose};

impl ForeignTarget {
    /// This target, as its lookups report to it.
    pub(super) fn name_target(&self) -> Weak<dyn NameTarget> {
        self.me.clone()
    }

    /// The transport is sending an automatic registration attempt, to the
    /// address in use: look the BBMD's name up alongside it, if it has one
    /// and the last lookup started 30 seconds ago or more.
    pub(in crate::bip6) fn attempting(&self) {
        let lookup = self.lock().lookups.lookup(false, &self.name_target());
        if let Some(lookup) = lookup {
            lookup.start();
        }
    }

    /// Where `answers` put the BBMD, for a device bound to `own`: `None` to
    /// stay with the address in use, which is still one of them, or else
    /// the registration at the first IPv6 address it can register with.
    fn choose(
        config: &Bip6ForeignDeviceConfig,
        own: SocketAddrV6,
        name: &BbmdName,
        answers: &[IpAddr],
    ) -> Result<Option<Bip6ForeignDeviceConfig>, BbmdLookupFailure> {
        let candidates: Vec<SocketAddrV6> = answers
            .iter()
            .filter_map(|ip| match ip {
                IpAddr::V6(ip) => Some(SocketAddrV6::new(*ip, name.port(), 0, 0)),
                IpAddr::V4(_) => None,
            })
            .collect();
        let at = |bbmd: SocketAddrV6| {
            let mut moved = config.clone();
            moved.bbmd_ip = *bbmd.ip();
            moved.bbmd_port = bbmd.port();
            moved
        };
        let current = SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0);
        let usable = |bbmd| Self::usable(own, &at(bbmd));
        let ip = |bbmd: SocketAddrV6| IpAddr::V6(*bbmd.ip());
        Ok(choose(&candidates, Some(current), usable, ip)?.map(at))
    }
}

impl NameTarget for ForeignTarget {
    /// One for an earlier activation, or one that ends with the transport
    /// stopped, changes and counts nothing: a start looks the name up
    /// again.
    fn looked_up(&self, generation: u64, answer: Result<Vec<IpAddr>, Failed>) {
        let mut state = self.lock();
        let own = state.own;
        let abandoned = answer
            .as_ref()
            .is_err_and(|failed| failed.failure == BbmdLookupFailure::Abandoned);
        let target = self.name_target();
        let name = match state
            .lookups
            .ended(generation, abandoned, own.is_some(), &target)
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
        let Some(own) = own else {
            return;
        };
        let verdict = answer.and_then(|addresses| {
            Self::choose(&state.config, own, &name, &addresses).map_err(Failed::from)
        });
        state
            .counters
            .ended(verdict.as_ref().err().map(|failed| failed.failure));
        let moved_to = match verdict {
            Ok(moved_to) => moved_to,
            Err(failed) => {
                let bbmd = SocketAddrV6::new(state.config.bbmd_ip, state.config.bbmd_port, 0, 0);
                state.lookups.failed(&name, Some(bbmd), &failed);
                return;
            }
        };
        state.lookups.succeeded(&name);
        let Some(config) = moved_to else {
            return;
        };
        let bbmd = SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0);
        info!(host = %name.host(), %bbmd, "The B/IPv6 BBMD's host name now resolves elsewhere; registering there");
        state.take(config);
        drop(state);
        self.changed.notify_one();
    }
}
