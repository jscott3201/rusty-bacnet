//! Local return is independent of physical ownership, Write and the filter.

use bacnet_mstp_core::{Duration, Instant};

use crate::line::{Character, TxId};
use crate::report::ViolationKind;
use crate::sim::Sim;
use crate::trace::{TraceKind, Who};

pub(super) const RETURN_BOUND: usize = 4096;

/// Captured once at confirmed physical start.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Profile {
    pub incarnation: u64,
    pub delay: Duration,
}

/// Own the observed character; neither the live line nor a later Write may
/// erase it. FIFO preserves physical observation order across profile changes.
pub(super) struct Return {
    pub tx: TxId,
    pub incarnation: u64,
    pub character: Character,
    pub wire_end: Instant,
    pub requested_at: Instant,
    pub available_at: Instant,
}

impl Sim {
    pub(in crate::sim) fn host_local_echo(&mut self, node: usize, tx: TxId, character: Character) {
        let wire_end = self.now;
        let profile = self
            .meta
            .get(&tx)
            .and_then(|meta| meta.echo_profile)
            .expect("a captured host return profile");
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        if host.local_echo.len() == RETURN_BOUND {
            let first = !std::mem::replace(&mut host.local_echo_overflow, true);
            self.report.local_echo.overflows += 1;
            if first {
                self.violation(
                    ViolationKind::EventGrowth,
                    "local echo return exceeded 4096 characters".into(),
                );
            }
            return;
        }
        let requested_at = wire_end + profile.delay;
        let available_at = host
            .local_echo
            .back()
            .map_or(requested_at, |last| last.available_at.max(requested_at));
        host.local_echo.push_back(Return {
            tx,
            incarnation: profile.incarnation,
            character,
            wire_end,
            requested_at,
            available_at,
        });
        let pending = host.local_echo.len();
        let stats = &mut self.report.local_echo;
        stats.captured += 1;
        stats.pending += 1;
        stats.max_pending = stats.max_pending.max(pending);
    }

    /// One queue-head wakeup releases every due character before host service.
    /// Provenance is diagnostic only; all bytes face the ordinary echo filter.
    pub(in crate::sim) fn local_echo_service(&mut self, node: usize) {
        let now = self.now;
        let station = self.nodes[node].station;
        loop {
            let master = self.master_mut(node).unwrap();
            let Some(host) = master.host.as_mut() else {
                return;
            };
            if !host
                .local_echo
                .front()
                .is_some_and(|echo| echo.available_at <= now)
            {
                return;
            }
            let echo = host.local_echo.pop_front().unwrap();
            host.local_echo_overflow = false;
            let current = echo.incarnation == master.incarnation;
            self.report.local_echo.pending -= 1;
            if !current {
                self.report.local_echo.cancelled += 1;
                continue;
            }
            self.report.local_echo.released += 1;
            self.note(
                Who::Station(station),
                TraceKind::LocalEcho {
                    transmission: echo.tx,
                    incarnation: echo.incarnation,
                    wire_end: echo.wire_end,
                    requested_at: echo.requested_at,
                    byte: match echo.character {
                        Character::Clean(byte) => Some(byte),
                        Character::Garbled => None,
                    },
                },
            );
            self.host_character_at(node, echo.character, echo.wire_end);
        }
    }

    /// Power loss wins a same-time return: scenario events precede the local
    /// availability class, and the incarnation owns its sole scheduler entry.
    pub(in crate::sim) fn cancel_local_echo(&mut self, node: usize) {
        let cancelled = self
            .master_mut(node)
            .and_then(|master| master.host.as_mut())
            .map_or(0, |host| {
                let count = host.local_echo.len();
                host.local_echo.clear();
                host.local_echo_overflow = false;
                count
            });
        self.report.local_echo.pending -= cancelled;
        self.report.local_echo.cancelled += cancelled as u64;
        if cancelled > 0 {
            self.note(
                Who::Station(self.nodes[node].station),
                TraceKind::Note(format!(
                    "power loss cancelled {cancelled} local echo characters"
                )),
            );
        }
    }
}
