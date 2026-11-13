//! The server logs the deliveries a pass suppressed, at debug, with the
//! text the commit once logged itself (#2319).

use std::sync::{Arc, Mutex, OnceLock};

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EngineeringUnits, EventType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

use super::super::SuppressedDelivery;

/// One debug event: its message, and its other fields as they print.
#[derive(Clone, Debug, Default)]
struct Logged {
    message: String,
    fields: Vec<(String, String)>,
}

/// Each debug event, in order.
#[derive(Clone, Default)]
struct Messages(Arc<Mutex<Vec<Logged>>>);

impl Messages {
    fn logged(&self) -> Vec<Logged> {
        self.0.lock().unwrap().clone()
    }

    fn messages(&self) -> Vec<String> {
        self.logged()
            .into_iter()
            .map(|logged| logged.message)
            .collect()
    }
}

impl Visit for Logged {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_owned(), format!("{value:?}")));
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
        let mut logged = Logged::default();
        event.record(&mut logged);
        self.0.lock().unwrap().push(logged);
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

/// Capture this thread's debug events until the guard drops.
fn capture() -> (Messages, tracing::subscriber::DefaultGuard) {
    // As the other capture tests: keep a disabled dispatcher alive so a
    // callsite another test cached as never enabled is looked at again.
    static PEER: OnceLock<tracing::Dispatch> = OnceLock::new();
    PEER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    let messages = Messages::default();
    let guard = tracing::subscriber::set_default(messages.clone());
    (messages, guard)
}

const SUPPRESSED: &str = "Committed Event Enrollment payload rejected; suppressing distribution";

/// AI-1 reporting through Notification Class 90, an Unsigned, and EE-1
/// watching that with OUT_OF_RANGE over 80: its HIGH_LIMIT commits, with no
/// REAL to project, so it is suppressed. A Device for a server to start on.
fn unsigned_out_of_range() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let class = PropertyValue::Unsigned(90);
    ai.write_property(PropertyIdentifier::NOTIFICATION_CLASS, None, class, None)
        .unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::NOTIFICATION_CLASS.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
        ..DeviceConfig::default()
    });
    db.add(Box::new(device.unwrap())).unwrap();
    (db, ee_oid)
}

/// What the public wrapper logs of a suppressed transition: the text, the
/// enrollment and the event type.
#[test]
fn the_report_wrapper_logs_a_suppressed_transition_with_its_fields() {
    let (mut db, ee_oid) = unsigned_out_of_range();
    let (messages, _guard) = capture();
    let report = super::super::evaluate_event_enrollments_report(&mut db, 1);
    assert_eq!(report.transitions.len(), 1);
    let logged = messages.logged();
    let suppressed = logged.iter().find(|logged| logged.message == SUPPRESSED);
    let fields = &suppressed
        .unwrap_or_else(|| panic!("not logged: {logged:?}"))
        .fields;
    let expected = [
        ("enrollment".to_owned(), ee_oid.to_string()),
        (
            "event_type".to_owned(),
            format!("{:?}", EventType::OUT_OF_RANGE),
        ),
    ];
    assert_eq!(fields, &expected);
}

/// The server's own pass, on its task, logs it too.
#[tokio::test(start_paused = true)]
async fn the_task_logs_a_suppressed_transition() {
    let (db, _) = unsigned_out_of_range();
    let (messages, _guard) = capture();
    let (transport, _tx) = crate::server::test_transport::TestTransport::inbound(16);
    let config = crate::server::ServerConfig {
        event_enrollment_interval_secs: 1,
        ..crate::server::ServerConfig::default()
    };
    let server = crate::server::BACnetServer::start(config, db, transport)
        .await
        .unwrap();
    // The first pass runs at once.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let logged = messages.messages();
    assert!(
        logged.iter().any(|message| message == SUPPRESSED),
        "{logged:?}"
    );
    drop(server);
}

#[test]
fn suppressed_deliveries_log_at_debug_with_the_commits_text() {
    let (messages, _guard) = capture();
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
    assert_eq!(
        messages.messages(),
        [
            "Committed Event Enrollment reliability payload rejected; suppressing distribution",
            "Committed Event Enrollment payload rejected; suppressing distribution",
        ]
    );
}
