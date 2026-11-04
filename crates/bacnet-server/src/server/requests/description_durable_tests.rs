//! A peer-written Description through the server (#2364): a
//! WritePropertyMultiple of Description and another kept property stages
//! one save on each object type that keeps it, and a NULL to Description
//! saves nothing. Each type's single writes, a failed save and the save off
//! the guard are covered beside the other kept rows, in
//! `reporting_durable_tests` and `write_config_durable_tests`.

use super::durable_write_wire_tests::{SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::reporting_durable_tests::{self as reporting, INSTANCE, TYPES, write_property};
use super::write_config_durable_tests::{self as config, ConfigStorage};
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

fn attempt(property: P, written: &PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: value(written.clone()),
        priority: None,
    }
}

fn lobby() -> PropertyValue {
    PropertyValue::CharacterString("lobby".into())
}

/// Each type, the instance it is served at, a kept property beside
/// Description and a value a write gives it.
fn objects() -> Vec<(ObjectType, u32, P, PropertyValue)> {
    let mut objects: Vec<_> = TYPES
        .into_iter()
        .map(|kind| {
            (
                kind,
                INSTANCE,
                P::NOTIFICATION_CLASS,
                PropertyValue::Unsigned(9),
            )
        })
        .collect();
    objects.push((
        ObjectType::TIMER,
        1,
        P::DEFAULT_TIMEOUT,
        PropertyValue::Unsigned(5_000),
    ));
    objects.push((
        ObjectType::CHANNEL,
        1,
        P::CHANNEL_NUMBER,
        PropertyValue::Unsigned(77),
    ));
    objects
}

async fn served_by(kind: ObjectType, storage: &Arc<ConfigStorage>) -> Arc<Fixture> {
    match kind {
        ObjectType::TIMER | ObjectType::CHANNEL => config::served_by(kind, storage).await,
        _ => reporting::served_by(kind, storage).await,
    }
}

/// Description and another kept property in one request stage one save,
/// and a server rebuilt on the storage serves both.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_with_description_saves_once_on_each_type() {
    for (kind, instance, property, written) in objects() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: oid(kind, instance),
            list_of_properties: vec![
                attempt(P::DESCRIPTION, &lobby()),
                attempt(property, &written),
            ],
        }]);
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request,
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WPM, "{kind:?}");
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
        drop(fixture);
        let rebuilt = served_by(kind, &storage).await;
        let object = oid(kind, instance);
        assert_eq!(
            rebuilt.read(object, P::DESCRIPTION).await,
            lobby(),
            "{kind:?}"
        );
        assert_eq!(rebuilt.read(object, property).await, written, "{kind:?}");
    }
}

/// A NULL to Description succeeds, changes nothing and saves nothing, on
/// every type that keeps Description.
#[tokio::test(start_paused = true)]
async fn a_null_description_saves_nothing() {
    for (kind, instance, _, _) in objects() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let request = if instance == INSTANCE {
            write_property(kind, P::DESCRIPTION, None, value(PropertyValue::Null))
        } else {
            let mut request = BytesMut::new();
            WritePropertyRequest {
                object_identifier: oid(kind, instance),
                property_identifier: P::DESCRIPTION,
                property_array_index: None,
                property_value: value(PropertyValue::Null),
                priority: None,
            }
            .encode(&mut request)
            .unwrap();
            request.freeze()
        };
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?}");
        assert_eq!(storage.saves.load(Ordering::SeqCst), 0, "{kind:?}");
    }
}
