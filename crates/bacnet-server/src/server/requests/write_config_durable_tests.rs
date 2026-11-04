//! The configuration a peer writes to a Timer or a Channel saves through the
//! server (#2243; a Timer's Default_Timeout too, #2312): each covered
//! WriteProperty stages its save, the write takes it, and a server rebuilt
//! on the same storage serves what was written. A write that cannot be
//! saved is refused on the wire with the old value kept.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    ERROR_PDU, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::*;
use bacnet_encoding::constructed::{
    encode_device_object_property_reference, encode_timer_state_change_value,
};
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::timer::{StateChangeValues, TimerObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::write_config::{WriteConfigPersistence, WriteConfigSnapshot};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetTimerStateChangeValue};
use std::sync::atomic::Ordering;

/// The held storage's snapshot is an `Option`, which has a default.
pub(super) type ConfigStorage = HeldStorage<Option<WriteConfigSnapshot>>;

impl WriteConfigPersistence for ConfigStorage {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        Ok(self.load_saved().flatten())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        self.store(&Some(snapshot.clone()))
    }
}

const TIMER: ObjectType = ObjectType::TIMER;
const CHANNEL: ObjectType = ObjectType::CHANNEL;
const VALUES: PropertyIdentifier = PropertyIdentifier::STATE_CHANGE_VALUES;
const TIMEOUT: PropertyIdentifier = PropertyIdentifier::DEFAULT_TIMEOUT;
const REFERENCES: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
const DELAYS: PropertyIdentifier = PropertyIdentifier::EXECUTION_DELAY;
const NUMBER: PropertyIdentifier = PropertyIdentifier::CHANNEL_NUMBER;
const GROUPS: PropertyIdentifier = PropertyIdentifier::CONTROL_GROUPS;
const INHIBIT: PropertyIdentifier = PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT;
const DESCRIPTION: PropertyIdentifier = PropertyIdentifier::DESCRIPTION;

fn reference(kind: ObjectType, instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: oid(kind, instance),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

fn references(references: &[BACnetDeviceObjectPropertyReference]) -> Vec<u8> {
    let mut octets = BytesMut::new();
    for reference in references {
        encode_device_object_property_reference(&mut octets, reference);
    }
    octets.to_vec()
}

fn state_change_values(first: f32) -> StateChangeValues {
    std::array::from_fn(|index| match index {
        0 => BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(first)),
        _ => BACnetTimerStateChangeValue::NoValue,
    })
}

/// The object `kind` 1, kept in `storage` and configured as an application
/// would.
fn object(kind: ObjectType, storage: &Arc<ConfigStorage>) -> Box<dyn BACnetObject> {
    let persistence = Arc::clone(storage) as Arc<dyn WriteConfigPersistence>;
    if kind == TIMER {
        let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
        let configured = vec![reference(ObjectType::ANALOG_VALUE, 1)];
        timer
            .set_state_change(state_change_values(1.0), configured, 9)
            .unwrap();
        timer.set_default_timeout(30_000).unwrap();
        return Box::new(timer);
    }
    let mut channel = ChannelObject::with_persistence(1, "CH", 4, persistence).unwrap();
    channel
        .set_members(vec![reference(ObjectType::ANALOG_OUTPUT, 1)])
        .unwrap();
    channel.set_execution_delay(vec![10]).unwrap();
    channel.set_control_groups(vec![2]).unwrap();
    Box::new(channel)
}

pub(super) async fn served_by(kind: ObjectType, storage: &Arc<ConfigStorage>) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    fixture.db.write().await.add(object(kind, storage)).unwrap();
    Arc::new(fixture)
}

/// What the object serves of the properties a write could change.
async fn configuration(fixture: &Fixture, kind: ObjectType) -> Vec<PropertyValue> {
    let properties: &[PropertyIdentifier] = if kind == TIMER {
        &[
            VALUES,
            REFERENCES,
            PropertyIdentifier::PRIORITY_FOR_WRITING,
            TIMEOUT,
            DESCRIPTION,
        ]
    } else {
        &[REFERENCES, DELAYS, NUMBER, GROUPS, INHIBIT, DESCRIPTION]
    };
    let mut values = Vec::new();
    for property in properties {
        values.push(fixture.read(oid(kind, 1), *property).await);
    }
    values
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

/// Every covered write: the object, the property, the index and the value.
fn covered() -> Vec<(ObjectType, PropertyIdentifier, Option<u32>, Vec<u8>)> {
    let mut whole_values = BytesMut::new();
    for element in state_change_values(30.0) {
        encode_timer_state_change_value(&mut whole_values, &element).unwrap();
    }
    let two = [
        reference(ObjectType::ANALOG_VALUE, 5),
        reference(ObjectType::BINARY_VALUE, 6),
    ];
    let mut groups = value(PropertyValue::Unsigned(5));
    groups.extend(value(PropertyValue::Unsigned(6)));
    // Kept beside the configuration (#2364).
    let lobby = value(PropertyValue::CharacterString("lobby".into()));
    vec![
        (TIMER, VALUES, None, whole_values.to_vec()),
        (TIMER, VALUES, Some(2), value(PropertyValue::Real(42.0))),
        (TIMER, REFERENCES, None, references(&two)),
        (TIMER, TIMEOUT, None, value(PropertyValue::Unsigned(5_000))),
        (CHANNEL, REFERENCES, None, references(&two)),
        (CHANNEL, REFERENCES, Some(1), references(&two[1..])),
        (
            CHANNEL,
            REFERENCES,
            Some(0),
            value(PropertyValue::Unsigned(3)),
        ),
        (
            CHANNEL,
            DELAYS,
            Some(1),
            value(PropertyValue::Unsigned(500)),
        ),
        (CHANNEL, NUMBER, None, value(PropertyValue::Unsigned(77))),
        (CHANNEL, GROUPS, None, groups),
        (CHANNEL, INHIBIT, None, value(PropertyValue::Boolean(true))),
        (TIMER, DESCRIPTION, None, lobby.clone()),
        (CHANNEL, DESCRIPTION, None, lobby),
    ]
}

/// Each covered WriteProperty takes its staged save, without saving again,
/// and a server rebuilt on the storage serves what it wrote.
#[tokio::test(start_paused = true)]
async fn each_covered_write_survives_a_rebuild() {
    for (kind, property, index, octets) in covered() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let before = configuration(&fixture, kind).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(
            response, SIMPLE_ACK_WRITE,
            "{kind:?} {property:?} {index:?}"
        );
        let written = configuration(&fixture, kind).await;
        assert_ne!(
            written, before,
            "{kind:?} {property:?} {index:?} changed nothing"
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{property:?}");
        drop(fixture);
        let rebuilt = served_by(kind, &storage).await;
        assert_eq!(
            configuration(&rebuilt, kind).await,
            written,
            "{kind:?} {property:?} {index:?} after a rebuild"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_write_that_cannot_be_saved_is_refused_on_the_wire_and_the_old_value_stays() {
    for (kind, property, index, octets) in covered() {
        let storage = Arc::new(ConfigStorage::default());
        storage.fail.store(true, Ordering::SeqCst);
        let fixture = served_by(kind, &storage).await;
        let before = configuration(&fixture, kind).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        // An Error PDU: invoke ID 5, WriteProperty, DEVICE (0) and
        // OPERATIONAL_PROBLEM (25).
        assert_eq!(
            response,
            [ERROR_PDU, 5, 15, 0x91, 0, 0x91, 25],
            "{property:?}"
        );
        assert_eq!(configuration(&fixture, kind).await, before);
    }
}

/// A WritePropertyMultiple's writes to one Channel save once, together.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_saves_a_channels_writes_once() {
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served_by(CHANNEL, &storage).await;
    let attempt = |property, octets| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets,
        priority: None,
    };
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: oid(CHANNEL, 1),
        list_of_properties: vec![
            attempt(NUMBER, value(PropertyValue::Unsigned(12))),
            attempt(INHIBIT, value(PropertyValue::Boolean(true))),
        ],
    }]);
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    let written = configuration(&fixture, CHANNEL).await;
    drop(fixture);
    let rebuilt = served_by(CHANNEL, &storage).await;
    assert_eq!(configuration(&rebuilt, CHANNEL).await, written);
}

/// The save runs with the database guard dropped: the database answers a
/// reader and a writer while it is held, for a set write, a
/// Default_Timeout write (#2312) and a Description write (#2364) alike.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timer_save_runs_while_the_database_stays_available() {
    for (property, index, written) in [
        (VALUES, Some(3), PropertyValue::Real(5.0)),
        (TIMEOUT, None, PropertyValue::Unsigned(5_000)),
        (
            DESCRIPTION,
            None,
            PropertyValue::CharacterString("lobby".into()),
        ),
    ] {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(TIMER, &storage).await;
        let request = write_property(TIMER, property, index, value(written));
        let (response, available) = while_saving(
            &fixture,
            &storage,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request,
            WAIT,
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{property:?}");
        assert!(
            available,
            "the database was held while the Timer saved {property:?}"
        );
    }
}
