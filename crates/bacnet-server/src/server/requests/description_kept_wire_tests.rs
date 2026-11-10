//! A peer-written Description on the types whose snapshot formats keep it
//! (#2364): the three logs, a Notification Class, a Notification Forwarder
//! and an Access Rights object. A WritePropertyMultiple of Description and
//! another kept property stages one save on each, a NULL to Description
//! saves nothing, and a Description that can't be saved is refused on the
//! wire with the old one served.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::access_rights_durable_tests::RightsStorage;
use super::durable_write_wire_tests::{
    ERROR_PDU, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, destination,
};
use super::log_reporting_durable_tests::LogStorage;
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::notification_class_durable_tests::ClassStorage;
use super::*;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_objects::access_control::AccessRightsObject;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use PropertyIdentifier as P;

const KINDS: [ObjectType; 6] = [
    ObjectType::TREND_LOG,
    ObjectType::TREND_LOG_MULTIPLE,
    ObjectType::EVENT_LOG,
    ObjectType::NOTIFICATION_CLASS,
    ObjectType::NOTIFICATION_FORWARDER,
    ObjectType::ACCESS_RIGHTS,
];

/// One storage per type.
#[derive(Default)]
struct Storages {
    trend: Arc<LogStorage<BACnetLogRecord>>,
    multiple: Arc<LogStorage<BACnetLogMultipleRecord>>,
    event: Arc<LogStorage<BACnetEventLogRecord>>,
    class: Arc<ClassStorage>,
    forwarder: Arc<HeldStorage>,
    rights: Arc<RightsStorage>,
}

impl Storages {
    fn counts(&self, kind: ObjectType) -> (&AtomicUsize, &AtomicBool) {
        match kind {
            ObjectType::TREND_LOG => (&self.trend.saves, &self.trend.fail),
            ObjectType::TREND_LOG_MULTIPLE => (&self.multiple.saves, &self.multiple.fail),
            ObjectType::EVENT_LOG => (&self.event.saves, &self.event.fail),
            ObjectType::NOTIFICATION_CLASS => (&self.class.saves, &self.class.fail),
            ObjectType::NOTIFICATION_FORWARDER => (&self.forwarder.saves, &self.forwarder.fail),
            _ => (&self.rights.saves, &self.rights.fail),
        }
    }

    fn saves(&self, kind: ObjectType) -> usize {
        self.counts(kind).0.load(Ordering::SeqCst)
    }

    fn fail(&self, kind: ObjectType) {
        self.counts(kind).1.store(true, Ordering::SeqCst);
    }
}

/// Instance 1 of each type, kept in `storages`, and configured with a
/// Description the way an application does at each start: a saved one
/// wins.
async fn served_by(storages: &Storages) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    let mut trend = TrendLogObject::with_persistence(1, "TL", 8, storages.trend.clone()).unwrap();
    trend.set_description("configured");
    let mut multiple =
        TrendLogMultipleObject::with_persistence(1, "TLM", 8, storages.multiple.clone()).unwrap();
    multiple.set_description("configured");
    let mut event = EventLogObject::with_persistence(1, "EL", 8, storages.event.clone()).unwrap();
    event.set_description("configured");
    let mut class = NotificationClass::with_persistence(1, "NC", storages.class.clone()).unwrap();
    class.set_description("configured");
    let mut forwarder =
        NotificationForwarderObject::with_persistence(1, "NF", storages.forwarder.clone()).unwrap();
    forwarder.set_description("configured");
    let rights = AccessRightsObject::with_persistence(1, "AR", storages.rights.clone()).unwrap();
    let objects: [Box<dyn BACnetObject>; 6] = [
        Box::new(trend),
        Box::new(multiple),
        Box::new(event),
        Box::new(class),
        Box::new(forwarder),
        Box::new(rights),
    ];
    let mut db = fixture.db.write().await;
    for object in objects {
        db.add(object).unwrap();
    }
    drop(db);
    Arc::new(fixture)
}

/// Another property `kind` keeps, and the octets a write gives it.
fn other(kind: ObjectType) -> (P, Vec<u8>) {
    match kind {
        ObjectType::NOTIFICATION_CLASS | ObjectType::NOTIFICATION_FORWARDER => {
            let mut list = BytesMut::new();
            encode_destination_list(&mut list, &[destination(3)]).unwrap();
            (P::RECIPIENT_LIST, list.to_vec())
        }
        ObjectType::ACCESS_RIGHTS => (P::ENABLE, value(PropertyValue::Boolean(false))),
        _ => (P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(9))),
    }
}

fn text(text: &str) -> PropertyValue {
    PropertyValue::CharacterString(text.into())
}

fn write_description(kind: ObjectType, written: PropertyValue) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(kind, 1),
        property_identifier: P::DESCRIPTION,
        property_array_index: None,
        property_value: value(written),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// Description and another kept property in one request stage one save,
/// and a server rebuilt on the storage serves both.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_with_description_saves_once_on_each_type() {
    for kind in KINDS {
        let storages = Storages::default();
        let fixture = served_by(&storages).await;
        let (property, octets) = other(kind);
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: oid(kind, 1),
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: P::DESCRIPTION,
                    property_array_index: None,
                    value: value(text("lobby")),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value: octets,
                    priority: None,
                },
            ],
        }]);
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WPM);
        assert_eq!(storages.saves(kind), 1, "{kind:?}");
        let served = fixture.read(oid(kind, 1), property).await;
        drop(fixture);
        let rebuilt = served_by(&storages).await;
        assert_eq!(
            rebuilt.read(oid(kind, 1), P::DESCRIPTION).await,
            text("lobby"),
            "{kind:?}"
        );
        assert_eq!(rebuilt.read(oid(kind, 1), property).await, served);
    }
}

/// A NULL to Description succeeds, changes nothing and saves nothing.
#[tokio::test(start_paused = true)]
async fn a_null_description_saves_nothing() {
    for kind in KINDS {
        let storages = Storages::default();
        let fixture = served_by(&storages).await;
        let before = fixture.read(oid(kind, 1), P::DESCRIPTION).await;
        let request = write_description(kind, PropertyValue::Null);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?}");
        assert_eq!(storages.saves(kind), 0, "{kind:?}");
        assert_eq!(fixture.read(oid(kind, 1), P::DESCRIPTION).await, before);
    }
}

/// A Description whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the one written before stays served.
#[tokio::test(start_paused = true)]
async fn a_description_that_cannot_be_saved_is_refused_on_the_wire() {
    for kind in KINDS {
        let storages = Storages::default();
        let fixture = served_by(&storages).await;
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let request = write_description(kind, text("lobby"));
        assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WRITE);
        storages.fail(kind);
        let request = write_description(kind, text("hall"));
        assert_eq!(
            wire(&fixture, service, request).await,
            [ERROR_PDU, 5, 15, 0x91, 0, 0x91, 25],
            "{kind:?}"
        );
        assert_eq!(
            fixture.read(oid(kind, 1), P::DESCRIPTION).await,
            text("lobby"),
            "{kind:?}"
        );
    }
}
