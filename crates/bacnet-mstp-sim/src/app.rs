//! A simulated master's application: what it does with the NPDUs its core
//! delivers (#1796).
//!
//! The simulator hands every NPDU a master's core delivers to the node's
//! [`Application`]. The application may answer it, after a delay it
//! chooses, and may queue NPDUs of its own (an I-Am broadcast, say) for the
//! node's next token through the [`Outbox`]. A [`ReplyPolicy`] is the
//! fixed application most scenarios use; a callback can run real
//! application code, such as a firmware image's request handler, on the
//! simulated bus.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use bacnet_mstp_core::{Duration, Instant};

/// The broadcast MAC address, for [`Outbox::send`] and
/// [`Received::destination`].
pub const BROADCAST: u8 = 0xFF;

/// An NPDU a master's core delivered to its application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received<'a> {
    /// When the core delivered it, on the line's clock, not the node's
    /// own skewed one (see [`Skew`](crate::Skew)).
    pub at: Instant,
    /// The station that sent it.
    pub source: u8,
    /// The frame's destination: this station, or [`BROADCAST`].
    pub destination: u8,
    /// The NPDU.
    pub npdu: &'a [u8],
    /// Whether it came in a data-expecting-reply frame the core is holding
    /// open for an answer. Only then does an [`Answer`] go out in the reply
    /// window; otherwise it waits in the queue for the next token.
    pub expects_reply: bool,
}

/// How an application answers a delivered NPDU. Its delays run on the
/// line's clock, whatever the node's [`Skew`](crate::Skew).
///
/// For a request whose frame [awaits an answer](Received::expects_reply), an answer
/// the core refuses as too late (it already sent Reply Postponed, or the
/// requester may have stopped waiting) is queued as an ordinary NPDU for
/// the next token, as a real stack would send it. For any other NPDU, the
/// answer is queued to its source after the delay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Answer with `npdu` after `delay`.
    Reply {
        /// Time from the delivery to the answer.
        delay: Duration,
        /// The answering NPDU; it must not expect a reply itself.
        npdu: Vec<u8>,
    },
    /// Decide to postpone after `delay`, then queue `npdu` for the next
    /// token.
    Postpone {
        /// Time from the delivery to the decision.
        delay: Duration,
        /// The answering NPDU, sent later.
        npdu: Vec<u8>,
    },
}

/// NPDUs an application queues at its node, which go out when the node
/// next holds the token.
#[derive(Debug, Default)]
pub struct Outbox {
    pub(crate) sends: Vec<(u8, Vec<u8>)>,
}

impl Outbox {
    /// Queue `npdu` to every station.
    pub fn broadcast(&mut self, npdu: &[u8]) {
        self.send(BROADCAST, npdu);
    }

    /// Queue `npdu` to `destination`.
    pub fn send(&mut self, destination: u8, npdu: &[u8]) {
        self.sends.push((destination, npdu.to_vec()));
    }
}

/// A simulated master's application.
///
/// It sees each NPDU its core delivers, in order, and returns the answer
/// it gives, if any. A request it returns no answer for stays undecided, so
/// the core sends Reply Postponed when its decision deadline passes. Whole
/// frames the core delivers (a Test_Response, a proprietary frame) don't
/// reach it.
///
/// Any `FnMut(&Received, &mut Outbox) -> Option<Answer>` is one.
pub trait Application {
    /// React to one delivered NPDU.
    fn on_npdu(&mut self, received: &Received<'_>, outbox: &mut Outbox) -> Option<Answer>;
}

impl<F> Application for F
where
    F: FnMut(&Received<'_>, &mut Outbox) -> Option<Answer>,
{
    fn on_npdu(&mut self, received: &Received<'_>, outbox: &mut Outbox) -> Option<Answer> {
        self(received, outbox)
    }
}

/// A fixed application: it answers every request the same way and ignores
/// every other NPDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPolicy {
    /// Answer every request with `payload` after `delay`.
    Answer {
        /// Time from the delivery to the answer.
        delay: Duration,
        /// Reply NPDU; it must not expect a reply itself.
        payload: Vec<u8>,
    },
    /// Decide to postpone after `delay`, then queue `payload` for the next
    /// token.
    Postpone {
        /// Time from the delivery to the decision.
        delay: Duration,
        /// Reply NPDU sent later.
        payload: Vec<u8>,
    },
    /// Never decide. The core sends Reply Postponed when its decision
    /// deadline passes.
    Ignore,
}

impl Default for ReplyPolicy {
    fn default() -> Self {
        Self::Answer {
            delay: Duration::from_millis(2),
            payload: vec![0x01, 0x00, 0xAC],
        }
    }
}

impl Application for ReplyPolicy {
    fn on_npdu(&mut self, received: &Received<'_>, _: &mut Outbox) -> Option<Answer> {
        if !received.expects_reply {
            return None;
        }
        match self {
            Self::Answer { delay, payload } => Some(Answer::Reply {
                delay: *delay,
                npdu: payload.clone(),
            }),
            Self::Postpone { delay, payload } => Some(Answer::Postpone {
                delay: *delay,
                npdu: payload.clone(),
            }),
            Self::Ignore => None,
        }
    }
}

/// A master's application, shared by the clones of its spec: the
/// application keeps its state across a power cycle, as a scenario's
/// stand-in for code outside the MS/TP stack.
#[derive(Clone)]
pub(crate) struct Shared(pub(crate) Rc<RefCell<dyn Application>>);

impl Shared {
    pub(crate) fn new(application: impl Application + 'static) -> Self {
        Self(Rc::new(RefCell::new(application)))
    }
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Application")
    }
}
