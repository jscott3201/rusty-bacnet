//! Capture this crate's log events on the test's thread, so a test can check
//! the level a message logs at (#1643). Install it on a current-thread
//! runtime, the `#[tokio::test]` default, so the tasks the test spawns log to
//! it too.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Interest};
use tracing::{span, Event, Level, Metadata, Subscriber};

/// One captured event.
#[derive(Clone, Debug)]
pub(crate) struct Captured {
    pub(crate) level: Level,
    /// The event's fields, the message among them, as text.
    pub(crate) fields: BTreeMap<String, String>,
}

/// Every event from this crate logged on the capturing thread.
#[derive(Clone, Default)]
pub(crate) struct LogCapture(Arc<Mutex<Vec<Captured>>>);

impl LogCapture {
    /// Capture until the returned guard drops.
    pub(crate) fn start() -> (Self, DefaultGuard) {
        // As in the SC reconnect tests: a long-lived peer dispatcher keeps a
        // callsite first hit on a thread without this subscriber from caching
        // "never" for every thread.
        static PEER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        PEER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        let capture = Self::default();
        let guard = tracing::subscriber::set_default(capture.clone());
        (capture, guard)
    }

    /// The events whose message starts with `message`, in order.
    pub(crate) fn matching(&self, message: &str) -> Vec<Captured> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                event
                    .fields
                    .get("message")
                    .is_some_and(|text| text.starts_with(message))
            })
            .cloned()
            .collect()
    }

    /// The levels of the events whose message starts with `message`.
    pub(crate) fn levels(&self, message: &str) -> Vec<Level> {
        self.matching(message)
            .iter()
            .map(|event| event.level)
            .collect()
    }

    /// The events at warn or above, whatever their message.
    pub(crate) fn warnings(&self) -> Vec<Captured> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.level <= Level::WARN)
            .cloned()
            .collect()
    }
}

impl Subscriber for LogCapture {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::TRACE)
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target().starts_with("bacnet_transport")
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        struct Fields(BTreeMap<String, String>);
        impl Visit for Fields {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.insert(field.name().into(), value.into());
            }
        }
        let mut fields = Fields(BTreeMap::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(Captured {
            level: *event.metadata().level(),
            fields: fields.0,
        });
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}
