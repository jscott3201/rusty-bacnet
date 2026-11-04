//! A B/IPv6 foreign device whose configured BBMD is a host name (#2365),
//! before the name first resolves.
//!
//! A transport bound to `::` picks its source address from the route to
//! the BBMD, so it can't bind before it has that address: its start looks
//! the name up in place and fails if the name doesn't resolve to an IPv6
//! address it could register with ([`ForeignTarget::start_bbmd`]). The
//! address found is the registration's once the start succeeds.
//!
//! On an explicit interface the start goes ahead and the device waits: no
//! registration is sent, broadcasts are refused and counted (B/IPv6 keeps
//! no hold queue), Address-Resolutions aren't sent, and no Forwarded-NPDU
//! is trusted. The name is looked up at start and alongside the
//! re-registration task's passes, as a written name is, and, after each
//! failure, again 30 seconds after it: the re-registration task keeps that
//! time ([`ForeignTarget::retry_due`]), which each failure sets afresh and
//! the first success clears; the task ends at stop. The first IPv6 answer the
//! device can register with ends the wait, and it registers there at once.

use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddrV6};

use bacnet_types::error::Error;
use tokio::time::Instant;

use super::release::Bip6ReleaseCounters;
use super::{Bip6ForeignBbmd, ForeignTarget};
use crate::bbmd_lookup::{BbmdLookupCounters, BbmdLookupFailure, Ended, Failed, choose};

/// How often a start on `::` checks whether a lookup left running by a
/// quick restart has ended.
const SLOT_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// A B/IPv6 foreign device's registration as it stands
/// ([`Bip6Transport::foreign_status`](crate::bip6::Bip6Transport::foreign_status)).
/// The totals survive a restart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bip6ForeignStatus {
    /// The BBMD's address in use: the configured one, or the one its host
    /// name last resolved to. `None` until a configured name first
    /// resolves.
    pub bbmd: Option<SocketAddrV6>,
    /// The host name and port the BBMD is known by, configured or written
    /// to FD_BBMD_Address, as a [`Bip6ForeignBbmd::Name`]; `None` for a
    /// BBMD known by address. A waiting device waits for this name.
    pub bbmd_name: Option<Bip6ForeignBbmd>,
    /// The time-to-live it registers with, in seconds.
    pub ttl: u16,
    /// Whether the running device waits for its BBMD's host name to
    /// resolve (#2365): nothing is registered meanwhile, and broadcasts
    /// are refused. False once stopped.
    pub waiting_for_bbmd: bool,
    /// What came of the lookups of the BBMD's host name, as
    /// [`bbmd_lookup_counters`](crate::bip6::Bip6Transport::bbmd_lookup_counters)
    /// reports them.
    pub lookups: BbmdLookupCounters,
    /// What came of the releases of the BBMDs the device moved away from,
    /// as [`previous_bbmd_releases`](crate::bip6::Bip6Transport::previous_bbmd_releases)
    /// reports them.
    pub releases: Bip6ReleaseCounters,
    /// Broadcasts refused, with an error, because the device waited for
    /// its BBMD's host name.
    pub broadcasts_refused: u64,
    /// Register-Foreign-Device messages sent, to whichever BBMD. B/IPv6
    /// doesn't track the BBMD's answers.
    pub registrations_sent: u64,
}

/// What a start learned of its BBMD before binding.
pub(in crate::bip6) struct StartBbmd {
    /// The BBMD's address to bind by, if one is known.
    pub(in crate::bip6) bbmd: Option<SocketAddrV6>,
    /// The start looked the name up already, so its first pass needn't.
    pub(in crate::bip6) looked_up: bool,
}

/// Why a start's lookup failed, for its error.
fn reason(failed: &Failed) -> String {
    match (failed.failure, failed.error()) {
        (BbmdLookupFailure::Error(_), Some(error)) => error.to_string(),
        (BbmdLookupFailure::Error(kind), None) => kind.to_string(),
        (BbmdLookupFailure::TimedOut, _) => "the resolver had no answer within 10 s".into(),
        (BbmdLookupFailure::NoAddress, _) => "the name has no IPv6 address".into(),
        (BbmdLookupFailure::Unusable(ip), _) => {
            format!("the name stands only for addresses it can't register with, such as {ip}")
        }
        (BbmdLookupFailure::Abandoned, _) => "the lookup ended without an answer".into(),
    }
}

/// Whether a start on `::` could register with `bbmd`: an IPv6 unicast
/// host this host's routes reach.
fn usable_at_start(bbmd: SocketAddrV6) -> bool {
    let config = super::Bip6ForeignDeviceConfig {
        bbmd: bbmd.into(),
        ttl: 1,
    };
    config.check().is_ok() && super::super::socket::route_source(bbmd).is_ok()
}

impl ForeignTarget {
    /// This registration as it stands.
    pub(in crate::bip6) fn status(&self) -> Bip6ForeignStatus {
        let state = self.lock();
        let bbmd = state.config.bbmd.address();
        let bbmd_name = match (state.lookups.name(), &state.config.bbmd) {
            (Some(name), _) => Some(Bip6ForeignBbmd::Name {
                host: name.host().to_owned(),
                port: name.port(),
            }),
            (None, named @ Bip6ForeignBbmd::Name { .. }) => Some(named.clone()),
            (None, Bip6ForeignBbmd::Address(_)) => None,
        };
        Bip6ForeignStatus {
            bbmd,
            bbmd_name,
            ttl: state.config.ttl,
            waiting_for_bbmd: state.own.is_some() && bbmd.is_none(),
            lookups: state.counters,
            releases: state.releases,
            broadcasts_refused: state.broadcasts_refused,
            registrations_sent: state.registrations_sent,
        }
    }

    /// Before a start on `interface`: the BBMD's address to bind by. A BBMD
    /// known by address, or an explicit interface, needs no lookup now;
    /// the device then waits for a name. On `::` a name is looked up in
    /// place, in the lookups' one slot, counted as any lookup is, and the
    /// start fails unless it stands for an IPv6 address the device could
    /// register with. The address found becomes the registration's only
    /// once the start succeeds ([`attach`](Self::attach)): a start that
    /// fails later keeps the name, which the next start looks up again.
    pub(in crate::bip6) async fn start_bbmd(
        &self,
        interface: Ipv6Addr,
    ) -> Result<StartBbmd, Error> {
        let (name, lookup) = loop {
            // `None` while a lookup left running by a quick restart holds
            // the slot, which it frees within the resolver's 10 s.
            let slot = {
                let mut state = self.lock();
                if let Some(bbmd) = state.config.bbmd.address() {
                    return Ok(StartBbmd {
                        bbmd: Some(bbmd),
                        looked_up: false,
                    });
                }
                let (true, Some(name)) = (interface.is_unspecified(), state.config.bbmd.name())
                else {
                    return Ok(StartBbmd {
                        bbmd: None,
                        looked_up: false,
                    });
                };
                if state.lookups.is_running() {
                    None
                } else {
                    // The name is the lookups' own: a lookup always comes.
                    let lookup = state.lookups.lookup(true, &self.name_target());
                    Some(lookup.map(|lookup| (name, lookup)))
                }
            };
            match slot {
                Some(Some(found)) => break found,
                Some(None) => {
                    return Ok(StartBbmd {
                        bbmd: None,
                        looked_up: false,
                    });
                }
                None => tokio::time::sleep(SLOT_POLL).await,
            }
        };
        // Dropped with a cancelled start, the lookup still frees the slot.
        let (generation, answer) = lookup.in_place().await;
        let verdict = answer.and_then(|answers| {
            let candidates: Vec<SocketAddrV6> = answers
                .iter()
                .filter_map(|ip| match ip {
                    IpAddr::V6(ip) => Some(SocketAddrV6::new(*ip, name.port(), 0, 0)),
                    IpAddr::V4(_) => None,
                })
                .collect();
            let ip = |bbmd: SocketAddrV6| IpAddr::V6(*bbmd.ip());
            // With no address in use, a found address is the only `Ok`.
            choose(&candidates, None, usable_at_start, ip)
                .and_then(|found| found.ok_or(BbmdLookupFailure::NoAddress))
                .map_err(Failed::from)
        });
        let mut state = self.lock();
        // Nothing can change the row before the start attaches, so this
        // lookup is the current one; it frees the slot.
        let ended = state
            .lookups
            .ended(generation, false, false, &self.name_target());
        debug_assert!(matches!(ended, Ended::Current(_)));
        drop(ended);
        state
            .counters
            .ended(verdict.as_ref().err().map(|failed| failed.failure));
        match verdict {
            Ok(bbmd) => {
                state.lookups.succeeded(&name);
                Ok(StartBbmd {
                    bbmd: Some(bbmd),
                    looked_up: true,
                })
            }
            Err(failed) => {
                let reason = reason(&failed);
                Err(Error::Transport(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "BIP6 foreign-device BBMD {}:{} has no usable IPv6 address ({reason}); \
                         a start on '::' needs one to pick its source address, so give an \
                         explicit interface to start and wait for the name",
                        name.host(),
                        name.port()
                    ),
                )))
            }
        }
    }

    /// When the waiting device looks its BBMD's name up again, if it does.
    pub(in crate::bip6) fn retry_due(&self) -> Option<Instant> {
        self.lock().retry_at
    }

    /// Resolve when a failed lookup sets when the name is tried again.
    pub(in crate::bip6) async fn rearmed(&self) {
        self.rearmed.notified().await;
    }

    /// The retry [`retry_due`](Self::retry_due) set for `due` is due: look
    /// the name up, if the device still runs and waits, and no lookup runs
    /// already (a failure of that one sets the next retry). A retry a newer
    /// failure replaced while it woke does nothing: the newer one stands.
    pub(in crate::bip6) fn retry(&self, due: Instant) {
        let mut state = self.lock();
        if state.retry_at != Some(due) {
            return;
        }
        state.retry_at = None;
        let waiting = state.own.is_some() && state.config.bbmd.address().is_none();
        if !waiting || state.lookups.is_running() {
            return;
        }
        let lookup = state.lookups.lookup(true, &self.name_target());
        drop(state);
        if let Some(lookup) = lookup {
            lookup.start();
        }
    }

    /// A Register-Foreign-Device went out.
    pub(in crate::bip6) fn registration_sent(&self) {
        let mut state = self.lock();
        state.registrations_sent = state.registrations_sent.saturating_add(1);
    }

    /// A broadcast was refused while the device waited for its BBMD.
    pub(in crate::bip6) fn broadcast_refused(&self) {
        let mut state = self.lock();
        state.broadcasts_refused = state.broadcasts_refused.saturating_add(1);
    }
}
