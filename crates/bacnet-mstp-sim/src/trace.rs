//! A bounded, human-readable record of everything that happened.

use std::collections::VecDeque;
use std::fmt;

use bacnet_mstp_core::{Instant, MasterState};

/// Who a trace entry is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// A node, by station address.
    Station(u8),
    /// The noise source.
    Noise,
    /// The shared line as receivers see it.
    Line,
    /// The simulator itself (faults, checks).
    Sim,
}

impl fmt::Display for Who {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Station(station) => format!("st{station:03}"),
            Self::Noise => "noise".to_owned(),
            Self::Line => "line".to_owned(),
            Self::Sim => "sim".to_owned(),
        };
        f.pad(&text)
    }
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceKind {
    /// Receiver-specific observation of a real contended physical window.
    Contention {
        /// Common physical character start; the entry time is its end.
        start: Instant,
        /// Selected actual contributor, if the policy captured one.
        transmission: Option<u64>,
        /// Index in the selected contributor's physical transmission.
        index: Option<usize>,
        /// UART-clean data, or `None` for a framing error.
        byte: Option<u8>,
    },
    /// A physically observed own character became locally available. The
    /// entry timestamp is availability; later profiles cannot overtake older
    /// queued returns, so it may exceed `requested_at` under the FIFO policy.
    LocalEcho {
        /// Original monotonic line transmission identity.
        transmission: u64,
        /// Powered incarnation that observed the character.
        incarnation: u64,
        /// Physical stop time, retained through delayed buffering.
        wire_end: Instant,
        /// Physical stop plus the delay captured at transmit start.
        requested_at: Instant,
        /// Actual observed byte, or `None` for a framing error.
        byte: Option<u8>,
    },
    /// A driver started putting an octet on the wire.
    Octet {
        /// Index of the octet within its transmission.
        index: usize,
        /// The octet as sent.
        byte: u8,
    },
    /// Receivers got a character that was not a clean octet.
    Garbled,
    /// A transmission began; the text summarizes the frame.
    FrameStart(String),
    /// A transmission ended; the text says how.
    FrameEnd(String),
    /// A core action the adapter executed, or an adapter event.
    Action(String),
    /// A master's state changed across one core call.
    State {
        /// State before the call.
        from: MasterState,
        /// State after it.
        to: MasterState,
    },
    /// An injected fault.
    Fault(String),
    /// Anything else worth knowing when reading a failure.
    Note(String),
    /// An invariant failed.
    Violation(String),
}

/// One trace record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEntry {
    /// Virtual time.
    pub at: Instant,
    /// Subject.
    pub who: Who,
    /// Event.
    pub kind: TraceKind,
}

impl fmt::Display for TraceEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:<6} ", Millis(self.at), self.who)?;
        match &self.kind {
            TraceKind::Contention {
                start,
                transmission,
                index,
                byte,
            } => write!(
                f,
                "contention from {} tx{transmission:?}[{index:?}] {byte:?}",
                Millis(*start)
            ),
            TraceKind::LocalEcho {
                transmission,
                incarnation,
                wire_end,
                requested_at,
                byte,
            } => write!(
                f,
                "local echo tx{transmission}/boot{incarnation} wire {} requested {} {byte:?}",
                Millis(*wire_end),
                Millis(*requested_at)
            ),
            TraceKind::Octet { index, byte } => write!(f, "octet[{index}] {byte:02X}"),
            TraceKind::Garbled => f.write_str("garbled character"),
            TraceKind::FrameStart(text) => write!(f, "TX start {text}"),
            TraceKind::FrameEnd(text) => write!(f, "TX end {text}"),
            TraceKind::Action(text) => write!(f, "{text}"),
            TraceKind::State { from, to } => write!(f, "state {from:?} -> {to:?}"),
            TraceKind::Fault(text) => write!(f, "FAULT {text}"),
            TraceKind::Note(text) => write!(f, "note: {text}"),
            TraceKind::Violation(text) => write!(f, "VIOLATION {text}"),
        }
    }
}

/// Formats an instant as milliseconds with microsecond precision.
#[derive(Debug, Clone, Copy)]
pub struct Millis(pub Instant);

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let micros = self.0.as_micros();
        write!(f, "{:>7}.{:03}ms", micros / 1_000, micros % 1_000)
    }
}

/// The most recent trace entries, up to a fixed capacity.
#[derive(Debug, Clone)]
pub struct Trace {
    entries: VecDeque<TraceEntry>,
    capacity: usize,
    dropped: u64,
}

impl Trace {
    /// An empty trace keeping at most `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.min(65_536)),
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Record an entry, dropping the oldest when full.
    pub fn push(&mut self, at: Instant, who: Who, kind: TraceKind) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(TraceEntry { at, who, kind });
    }

    /// The retained entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &TraceEntry> {
        self.entries.iter()
    }

    /// Number of entries dropped to stay within capacity.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The last `count` entries, one per line.
    #[must_use]
    pub fn tail(&self, count: usize) -> String {
        let skip = self.entries.len().saturating_sub(count);
        let mut text = String::new();
        if skip > 0 || self.dropped > 0 {
            text.push_str(&format!(
                "  ... {} earlier entries not shown\n",
                self.dropped + skip as u64
            ));
        }
        for entry in self.entries.iter().skip(skip) {
            text.push_str("  ");
            text.push_str(&entry.to_string());
            text.push('\n');
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{Trace, TraceKind, Who};
    use bacnet_mstp_core::Instant;

    #[test]
    fn keeps_the_newest_entries_and_formats_them() {
        let mut trace = Trace::new(2);
        for index in 0..3 {
            trace.push(
                Instant::from_micros(1_500),
                Who::Station(3),
                TraceKind::Octet { index, byte: 0x55 },
            );
        }
        assert_eq!(trace.dropped(), 1);
        assert_eq!(trace.entries().count(), 2);
        let tail = trace.tail(5);
        assert!(tail.contains("1 earlier entries"), "{tail}");
        assert!(tail.contains("      1.500ms st003  octet[2] 55"), "{tail}");
    }
}
