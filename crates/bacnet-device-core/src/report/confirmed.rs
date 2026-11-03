//! The confirmed table: each confirmed notification handed out and not yet
//! answered, with its invoke ID, peer, deadline and retries left
//! (Clause 5.4.4).
//!
//! A notification [`pop`](super::Reporter::pop) hands out waits for its
//! answer until its deadline, one APDU timeout away. An answer from its peer
//! with its invoke ID ends it. At the deadline it is due to be sent again
//! while retries are left, and `pop` resends it under the same invoke ID;
//! with none left it has failed. A report the COV table fenced, or the host
//! couldn't send, leaves the table at once.

use alloc::vec::Vec;

use bacnet_time::{Duration, Instant};
use bacnet_types::constructed::BACnetAddress;

use crate::cov::{SubscriberEndpoint, Token};

/// One confirmed notification out.
#[derive(Debug)]
pub(super) struct Outstanding {
    pub(super) token: Token,
    /// Who answers it, and whose invoke ID it holds.
    pub(super) peer: BACnetAddress,
    pub(super) to: SubscriberEndpoint,
    pub(super) invoke_id: u8,
    /// Its service parameters, for a resend.
    pub(super) params: Vec<u8>,
    /// When the current attempt's answer is overdue.
    deadline: Instant,
    retries_left: u8,
    /// Its deadline passed with retries left: the next `pop` resends it.
    resend: bool,
}

impl Outstanding {
    pub(super) fn new(
        token: Token,
        peer: BACnetAddress,
        to: SubscriberEndpoint,
        invoke_id: u8,
        params: Vec<u8>,
        deadline: Instant,
        retries: u8,
    ) -> Self {
        Self {
            token,
            peer,
            to,
            invoke_id,
            params,
            deadline,
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

    /// Whether `token` names a notification out.
    pub(super) fn holds(&self, token: Token) -> bool {
        self.out.iter().any(|out| out.token == token)
    }

    /// Every notification whose answer is overdue at `now`: one with retries
    /// left becomes due to resend; one without is taken out and returned.
    pub(super) fn time_out(&mut self, now: Instant) -> Vec<Outstanding> {
        let mut failed = Vec::new();
        let mut at = 0;
        while at < self.out.len() {
            let out = &mut self.out[at];
            if out.resend || now < out.deadline {
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

    /// The earliest deadline still to come, or `None` when one is due to
    /// resend now (the host's next `pop` sends it) or nothing is out.
    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.out
            .iter()
            .filter(|out| !out.resend)
            .map(|out| out.deadline)
            .min()
    }

    /// Whether a notification waits to be resent.
    pub(super) fn resend_due(&self) -> bool {
        self.out.iter().any(|out| out.resend)
    }
}
