//! The server logs the deliveries a pass suppressed, at debug, with the
//! text the commit once logged itself (#2319).

use std::sync::{Arc, Mutex, OnceLock};

use bacnet_types::enums::{EventType, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

use super::super::SuppressedDelivery;

/// Each debug event's message.
#[derive(Clone, Default)]
struct Messages(Arc<Mutex<Vec<String>>>);

struct Message<'a>(&'a mut String);

impl Visit for Message<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Messages {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() == Level::DEBUG
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = String::new();
        event.record(&mut Message(&mut message));
        self.0.lock().unwrap().push(message);
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

#[test]
fn suppressed_deliveries_log_at_debug_with_the_commits_text() {
    // As the other capture tests: keep a disabled dispatcher alive so a
    // callsite another test cached as never enabled is looked at again.
    static PEER: OnceLock<tracing::Dispatch> = OnceLock::new();
    PEER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    let messages = Messages::default();
    let _guard = tracing::subscriber::set_default(messages.clone());
    let enrollment = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();
    super::log_suppressed_deliveries(&[
        SuppressedDelivery {
            enrollment_oid: enrollment,
            event_type: None,
        },
        SuppressedDelivery {
            enrollment_oid: enrollment,
            event_type: Some(EventType::OUT_OF_RANGE),
        },
    ]);
    let logged = messages.0.lock().unwrap().clone();
    assert_eq!(
        logged,
        [
            "Committed Event Enrollment reliability payload rejected; suppressing distribution",
            "Committed Event Enrollment payload rejected; suppressing distribution",
        ]
    );
}
