//! The notification budget of one input: how many COV notifications, and
//! how many octets of them, one change may send.

use super::CovPolicy;

/// What one input (a committed change, an initial notification, a revisit)
/// may still send: [`CovPolicy::max_notifications_per_event`] notifications
/// and [`CovPolicy::max_notification_bytes_per_event`] octets of APDU.
#[derive(Debug)]
pub struct EventBudget {
    max_notifications: usize,
    max_bytes: usize,
    notifications_sent: usize,
    bytes_sent: usize,
}

impl EventBudget {
    /// A full budget under `policy`.
    pub fn new(policy: &CovPolicy) -> Self {
        Self {
            max_notifications: policy.max_notifications_per_event,
            max_bytes: policy.max_notification_bytes_per_event,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    /// A full budget of `max_notifications` and `max_bytes`.
    pub fn with_limits(max_notifications: usize, max_bytes: usize) -> Self {
        Self {
            max_notifications,
            max_bytes,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    /// Notifications left.
    pub fn remaining_notifications(&self) -> usize {
        self.max_notifications
            .saturating_sub(self.notifications_sent)
    }

    /// Octets left.
    pub fn remaining_bytes(&self) -> usize {
        self.max_bytes.saturating_sub(self.bytes_sent)
    }

    /// Whether nothing more may be sent.
    pub fn is_exhausted(&self) -> bool {
        self.notifications_sent >= self.max_notifications || self.bytes_sent >= self.max_bytes
    }

    /// Spend one notification of `bytes` octets, if both fit.
    pub fn try_consume(&mut self, bytes: usize) -> bool {
        if self.notifications_sent >= self.max_notifications {
            return false;
        }
        if self.bytes_sent.saturating_add(bytes) > self.max_bytes {
            return false;
        }
        self.notifications_sent += 1;
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
        true
    }

    /// Give back one notification of `bytes` octets that wasn't sent.
    pub fn refund(&mut self, bytes: usize) {
        self.notifications_sent = self.notifications_sent.saturating_sub(1);
        self.bytes_sent = self.bytes_sent.saturating_sub(bytes);
    }

    /// Spend what `sub_budget`, a share of this budget, spent.
    pub fn consume_sub_budget(&mut self, sub_budget: &EventBudget) {
        self.notifications_sent = self
            .notifications_sent
            .saturating_add(sub_budget.notifications_sent);
        self.bytes_sent = self.bytes_sent.saturating_add(sub_budget.bytes_sent);
    }
}
