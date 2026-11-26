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
//! What-Is-Network-Number of its own (Clause 6.4.19):
//!
//! - Each question it sends opens a window ([`window_for`]): the client's
//!   APDU timeout or the longest a node that is not a router may hold its
//!   answer ([`ANSWER_DELAY_MAX`]), whichever is longer, and
//!   [`ANSWER_MARGIN`] more. A Network-Number-Is heard in the window is an
//!   answer. A router knows the number of every network it is attached to
//!   (Clause 6.6.1) and may not hold its answer back, so on a link with a
//!   router that answers the message, its answer comes in the window unless
//!   it is late or lost.
//! - When the window closes, a number is taken only when every answer in it
//!   named that number: two numbers decide nothing, whatever their flags.
//!   That is stricter than Clause 6.4.20, under which a configured
//!   announcement replaces a learned number (maintainer decision,
//!   2026-11-26). The flags only set the quality of a number all the
//!   answers agree on: configured when any was flagged so, by the clause's
//!   precedence, which also keeps a learned answer from replacing a number
//!   taken as configured.
//! - A window in which the client may have lost an answer decides nothing
//!   either: one the network layer's control queue or the Number owner's
//!   own queue turned away, full, after the question went. What is lost
//!   before those queues, in the socket's buffer or on the link, is not
//!   seen.
//! - A number the client reaches through a router (it holds routed rows on
//!   it, or a routed path to it) is taken only after the router check
//!   (`number_check`): no router on this link may answer that it routes
//!   there.
//! - A window that decides nothing for two numbers or a possibly lost
//!   answer, or that the router check refuses, keeps what the client held
//!   and counts in
//!   [`BACnetClient::network_number_disagreements`](super::BACnetClient::network_number_disagreements).
//!   A window whose question may not have gone (its send failed or was cut
//!   off after an answer came) decides nothing and isn't counted.
//! - An answer is judged by when it came, not when the Number owner took
//!   it from its queue, and a window doesn't close while controls that
//!   may hold answers wait there.
//! - A window lasts at most [`LONGEST_WINDOW`], however long the APDU
//!   timeout.
//! - An announcement heard outside a window that names another number than
//!   the client holds, unknown included, is a cue: the client asks, at once
//!   the first time, then at most once per hold, which starts at a minute
//!   and doubles to ten while cues keep coming, and starts over at a
//!   minute when ten minutes pass after a hold ends with no cue. Only a
//!   question that goes takes the hold. One naming the number the client
//!   holds changes no route and is applied as it comes.
//!
//! An honest site ends where it did, one window later, and one router check
//! later still when the client reaches the number through a router (a
//! router relaying local I-Ams back, #2572): a router announcing its number
//! at its start draws one question, and the router's answer is taken when
//! the window closes. A station that announces another number now has to
//! answer the question too; a router on the link that answers it names the
//! real number, and one that routes to the claimed number refuses it in the
//! check. Left open, as a class: a link where no station that knows the
//! number answers the question and no router routes to the claimed
//! network (no router, or only routers older than the message), on which a
//! station that answers alone still decides it; and an answer lost before
//! the queues, which leaves a station's lone answer standing.
//!
//! This is the standalone client's rule. A client on an endpoint's seat
//! holds its node's number, and the node, like the full server, still
//! takes any announcement by Clause 6.4.20; a registered Network Port with
//! a configured number takes none.

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

/// The longest a window stays open: a day. A longer APDU timeout, the
/// longest a configuration holds included, opens a window this long, so
/// the time it closes never overflows the clock.
pub(super) const LONGEST_WINDOW: Duration = Duration::from_secs(86_400);

/// How long the answers to a question count, for a client whose APDU
/// timeout is `apdu_timeout`.
pub(super) fn window_for(apdu_timeout: Duration) -> Duration {
    apdu_timeout
        .max(ANSWER_DELAY_MAX)
        .saturating_add(ANSWER_MARGIN)
        .min(LONGEST_WINDOW)
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

/// The numbers heard in one window: the first, and whether another one
/// came.
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
    /// The answers all named one number.
    Agreed {
        /// The number they named.
        number: u16,
        /// One when any of them was flagged configured.
        flag: u8,
    },
    /// They named more than one.
    Split,
    /// They named one number or none, but an answer may have been lost
    /// while the window was open.
    Lost,
    /// The question the window was open for may never have gone: its send
    /// failed or was cut off after an answer had come, and the note of it
    /// went back. What came may have been no answer at all.
    Void,
}

/// The answers to the client's question, until the window closes.
#[derive(Debug)]
struct Round {
    closes: Instant,
    /// The losses before the earliest question the window holds answers
    /// to.
    baseline: u64,
    /// The numbers heard, whatever their flags.
    votes: Votes,
    /// Whether any answer was flagged configured.
    configured: bool,
}

impl Round {
    /// What the answers decided, with `losses` lost by the close. Two
    /// numbers decide nothing whatever their flags (maintainer decision,
    /// 2026-11-26), where Clause 6.4.20 would let a configured one win; the
    /// flags only say which quality an agreed number takes.
    fn decide(&self, losses: u64) -> Decision {
        match self.votes.number {
            // Two numbers heard disagree, whatever else was lost.
            Some(_) if self.votes.split => Decision::Split,
            _ if losses > self.baseline => Decision::Lost,
            None => Decision::Unanswered,
            Some(number) => Decision::Agreed {
                number,
                flag: u8::from(self.configured),
            },
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
    /// long answers to a question count, at most [`LONGEST_WINDOW`].
    pub(super) fn new(window: Duration) -> Self {
        Self {
            window: window.min(LONGEST_WINDOW),
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
                votes: Votes::default(),
                configured: false,
            });
            round.closes = round.closes.max(closes);
            round.baseline = round.baseline.min(losses);
            round.votes.heard(number);
            round.configured |= flag == 1;
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

    /// The window has closed, with `losses` controls lost by now and the
    /// client's latest question `asked`: what its answers decided. A window
    /// no noted question still covers, its question's note gone back, is
    /// [`Decision::Void`].
    pub(super) fn close(&mut self, losses: u64, asked: Option<Question>) -> Decision {
        let Some(round) = self.round.take() else {
            return Decision::Unanswered;
        };
        let covered = asked.is_some_and(|asked| asked.at + self.window >= round.closes);
        if !covered {
            return Decision::Void;
        }
        round.decide(losses)
    }
}

#[cfg(test)]
#[path = "number_rounds_tests.rs"]
mod tests;
