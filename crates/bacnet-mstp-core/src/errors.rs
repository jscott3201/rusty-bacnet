//! The refusals of the two calls that can refuse: a transmit start and a
//! request's answer. Construction refuses with
//! [`ConfigError`](crate::ConfigError); every other input is infallible.

use core::fmt;

/// Why [`MasterCore::on_transmit_started`](crate::MasterCore::on_transmit_started)
/// refused a start. The adapter sends nothing for that id; only
/// [`Self::NotPolled`] leaves it live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransmitError {
    /// The id is not the active transmit's: stale, never handed over, or a
    /// frame the core took back before its start was confirmed.
    Stale,
    /// The line was active within T_turnaround of the start, so the frame
    /// was taken back to wait for a quiet line again (Clause 9.5.5.1). It
    /// comes back as a new [`Action::Transmit`](crate::Action::Transmit)
    /// unless its waiting rules drop it.
    Deferred,
    /// The answer would start once the asker may have stopped waiting, so
    /// it was dropped and its request left unanswered.
    Expired,
    /// The start was already confirmed. An adapter bug, counted in
    /// [`MstpCounters::adapter_errors`](crate::MstpCounters::adapter_errors).
    AlreadyStarted,
    /// The [`Action::Transmit`](crate::Action::Transmit) for this id has not
    /// been polled yet. An adapter bug, counted in
    /// [`MstpCounters::adapter_errors`](crate::MstpCounters::adapter_errors);
    /// unlike the others it leaves the id live: poll the action, then start.
    NotPolled,
}

impl fmt::Display for TransmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stale => "the transmit id is not the active transmit's",
            Self::Deferred => "the line was active within T_turnaround; the frame waits again",
            Self::Expired => "too late to answer; the answer was dropped",
            Self::AlreadyStarted => "the transmit was already started",
            Self::NotPolled => "the transmit has not been polled yet",
        })
    }
}

impl core::error::Error for TransmitError {}

/// Why [`MasterCore::complete_request`](crate::MasterCore::complete_request)
/// refused a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestError {
    /// No request is waiting for an answer.
    NoPending,
    /// The id is not the waiting request's.
    Stale,
    /// Too late for this decision. Either a data answer came after the
    /// decision deadline (the core still owns the request and sends Reply
    /// Postponed), or the requester may have stopped waiting, so the
    /// request was released unanswered.
    Expired,
    /// The answer is longer than this station sends: 501 octets, or 1497
    /// with [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
    /// on.
    Oversize {
        /// The answer's length.
        length: usize,
        /// The longest answer this station sends.
        maximum: usize,
    },
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPending => f.write_str("no request is waiting for an answer"),
            Self::Stale => f.write_str("the request id is not the waiting request's"),
            Self::Expired => f.write_str("too late to answer the request"),
            Self::Oversize { length, maximum } => write!(
                f,
                "a reply of {length} octets is over the {maximum} this station sends"
            ),
        }
    }
}

impl core::error::Error for RequestError {}
