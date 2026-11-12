//! The confirmed table: each confirmed notification handed out and not yet
//! answered, with its invoke ID, peer, deadline and retries left
//! (Clause 5.4.4).
//!
//! A notification [`pop`](super::Reporter::pop) hands out waits for its
//! answer until its deadline, one APDU timeout away. An answer from its peer
//! with its invoke ID ends it. At the deadline it is due to be sent again
//! while retries are left, and `pop` resends it under the same invoke ID;
//! with none left it has failed. However late it first went out, it fails
//! at its cutoff ([`FlightTiming::cutoff`](crate::cov::FlightTiming::cutoff)
//! from when the table began it), as the server's worker does, so its
//! last answer is never awaited past the table's deadline for the report.
//! A report the COV table fenced, the host couldn't send, or whose object
//! the host deleted, leaves the table at once.

use alloc::vec::Vec;

use bacnet_time::{Duration, Instant};
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::primitives::ObjectIdentifier;

use super::NotifyService;
use crate::cov::{SubscriberEndpoint, Token};

/// One confirmed notification out.
#[derive(Debug)]
pub(super) struct Outstanding {
    pub(super) token: Token,
    /// The object it reports, encoded: deleting it ends the notification.
    /// Four octets: an entry is no larger on a 64-bit host, and 8 octets
    /// larger on a 32-bit target.
    object: [u8; 4],
    /// Who answers it, and whose invoke ID it holds.
    pub(super) peer: BACnetAddress,
    pub(super) to: SubscriberEndpoint,
    pub(super) invoke_id: u8,
    /// Which service it is.
    pub(super) service: NotifyService,
    /// Its service parameters, for a resend.
    pub(super) params: Vec<u8>,
    /// When the current attempt's answer is overdue.
    deadline: Instant,
    /// When the report fails whatever its attempts: its begin plus the
    /// cutoff.
    cutoff: Instant,
    retries_left: u8,
    /// Its deadline passed with retries left: the next `pop` resends it.
    resend: bool,
}

impl Outstanding {
    pub(super) fn new(
        (token, object): (Token, ObjectIdentifier),
        peer: BACnetAddress,
        to: SubscriberEndpoint,
        (invoke_id, service): (u8, NotifyService),
        params: Vec<u8>,
        (deadline, cutoff): (Instant, Instant),
        retries: u8,
    ) -> Self {
        Self {
            token,
            object: object.encode(),
            peer,
            to,
            invoke_id,
            service,
            params,
            deadline,
            cutoff,
            retries_left: retries,
            resend: false,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ConfirmedTable {
    out: Vec<Outstanding>,
}

impl ConfirmedTable {
    pub(super) fn insert(&mut self, outstanding: Outstanding) {
        self.out.push(outstanding);
    }

    /// The notification `peer` answers under `invoke_id`, taken out.
    pub(super) fn answered(&mut self, peer: &BACnetAddress, invoke_id: u8) -> Option<Outstanding> {
        let at = self
            .out
            .iter()
            .position(|out| out.invoke_id == invoke_id && out.peer == *peer)?;
        Some(self.out.swap_remove(at))
    }

    /// The notification `token` names, taken out.
    pub(super) fn take(&mut self, token: Token) -> Option<Outstanding> {
        let at = self.out.iter().position(|out| out.token == token)?;
        Some(self.out.swap_remove(at))
    }

    /// A notification reporting `object`, taken out.
    pub(super) fn take_object(&mut self, object: ObjectIdentifier) -> Option<Outstanding> {
        let object = object.encode();
        let at = self.out.iter().position(|out| out.object == object)?;
        Some(self.out.swap_remove(at))
    }

    /// Whether `token` names a notification out.
    pub(super) fn holds(&self, token: Token) -> bool {
        self.out.iter().any(|out| out.token == token)
    }

    /// Every notification whose answer is overdue at `now`: one past its
    /// cutoff, or with no retries left, is taken out and returned as
    /// failed; one with retries left becomes due to resend.
    pub(super) fn time_out(&mut self, now: Instant) -> Vec<Outstanding> {
        let mut failed = Vec::new();
        let mut at = 0;
        while at < self.out.len() {
            let out = &mut self.out[at];
            if now >= out.cutoff {
                failed.push(self.out.swap_remove(at));
            } else if out.resend || now < out.deadline {
                at += 1;
            } else if out.retries_left > 0 {
                out.retries_left -= 1;
                out.resend = true;
                at += 1;
            } else {
                failed.push(self.out.swap_remove(at));
            }
        }
        failed
    }

    /// A notification due to be sent again, its next deadline `timeout`
    /// from `now`.
    pub(super) fn resend(&mut self, now: Instant, timeout: Duration) -> Option<&Outstanding> {
        let out = self.out.iter_mut().find(|out| out.resend)?;
        out.resend = false;
        out.deadline = now.saturating_add(timeout);
        Some(out)
    }

    /// The earliest deadline or cutoff still to come; a report due to
    /// resend waits for the host's next `pop`, and for its cutoff.
    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.out
            .iter()
            .map(|out| {
                if out.resend {
                    out.cutoff
                } else {
                    out.deadline.min(out.cutoff)
                }
            })
            .min()
    }

    /// When the notification `token` names times out next, as
    /// [`next_deadline`](Self::next_deadline) reckons each.
    pub(super) fn deadline_of(&self, token: Token) -> Option<Instant> {
        let out = self.out.iter().find(|out| out.token == token)?;
        Some(if out.resend {
            out.cutoff
        } else {
            out.deadline.min(out.cutoff)
        })
    }

    /// Whether a notification waits to be resent.
    pub(super) fn resend_due(&self) -> bool {
        self.out.iter().any(|out| out.resend)
    }
}
