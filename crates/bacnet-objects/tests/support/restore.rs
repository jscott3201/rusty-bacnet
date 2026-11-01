//! Every format the file persistence backends save, a sample of each written
//! through the public API, and the check a restore from any octets must pass
//! (#2180, #2183).
//!
//! Shared by `tests/persistence_corruption.rs` and the fuzz crate
//! (`crates/bacnet-objects/fuzz`), which builds this file with `#[path]`.
//!
//! The check, [`Format::check`], loads what the files hold through the
//! backend, then builds the object on the same storage:
//! - a load that succeeds names the expected object, and its snapshot saves
//!   to fresh storage and loads back equal (compared by `Debug` text, so a
//!   NaN matches itself);
//! - a load that fails makes the object's build fail too;
//! - a build that fails leaves the files as they were: a refused restore
//!   writes nothing;
//! - a build that succeeds from a loaded snapshot serves its state: a log's
//!   Total_Record_Count, an Audit Log's records, counts and generation.
//!
//! A panic anywhere, or an allocation past the fuzzer's limit, fails too.

#![allow(dead_code)]

#[path = "restore_samples.rs"]
mod samples;

use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_objects::access_control::{
    AccessRightsObject, AccessRightsPersistence, AccessRightsSnapshot, FileAccessRightsPersistence,
};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, FileAuditLogPersistence};
use bacnet_objects::color::{
    ColorObject, ColorOutputPersistence, ColorOutputSnapshot, ColorTemperatureObject,
    FileColorOutputPersistence,
};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::log_persistence::{FileLogPersistence, LogPersistence};
use bacnet_objects::notification_class::{
    FileNotificationClassPersistence, NotificationClass, NotificationClassPersistence,
    NotificationClassSnapshot,
};
use bacnet_objects::notification_forwarder::{
    FileNotificationForwarderPersistence, ForwarderSnapshot, NotificationForwarderObject,
    NotificationForwarderPersistence,
};
use bacnet_objects::object_profile::{FileTagsPersistence, TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// One saved format.
pub struct Format {
    /// What it holds, for messages and seed names.
    pub name: &'static str,
    /// Two checksummed slots (`base.slot0`, `base.slot1`) rather than one
    /// file at `base`. Damage to such a slot never loads.
    pub two_slot: bool,
    /// Write a sample through the backend, at `base`.
    pub sample: fn(&Path),
    /// Restore from what `base` holds; see the module docs.
    pub check: fn(&Path) -> Outcome,
}

/// What a restore came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// The backend loaded a snapshot.
    pub loaded: bool,
    /// The object built on the storage.
    pub built: bool,
}

/// Every format, in a fixed order the fuzz targets' selectors index.
pub const FORMATS: &[Format] = &[
    Format {
        name: "trend_log",
        two_slot: true,
        sample: samples::trend_log,
        check: |base| {
            check_log(base, oid(ObjectType::TREND_LOG), |size, storage| {
                TrendLogObject::with_persistence(1, "TL", size, storage)
                    .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
            })
        },
    },
    Format {
        name: "trend_log_multiple",
        two_slot: true,
        sample: samples::trend_log_multiple,
        check: |base| {
            check_log(
                base,
                oid(ObjectType::TREND_LOG_MULTIPLE),
                |size, storage| {
                    TrendLogMultipleObject::with_persistence(1, "TLM", size, storage)
                        .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
                },
            )
        },
    },
    Format {
        name: "event_log",
        two_slot: true,
        sample: samples::event_log,
        check: |base| {
            check_log(base, oid(ObjectType::EVENT_LOG), |size, storage| {
                EventLogObject::with_persistence(1, "EL", size, storage)
                    .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
            })
        },
    },
    Format {
        name: "audit_log",
        two_slot: true,
        sample: samples::audit_log,
        check: check_audit,
    },
    Format {
        name: "notification_forwarder",
        two_slot: false,
        sample: samples::notification_forwarder,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::NOTIFICATION_FORWARDER),
                |path| FileNotificationForwarderPersistence::new(path).unwrap(),
                |store| {
                    NotificationForwarderObject::with_persistence(1, "NF", Arc::new(store))
                        .map(drop)
                },
            )
        },
    },
    Format {
        name: "notification_class",
        two_slot: false,
        sample: samples::notification_class,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::NOTIFICATION_CLASS),
                |path| FileNotificationClassPersistence::new(path).unwrap(),
                |store| NotificationClass::with_persistence(1, "NC", Arc::new(store)).map(drop),
            )
        },
    },
    Format {
        name: "tags",
        two_slot: false,
        sample: samples::tags,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::ANALOG_INPUT),
                |path| FileTagsPersistence::new(path).unwrap(),
                |store| {
                    let units = EngineeringUnits::DEGREES_CELSIUS;
                    AnalogInputObject::with_tags_persistence(1, "AI", units, Arc::new(store))
                        .map(drop)
                },
            )
        },
    },
    Format {
        name: "color",
        two_slot: false,
        sample: samples::color,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::COLOR),
                |path| FileColorOutputPersistence::new(path).unwrap(),
                |store| {
                    ColorObject::new(1, "CLR")?
                        .with_output_persistence(Arc::new(store))
                        .map(drop)
                },
            )
        },
    },
    Format {
        name: "color_temperature",
        two_slot: false,
        sample: samples::color_temperature,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::COLOR_TEMPERATURE),
                |path| FileColorOutputPersistence::new(path).unwrap(),
                |store| {
                    ColorTemperatureObject::new(1, "CT")?
                        .with_output_persistence(Arc::new(store))
                        .map(drop)
                },
            )
        },
    },
    Format {
        name: "access_rights",
        two_slot: false,
        sample: samples::access_rights,
        check: |path| {
            check_file(
                path,
                oid(ObjectType::ACCESS_RIGHTS),
                |path| FileAccessRightsPersistence::new(path).unwrap(),
                |store| AccessRightsObject::with_persistence(1, "AR", Arc::new(store)).map(drop),
            )
        },
    },
];

impl Format {
    /// The files this format keeps at `base`.
    pub fn files(&self, base: &Path) -> Vec<PathBuf> {
        if self.two_slot {
            vec![suffixed(base, ".slot0"), suffixed(base, ".slot1")]
        } else {
            vec![base.to_path_buf()]
        }
    }
}

/// The two-slot header the log and Audit Log formats share: magic tag,
/// schema version, object identifier, generation, payload length.
const SLOT_HEADER_LEN: usize = 8 + 2 + 4 + 8 + 4;
const SLOT_TRAILER_LEN: usize = 4;

/// The CRC-32 (IEEE 802.3, reflected) the slots end with, written here
/// from its definition.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Give a two-slot format's `slot` the length field and checksum of what
/// it holds, if it is long enough to have them, so damage past the header
/// reaches the payload decoders.
pub fn seal(slot: &mut [u8]) {
    if slot.len() < SLOT_HEADER_LEN + SLOT_TRAILER_LEN {
        return;
    }
    let payload = (slot.len() - SLOT_HEADER_LEN - SLOT_TRAILER_LEN) as u32;
    slot[22..26].copy_from_slice(&payload.to_be_bytes());
    let at = slot.len() - SLOT_TRAILER_LEN;
    let checksum = crc32(&slot[..at]);
    slot[at..].copy_from_slice(&checksum.to_be_bytes());
}

/// The object every sample and check is for: instance 1 of `kind`.
pub fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 1).expect("instance 1")
}

fn suffixed(base: &Path, suffix: &str) -> PathBuf {
    let mut path = base.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

/// A directory of its own under the system's temporary one, removed on
/// drop.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusty-bacnet-{label}-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self(dir)
    }

    /// A base path in the directory.
    pub fn base(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Each file's octets, `None` for a missing one.
pub fn read_files(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths.iter().map(|path| std::fs::read(path).ok()).collect()
}

/// Make each file hold its octets, removing those given `None`.
pub fn write_files(paths: &[PathBuf], contents: &[Option<Vec<u8>>]) {
    for (path, content) in paths.iter().zip(contents) {
        match content {
            Some(octets) => std::fs::write(path, octets).expect("a scratch file"),
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn same<T: Debug>(left: &T, right: &T) -> bool {
    format!("{left:?}") == format!("{right:?}")
}

/// A sibling base for the round trip, with its files removed.
fn copy_base(base: &Path, files: usize) -> PathBuf {
    let copy = suffixed(base, ".copy");
    let paths = if files == 2 {
        vec![suffixed(&copy, ".slot0"), suffixed(&copy, ".slot1")]
    } else {
        vec![copy.clone()]
    };
    write_files(&paths, &vec![None; files]);
    copy
}

fn check_log<R: Debug + Send + Sync + 'static>(
    base: &Path,
    oid: ObjectIdentifier,
    build: impl Fn(u32, Arc<dyn LogPersistence<R>>) -> Result<(bool, Box<dyn BACnetObject>), Error>,
) -> Outcome
where
    FileLogPersistence: LogPersistence<R>,
{
    let store = FileLogPersistence::new(base).unwrap();
    let paths = store.slot_paths();
    let before = read_files(&paths);
    let loaded = LogPersistence::<R>::load(&store, oid);
    if let Ok(Some(snapshot)) = &loaded {
        assert_eq!(
            snapshot.object_identifier, oid,
            "loaded another object's snapshot"
        );
        let copy = FileLogPersistence::new(copy_base(base, 2)).unwrap();
        // No backend documents this; the harness holds it as the round trip
        // a restore owes, so a failure here is a finding to triage.
        copy.commit(snapshot)
            .unwrap_or_else(|error| panic!("a loaded snapshot doesn't commit again: {error:?}"));
        let again = LogPersistence::<R>::load(&copy, oid).expect("the copy loads");
        assert!(
            same(&again.as_ref(), &Some(snapshot)),
            "the copy loads {again:?}"
        );
        write_files(&copy.slot_paths(), &[None, None]);
    }
    let size = match &loaded {
        Ok(Some(snapshot)) => snapshot.buffer_size.clamp(1, 64),
        _ => 8,
    };
    let built = build(size, Arc::new(store));
    match (&loaded, built) {
        (Err(_), Ok(_)) => panic!("the log built although its storage didn't load"),
        (_, Err(_)) => assert!(
            read_files(&paths) == before,
            "a refused restore changed the slots"
        ),
        (Ok(snapshot), Ok((reopened, log))) => {
            assert_eq!(reopened, snapshot.is_some(), "reopened() is wrong");
            if let Some(snapshot) = snapshot {
                let total = log.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None);
                assert_eq!(
                    total.ok(),
                    Some(PropertyValue::Unsigned(snapshot.total_record_count.into())),
                    "the reopened log lost its Total_Record_Count"
                );
            }
            return Outcome {
                loaded: snapshot.is_some(),
                built: true,
            };
        }
    }
    Outcome {
        loaded: matches!(loaded, Ok(Some(_))),
        built: false,
    }
}

fn check_audit(base: &Path) -> Outcome {
    let store = FileAuditLogPersistence::new(base).unwrap();
    let paths = store.slot_paths();
    let before = read_files(&paths);
    let oid = oid(ObjectType::AUDIT_LOG);
    let loaded = store.load(oid);
    if let Ok(Some(snapshot)) = &loaded {
        assert_eq!(
            snapshot.object_identifier, oid,
            "loaded another object's snapshot"
        );
        let copy = FileAuditLogPersistence::new(copy_base(base, 2)).unwrap();
        // No backend documents this; the harness holds it as the round trip
        // a restore owes, so a failure here is a finding to triage.
        copy.commit(snapshot)
            .unwrap_or_else(|error| panic!("a loaded snapshot doesn't commit again: {error:?}"));
        let again = copy.load(oid).expect("the copy loads");
        assert!(
            same(&again.as_ref(), &Some(snapshot)),
            "the copy loads {again:?}"
        );
        write_files(&copy.slot_paths(), &[None, None]);
    }
    let capacity = match &loaded {
        Ok(Some(snapshot)) => snapshot.capacity,
        _ => 4,
    };
    let built = AuditLogObject::new(1, "AL", capacity, Arc::new(store));
    let outcome = Outcome {
        loaded: matches!(loaded, Ok(Some(_))),
        built: built.is_ok(),
    };
    match (loaded, built) {
        (Err(_), Ok(_)) => panic!("the Audit Log built although its storage didn't load"),
        (_, Err(_)) => assert!(
            read_files(&paths) == before,
            "a refused restore changed the slots"
        ),
        (Ok(None), Ok(_)) => {}
        (Ok(Some(snapshot)), Ok(log)) => {
            assert!(
                same(
                    &log.records().iter().collect::<Vec<_>>(),
                    &snapshot.records.iter().collect()
                ),
                "the restored Audit Log serves other records"
            );
            assert_eq!(log.total_record_count(), snapshot.total_record_count);
            assert_eq!(log.buffer_size(), snapshot.capacity);
            assert_eq!(log.log_enable(), snapshot.log_enable);
            assert_eq!(log.generation(), snapshot.generation);
        }
    }
    outcome
}

/// A one-file backend: what [`check_file`] needs of it.
trait OneFile {
    type Snapshot: Debug;
    fn load_for(&self, oid: ObjectIdentifier) -> Result<Option<Self::Snapshot>, Error>;
    fn save_for(&self, oid: ObjectIdentifier, snapshot: &Self::Snapshot) -> Result<(), Error>;
}

macro_rules! one_file {
    ($($store:ty => $trait:ident, $snapshot:ty;)*) => {$(
        impl OneFile for $store {
            type Snapshot = $snapshot;
            fn load_for(&self, oid: ObjectIdentifier) -> Result<Option<$snapshot>, Error> {
                $trait::load(self, oid)
            }
            fn save_for(&self, oid: ObjectIdentifier, snapshot: &$snapshot) -> Result<(), Error> {
                $trait::save(self, oid, snapshot)
            }
        }
    )*};
}

one_file! {
    FileNotificationForwarderPersistence => NotificationForwarderPersistence, ForwarderSnapshot;
    FileNotificationClassPersistence => NotificationClassPersistence, NotificationClassSnapshot;
    FileTagsPersistence => TagsPersistence, TagsSnapshot;
    FileColorOutputPersistence => ColorOutputPersistence, ColorOutputSnapshot;
    FileAccessRightsPersistence => AccessRightsPersistence, AccessRightsSnapshot;
}

fn check_file<S: OneFile>(
    path: &Path,
    oid: ObjectIdentifier,
    open: fn(&Path) -> S,
    build: impl Fn(S) -> Result<(), Error>,
) -> Outcome {
    let paths = [path.to_path_buf()];
    let before = read_files(&paths);
    let loaded = open(path).load_for(oid);
    // Whether a loaded snapshot saves again; one that doesn't is one the
    // object must refuse, since it could never save it.
    let mut saves = true;
    if let Ok(Some(snapshot)) = &loaded {
        let copy = [copy_base(path, 1)];
        let store = open(&copy[0]);
        saves = store.save_for(oid, snapshot).is_ok();
        if saves {
            let again = store.load_for(oid).expect("the copy loads");
            assert!(
                same(&again.as_ref(), &Some(snapshot)),
                "the copy loads {again:?}"
            );
            // The saved octets are the canonical form: saving them again
            // changes nothing.
            let first = read_files(&copy);
            store
                .save_for(oid, again.as_ref().unwrap())
                .expect("the copy saves again");
            assert!(
                read_files(&copy) == first,
                "saving a loaded snapshot isn't stable"
            );
        }
        write_files(&copy, &[None]);
    }
    let built = build(open(path)).is_ok();
    if built {
        assert!(
            loaded.is_ok(),
            "the object built although its storage didn't load"
        );
        // Harness-invented, like the round trip above: a failure is a finding.
        assert!(saves, "the object took a snapshot its storage can't save");
    } else {
        assert!(
            read_files(&paths) == before,
            "a refused restore changed the file"
        );
    }
    Outcome {
        loaded: matches!(loaded, Ok(Some(_))),
        built,
    }
}
