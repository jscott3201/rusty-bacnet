//! Capture this crate's log events on the test's thread, so a test can check
//! the level a message logs at (#1643). Install it on a current-thread
//! runtime, the `#[tokio::test]` default, so the tasks the test spawns log to
//! it too.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Interest};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// One captured event.
#[derive(Clone, Debug)]
pub(crate) struct Captured {
    /// Tokio time at capture, including a paused test clock.
    pub(crate) at: tokio::time::Instant,
    pub(crate) level: Level,
    /// The event's fields, the message among them, as text.
    pub(crate) fields: BTreeMap<String, String>,
}

/// Every event from this crate logged on the capturing thread.
#[derive(Clone, Default)]
pub(crate) struct LogCapture {
    events: Arc<Mutex<Vec<Captured>>>,
    target: Option<&'static str>,
    max_level: Option<tracing::metadata::LevelFilter>,
}

impl LogCapture {
    /// Capture this crate's events until the returned guard drops.
    pub(crate) fn start() -> (Self, DefaultGuard) {
        Self::default().install()
    }

    /// Capture only one exact target, retaining the caller's level hint.
    pub(crate) fn start_for_target(
        target: &'static str,
        max_level: tracing::metadata::LevelFilter,
    ) -> (Self, DefaultGuard) {
        Self {
            target: Some(target),
            max_level: Some(max_level),
            ..Self::default()
        }
        .install()
    }

    fn install(self) -> (Self, DefaultGuard) {
        // A long-lived peer dispatcher keeps a callsite first hit on a thread
        // without this subscriber from caching "never" for every thread.
        static PEER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        PEER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        let guard = tracing::subscriber::set_default(self.clone());
        (self, guard)
    }

    /// Number of events admitted by this capture's target filter.
    pub(crate) fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// The events whose message starts with `message`, in order.
    pub(crate) fn matching(&self, message: &str) -> Vec<Captured> {
        self.events
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
        self.events
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
        Some(
            self.max_level
                .unwrap_or(tracing::metadata::LevelFilter::TRACE),
        )
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        match self.target {
            Some(target) => metadata.target() == target,
            None => metadata.target().starts_with("bacnet_transport"),
        }
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
        self.events.lock().unwrap().push(Captured {
            at: tokio::time::Instant::now(),
            level: *event.metadata().level(),
            fields: fields.0,
        });
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_probe(attempt: u64) {
        tracing::info!(target: "bacnet_transport::sc::recovery", attempt, reason = "retry", "SC capture probe");
    }

    #[tokio::test(start_paused = true)]
    async fn exact_target_capture_keeps_time_fields_order_and_restores_the_outer_capture() {
        let (outer, _outer_guard) = LogCapture::start();
        tracing::warn!(target: "bacnet_transport::other", "outer warning");
        {
            let (capture, _guard) = LogCapture::start_for_target(
                "bacnet_transport::sc::recovery",
                tracing::metadata::LevelFilter::INFO,
            );
            // First hit this callsite on a thread without either scoped
            // subscriber. It must still be captured on our thread afterward.
            std::thread::spawn(|| recovery_probe(0)).join().unwrap();
            let first = tokio::time::Instant::now();
            recovery_probe(1);
            tracing::warn!(target: "bacnet_transport::sc::recovery_extra", "SC capture probe wrong target");
            tracing::warn!(target: "bacnet_transport::other", "SC capture probe another target");
            tokio::time::advance(std::time::Duration::from_secs(1)).await;
            tokio::spawn(async {
                recovery_probe(2);
            })
            .await
            .unwrap();
            let events = capture.matching("SC capture");
            assert_eq!(events.len(), 2);
            assert_eq!(capture.len(), 2);
            assert_eq!(events[0].at, first);
            assert_eq!(
                events[1].at - events[0].at,
                std::time::Duration::from_secs(1)
            );
            assert_eq!(capture.levels("SC capture"), [Level::INFO, Level::INFO]);
            assert!(capture.warnings().is_empty());
            for (event, attempt) in events.iter().zip(["1", "2"]) {
                assert_eq!(event.fields["attempt"], attempt);
                assert_eq!(event.fields["reason"], "retry");
                assert_eq!(event.fields["message"], "SC capture probe");
            }
        }
        recovery_probe(3);
        assert_eq!(outer.len(), 2);
        assert_eq!(outer.warnings()[0].fields["message"], "outer warning");
        assert_eq!(outer.matching("SC capture")[0].fields["attempt"], "3");
    }
}
