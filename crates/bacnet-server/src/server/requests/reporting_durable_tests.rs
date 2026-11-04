//! The reporting settings a peer writes to an object keeping them in storage
//! save through the server (#2331): the analog, binary and multi-state
//! objects, Access Door and Zone, and Event and Alert Enrollment. Each
//! covered WriteProperty stages one save, the write takes it, and a server
//! rebuilt on the same storage serves what was written. A write that cannot
//! be saved is refused on the wire with the old value kept, and a request
//! writing Tags beside them saves each once.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    ERROR_PDU, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::access_control::{AccessDoorObject, AccessZoneObject};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::object_profile::{ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::write_config::WriteConfigPersistence;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetNameValue, BACnetObjectPropertyReference};
use bacnet_types::enums::{EngineeringUnits, EventType, NotifyType};
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

const TYPES: [ObjectType; 13] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

/// The fixture's database already holds Binary Values 1 and 2 and
/// Multi-State Input 1.
pub(super) const INSTANCE: u32 = 5;

/// Object 5 of `kind`, keeping its reporting settings in `storage`.
pub(super) fn object(kind: ObjectType, storage: &Arc<ConfigStorage>) -> Box<dyn BACnetObject> {
    let units = EngineeringUnits::NO_UNITS;
    let kept = Arc::clone(storage) as Arc<dyn WriteConfigPersistence>;
    macro_rules! build {
        ($object:expr_2021) => {
            Box::new(
                $object
                    .and_then(|object| object.with_reporting_persistence(kept))
                    .unwrap(),
            )
        };
    }
    match kind {
        ObjectType::ANALOG_INPUT => build!(AnalogInputObject::new(INSTANCE, "AI", units)),
        ObjectType::ANALOG_OUTPUT => build!(AnalogOutputObject::new(INSTANCE, "AO", units)),
        ObjectType::ANALOG_VALUE => build!(AnalogValueObject::new(INSTANCE, "AV", units)),
        ObjectType::BINARY_INPUT => build!(BinaryInputObject::new(INSTANCE, "BI")),
        ObjectType::BINARY_OUTPUT => build!(BinaryOutputObject::new(INSTANCE, "BO")),
        ObjectType::BINARY_VALUE => build!(BinaryValueObject::new(INSTANCE, "BV")),
        ObjectType::MULTI_STATE_INPUT => build!(MultiStateInputObject::new(INSTANCE, "MSI", 3)),
        ObjectType::MULTI_STATE_OUTPUT => build!(MultiStateOutputObject::new(INSTANCE, "MSO", 3)),
        ObjectType::MULTI_STATE_VALUE => build!(MultiStateValueObject::new(INSTANCE, "MSV", 3)),
        ObjectType::ACCESS_DOOR => build!(AccessDoorObject::new(INSTANCE, "Door")),
        ObjectType::ACCESS_ZONE => build!(AccessZoneObject::new(INSTANCE, "Zone")),
        ObjectType::EVENT_ENROLLMENT => build!(EventEnrollmentObject::new(
            INSTANCE,
            "EE",
            EventType::CHANGE_OF_STATE
        )),
        _ => build!(AlertEnrollmentObject::new(
            INSTANCE,
            "AE",
            oid(ObjectType::ANALOG_INPUT, 1)
        )),
    }
}

pub(super) async fn served_by(kind: ObjectType, storage: &Arc<ConfigStorage>) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    fixture.db.write().await.add(object(kind, storage)).unwrap();
    Arc::new(fixture)
}

fn texts() -> Vec<u8> {
    ["off", "fault", "back"]
        .into_iter()
        .flat_map(|text| value(PropertyValue::CharacterString(text.into())))
        .collect()
}

fn reference_octets() -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(
            oid(ObjectType::BINARY_VALUE, 1),
            P::PRESENT_VALUE.to_raw(),
        ),
    );
    octets.to_vec()
}

/// Every covered write on every type: the object type, the property, the
/// index and the value. Detection is turned to the other value; the
/// inhibit, which takes a write only while detection is on, is left out on
/// the Outputs, whose detection starts off.
fn cases() -> Vec<(ObjectType, P, Option<u32>, Vec<u8>)> {
    let mut cases = Vec::new();
    for kind in TYPES {
        let output = matches!(
            kind,
            ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT
        );
        let enable = PropertyValue::BitString {
            unused_bits: 5,
            data: vec![EventTransitionBits::TO_FAULT.to_bacnet()],
        };
        let mut writes = vec![
            (
                P::NOTIFICATION_CLASS,
                None,
                value(PropertyValue::Unsigned(9)),
            ),
            (P::EVENT_ENABLE, None, value(enable)),
            (
                P::NOTIFY_TYPE,
                None,
                value(PropertyValue::Enumerated(NotifyType::EVENT.to_raw())),
            ),
            (
                P::EVENT_DETECTION_ENABLE,
                None,
                value(PropertyValue::Boolean(output)),
            ),
            (P::EVENT_MESSAGE_TEXTS_CONFIG, None, texts()),
            (
                P::EVENT_MESSAGE_TEXTS_CONFIG,
                Some(3),
                value(PropertyValue::CharacterString("back".into())),
            ),
            (P::EVENT_ALGORITHM_INHIBIT_REF, None, reference_octets()),
        ];
        if !output {
            writes.push((
                P::EVENT_ALGORITHM_INHIBIT,
                None,
                value(PropertyValue::Boolean(true)),
            ));
        }
        cases.extend(
            writes
                .into_iter()
                .map(|(property, index, octets)| (kind, property, index, octets)),
        );
    }
    cases
}

pub(super) fn write_property(
    kind: ObjectType,
    property: P,
    index: Option<u32>,
    octets: Vec<u8>,
) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(kind, INSTANCE),
        property_identifier: property,
        property_array_index: index,
        property_value: octets,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// Each covered WriteProperty takes its staged save, without saving again,
/// and a server rebuilt on the storage serves what it wrote.
#[tokio::test(start_paused = true)]
async fn each_covered_write_survives_a_rebuild() {
    for (kind, property, index, octets) in cases() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let before = fixture.read(oid(kind, INSTANCE), property).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(
            response, SIMPLE_ACK_WRITE,
            "{kind:?} {property:?} {index:?}"
        );
        let written = fixture.read(oid(kind, INSTANCE), property).await;
        assert_ne!(
            written, before,
            "{kind:?} {property:?} {index:?} changed nothing"
        );
        assert_eq!(
            storage.saves.load(Ordering::SeqCst),
            1,
            "{kind:?} {property:?}"
        );
        drop(fixture);
        let rebuilt = served_by(kind, &storage).await;
        assert_eq!(
            rebuilt.read(oid(kind, INSTANCE), property).await,
            written,
            "{kind:?} {property:?} {index:?} after a rebuild"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_write_that_cannot_be_saved_is_refused_on_the_wire_and_the_old_value_stays() {
    for (kind, property, index, octets) in cases() {
        let storage = Arc::new(ConfigStorage::default());
        storage.fail.store(true, Ordering::SeqCst);
        let fixture = served_by(kind, &storage).await;
        let before = fixture.read(oid(kind, INSTANCE), property).await;
        let request = write_property(kind, property, index, octets);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        // An Error PDU: invoke ID 5, WriteProperty, DEVICE (0) and
        // OPERATIONAL_PROBLEM (25).
        assert_eq!(
            response,
            [ERROR_PDU, 5, 15, 0x91, 0, 0x91, 25],
            "{kind:?} {property:?}"
        );
        assert_eq!(fixture.read(oid(kind, INSTANCE), property).await, before);
    }
}

/// Tags storage for the joint request.
type TagsStorage = HeldStorage<TagsSnapshot>;

impl TagsPersistence for TagsStorage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.load_saved())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// An Analog Input keeping both its Tags and its reporting settings.
fn tagged(tags: &Arc<TagsStorage>, config: &Arc<ConfigStorage>) -> Box<dyn BACnetObject> {
    let mut object = AnalogInputObject::with_tags_persistence(
        INSTANCE,
        "AI",
        EngineeringUnits::NO_UNITS,
        Arc::clone(tags) as Arc<dyn TagsPersistence>,
    )
    .and_then(|object| {
        object.with_reporting_persistence(Arc::clone(config) as Arc<dyn WriteConfigPersistence>)
    })
    .unwrap();
    object
        .set_profile(ObjectProfile {
            tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
            ..ObjectProfile::default()
        })
        .unwrap();
    Box::new(object)
}

/// One WritePropertyMultiple writing Tags and two reporting settings saves
/// each storage once, and a rebuild serves all three.
#[tokio::test(start_paused = true)]
async fn a_request_writing_tags_and_reporting_settings_saves_each_once() {
    let tags = Arc::new(TagsStorage::default());
    let config = Arc::new(ConfigStorage::default());
    let fixture = Fixture::new(None);
    fixture
        .db
        .write()
        .await
        .add(tagged(&tags, &config))
        .unwrap();
    let attempt = |property, octets| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets,
        priority: None,
    };
    let floor: &[u8] = &[0x0d, 6, 0, b'f', b'l', b'o', b'o', b'r', 0x21, 3];
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: oid(ObjectType::ANALOG_INPUT, INSTANCE),
        list_of_properties: vec![
            attempt(P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(12))),
            attempt(P::TAGS, floor.to_vec()),
            attempt(
                P::NOTIFY_TYPE,
                value(PropertyValue::Enumerated(NotifyType::EVENT.to_raw())),
            ),
        ],
    }]);
    let fixture = Arc::new(fixture);
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert_eq!(tags.saves.load(Ordering::SeqCst), 1);
    assert_eq!(config.saves.load(Ordering::SeqCst), 1);
    let ai = oid(ObjectType::ANALOG_INPUT, INSTANCE);
    let mut written = Vec::new();
    for property in [P::NOTIFICATION_CLASS, P::TAGS, P::NOTIFY_TYPE] {
        written.push(fixture.read(ai, property).await);
    }
    drop(fixture);
    let rebuilt = Fixture::new(None);
    rebuilt
        .db
        .write()
        .await
        .add(tagged(&tags, &config))
        .unwrap();
    for (property, value) in [P::NOTIFICATION_CLASS, P::TAGS, P::NOTIFY_TYPE]
        .into_iter()
        .zip(written)
    {
        assert_eq!(rebuilt.read(ai, property).await, value, "{property:?}");
    }
}

/// The save runs with the database guard dropped: the database answers a
/// reader and a writer while each covered write's save is held.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reporting_save_runs_while_the_database_stays_available() {
    for (kind, property, index, octets) in cases() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let request = write_property(kind, property, index, octets);
        let (response, available) = while_saving(
            &fixture,
            &storage,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request,
            WAIT,
        )
        .await;
        assert_eq!(
            response, SIMPLE_ACK_WRITE,
            "{kind:?} {property:?} {index:?}"
        );
        assert!(
            available,
            "the database was held while the {kind:?} saved {property:?} {index:?}"
        );
        assert_eq!(
            storage.saves.load(Ordering::SeqCst),
            1,
            "{kind:?} {property:?}"
        );
    }
}

/// A request writing Tags and a reporting setting stages both: the
/// reporting save runs off the guard too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_joint_request_saves_its_reporting_settings_off_the_guard() {
    let tags = Arc::new(TagsStorage::default());
    let config = Arc::new(ConfigStorage::default());
    let fixture = Fixture::new(None);
    fixture
        .db
        .write()
        .await
        .add(tagged(&tags, &config))
        .unwrap();
    let fixture = Arc::new(fixture);
    let attempt = |property, octets| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets,
        priority: None,
    };
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: oid(ObjectType::ANALOG_INPUT, INSTANCE),
        list_of_properties: vec![
            attempt(P::TAGS, vec![0x09, 0]),
            attempt(P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(12))),
        ],
    }]);
    let (response, available) = while_saving(
        &fixture,
        &config,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
        WAIT,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert!(
        available,
        "the database was held while the reporting settings saved"
    );
    assert_eq!(config.saves.load(Ordering::SeqCst), 1);
    assert_eq!(tags.saves.load(Ordering::SeqCst), 1);
}
