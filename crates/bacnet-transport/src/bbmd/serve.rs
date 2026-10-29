//! The hosted BBMD's answers to the management requests it serves
//! (Register-Foreign-Device, the two table reads and
//! Delete-Foreign-Device-Table-Entry, Annex J.4.4.1 and J.5.2): the core's
//! [`Tables::serve`](bacnet_bip_core::bbmd::Tables::serve), under the
//! host's own [`Rules`].
//!
//! [`BbmdState::serve`] runs under the BBMD lock and copies the answer out
//! of the tables ([`Reply`]), so the caller releases the lock before it
//! encodes or sends anything.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Instant;

use bacnet_bip_codec::{self as codec, BdtEntries, BvlcResultCode, Bvll, FdtEntries};
use bacnet_bip_core::Instant as CoreInstant;
use bacnet_bip_core::bbmd::{
    RegistrationLimits, RegistrationRefusal, Request, Reservation, Rules, Served,
};

use super::policy::ForeignDeviceRateTracker;
use super::{BbmdState, ForeignDevicePolicy, MAX_FDT};

/// The hosted BBMD's [`Rules`]: its [`ForeignDevicePolicy`], registration
/// rate limit and Delete-FDT-Entry access list.
struct HostRules<'a> {
    /// The registration policy; without one every registration is refused.
    policy: Option<&'a ForeignDevicePolicy>,
    /// The registration rate limit's window. It is only checked here: a
    /// registration is recorded against it once accepted
    /// ([`BbmdState::serve`]).
    rate_tracker: &'a mut ForeignDeviceRateTracker,
    /// Who may delete a registration; empty denies everyone.
    management_acl: &'a [[u8; 4]],
    /// The host's time for the rate limit, which runs on `std` instants.
    now: Instant,
}

impl Rules for HostRules<'_> {
    fn registration_limits(&mut self) -> RegistrationLimits {
        let mut limits = RegistrationLimits::DEFAULT;
        if let Some(policy) = self.policy {
            limits.min_ttl = policy.min_ttl;
            limits.max_ttl = policy.max_ttl;
            limits.max_entries_per_source = policy.max_entries_per_source;
            limits.reserved_capacity = policy.reserved_capacity;
        }
        limits
    }

    fn reservation(&mut self, source: SocketAddrV4) -> Reservation {
        let reserved = self
            .policy
            .is_some_and(|policy| policy.reserved_sources.contains(&source.ip().octets()));
        if reserved {
            Reservation::Reserved
        } else {
            Reservation::Ordinary
        }
    }

    /// No policy refuses everyone. With one, the source must be on its
    /// access list, when it has one, and within the rate limit.
    fn admit_registration(&mut self, source: SocketAddrV4, _now: CoreInstant) -> bool {
        let Some(policy) = self.policy else {
            return false;
        };
        let ip = source.ip().octets();
        policy
            .allowed_sources
            .as_ref()
            .is_none_or(|allowed| allowed.contains(&ip))
            && !self.rate_tracker.is_rate_exceeded(
                ip,
                self.now,
                policy.rate_window,
                policy.registration_rate_per_source,
                policy.registration_rate_global,
            )
    }

    /// Fail closed: only a sender the access list names may delete.
    fn admit_delete(&mut self, sender: SocketAddrV4, _entry: SocketAddrV4) -> bool {
        self.management_acl.contains(&sender.ip().octets())
    }
}

/// An answer [`BbmdState::serve`] copied out of the tables, to encode and
/// send once the lock is released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// A BVLC-Result.
    Result(BvlcResultCode),
    /// A Read-Broadcast-Distribution-Table-Ack with these rows.
    BdtAck(Vec<codec::BdtEntry>),
    /// A Read-Foreign-Device-Table-Ack with these rows.
    FdtAck(Vec<codec::FdtEntry>),
}

impl Reply {
    /// The message to encode.
    pub(crate) fn message(&self) -> Bvll<'_> {
        match self {
            Self::Result(code) => Bvll::BvlcResult(*code),
            Self::BdtAck(rows) => {
                Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(rows))
            }
            Self::FdtAck(rows) => Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(rows)),
        }
    }
}

impl BbmdState {
    /// Answer `request` at `now` through the core's
    /// [`Tables::serve`](bacnet_bip_core::bbmd::Tables::serve), under this
    /// BBMD's rules, and count what it came to.
    ///
    /// A table read or delete purges the lapsed registrations first, so
    /// [`FdtCounters::registrations_expired`](super::FdtCounters) counts
    /// them; a lapsed registration no longer exists (Annex J.5.2.3), so a
    /// delete naming one gets the NAK (J.5.2.1.2). A registration counts
    /// against the rate limit only once accepted.
    pub(crate) fn serve(&mut self, request: Request, now: Instant) -> Reply {
        if matches!(
            request,
            Request::ReadFdt { .. } | Request::DeleteFdtEntry { .. }
        ) {
            self.purge_expired_at(now);
        }
        let mut rules = HostRules {
            policy: self.foreign_device_policy.as_ref(),
            rate_tracker: &mut self.rate_tracker,
            management_acl: &self.management_acl,
            now,
        };
        let mut rows =
            [codec::FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0); MAX_FDT];
        let core_now = self.clock.instant(now);
        let answer = self.tables.serve(request, core_now, &mut rules, &mut rows);
        let reply = match answer.message {
            Bvll::BvlcResult(code) => Reply::Result(code),
            Bvll::ReadBroadcastDistributionTableAck(rows) => Reply::BdtAck(rows.iter().collect()),
            Bvll::ReadForeignDeviceTableAck(rows) => Reply::FdtAck(rows.iter().collect()),
            other => unreachable!("serve answers with a result or a table ACK, not {other:?}"),
        };
        let served = answer.served;
        if let Served::Register(outcome) = served {
            self.counters.registrations_expired += outcome.purged as u64;
            match outcome.result {
                Ok(_) => {
                    self.rate_tracker.record(request.sender().ip().octets());
                    self.counters.registrations_accepted += 1;
                }
                Err(refusal) => {
                    if refusal == RegistrationRefusal::TableFull {
                        self.counters.capacity_exhausted += 1;
                    }
                    self.counters.registrations_rejected += 1;
                }
            }
        }
        reply
    }
}
