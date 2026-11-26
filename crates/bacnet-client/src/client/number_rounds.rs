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
//! - Each question it sends opens a window of one APDU timeout, the time
//!   the client gives any station to answer it. A Network-Number-Is heard
//!   in the window is an answer. A router knows the number of every network
//!   it is attached to (Clause 6.6.1) and has no leave to hold its answer
//!   back, so on a link with a router, the router's answer is among them.
//! - When the window closes, the answers flagged configured decide, or the
//!   learned ones when no answer was flagged configured: one number among
//!   them is applied by Clause 6.4.20's precedence, as every announcement
//!   was before. Two or more decide nothing: the client keeps the number it
//!   holds and counts the window
//!   ([`BACnetClient::network_number_disagreements`](super::BACnetClient::network_number_disagreements)).
//! - An announcement heard outside a window that names another number than
//!   the client holds, unknown included, is a cue: the client asks, at once
//!   the first time, then at most once per hold, which starts at a minute
//!   and doubles to ten while cues keep coming, and starts over at a
//!   minute when ten minutes pass after a hold ends with no cue. One naming
//!   the number the client holds changes no route and is applied as it
//!   comes.
//!
//! An honest site ends where it did, one window later: a router announcing
//! its number at its start draws one question, and the router's answer is
//! taken when the window closes. A station that announces another number
//! now has to answer the question too, and a router on the link answers it
//! with the real number, so the window decides nothing. Left open: a link
//! where no station that knows the number answers the question (no router,
//! or only routers older than the message), on which a station that
//! answers alone still decides it.

use tokio::time::{Duration, Instant, sleep_until};

/// The hold after the first question a cue calls for.
pub(super) const CUE_HOLD_MIN: Duration = Duration::from_secs(60);

/// The longest hold between two questions that cues call for, and the
/// quiet after which the hold starts over.
pub(super) const CUE_HOLD_MAX: Duration = Duration::from_secs(600);

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
}

/// The answers to the client's question, until the window closes.
#[derive(Debug)]
struct Round {
    closes: Instant,
    configured: Votes,
    learned: Votes,
}

impl Round {
    fn decide(&self) -> Decision {
        let (votes, flag) = if self.configured.number.is_some() {
            (self.configured, 1)
        } else {
            (self.learned, 0)
        };
        match votes.number {
            None => Decision::Unanswered,
            Some(_) if votes.split => Decision::Split,
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
    /// hold allows it.
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
    /// When a cue may next ask; `None` until the first cue asks.
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
    /// question went at `asked`. An answer inside a window is kept; any
    /// other is [`Heard::Same`] or a [`Heard::Cue`].
    pub(super) fn heard(
        &mut self,
        (ours, number, flag): (u16, u16, u8),
        asked: Option<Instant>,
        now: Instant,
    ) -> Heard {
        let window = asked
            .map(|asked| asked + self.window)
            .filter(|closes| now < *closes);
        let open = self.round.as_ref().filter(|round| now < round.closes);
        if window.is_some() || open.is_some() {
            let round = self.round.get_or_insert_with(|| Round {
                closes: now,
                configured: Votes::default(),
                learned: Votes::default(),
            });
            round.closes = round.closes.max(window.unwrap_or(now));
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
            ask: self.cue_asks(now),
        }
    }

    /// Whether a cue at `now` asks, taking the hold if it does.
    fn cue_asks(&mut self, now: Instant) -> bool {
        if let Some(next) = self.next_cue {
            if now < next {
                return false;
            }
            if now >= next + CUE_HOLD_MAX {
                self.hold = CUE_HOLD_MIN;
            }
        }
        self.next_cue = Some(now + self.hold);
        self.hold = (self.hold * 2).min(CUE_HOLD_MAX);
        true
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

    /// The window has closed: what its answers decided.
    pub(super) fn close(&mut self) -> Decision {
        self.round
            .take()
            .map_or(Decision::Unanswered, |round| round.decide())
    }
}

#[cfg(test)]
#[path = "number_rounds_tests.rs"]
mod tests;
