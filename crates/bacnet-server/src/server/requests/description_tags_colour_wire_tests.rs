//! A peer-written Description on the types that keep it in the Tags format
//! or the colour output's (#2364): a Device, a Lighting Output, a Binary
//! Lighting Output, a Color and a Color Temperature object. A
//! WritePropertyMultiple of Description and another kept property stages
//! one save on each, a NULL to Description saves nothing, and a Description
//! that can't be saved is refused on the wire with the old one served.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{ERROR_PDU, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::encode_name_value;
use bacnet_objects::color::{
    ColorObject, ColorOutputPersistence, ColorOutputSnapshot, ColorTemperatureObject,
};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::object_profile::{ObjectProfile, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetNameValue;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

const KINDS: [ObjectType; 5] = [
    ObjectType::DEVICE,
    ObjectType::LIGHTING_OUTPUT,
    ObjectType::BINARY_LIGHTING_OUTPUT,
    ObjectType::COLOR,
    ObjectType::COLOR_TEMPERATURE,
];

impl ColorOutputPersistence for HeldStorage<ColorOutputSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(self.load_saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// The storage of one object: Tags storage for the first three types
/// (`reporting_durable_tests` implements it for `HeldStorage`), a colour
/// output's for the other two.
#[derive(Default)]
struct Storages {
    tags: Arc<HeldStorage<TagsSnapshot>>,
    colour: Arc<HeldStorage<ColorOutputSnapshot>>,
}

impl Storages {
    fn saves(&self) -> usize {
        self.tags.saves.load(Ordering::SeqCst) + self.colour.saves.load(Ordering::SeqCst)
    }

    fn fail(&self) {
        self.tags.fail.store(true, Ordering::SeqCst);
        self.colour.fail.store(true, Ordering::SeqCst);
    }
}

/// Instance 1 of `kind`, kept in `storages`, and configured with a
/// Description the way an application does at each start, on the types
/// with a setter: a saved one wins.
async fn served_by(kind: ObjectType, storages: &Storages) -> Arc<Fixture> {
    let profile = ObjectProfile {
        tags: Some(vec![BACnetNameValue::semantic("configured")]),
        ..ObjectProfile::default()
    };
    let tags = storages.tags.clone();
    let colour = storages.colour.clone();
    let object: Box<dyn BACnetObject> = match kind {
        ObjectType::DEVICE => {
            let config = DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(1, 42).unwrap(),
                ..DeviceConfig::default()
            };
            let mut device = DeviceObject::with_tags_persistence(config, tags).unwrap();
            device.set_profile(profile).unwrap();
            device.set_description("configured");
            Box::new(device)
        }
        ObjectType::LIGHTING_OUTPUT => {
            let mut output = LightingOutputObject::with_tags_persistence(1, "LO", tags).unwrap();
            output.set_profile(profile).unwrap();
            output.set_description("configured");
            Box::new(output)
        }
        ObjectType::BINARY_LIGHTING_OUTPUT => {
            let mut output =
                BinaryLightingOutputObject::with_tags_persistence(1, "BLO", tags).unwrap();
            output.set_profile(profile).unwrap();
            output.set_description("configured");
            Box::new(output)
        }
        ObjectType::COLOR => {
            let color = ColorObject::new(1, "CLR").unwrap();
            Box::new(color.with_output_persistence(colour).unwrap())
        }
        _ => {
            let color = ColorTemperatureObject::new(1, "CT").unwrap();
            Box::new(color.with_output_persistence(colour).unwrap())
        }
    };
    let fixture = Fixture::new(None);
    fixture.db.write().await.add(object).unwrap();
    Arc::new(fixture)
}

/// Another property `kind` keeps, and the octets a write gives it.
fn other(kind: ObjectType) -> (P, Vec<u8>) {
    match kind {
        ObjectType::COLOR => {
            let mut xy = value(PropertyValue::Real(0.25));
            xy.extend(value(PropertyValue::Real(0.5)));
            (P::DEFAULT_COLOR, xy)
        }
        ObjectType::COLOR_TEMPERATURE => (
            P::DEFAULT_COLOR_TEMPERATURE,
            value(PropertyValue::Unsigned(3000)),
        ),
        _ => {
            let mut tags = BytesMut::new();
            encode_name_value(&mut tags, &BACnetNameValue::semantic("written")).unwrap();
            (P::TAGS, tags.to_vec())
        }
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
        let fixture = served_by(kind, &storages).await;
        let (property, octets) = other(kind);
        let before = fixture.read(oid(kind, 1), property).await;
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
        assert_eq!(storages.saves(), 1, "{kind:?}");
        let served = fixture.read(oid(kind, 1), property).await;
        assert_ne!(served, before, "{kind:?}");
        drop(fixture);
        let rebuilt = served_by(kind, &storages).await;
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
        let fixture = served_by(kind, &storages).await;
        let before = fixture.read(oid(kind, 1), P::DESCRIPTION).await;
        let request = write_description(kind, PropertyValue::Null);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?}");
        assert_eq!(storages.saves(), 0, "{kind:?}");
        assert_eq!(fixture.read(oid(kind, 1), P::DESCRIPTION).await, before);
    }
}

/// A Description whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the one written before stays served, here and
/// after a rebuild.
#[tokio::test(start_paused = true)]
async fn a_description_that_cannot_be_saved_is_refused_on_the_wire() {
    for kind in KINDS {
        let storages = Storages::default();
        let fixture = served_by(kind, &storages).await;
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let request = write_description(kind, text("lobby"));
        assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WRITE);
        assert_eq!(storages.saves(), 1, "{kind:?}");
        storages.fail();
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
        drop(fixture);
        let rebuilt = served_by(kind, &storages).await;
        assert_eq!(
            rebuilt.read(oid(kind, 1), P::DESCRIPTION).await,
            text("lobby"),
            "{kind:?}"
        );
    }
}

/// A WritePropertyMultiple of Description and another kept property whose
/// one staged save fails, in either order: the first of the two rows is
/// refused with DEVICE / OPERATIONAL_PROBLEM, nothing is saved, and both
/// old values stay served, here and after a rebuild.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_whose_save_fails_changes_nothing() {
    for kind in KINDS {
        for description_first in [true, false] {
            let storages = Storages::default();
            let fixture = served_by(kind, &storages).await;
            let (property, octets) = other(kind);
            let old = (
                fixture.read(oid(kind, 1), P::DESCRIPTION).await,
                fixture.read(oid(kind, 1), property).await,
            );
            let mut rows = vec![
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
            ];
            if !description_first {
                rows.reverse();
            }
            let first = rows[0].property_identifier;
            let request = wpm(vec![WriteAccessSpecification {
                object_identifier: oid(kind, 1),
                list_of_properties: rows,
            }]);
            storages.fail();
            let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
            let response = wire(&fixture, service, request).await;
            let what = format!("{kind:?}, Description first: {description_first}");
            let Apdu::Error(error) = decode_apdu(Bytes::from(response)).unwrap() else {
                panic!("{what}: expected a WritePropertyMultiple error");
            };
            let refused = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
            assert_eq!(
                (refused.error_class, refused.error_code),
                (ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{what}"
            );
            let failed = refused.first_failed_write_attempt;
            assert_eq!(failed.object_identifier, oid(kind, 1), "{what}");
            assert_eq!(failed.property_identifier, first.to_raw(), "{what}");
            let served = |fixture: Arc<Fixture>| async move {
                (
                    fixture.read(oid(kind, 1), P::DESCRIPTION).await,
                    fixture.read(oid(kind, 1), property).await,
                )
            };
            assert_eq!(served(fixture).await, old, "{what}");
            assert_eq!(storages.saves(), 0, "{what}");
            assert_eq!(storages.tags.load_saved(), None, "{what}");
            assert_eq!(storages.colour.load_saved(), None, "{what}");
            let rebuilt = served_by(kind, &storages).await;
            assert_eq!(served(rebuilt).await, old, "{what}, rebuilt");
        }
    }
}
