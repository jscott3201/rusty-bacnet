//! What a [`Node`](super::Node) counts, drained by
//! [`Node::take_counters`](super::Node::take_counters) into the host's own
//! totals.

use core::net::SocketAddrV4;

use bacnet_bip_codec::BvlcResultCode;

use super::exchange::Operation;
use super::registration::AttemptOutcome;

/// One operation's exchanges since the counters were last taken, manual
/// and automatic alike. Counts saturate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExchangeCounters {
    /// Requests the caller reported sent.
    pub sent: u32,
    /// Matched, accepted table ACKs.
    pub acknowledgements: u32,
    /// Matched, well-formed BVLC-Results: success or the operation's NAK.
    pub results: u32,
    /// The newest of those results' codes.
    pub last_result: Option<BvlcResultCode>,
    /// Sent requests whose response timeout passed unanswered.
    pub timeouts: u32,
    /// Requests refused because their exchange's slot was taken.
    pub busy: u32,
    /// Requests whose send the caller reported failed.
    pub local_errors: u32,
    /// Malformed responses from the peer of a live exchange of this
    /// operation, counted against the first such exchange in slot order.
    pub malformed_responses: u32,
}

/// A Register-Foreign-Device attempt, manual or automatic, as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegisterAttempt {
    /// The BBMD it went to.
    pub bbmd: SocketAddrV4,
    /// The time-to-live it asked for.
    pub ttl: u16,
    /// What it came to.
    pub outcome: AttemptOutcome,
    /// The result code that answered it, when one did.
    pub result: Option<BvlcResultCode>,
}

/// Everything a node counts, since the counters were last taken. Counts
/// saturate; the `last_` fields hold the newest value seen since, if any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NodeCounters {
    /// Read-Broadcast-Distribution-Table exchanges.
    pub read_bdt: ExchangeCounters,
    /// Write-Broadcast-Distribution-Table exchanges.
    pub write_bdt: ExchangeCounters,
    /// Read-Foreign-Device-Table exchanges.
    pub read_fdt: ExchangeCounters,
    /// Delete-Foreign-Device-Table-Entry exchanges.
    pub delete_fdt_entry: ExchangeCounters,
    /// Register-Foreign-Device exchanges.
    pub register_foreign_device: ExchangeCounters,
    /// Register-Foreign-Device attempts admitted, manual and automatic.
    pub registration_attempts: u32,
    /// Of those, answered with success.
    pub registrations_accepted: u32,
    /// Of those, answered with the Register-Foreign-Device NAK.
    pub registrations_refused: u32,
    /// The newest attempt, whenever it changed.
    pub last_registration_attempt: Option<RegisterAttempt>,
    /// Well-formed responses that matched no live exchange.
    pub unmatched_responses: u32,
    /// Of those, the ones within one response timeout of an exchange they
    /// fit ending.
    pub late_responses: u32,
    /// The newest unmatched BVLC-Result's code.
    pub last_unmatched_result: Option<BvlcResultCode>,
    /// Malformed responses, matched or not: refused by the codec or by
    /// [`Policy::accept_table`](super::Policy::accept_table).
    pub malformed_responses: u32,
    /// Distribute-Broadcast-To-Network NAKs from the foreign device's own
    /// BBMD, taken as a sign it lost the registration: with
    /// [`Retry::Backoff`](super::Retry::Backoff) only.
    pub renewal_hints: u32,
    /// Leases that ended with nothing newer proven.
    pub lapses: u32,
    /// Broadcasts held for the first registration attempt.
    pub broadcasts_held: u32,
    /// Held broadcasts released when that attempt succeeded or went
    /// unanswered.
    pub broadcasts_released: u32,
    /// Held broadcasts discarded when the BBMD refused that attempt.
    pub broadcasts_discarded: u32,
    /// Broadcasts refused because the hold was full.
    pub broadcasts_hold_full: u32,
    /// Broadcasts sent while no lease was live.
    pub broadcasts_unregistered: u32,
}

impl NodeCounters {
    /// The counters of `operation`'s exchanges.
    #[must_use]
    pub const fn exchange(&self, operation: Operation) -> &ExchangeCounters {
        match operation {
            Operation::ReadBdt => &self.read_bdt,
            Operation::WriteBdt => &self.write_bdt,
            Operation::ReadFdt => &self.read_fdt,
            Operation::DeleteFdtEntry => &self.delete_fdt_entry,
            Operation::RegisterForeignDevice => &self.register_foreign_device,
        }
    }

    pub(crate) fn exchange_mut(&mut self, operation: Operation) -> &mut ExchangeCounters {
        match operation {
            Operation::ReadBdt => &mut self.read_bdt,
            Operation::WriteBdt => &mut self.write_bdt,
            Operation::ReadFdt => &mut self.read_fdt,
            Operation::DeleteFdtEntry => &mut self.delete_fdt_entry,
            Operation::RegisterForeignDevice => &mut self.register_foreign_device,
        }
    }
}

/// Add one, saturating.
pub(crate) fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

/// Add `count`, saturating.
pub(crate) fn add(counter: &mut u32, count: u16) {
    *counter = counter.saturating_add(u32::from(count));
}
