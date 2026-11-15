//! When a node that is not a router answers a What-Is-Network-Number.
//!
//! Clause 6.4.19 has every device that knows its network's number answer
//! the question with a broadcast Network-Number-Is. On a segment with many
//! such nodes one broadcast question would draw one broadcast from each,
//! so the clause gives a node that is not a router two options for a
//! question that came as a broadcast: hold the answer for up to ten
//! seconds, and skip it when another device answers in that time. The
//! client, the server and the endpoint take both (#2597). A router answers
//! at once and never comes here.
//!
//! - A broadcast question starts one timer, [`ANSWER_DELAY_MIN`] to
//!   [`ANSWER_DELAY_MAX`] ahead, drawn at random so the nodes of a segment
//!   spread out. A second question while it runs changes nothing: one
//!   answer serves both.
//! - A question sent to the node alone is answered at once. The answer is
//!   a broadcast all the same, so it also settles a pending one.
//! - A Network-Number-Is heard while the timer runs drops the pending
//!   answer when it tells the segment as much as this node's would: the
//!   same number, and flagged configured if this node's would be. The
//!   clause allows dropping it for any answer heard; this narrower rule
//!   keeps a configured node answering after a learned one, so the asker
//!   can hold the number as configured (Clause 6.4.20), and keeps a node
//!   whose number differs answering, so the asker hears both.
//! - The owner applies a heard answer to its number first, by the rules of
//!   Clause 6.4.20 it always followed, and compares after: a node that
//!   takes the heard number over is then in agreement and stays quiet.
//!
//! The timer is a deadline on Tokio's clock, awaited by the task that owns
//! the node's Number controls; nothing is spawned, so stopping that task
//! drops the pending answer with it.
use bacnet_types::network_number::NetworkNumber;
use tokio::time::{Duration, Instant, sleep_until};

/// The shortest wait before answering a broadcast question. A router on
/// the segment answers at once, and this gives its answer time to arrive
/// and settle the question first.
pub const ANSWER_DELAY_MIN: Duration = Duration::from_millis(500);

/// The longest wait before answering a broadcast question: all that
/// Clause 6.4.19 allows.
pub const ANSWER_DELAY_MAX: Duration = Duration::from_secs(10);

/// One node's answer to What-Is-Network-Number: none pending, or one due at
/// a deadline. There is never more than one. Hidden: it is the stack's own
/// owners' machinery, shared across its crates, not an application API.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct DelayedAnswer {
    due: Option<Instant>,
}

impl DelayedAnswer {
    /// No answer pending.
    pub const fn new() -> Self {
        Self { due: None }
    }

    /// A question arrived while this node holds `ours`. `true` when the
    /// answer goes now: the question came to this node alone. A broadcast
    /// one starts the timer unless it runs already. A node that doesn't
    /// know its number has no answer, now or later.
    pub fn asked(&mut self, ours: NetworkNumber, broadcast: bool) -> bool {
        if ours.snapshot().0 == 0 {
            return false;
        }
        if broadcast {
            self.due
                .get_or_insert_with(|| Instant::now() + answer_delay());
            false
        } else {
            // The answer about to go is a broadcast, and it is the one a
            // pending answer would be.
            self.due = None;
            true
        }
    }

    /// A Network-Number-Is with `number` and `flag` was heard, and `ours`
    /// is this node's number after applying it. Drops the pending answer
    /// when the heard one says as much: the same number, and the configured
    /// flag if this node's answer would carry it.
    pub fn heard(&mut self, ours: NetworkNumber, number: u16, flag: u8) {
        if says_as_much(ours, number, flag) {
            self.due = None;
        }
    }

    /// Wait for the pending answer's deadline; forever when none is
    /// pending. The answer stays pending until [`Self::taken`], so a
    /// caller that is dropped after the deadline, before it has sent,
    /// finds it due again.
    pub async fn due(&self) {
        match self.due {
            Some(at) => sleep_until(at).await,
            None => std::future::pending().await,
        }
    }

    /// The caller is sending the answer that came due.
    pub fn taken(&mut self) {
        self.due = None;
    }
}

/// Whether a heard Network-Number-Is with `number` and `flag` tells the
/// segment as much as the answer a node holding `ours` would send: the same
/// number, and the configured flag when that answer would carry it. Only a
/// node configured with its number sends the flag (Clause 6.4.20); one that
/// learned it, from a configured announcement or not, sends it clear.
fn says_as_much(ours: NetworkNumber, number: u16, flag: u8) -> bool {
    let (our_number, quality) = ours.snapshot();
    let our_flag = u8::from(quality == 3);
    our_number != 0 && number == our_number && flag >= our_flag
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static QUEUED_WAITS: std::cell::RefCell<std::collections::VecDeque<Duration>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Give the next broadcast questions taken in on this thread these waits,
/// in order, in place of random ones; once they are used up the waits are
/// random again. For a test of several nodes on one current-thread
/// runtime, where which node answers first has to be known: hand each node
/// its question in turn, so the waits go to the nodes in that order.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn queue_answer_waits(waits: impl IntoIterator<Item = Duration>) {
    QUEUED_WAITS.with(|queued| queued.borrow_mut().extend(waits));
}

/// A wait drawn from the OS's random source. Without one the node waits
/// the longest, which leaves the answer to every other node first.
fn answer_delay() -> Duration {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(wait) = QUEUED_WAITS.with(|queued| queued.borrow_mut().pop_front()) {
        return wait;
    }
    let mut random = [0u8; 4];
    match getrandom::fill(&mut random) {
        Ok(()) => delay_from(u32::from_ne_bytes(random)),
        Err(_) => ANSWER_DELAY_MAX,
    }
}

/// Map a random word onto [`ANSWER_DELAY_MIN`]..=[`ANSWER_DELAY_MAX`], in
/// milliseconds.
fn delay_from(random: u32) -> Duration {
    let spread = (ANSWER_DELAY_MAX - ANSWER_DELAY_MIN).as_millis() as u32;
    ANSWER_DELAY_MIN + Duration::from_millis(u64::from(random % (spread + 1)))
}

#[cfg(test)]
mod tests;
