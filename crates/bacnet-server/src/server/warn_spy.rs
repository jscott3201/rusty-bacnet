//! Records this crate's warnings on the test's thread, so a test can show a
//! code path was reached when nothing else observes it, or count how often
//! a throttled warning went out. Install it on a current-thread runtime, the
//! `#[tokio::test]` default, so the tasks the test spawns log to it too.

use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Level, Metadata, Subscriber, span};

/// One warning: its message, its `error` field, if any, and the
/// `occurrences` a throttled warning reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Warning {
    pub(crate) message: String,
    pub(crate) error: String,
    pub(crate) occurrences: Option<u64>,
}

/// The warnings `bacnet_server` logged while the guard was installed.
#[derive(Clone, Default)]
pub(crate) struct WarnSpy(Arc<Mutex<Vec<Warning>>>);

impl WarnSpy {
    pub(crate) fn install() -> (Self, DefaultGuard) {
        let spy = Self::default();
        let guard = tracing::subscriber::set_default(spy.clone());
        (spy, guard)
    }

    pub(crate) fn warnings(&self) -> Vec<Warning> {
        self.0.lock().unwrap().clone()
    }
}

struct Fields<'a>(&'a mut Warning);

impl Visit for Fields<'_> {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "occurrences" {
            self.0.occurrences = Some(value);
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.0.message = format!("{value:?}"),
            "error" => self.0.error = format!("{value:?}"),
            _ => {}
        }
    }
}

impl Subscriber for WarnSpy {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::WARN && metadata.target().starts_with("bacnet_server")
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut warning = Warning::default();
        event.record(&mut Fields(&mut warning));
        self.0.lock().unwrap().push(warning);
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}
