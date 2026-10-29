//! A tracing layer that keeps recent log lines in memory for the log pane.
//!
//! While the terminal is in raw mode nothing may write to stdout or stderr, so
//! in TUI mode tracing goes here (and optionally to `--log-file`) instead of
//! the usual fmt subscriber.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Lines kept for the log pane.
pub(crate) const LOG_CAPACITY: usize = 1_000;

/// Longest message kept, in characters.
const MAX_LINE_CHARS: usize = 400;

/// One log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogLine {
    /// UTC wall-clock time, `HH:MM:SS`.
    pub(crate) time: String,
    /// Severity.
    pub(crate) level: Level,
    /// Message and fields, on one line with control characters removed.
    pub(crate) text: String,
}

struct Ring {
    lines: VecDeque<LogLine>,
    capacity: usize,
}

/// A bounded, shared ring of log lines.
///
/// Cloning shares the ring. The generation counter changes on every push, so
/// the UI can tell whether the log pane needs a redraw without locking.
#[derive(Clone)]
pub(crate) struct LogRing {
    ring: Arc<Mutex<Ring>>,
    generation: Arc<AtomicU64>,
}

impl LogRing {
    /// An empty ring that keeps at most `capacity` lines.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            ring: Arc::new(Mutex::new(Ring {
                lines: VecDeque::with_capacity(capacity.min(64)),
                capacity: capacity.max(1),
            })),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Ring> {
        // A panic while holding the lock leaves only a complete line or none.
        self.ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Append a line, dropping the oldest when full.
    pub(crate) fn push(&self, line: LogLine) {
        {
            let mut ring = self.lock();
            if ring.lines.len() == ring.capacity {
                ring.lines.pop_front();
            }
            ring.lines.push_back(line);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Append an info line stamped with the current time.
    pub(crate) fn info(&self, text: impl Into<String>) {
        self.push(LogLine {
            time: utc_hms(SystemTime::now()),
            level: Level::INFO,
            text: sanitize(&text.into()),
        });
    }

    /// Changes on every push.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Number of lines held.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().lines.len()
    }

    /// The newest `n` lines, oldest first.
    pub(crate) fn tail(&self, n: usize) -> Vec<LogLine> {
        let ring = self.lock();
        let skip = ring.lines.len().saturating_sub(n);
        ring.lines.iter().skip(skip).cloned().collect()
    }
}

/// The tracing layer that feeds a [`LogRing`].
pub(crate) struct RingLayer {
    ring: LogRing,
}

impl RingLayer {
    /// A layer writing into `ring`.
    pub(crate) fn new(ring: LogRing) -> Self {
        Self { ring }
    }
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = LineVisitor::default();
        event.record(&mut visitor);
        let mut text = visitor.message;
        if !visitor.fields.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&visitor.fields);
        }
        self.ring.push(LogLine {
            time: utc_hms(SystemTime::now()),
            level: *event.metadata().level(),
            text: sanitize(&text),
        });
    }
}

#[derive(Default)]
struct LineVisitor {
    message: String,
    fields: String,
}

impl Visit for LineVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.push_field(field, format_args!("{value}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            self.push_field(field, format_args!("{value:?}"));
        }
    }
}

impl LineVisitor {
    fn push_field(&mut self, field: &Field, value: std::fmt::Arguments<'_>) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{}={value}", field.name());
    }
}

/// One line, no control characters or line and paragraph separators (they
/// would move the cursor or break the line) and no invisible format
/// characters (they would reorder or hide what follows),
/// bounded.
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_LINE_CHARS));
    let shown = text
        .chars()
        .filter(|ch| !bacnet_types::text::is_hidden_on_screen(*ch));
    for (count, ch) in shown.enumerate() {
        if count == MAX_LINE_CHARS {
            out.push('…');
            break;
        }
        out.push(if bacnet_types::text::breaks_line(ch) {
            ' '
        } else {
            ch
        });
    }
    out
}

/// `HH:MM:SS` in UTC; the standard library has no local time zone.
fn utc_hms(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn ring_is_bounded_and_keeps_the_newest_lines() {
        let ring = LogRing::new(3);
        let before = ring.generation();
        for i in 0..5 {
            ring.info(format!("line {i}"));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.generation(), before + 5);
        let texts: Vec<_> = ring.tail(10).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, ["line 2", "line 3", "line 4"]);
        assert_eq!(ring.tail(1)[0].text, "line 4");
    }

    #[test]
    fn layer_records_message_fields_and_level_on_one_line() {
        let ring = LogRing::new(10);
        let subscriber = tracing_subscriber::registry().with(RingLayer::new(ring.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(device = 101, "late reply\nfrom peer");
        });
        let lines = ring.tail(10);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].level, Level::WARN);
        assert_eq!(lines[0].text, "late reply from peer device=101");
    }

    #[test]
    fn long_lines_are_truncated_and_times_are_utc() {
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        assert_eq!(sanitize(&long).chars().count(), MAX_LINE_CHARS + 1);
        // A right-to-left override or an isolate is dropped (#2122).
        assert_eq!(sanitize("High\u{202E} \u{2066}1\u{2069}"), "High 1");
        // Line and paragraph separators become spaces (#2148).
        assert_eq!(sanitize("a\u{2028}b\u{2029}c"), "a b c");
        let t = UNIX_EPOCH + Duration::from_secs(86_400 * 3 + 3_723);
        assert_eq!(utc_hms(t), "01:02:03");
    }
}
