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
use bacnet_objects::property_metadata::{has_property, property_list};
#[path = "../../bacnet-objects/tests/support/forwarding/mod.rs"]
mod forwarding;

use forwarding::probe::{CallLog, Probe, take};
use forwarding::rows::queries;

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
    for (name, query) in queries() {
        let expected = (query(&probe), forwarded(&log));
        let answered = (query(&view), forwarded(&log));
        assert_eq!(answered, expected, "{name}: the view must forward it");
    }
}

#[test]
fn a_device_keeps_its_owned_answers_and_forwards_every_other_query() {
    // The rows the view answers itself for a Device, to serve the
    // executor-owned properties and keep them out of any frozen copy.
    const OWNED: [&str; 1] = ["property_metadata"];
    let db = ObjectDatabase::new();
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::DEVICE, log.clone());
    let view = context.object(&probe);
    for (name, query) in queries() {
        let expected = (query(&probe), forwarded(&log));
        let answered = (query(&view), forwarded(&log));
        if OWNED.contains(&name) {
            assert_ne!(answered.0, expected.0, "{name}: the view answers it");
        } else {
            assert_eq!(answered, expected, "{name}: the view must forward it");
        }
    }
    let rows = view.property_metadata();
    assert!(has_property(&rows, P::PROTOCOL_SERVICES_SUPPORTED));
    // The view serves the Property_List its own rows derive, overlay rows
    // included (#2197).
    let derived = property_list(&rows)
        .map(|p| PropertyValue::Enumerated(p.to_raw()))
        .collect();
    assert_eq!(
        view.read_property(P::PROPERTY_LIST, None).unwrap(),
        PropertyValue::List(derived)
    );
    assert!(property_list(&rows).any(|p| p == P::DEVICE_ADDRESS_BINDING));
}

#[test]
fn a_read_view_lends_no_batch_capability() {
    let db = ObjectDatabase::new();
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::ACCESS_POINT, log.clone());
    let mut view = context.object(&probe);
    // The view borrows the object shared, so it lends no batch capability,
    // though the probe has one.
    assert!(view.access_events().is_none());
    assert!(forwarded(&log).is_empty());
}

/// The view's array, list and writability answers before #2177 part 2, when
/// it overrode the three trait methods, frozen here against the free
/// functions over the rows it serves: equal for every property, with and
/// without the identity overlay, under both execution profiles.
#[test]
fn the_view_rows_answer_shapes_and_writability_as_its_old_overrides_did() {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::property_metadata::{
        is_array_property, is_list_property, is_writable_property,
    };
    let db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let instance = device.object_identifier().instance_number();
    let identity = bacnet_objects::DeviceIdentity::new(instance, 42).unwrap();
    let properties = (0..=1023).map(P::from_raw).chain(
        P::ALL_NAMED
            .iter()
            .map(|&(_, property)| property)
            .filter(|property| property.to_raw() > 1023),
    );
    let properties: Vec<P> = properties.collect();
    for execution in [
        DeviceExecution::FullServer,
        DeviceExecution::Endpoint {
            writes: true,
            reinitialize: true,
            cov: true,
        },
    ] {
        for identity in [None, Some(&identity)] {
            let context = DeviceReadContext::new(&db, execution).with_identity(identity);
            let view = context.object(&device);
            for &property in &properties {
                let owned = view.owns(property);
                let old_array = if owned {
                    property == P::PROPERTY_LIST
                } else {
                    is_array_property(&device, property)
                };
                let old_list = if owned {
                    matches!(
                        property,
                        P::ACTIVE_COV_SUBSCRIPTIONS
                            | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
                            | P::DEVICE_ADDRESS_BINDING
                    )
                } else {
                    is_list_property(&device, property)
                };
                let old_writable = !owned && is_writable_property(&device, property);
                let new = (
                    is_array_property(&view, property),
                    is_list_property(&view, property),
                    is_writable_property(&view, property),
                );
                assert_eq!(
                    new,
                    (old_array, old_list, old_writable),
                    "{property:?} with identity {}",
                    identity.is_some()
                );
            }
        }
    }
}
