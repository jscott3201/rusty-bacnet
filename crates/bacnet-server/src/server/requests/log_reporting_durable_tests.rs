//! The reporting settings a peer writes to a durable Trend Log, Trend Log
//! Multiple or Event Log save through the server (#1952): each covered
//! WriteProperty stages one commit of the log, the write takes it, and a
//! server rebuilt on the same storage serves what was written. A write that
//! cannot be saved is refused on the wire with the old value kept.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    ERROR_PDU, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::log_persistence::{LogPersistence, LogSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, BACnetObjectPropertyReference,
};
use bacnet_types::enums::NotifyType;
use std::sync::atomic::Ordering;

/// Storage holding a log's last snapshot. The held snapshot is an
/// `Option`, which has a default.
pub(super) type LogStorage<R> = HeldStorage<Option<LogSnapshot<R>>>;

impl<R: Clone + Send + Sync + 'static> LogPersistence<R> for LogStorage<R> {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<LogSnapshot<R>>, Error> {
        Ok(self.load_saved().flatten())
    }

    fn commit(&self, snapshot: &LogSnapshot<R>) -> Result<(), Error> {
        self.store(&Some(snapshot.clone()))
    }
}

const TL: ObjectType = ObjectType::TREND_LOG;
const TLM: ObjectType = ObjectType::TREND_LOG_MULTIPLE;
const EL: ObjectType = ObjectType::EVENT_LOG;

/// One storage per log type.
#[derive(Default)]
struct Storages {
    trend: Arc<LogStorage<BACnetLogRecord>>,
    multiple: Arc<LogStorage<BACnetLogMultipleRecord>>,
    event: Arc<LogStorage<BACnetEventLogRecord>>,
}

impl Storages {
    fn saves(&self, kind: ObjectType) -> usize {
        match kind {
            TL => self.trend.saves.load(Ordering::SeqCst),
            TLM => self.multiple.saves.load(Ordering::SeqCst),
            _ => self.event.saves.load(Ordering::SeqCst),
        }
    }

    fn fail(&self) {
        self.trend.fail.store(true, Ordering::SeqCst);
        self.multiple.fail.store(true, Ordering::SeqCst);
        self.event.fail.store(true, Ordering::SeqCst);
    }
}

/// The three logs, kept in `storages` and configured as an application
/// does at each start: the setters give way to a saved peer write.
async fn served_by(storages: &Storages) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    let mut trend = TrendLogObject::with_persistence(1, "TL", 16, storages.trend.clone()).unwrap();
    trend.set_notification_class(3);
    trend.set_client_cov_increment(Some(1.0)).unwrap();
    let multiple =
        TrendLogMultipleObject::with_persistence(1, "TLM", 16, storages.multiple.clone()).unwrap();
    let mut event = EventLogObject::with_persistence(1, "EL", 16, storages.event.clone()).unwrap();
    event.set_notification_class(3);
    let objects: [Box<dyn BACnetObject>; 3] =
        [Box::new(trend), Box::new(multiple), Box::new(event)];
    let mut db = fixture.db.write().await;
    for object in objects {
        db.add(object).unwrap();
    }
    drop(db);
    Arc::new(fixture)
}

fn reference_octets() -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
    );
    octets.to_vec()
}

fn texts() -> Vec<u8> {
    ["off", "fault", "back"]
        .into_iter()
        .flat_map(|text| value(PropertyValue::CharacterString(text.into())))
        .collect()
}

/// Every covered write: the property, the index, the value, and whether
/// only a Trend Log has it.
fn covered() -> Vec<(PropertyIdentifier, Option<u32>, Vec<u8>, bool)> {
    use PropertyIdentifier as P;
    let enable = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![EventTransitionBits::TO_FAULT.to_bacnet()],
    };
    vec![
        (
            P::NOTIFICATION_CLASS,
            None,
            value(PropertyValue::Unsigned(9)),
            false,
        ),
        (P::EVENT_ENABLE, None, value(enable), false),
        (
            P::NOTIFY_TYPE,
            None,
            value(PropertyValue::Enumerated(NotifyType::ALARM.to_raw())),
            false,
        ),
        (
            P::EVENT_DETECTION_ENABLE,
            None,
            value(PropertyValue::Boolean(false)),
            false,
        ),
        (P::EVENT_MESSAGE_TEXTS_CONFIG, None, texts(), false),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(3),
            value(PropertyValue::CharacterString("back".into())),
            false,
        ),
        (
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            reference_octets(),
            false,
        ),
        (
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            value(PropertyValue::Boolean(true)),
            false,
        ),
        (
            P::CLIENT_COV_INCREMENT,
            None,
            value(PropertyValue::Null),
            true,
        ),
        (
            P::COV_RESUBSCRIPTION_INTERVAL,
            None,
            value(PropertyValue::Unsigned(120)),
            true,
        ),
    ]
}

fn write_property(
    kind: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    octets: Vec<u8>,
) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(kind, 1),
        property_identifier: property,
        property_array_index: index,
        property_value: octets,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// The covered writes each log takes.
fn cases() -> Vec<(ObjectType, PropertyIdentifier, Option<u32>, Vec<u8>)> {
    let mut cases = Vec::new();
    for (property, index, octets, trend_only) in covered() {
        for kind in [TL, TLM, EL] {
            if kind == TL || !trend_only {
                cases.push((kind, property, index, octets.clone()));
            }
        }
    }
    cases
}

/// Each covered WriteProperty takes its staged commit, without committing
/// again, and a server rebuilt on the storage serves what it wrote, over
/// the application's setters.
#[tokio::test(start_paused = true)]
async fn each_covered_write_survives_a_rebuild() {
    for (kind, property, index, octets) in cases() {
        let storages = Storages::default();
        let fixture = served_by(&storages).await;
        let before = fixture.read(oid(kind, 1), property).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(
            response, SIMPLE_ACK_WRITE,
            "{kind:?} {property:?} {index:?}"
        );
        let written = fixture.read(oid(kind, 1), property).await;
        assert_ne!(
            written, before,
            "{kind:?} {property:?} {index:?} changed nothing"
        );
        assert_eq!(storages.saves(kind), 1, "{kind:?} {property:?}");
        drop(fixture);
        let rebuilt = served_by(&storages).await;
        assert_eq!(
            rebuilt.read(oid(kind, 1), property).await,
            written,
            "{kind:?} {property:?} {index:?} after a rebuild"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_write_that_cannot_be_saved_is_refused_on_the_wire_and_the_old_value_stays() {
    for (kind, property, index, octets) in cases() {
        let storages = Storages::default();
        storages.fail();
        let fixture = served_by(&storages).await;
        let before = fixture.read(oid(kind, 1), property).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        // An Error PDU: invoke ID 5, WriteProperty, DEVICE (0) and
        // OPERATIONAL_PROBLEM (25).
        assert_eq!(
            response,
            [ERROR_PDU, 5, 15, 0x91, 0, 0x91, 25],
            "{kind:?} {property:?}"
        );
        assert_eq!(fixture.read(oid(kind, 1), property).await, before);
    }
}

/// A WritePropertyMultiple's writes to one log commit once, together.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_commits_a_logs_writes_once() {
    use PropertyIdentifier as P;
    let storages = Storages::default();
    let fixture = served_by(&storages).await;
    let attempt = |property, octets| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets,
        priority: None,
    };
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: oid(TL, 1),
        list_of_properties: vec![
            attempt(P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(12))),
            attempt(
                P::NOTIFY_TYPE,
                value(PropertyValue::Enumerated(NotifyType::ALARM.to_raw())),
            ),
            attempt(
                P::COV_RESUBSCRIPTION_INTERVAL,
                value(PropertyValue::Unsigned(90)),
            ),
        ],
    }]);
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert_eq!(storages.saves(TL), 1);
    let properties = [
        P::NOTIFICATION_CLASS,
        P::NOTIFY_TYPE,
        P::COV_RESUBSCRIPTION_INTERVAL,
    ];
    let mut written = Vec::new();
    for property in properties {
        written.push(fixture.read(oid(TL, 1), property).await);
    }
    drop(fixture);
    let rebuilt = served_by(&storages).await;
    for (property, value) in properties.into_iter().zip(written) {
        assert_eq!(
            rebuilt.read(oid(TL, 1), property).await,
            value,
            "{property:?}"
        );
    }
}

/// A message text past what the log's snapshot keeps is refused on the
/// wire before anything is saved, with RESOURCES (3) /
/// NO_SPACE_TO_WRITE_PROPERTY (20), whatever the storage.
#[tokio::test(start_paused = true)]
async fn an_over_long_message_text_is_refused_on_the_wire() {
    let storages = Storages::default();
    let fixture = served_by(&storages).await;
    let property = PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG;
    let before = fixture.read(oid(TL, 1), property).await;
    let long = PropertyValue::CharacterString("x".repeat(20 * 1024));
    let request = write_property(TL, property, Some(2), value(long));
    let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
    assert_eq!(response, [ERROR_PDU, 5, 15, 0x91, 3, 0x91, 20]);
    assert_eq!(fixture.read(oid(TL, 1), property).await, before);
    assert_eq!(storages.saves(TL), 0);
}

/// The commit runs with the database guard dropped: the database answers a
/// reader and a writer while each covered write's commit is held.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_log_commit_runs_while_the_database_stays_available() {
    for (kind, property, index, octets) in cases() {
        let storages = Storages::default();
        let fixture = served_by(&storages).await;
        let request = write_property(kind, property, index, octets);
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let (response, available) = match kind {
            TL => while_saving(&fixture, &storages.trend, service, request, WAIT).await,
            TLM => while_saving(&fixture, &storages.multiple, service, request, WAIT).await,
            _ => while_saving(&fixture, &storages.event, service, request, WAIT).await,
        };
        assert_eq!(
            response, SIMPLE_ACK_WRITE,
            "{kind:?} {property:?} {index:?}"
        );
        assert!(
            available,
            "the database was held while the {kind:?} committed {property:?} {index:?}"
        );
        assert_eq!(storages.saves(kind), 1, "{kind:?} {property:?} {index:?}");
    }
}
