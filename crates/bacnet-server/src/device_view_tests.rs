//! The full-server Device view answers every read-only `BACnetObject` query
//! with the wrapped object's own answer, and keeps answers of its own only
//! where a Device's executor-owned properties need them (#1076).
//!
//! The rows and the probe are shared with SourceReporter's tests in
//! bacnet-endpoint (#1097). The trait's methods are read from its source, so
//! a query added there fails `every_read_query_has_a_forwarding_check` until
//! it has a row, and `the_view_forwards_every_read_query` until the view
//! forwards it.

use super::*;
use bacnet_types::error::RemoteError;
#[path = "../../bacnet-objects/tests/support/forwarding/mod.rs"]
mod forwarding;

use forwarding::probe::{CallLog, Probe, take};
use forwarding::rows::QUERIES;

/// The calls the view handed on, less the identity reads it makes to tell a
/// Device apart.
fn forwarded(log: &CallLog) -> Vec<String> {
    take(log)
        .into_iter()
        .filter(|call| call != "object_identifier()")
        .collect()
}

#[test]
fn every_read_query_has_a_forwarding_check() {
    forwarding::assert_rows_cover_the_trait();
    forwarding::assert_probe_answers_every_row();
}

#[test]
fn the_view_forwards_every_read_query() {
    let db = ObjectDatabase::new();
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::ANALOG_VALUE, log.clone());
    let view = context.object(&probe);
    for (name, query) in QUERIES {
        let expected = (query(&probe), forwarded(&log));
        let answered = (query(&view), forwarded(&log));
        assert_eq!(answered, expected, "{name}: the view must forward it");
    }
}

#[test]
fn a_device_keeps_its_owned_answers_and_forwards_every_other_query() {
    // The rows the view answers itself for a Device, to serve the
    // executor-owned properties and keep them out of any frozen copy.
    const OWNED: [&str; 4] = [
        "property_metadata",
        "property_list",
        "required_properties",
        "cov_snapshot_internal",
    ];
    let db = ObjectDatabase::new();
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::DEVICE, log.clone());
    let view = context.object(&probe);
    for (name, query) in QUERIES {
        let expected = (query(&probe), forwarded(&log));
        let answered = (query(&view), forwarded(&log));
        if OWNED.contains(name) {
            assert_ne!(answered.0, expected.0, "{name}: the view answers it");
        } else {
            assert_eq!(answered, expected, "{name}: the view must forward it");
        }
    }
    assert_eq!(view.cov_snapshot_internal().map(|_| ()), None);
    assert!(
        view.property_list()
            .contains(&P::PROTOCOL_SERVICES_SUPPORTED)
    );
}

#[test]
fn a_read_view_refuses_batch_mutation_without_calling_the_wrapped_object() {
    let db = ObjectDatabase::new();
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::ACCESS_POINT, log.clone());
    let mut view = context.object(&probe);
    assert!(view.supports_access_event_batch_internal());
    take(&log);
    assert!(matches!(
        view.record_access_event_batch_internal(&[]),
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::OBJECT,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        }))
    ));
    assert!(forwarded(&log).is_empty());
}
