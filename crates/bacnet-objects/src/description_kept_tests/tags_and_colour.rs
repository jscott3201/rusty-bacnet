//! The types whose written Description joins a format this stack already
//! kept for them (#2364): the Device and the two lighting outputs in the
//! Tags format (`RBNTAG02`), and the Color and Color Temperature objects in
//! the colour output's (`RBNCLR03`). The shared checks run on them through
//! [`cases`]; the ones here are the formats' own.

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::ObjectType;

use super::*;
use crate::color::{
    ColorObject, ColorOutputPersistence, ColorOutputSnapshot, ColorTemperatureObject,
};
use crate::device::{DeviceConfig, DeviceObject};
use crate::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use crate::object_profile::{
    MAX_TAGS_SNAPSHOT_BYTES, ObjectProfile, ProfileState, TagsPersistence, TagsSnapshot,
};
use crate::snapshot;

impl TagsPersistence for Memory<TagsSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

impl ColorOutputPersistence for Memory<ColorOutputSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(self.saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// The staging an object with a profile lends, which the server reaches
/// through `durable_writes()`.
#[cfg(feature = "durable-writes")]
fn lent(object: &mut dyn BACnetObject) -> Option<&mut dyn DurableWrites> {
    object.durable_writes()
}

#[cfg(not(feature = "durable-writes"))]
fn lent(_: &mut dyn BACnetObject) -> Option<&mut dyn DurableWrites> {
    None
}

kept_object!(
    DeviceObject,
    |device| lent(device),
    |device, text| device.set_description(text),
    |device| Some(device.clear_written_description())
);
kept_object!(
    LightingOutputObject,
    |output| lent(output),
    |output, text| output.set_description(text),
    |output| Some(output.clear_written_description())
);
kept_object!(
    BinaryLightingOutputObject,
    |output| lent(output),
    |output, text| output.set_description(text),
    |output| Some(output.clear_written_description())
);
// The colour objects have no Description setter.
kept_object!(
    ColorObject,
    |color| Some(color),
    |_color, _text| (),
    |color| Some(color.clear_written_description())
);
kept_object!(
    ColorTemperatureObject,
    |color| Some(color),
    |_color, _text| (),
    |color| Some(color.clear_written_description())
);

/// A Tags row for the application to provision, so a peer may write it.
fn configured() -> ObjectProfile {
    ObjectProfile {
        tags: Some(vec![BACnetNameValue::semantic("configured")]),
        ..ObjectProfile::default()
    }
}

fn device(storage: Arc<Memory<TagsSnapshot>>) -> Result<DeviceObject, Error> {
    let mut device = DeviceObject::with_tags_persistence(DeviceConfig::default(), storage)?;
    device.set_profile(configured())?;
    Ok(device)
}

fn lighting_output(storage: Arc<Memory<TagsSnapshot>>) -> Result<LightingOutputObject, Error> {
    let mut output = LightingOutputObject::with_tags_persistence(1, "LO", storage)?;
    output.set_profile(configured())?;
    Ok(output)
}

fn color(storage: Arc<Memory<ColorOutputSnapshot>>) -> Result<ColorObject, Error> {
    ColorObject::new(1, "CLR")?.with_output_persistence(storage)
}

/// The five types on fresh storage, or on storage holding `saved` as their
/// Description.
pub(super) fn cases(saved: Option<&str>) -> Vec<Box<dyn Kept>> {
    let tagged = || {
        Memory::holding(saved.map(|text| TagsSnapshot {
            tags: None,
            description: Some(text.into()),
        }))
    };
    let coloured = || {
        Memory::holding(saved.map(|text| ColorOutputSnapshot {
            description: Some(text.into()),
            ..ColorOutputSnapshot::default()
        }))
    };
    // The whole array, written empty.
    let no_tags = (P::TAGS, PropertyValue::ApplicationData(Vec::new()));
    let tags_tag: fn(&TagsSnapshot) -> [u8; 8] =
        |saved| tag_of(snapshot::tags::encode(oid(ObjectType::DEVICE), saved));
    let colour_tag: fn(&ColorOutputSnapshot) -> [u8; 8] =
        |saved| tag_of(snapshot::color::encode(oid(ObjectType::COLOR), saved));
    vec![
        Box::new(Case {
            storage: tagged(),
            open: |storage| Ok(Box::new(device(storage)?) as _),
            description: |saved: &TagsSnapshot| saved.description.as_ref(),
            other: no_tags.clone(),
            tag: Some(tags_tag),
        }),
        Box::new(Case {
            storage: tagged(),
            open: |storage| Ok(Box::new(lighting_output(storage)?) as _),
            description: |saved: &TagsSnapshot| saved.description.as_ref(),
            other: no_tags.clone(),
            tag: Some(tags_tag),
        }),
        Box::new(Case {
            storage: tagged(),
            open: |storage| {
                let mut output =
                    BinaryLightingOutputObject::with_tags_persistence(1, "BLO", storage)?;
                output.set_profile(configured())?;
                Ok(Box::new(output) as _)
            },
            description: |saved: &TagsSnapshot| saved.description.as_ref(),
            other: no_tags,
            tag: Some(tags_tag),
        }),
        Box::new(Case {
            storage: coloured(),
            open: |storage| Ok(Box::new(color(storage)?) as _),
            description: |saved: &ColorOutputSnapshot| saved.description.as_ref(),
            other: (
                P::DEFAULT_COLOR,
                PropertyValue::List(vec![PropertyValue::Real(0.25), PropertyValue::Real(0.5)]),
            ),
            tag: Some(colour_tag),
        }),
        Box::new(Case {
            storage: coloured(),
            open: |storage| {
                let color =
                    ColorTemperatureObject::new(1, "CT")?.with_output_persistence(storage)?;
                Ok(Box::new(color) as _)
            },
            description: |saved: &ColorOutputSnapshot| saved.description.as_ref(),
            other: (P::DEFAULT_COLOR_TEMPERATURE, PropertyValue::Unsigned(3000)),
            tag: Some(colour_tag),
        }),
    ]
}

/// A Tags user that keeps its Description elsewhere, or nowhere, refuses
/// storage holding one, writes nothing to it, and never stages or saves a
/// Description there: its file stays version 1.
#[test]
fn a_tags_user_that_keeps_no_description_refuses_and_never_saves_one() {
    let described = || {
        Memory::holding(Some(TagsSnapshot {
            tags: None,
            description: Some("lobby".into()),
        }))
    };
    let object = oid(ObjectType::ANALOG_INPUT);
    let storage = described();
    assert!(ProfileState::persistent(object, storage.clone(), false).is_err());
    assert!(ProfileState::persistent(object, storage.clone(), true).is_ok());
    let units = bacnet_types::enums::EngineeringUnits::DEGREES_CELSIUS;
    let input =
        crate::analog::AnalogInputObject::with_tags_persistence(1, "AI", units, described());
    assert!(input.is_err());
    assert!(ColorObject::with_tags_persistence(1, "CLR", described()).is_err());
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);

    let storage = Memory::holding(None);
    let mut state = ProfileState::persistent(object, storage.clone(), false).unwrap();
    state.provision(configured()).unwrap();
    let step = state.stage_writes(&[pending(P::DESCRIPTION, text("lobby"))]);
    assert!(matches!(step, StageStep::Skip));
    let mut served = String::new();
    assert!(
        state
            .write_description(&mut served, &text("lobby"))
            .is_none()
    );
    let tags = PropertyValue::ApplicationData(Vec::new());
    state.write(P::TAGS, None, &tags).unwrap().unwrap();
    let saved = storage.saved().unwrap();
    assert_eq!(saved.description, None);
    assert_eq!(
        tag_of(snapshot::tags::encode(object, &saved)),
        snapshot::tags::MAGIC
    );
}

/// A tag whose name brings a Tags file to `octets` in all, with nothing
/// else in it.
fn tag_filling(octets: usize) -> PropertyValue {
    // The 12-octet header, the presence octet, and a long name's 6-octet
    // context header and character set octet.
    let name = "x".repeat(octets - 12 - 1 - 7);
    let mut encoded = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_name_value(&mut encoded, &BACnetNameValue::semantic(name))
        .unwrap();
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// The Description shares the Tags file's 1 MiB: a write of either that
/// would not fit beside the other is refused before anything is saved.
#[test]
fn a_description_and_tags_past_the_file_cap_are_refused() {
    let no_space = |result: &Result<(), Error>| {
        refused_with(
            result,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        )
    };
    let storage = Memory::holding(None);
    let mut output = lighting_output(storage.clone()).unwrap();
    write(&mut output, P::TAGS, tag_filling(MAX_TAGS_SNAPSHOT_BYTES)).unwrap();
    assert!(no_space(&write(&mut output, P::DESCRIPTION, text("lobby"))));
    assert_eq!(description(&output), text(""));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);

    // "lobby" takes four length octets and five of text.
    let storage = Memory::holding(None);
    let mut output = lighting_output(storage.clone()).unwrap();
    write(&mut output, P::DESCRIPTION, text("lobby")).unwrap();
    let full = tag_filling(MAX_TAGS_SNAPSHOT_BYTES - 8);
    assert!(no_space(&write(&mut output, P::TAGS, full)));
    let fits = tag_filling(MAX_TAGS_SNAPSHOT_BYTES - 9);
    write(&mut output, P::TAGS, fits).unwrap();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    let saved = storage.saved().unwrap();
    let octets = snapshot::tags::encode(oid(ObjectType::LIGHTING_OUTPUT), &saved).unwrap();
    assert_eq!(octets.len(), MAX_TAGS_SNAPSHOT_BYTES);
}

/// A Tags write on an object holding a saved Description keeps it in the
/// file, and a Present_Value save on a colour object does too.
#[test]
fn a_later_save_keeps_the_saved_description() {
    let storage = Memory::holding(None);
    let mut device = device(storage.clone()).unwrap();
    write(&mut device, P::DESCRIPTION, text("lobby")).unwrap();
    let tags = PropertyValue::ApplicationData(Vec::new());
    write(&mut device, P::TAGS, tags).unwrap();
    let saved = storage.saved().unwrap();
    assert_eq!(saved.description.as_deref(), Some("lobby"));
    assert_eq!(saved.tags, Some(Vec::new()));

    let storage = Memory::holding(None);
    let mut color = color(storage.clone()).unwrap();
    write(&mut color, P::DESCRIPTION, text("lobby")).unwrap();
    color
        .set_present_value(bacnet_types::constructed::BACnetXyColor::new(0.25, 0.5))
        .unwrap();
    color.wait_for_output_saves();
    let saved = storage.saved().unwrap();
    assert_eq!(saved.description.as_deref(), Some("lobby"));
    assert!(saved.present_value.is_some());
}

/// A directory of its own under the system's temporary one, removed on
/// drop.
// std: the file backends.
#[cfg(feature = "std")]
struct Scratch(std::path::PathBuf);

#[cfg(feature = "std")]
impl Scratch {
    fn new(label: &str) -> Self {
        let name = format!("rusty-bacnet-2364-{label}-{}", std::process::id());
        let directory = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

#[cfg(feature = "std")]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// On the file backend, a Device's file takes version 2 only while it
/// holds a Description: before the write and after the clear it is the
/// version 1 file, octet for octet.
#[cfg(feature = "std")]
#[test]
fn a_tags_file_is_version_2_only_while_it_holds_a_description() {
    use crate::object_profile::FileTagsPersistence;
    let scratch = Scratch::new("tags");
    let path = scratch.0.join("device");
    let open = || {
        let store = Arc::new(FileTagsPersistence::new(&path).unwrap());
        let mut device = DeviceObject::with_tags_persistence(DeviceConfig::default(), store)?;
        device.set_profile(configured())?;
        Ok::<_, Error>(device)
    };
    let mut device = open().unwrap();
    let tags = PropertyValue::ApplicationData(Vec::new());
    write(&mut device, P::TAGS, tags).unwrap();
    let plain = std::fs::read(&path).unwrap();
    assert!(plain.starts_with(&snapshot::tags::MAGIC));

    write(&mut device, P::DESCRIPTION, text("lobby")).unwrap();
    let described = std::fs::read(&path).unwrap();
    assert!(described.starts_with(&snapshot::tags::MAGIC_WITH_DESCRIPTION));
    drop(device);
    let mut device = open().unwrap();
    assert_eq!(description(&device), text("lobby"));

    device.clear_written_description().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), plain);
    drop(device);
    assert_eq!(description(&open().unwrap()), text(""));
}

/// The same for a Color object's file and version 3, which
/// `clear_written_default` leaves in place.
#[cfg(feature = "std")]
#[test]
fn a_colour_file_is_version_3_only_while_it_holds_a_description() {
    use crate::color::FileColorOutputPersistence;
    let scratch = Scratch::new("colour");
    let path = scratch.0.join("color");
    let open = || {
        let store = Arc::new(FileColorOutputPersistence::new(&path).unwrap());
        ColorObject::new(1, "CLR")?.with_output_persistence(store)
    };
    let (property, default) = (
        P::DEFAULT_COLOR,
        PropertyValue::List(vec![PropertyValue::Real(0.25), PropertyValue::Real(0.5)]),
    );
    let mut color = open().unwrap();
    write(&mut color, property, default.clone()).unwrap();
    let with_default = std::fs::read(&path).unwrap();
    assert!(with_default.starts_with(&snapshot::color::MAGIC));

    write(&mut color, P::DESCRIPTION, text("lobby")).unwrap();
    let described = std::fs::read(&path).unwrap();
    assert!(described.starts_with(&snapshot::color::MAGIC_WITH_DESCRIPTION));
    drop(color);
    let mut color = open().unwrap();
    assert_eq!(description(&color), text("lobby"));
    assert_eq!(color.read_property(property, None).unwrap(), default);

    // The default's clear drops only the default.
    color.clear_written_default().unwrap();
    let kept = std::fs::read(&path).unwrap();
    assert!(kept.starts_with(&snapshot::color::MAGIC_WITH_DESCRIPTION));
    write(&mut color, property, default).unwrap();
    color.clear_written_description().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), with_default);
    drop(color);
    assert_eq!(description(&open().unwrap()), text(""));
}

/// An object without Tags storage pays one pointer for it, whatever a
/// snapshot holds: its profile, its write count and that pointer, which an
/// alignment of eight may pad.
#[test]
fn an_object_without_tags_storage_pays_one_pointer_for_it() {
    use core::mem::size_of;
    let rows = size_of::<ObjectProfile>() + size_of::<u64>();
    assert!(size_of::<ProfileState>() <= rows + size_of::<u64>());
}
