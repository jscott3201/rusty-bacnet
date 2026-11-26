//! How the client takes its network's number from what it hears (#2826).
//!
//! Nothing in a Network-Number-Is shows who sent it, and any station on the
//! link can broadcast one flagged configured. Clause 6.4.20 has a node that
//! was not configured take such an announcement as its number, and the
//! client used to, the moment one came. One frame from any station could
//! then make every row the client held as routed on the named network a
//! local one: requests to those devices went to link addresses where they
//! are not, and a station holding such an address on this link could
//! answer them.
//!
//! The client now takes a number only from the answers to a
//! What-Is-Network-Number of its own (Clause 6.4.19), and only when they
//! agree:
//!
//! - Each question it sends opens a window ([`window_for`]): the client's
//!   APDU timeout or the longest a node that is not a router may hold its
//!   answer ([`ANSWER_DELAY_MAX`]), whichever is longer, and
//!   [`ANSWER_MARGIN`] more. A Network-Number-Is heard in the window is an
//!   answer. A router knows the number of every network it is attached to
//!   (Clause 6.6.1) and has no leave to hold its answer back, so on a link
//!   with a router, the router's answer is among them.
//! - When the window closes, the answers flagged configured decide, or the
//!   learned ones when no answer was flagged configured: one number among
//!   them is applied by Clause 6.4.20's precedence, as every announcement
//!   was before. Two or more decide nothing: the client keeps the number it
//!   holds and counts the window
//!   ([`BACnetClient::network_number_disagreements`](super::BACnetClient::network_number_disagreements)).
//! - A window in which the client may have lost an answer decides nothing
//!   either: one the network layer's control queue or the Number owner's
//!   own queue turned away, full, after the question went. What is lost
//!   before those queues, in the socket's buffer or the link, is not seen.
//! - An announcement heard outside a window that names another number than
//!   the client holds, unknown included, is a cue: the client asks, at once
//!   the first time, then at most once per hold, which starts at a minute
//!   and doubles to ten while cues keep coming, and starts over at a
//!   minute when ten minutes pass after a hold ends with no cue. Only a
//!   question that goes takes the hold. One naming the number the client
//!   holds changes no route and is applied as it comes.
//!
//! An honest site ends where it did, one window later: a router announcing
//! its number at its start draws one question, and the router's answer is
//! taken when the window closes. A station that announces another number
//! now has to answer the question too, and a router on the link answers it
//! with the real number, so the window decides nothing. Left open: a link
//! where no station that knows the number answers the question (no router,
//! or only routers older than the message), on which a station that
//! answers alone still decides it.

use bacnet_network::network_number::ANSWER_DELAY_MAX;
use tokio::time::{Duration, Instant, sleep_until};

/// The hold after the first question a cue calls for.
pub(super) const CUE_HOLD_MIN: Duration = Duration::from_secs(60);

/// The longest hold between two questions that cues call for, and the
/// quiet after which the hold starts over.
pub(super) const CUE_HOLD_MAX: Duration = Duration::from_secs(600);

/// How long a window stays open past the longer of the client's APDU
/// timeout and [`ANSWER_DELAY_MAX`]: room for an answer held to the end of
/// a node's wait to cross a slow link, an MS/TP trunk waiting for its
/// token.
pub(super) const ANSWER_MARGIN: Duration = Duration::from_secs(2);

/// How long the answers to a question count, for a client whose APDU
/// timeout is `apdu_timeout`.
pub(super) fn window_for(apdu_timeout: Duration) -> Duration {
    apdu_timeout.max(ANSWER_DELAY_MAX) + ANSWER_MARGIN
}

/// A question the client sent: when it went, and how many controls the
/// client had lost by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Question {
    /// When it went, or was about to.
    pub(super) at: Instant,
    /// The controls lost before it, against which those lost while its
    /// window is open are told.
    pub(super) losses: u64,
}

/// The numbers heard at one flag in one window: the first, and whether
/// another one came.
#[derive(Debug, Default, Clone, Copy)]
struct Votes {
    number: Option<u16>,
    split: bool,
}

impl Votes {
    fn heard(&mut self, number: u16) {
        match self.number {
            None => self.number = Some(number),
            Some(first) => self.split |= first != number,
        }
    }
}

/// What the answers heard in one window decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    /// Nobody answered.
    Unanswered,
    /// The answers at the deciding flag named one number.
    Agreed {
        /// The number they named.
        number: u16,
        /// One when they were flagged configured.
        flag: u8,
    },
    /// They named more than one.
    Split,
    /// They named one number or none, but an answer may have been lost
    /// while the window was open.
    Lost,
}

/// The answers to the client's question, until the window closes.
#[derive(Debug)]
struct Round {
    closes: Instant,
    /// The losses before the earliest question the window holds answers
    /// to.
    baseline: u64,
    configured: Votes,
    learned: Votes,
}

impl Round {
    /// What the answers decided, with `losses` lost by the close.
    fn decide(&self, losses: u64) -> Decision {
        let (votes, flag) = if self.configured.number.is_some() {
            (self.configured, 1)
        } else {
            (self.learned, 0)
        };
        match votes.number {
            // Two numbers heard disagree, whatever else was lost.
            Some(_) if votes.split => Decision::Split,
            _ if losses > self.baseline => Decision::Lost,
            None => Decision::Unanswered,
            Some(number) => Decision::Agreed { number, flag },
        }
    }
}

/// What a heard Network-Number-Is is to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Heard {
    /// An answer to the client's question, kept until its window closes.
    Answer,
    /// Unasked, naming the number the client holds.
    Same,
    /// Unasked, naming another number: the client asks when `ask` says the
    /// hold allows it, and [`Learning::cue_asked`] takes the hold once the
    /// question went.
    Cue {
        /// Whether to ask now.
        ask: bool,
    },
}

/// The Number owner's learning: the window open, if any, and the hold on
/// questions that cues call for. It lives in the owner's task alone.
#[derive(Debug)]
pub(super) struct Learning {
    window: Duration,
    round: Option<Round>,
    /// When a cue may next ask; `None` until the first cue's question goes.
    next_cue: Option<Instant>,
    hold: Duration,
}

impl Learning {
    /// No window open, and the first cue asks at once. `window` is how
    /// long answers to a question count.
    pub(super) fn new(window: Duration) -> Self {
        Self {
            window,
            round: None,
            next_cue: None,
            hold: CUE_HOLD_MIN,
        }
    }

    /// A Network-Number-Is naming `number` with `flag` was heard at `now`,
    /// while the client holds `ours` (zero when unknown) and its latest
    /// question is `asked`. An answer inside a window is kept; any other is
    /// [`Heard::Same`] or a [`Heard::Cue`].
    pub(super) fn heard(
        &mut self,
        (ours, number, flag): (u16, u16, u8),
        asked: Option<Question>,
        now: Instant,
    ) -> Heard {
        let window = asked
            .map(|asked| (asked.at + self.window, asked.losses))
            .filter(|(closes, _)| now < *closes);
        let open = self.round.as_ref().filter(|round| now < round.closes);
        if window.is_some() || open.is_some() {
            let (closes, losses) = window.unwrap_or((now, u64::MAX));
            let round = self.round.get_or_insert_with(|| Round {
                closes,
                baseline: losses,
                configured: Votes::default(),
                learned: Votes::default(),
            });
            round.closes = round.closes.max(closes);
            round.baseline = round.baseline.min(losses);
            match flag {
                1 => round.configured.heard(number),
                _ => round.learned.heard(number),
            }
            return Heard::Answer;
        }
        if number == ours {
            return Heard::Same;
        }
        Heard::Cue {
            ask: self.cue_may_ask(now),
        }
    }

    /// Whether a cue at `now` may ask. After ten quiet minutes past the
    /// end of the hold, the hold starts over.
    fn cue_may_ask(&mut self, now: Instant) -> bool {
        let Some(next) = self.next_cue else {
            return true;
        };
        if now < next {
            return false;
        }
        if now >= next + CUE_HOLD_MAX {
            self.hold = CUE_HOLD_MIN;
        }
        true
    }

    /// The question a cue at `now` called for went: it takes the hold.
    pub(super) fn cue_asked(&mut self, now: Instant) {
        self.next_cue = Some(now + self.hold);
        self.hold = (self.hold * 2).min(CUE_HOLD_MAX);
    }

    /// Whether the open window has closed by `now`, to be [`Self::close`]d
    /// before anything else is heard.
    pub(super) fn due(&self, now: Instant) -> bool {
        self.round.as_ref().is_some_and(|round| now >= round.closes)
    }

    /// Wait until the open window closes; forever while none is open.
    pub(super) async fn closes(&self) {
        match &self.round {
            Some(round) => sleep_until(round.closes).await,
            None => std::future::pending().await,
        }
    }

    /// The window has closed, with `losses` controls lost by now: what its
    /// answers decided.
    pub(super) fn close(&mut self, losses: u64) -> Decision {
        self.round
            .take()
            .map_or(Decision::Unanswered, |round| round.decide(losses))
    }
}

#[cfg(test)]
#[path = "number_rounds_tests.rs"]
mod tests;
