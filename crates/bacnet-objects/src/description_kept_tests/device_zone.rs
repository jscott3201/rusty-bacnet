//! A Device keeps the halves of its clock's zone a peer writes in its Tags
//! storage (#2395), as it keeps a written Description: Tags format version
//! 3, written only while a half is held; saved before the write is
//! answered; refused with DEVICE / OPERATIONAL_PROBLEM, the clock never
//! moved, when the save fails; one save for a request's writes; nothing
//! kept without the clock's opt-in, and a half held then left in storage
//! through later saves; dropped by the host's reset. Only a Device keeps
//! one there.

use std::sync::Mutex as StdMutex;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::database::ObjectDatabase;
use crate::device::{DeviceConfig, DeviceObject};
use crate::lighting::LightingOutputObject;
use crate::object_profile::{TagsSnapshot, WrittenZone};
use crate::snapshot::tags;

/// A clock standing at noon on Thursday 2024-07-04 in its zone, taking a
/// peer's zone writes when `writable`.
struct ZoneClock {
    zone: StdMutex<(i16, bool)>,
    writable: bool,
}

impl ZoneClock {
    fn new(writable: bool) -> Arc<Self> {
        Arc::new(Self {
            zone: StdMutex::new((300, false)),
            writable,
        })
    }

    fn zone(&self) -> (i16, bool) {
        *self.zone.lock().unwrap()
    }
}

impl ClockReader for ZoneClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        let (utc_offset, daylight_savings_status) = self.zone();
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 7,
                day: 4,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset,
            daylight_savings_status,
        })
    }

    fn takes_zone_writes(&self) -> bool {
        self.writable
    }

    fn check_zone(&self, _: i16, _: bool) -> Result<(), Error> {
        Ok(())
    }

    fn write_zone(&self, utc_offset: i16, daylight_savings_status: bool) -> Result<(), Error> {
        *self.zone.lock().unwrap() = (utc_offset, daylight_savings_status);
        Ok(())
    }
}

/// The default Device on `storage`, in a database on `clock`.
fn served(
    storage: &Arc<Memory<TagsSnapshot>>,
    clock: &Arc<ZoneClock>,
) -> (ObjectDatabase, ObjectIdentifier) {
    let device =
        DeviceObject::with_tags_persistence(DeviceConfig::default(), storage.clone()).unwrap();
    let oid = device.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.set_clock_reader(Some(Arc::clone(clock) as Arc<dyn ClockReader>));
    (db, oid)
}

fn device_of<'a>(db: &'a mut ObjectDatabase, oid: &ObjectIdentifier) -> &'a mut dyn BACnetObject {
    db.get_mut(oid).unwrap()
}

fn saves(storage: &Memory<TagsSnapshot>) -> usize {
    storage.saves.load(Ordering::SeqCst)
}

fn file_tag(saved: &TagsSnapshot) -> [u8; 8] {
    tag_of(tags::encode(oid(ObjectType::DEVICE), saved))
}

#[test]
fn a_written_zone_half_survives_a_rebuild_in_version_3() {
    let storage = Memory::holding(None);
    let clock = ZoneClock::new(true);
    let (mut db, device) = served(&storage, &clock);
    write(
        device_of(&mut db, &device),
        P::DAYLIGHT_SAVINGS_STATUS,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_eq!(clock.zone(), (300, true));
    assert_eq!(saves(&storage), 1);
    let saved = storage.saved().unwrap();
    assert_eq!(
        saved.zone,
        WrittenZone {
            utc_offset: None,
            daylight_savings_status: Some(true),
        }
    );
    assert_eq!(file_tag(&saved), tags::MAGIC_WITH_ZONE);
    write(
        device_of(&mut db, &device),
        P::UTC_OFFSET,
        PropertyValue::Signed(-60),
    )
    .unwrap();
    assert_eq!(clock.zone(), (-60, true));
    drop(db);

    let rebuilt =
        DeviceObject::with_tags_persistence(DeviceConfig::default(), storage.clone()).unwrap();
    assert_eq!(
        rebuilt.written_zone(),
        WrittenZone {
            utc_offset: Some(-60),
            daylight_savings_status: Some(true),
        }
    );
    // Over the zone a host gives, each held half wins.
    assert_eq!(rebuilt.written_zone().over((300, false)), (-60, true));
    let octets = tags::encode(oid(ObjectType::DEVICE), &storage.saved().unwrap()).unwrap();
    assert_eq!(
        tags::decode(oid(ObjectType::DEVICE), &octets).unwrap(),
        storage.saved().unwrap()
    );
}

#[test]
fn a_zone_half_whose_save_fails_is_refused_and_the_clock_never_moves() {
    let storage = Memory::holding(None);
    let clock = ZoneClock::new(true);
    let (mut db, device) = served(&storage, &clock);
    storage.fail.store(true, Ordering::SeqCst);
    let result = write(
        device_of(&mut db, &device),
        P::UTC_OFFSET,
        PropertyValue::Signed(360),
    );
    assert!(refused_with(
        &result,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM
    ));
    assert_eq!(clock.zone(), (300, false));
    assert_eq!(storage.saved(), None);
    assert!(
        db.get(&device)
            .unwrap()
            .read_property(P::UTC_OFFSET, None)
            .unwrap()
            == PropertyValue::Signed(300)
    );
}

/// Without the opt-in the write is refused as before, and nothing is
/// staged or saved; a NULL or a refused value saves nothing either.
#[test]
fn nothing_is_kept_without_the_opt_in_or_for_a_refused_write() {
    let storage = Memory::holding(None);
    let clock = ZoneClock::new(false);
    let (mut db, device) = served(&storage, &clock);
    let result = write(
        device_of(&mut db, &device),
        P::UTC_OFFSET,
        PropertyValue::Signed(360),
    );
    assert!(refused_with(
        &result,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED
    ));
    #[cfg(feature = "durable-writes")]
    {
        let staging = device_of(&mut db, &device).durable_writes().unwrap();
        let step = staging.stage_writes(&[pending(P::UTC_OFFSET, PropertyValue::Signed(360))]);
        assert!(matches!(step, StageStep::Skip));
    }
    assert_eq!(saves(&storage), 0);

    let storage = Memory::holding(None);
    let clock = ZoneClock::new(true);
    let (mut db, device) = served(&storage, &clock);
    for (property, value) in [
        (P::UTC_OFFSET, PropertyValue::Signed(7)),
        (P::UTC_OFFSET, PropertyValue::Boolean(true)),
        (P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Null),
    ] {
        assert!(write(device_of(&mut db, &device), property, value).is_err());
    }
    assert_eq!(saves(&storage), 0);
    assert_eq!(clock.zone(), (300, false));
}

/// A request writing both halves and a Description stages one save, and
/// each write takes it without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_writing_both_halves_saves_once() {
    let storage = Memory::holding(None);
    let clock = ZoneClock::new(true);
    let (mut db, device) = served(&storage, &clock);
    let writes = [
        pending(P::UTC_OFFSET, PropertyValue::Signed(-60)),
        pending(P::DESCRIPTION, text("lobby")),
        pending(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
    ];
    let staging = device_of(&mut db, &device).durable_writes().unwrap();
    let StageStep::Staged(wait) = staging.stage_writes(&writes) else {
        panic!("expected a staged write");
    };
    wait.block();
    assert_eq!(saves(&storage), 1);
    for write in writes {
        device_of(&mut db, &device)
            .write_property(write.property, None, write.value, None)
            .unwrap();
    }
    let staging = device_of(&mut db, &device).durable_writes().unwrap();
    staging.release_staged_write(&wait);
    assert_eq!(clock.zone(), (-60, true));
    drop(db);
    assert_eq!(saves(&storage), 1);
    let saved = storage.saved().unwrap();
    assert_eq!(saved.zone.over((0, false)), (-60, true));
    assert_eq!(saved.description.as_deref(), Some("lobby"));
}

/// The host's reset drops the halves, and the file goes back to the
/// version without them; one whose save fails keeps them. With the opt-in
/// off the reset leaves a half held alone, and so does every later save:
/// the half stays in the file, unapplied.
#[test]
fn the_reset_drops_the_halves_and_a_half_held_while_off_stays() {
    let zoned = || {
        Memory::holding(Some(TagsSnapshot {
            tags: None,
            description: Some("lobby".into()),
            zone: WrittenZone {
                utc_offset: Some(360),
                daylight_savings_status: None,
            },
        }))
    };
    let reset = |db: &mut ObjectDatabase, device: &ObjectIdentifier| {
        db.get_mut(device)
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .reset_written_zone()
    };
    let storage = zoned();
    let (mut db, device) = served(&storage, &ZoneClock::new(true));
    reset(&mut db, &device).unwrap();
    assert_eq!(saves(&storage), 1);
    let saved = storage.saved().unwrap();
    assert!(saved.zone.is_empty());
    assert_eq!(saved.description.as_deref(), Some("lobby"));
    assert_eq!(file_tag(&saved), tags::MAGIC_WITH_DESCRIPTION);
    // Nothing left to drop: no save.
    reset(&mut db, &device).unwrap();
    assert_eq!(saves(&storage), 1);

    let storage = zoned();
    let (mut db, device) = served(&storage, &ZoneClock::new(true));
    storage.fail.store(true, Ordering::SeqCst);
    assert!(reset(&mut db, &device).is_err());
    storage.fail.store(false, Ordering::SeqCst);
    assert_eq!(storage.saved().unwrap().zone.utc_offset, Some(360));

    // Opt-in off: the reset leaves it, and a Description save keeps it.
    let storage = zoned();
    let (mut db, device) = served(&storage, &ZoneClock::new(false));
    reset(&mut db, &device).unwrap();
    write(device_of(&mut db, &device), P::DESCRIPTION, text("hall")).unwrap();
    assert_eq!(saves(&storage), 1);
    let saved = storage.saved().unwrap();
    assert_eq!(saved.zone.utc_offset, Some(360));
    assert_eq!(saved.description.as_deref(), Some("hall"));
    assert_eq!(file_tag(&saved), tags::MAGIC_WITH_ZONE);
}

/// The host's reset staged off the guard: one save, which the reset then
/// takes without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_reset_saves_once() {
    let storage = Memory::holding(Some(TagsSnapshot {
        zone: WrittenZone {
            utc_offset: None,
            daylight_savings_status: Some(true),
        },
        ..TagsSnapshot::default()
    }));
    let (mut db, device) = served(&storage, &ZoneClock::new(true));
    let object = db.get_mut(&device).unwrap();
    let mut authority = object.device_authority_internal().unwrap();
    let StageStep::Staged(wait) = authority.stage_zone_reset() else {
        panic!("expected a staged reset");
    };
    wait.block();
    assert_eq!(saves(&storage), 1);
    assert!(storage.saved().unwrap().zone.is_empty());
    authority.reset_written_zone().unwrap();
    authority.release_staged(&wait);
    assert_eq!(saves(&storage), 1);
}

/// Only a Device keeps a zone in Tags storage: any other user refuses
/// storage holding one, and a zone a write would never leave fails a
/// Device's build too.
#[test]
fn only_a_device_keeps_a_zone_and_only_one_a_write_leaves() {
    let zoned = |utc_offset| {
        Memory::holding(Some(TagsSnapshot {
            zone: WrittenZone {
                utc_offset: Some(utc_offset),
                daylight_savings_status: None,
            },
            ..TagsSnapshot::default()
        }))
    };
    assert!(LightingOutputObject::with_tags_persistence(1, "LO", zoned(60)).is_err());
    assert!(DeviceObject::with_tags_persistence(DeviceConfig::default(), zoned(60)).is_ok());
    assert!(DeviceObject::with_tags_persistence(DeviceConfig::default(), zoned(7)).is_err());
    assert!(DeviceObject::with_tags_persistence(DeviceConfig::default(), zoned(1455)).is_err());
}

/// Version 3's octets: the octet saying what follows, each half held, then
/// version 2's or version 1's body; a marker with no zone half, or an
/// unknown bit, and a daylight-saving octet past 1 are damage.
#[test]
fn version_3_holds_the_halves_ahead_of_the_rest() {
    let object = oid(ObjectType::DEVICE);
    let snapshot = TagsSnapshot {
        tags: Some(Vec::new()),
        description: Some("d".into()),
        zone: WrittenZone {
            utc_offset: Some(-570),
            daylight_savings_status: Some(true),
        },
    };
    let octets = tags::encode(object, &snapshot).unwrap();
    assert_eq!(octets[..8], tags::MAGIC_WITH_ZONE);
    // Marker 7, -570 minutes, summer, then the Description and the Tags.
    assert_eq!(octets[12..16], [7, 0xFD, 0xC6, 1]);
    assert_eq!(octets[16..], [0, 0, 0, 1, b'd', 1]);
    assert_eq!(tags::decode(object, &octets).unwrap(), snapshot);
    for (at, octet) in [(12, 4), (12, 0x0B), (15, 2)] {
        let mut damaged = octets.clone();
        damaged[at] = octet;
        let error = tags::decode(object, &damaged).unwrap_err();
        assert!(error.is_damage(), "{at}: {error}");
    }
    let mut cut = octets.clone();
    cut.truncate(14);
    assert!(tags::decode(object, &cut).unwrap_err().is_damage());
    // An offset off the grid never saves, and never loads.
    let mut off_grid = snapshot.clone();
    off_grid.zone.utc_offset = Some(-569);
    assert!(tags::encode(object, &off_grid).is_err());
    let mut forged = octets.clone();
    forged[13..15].copy_from_slice(&(-569i16).to_be_bytes());
    assert!(tags::decode(object, &forged).unwrap_err().is_damage());
}
