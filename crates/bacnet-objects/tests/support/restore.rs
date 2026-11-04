//! Every format the file persistence backends save, a sample of each written
//! through the public API, and the check a restore from any octets must pass
//! (#2180, #2183).
//!
//! Shared by `tests/persistence_corruption.rs` and the fuzz crate
//! (`crates/bacnet-objects/fuzz`), which builds this file with `#[path]`.
//!
//! The check, [`Format::check`], runs on a [`Medium`]: the files at a base
//! path ([`Files`]), or, in the fuzz crate, the backends' in-memory mode
//! (#2233), the same backends with the octets kept in memory. It loads what
//! the storage holds through the backend, then builds the object on the same
//! storage:
//! - a load that succeeds names the expected object, and its snapshot saves
//!   to fresh storage and loads back equal (compared by `Debug` text, so a
//!   NaN matches itself);
//! - a load that fails makes the object's build fail too;
//! - a build that fails leaves the storage as it was: a refused restore
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
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::color::{
    ColorObject, ColorOutputPersistence, ColorOutputSnapshot, ColorTemperatureObject,
    FileColorOutputPersistence,
};
use bacnet_objects::event_enrollment::AlertEnrollmentObject;
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
use bacnet_objects::timer::TimerObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_objects::write_config::{
    FileWriteConfigPersistence, WriteConfigPersistence, WriteConfigSnapshot,
};
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
    /// Which object restores from it.
    kind: Kind,
}

/// The object a format restores, which picks the check.
#[derive(Clone, Copy)]
enum Kind {
    TrendLog,
    TrendLogMultiple,
    EventLog,
    AuditLog,
    NotificationForwarder,
    NotificationClass,
    Tags,
    Color,
    ColorTemperature,
    AccessRights,
    TimerWriteConfig,
    ChannelWriteConfig,
    ReportingWriteConfig,
    AlertReportingWriteConfig,
}

/// Where a check's storage lives. Each method opens one backend on it; a
/// format's check opens only its own.
pub trait Medium: Sized {
    fn log(&self) -> FileLogPersistence;
    fn audit(&self) -> FileAuditLogPersistence;
    fn forwarder(&self) -> FileNotificationForwarderPersistence;
    fn class(&self) -> FileNotificationClassPersistence;
    fn tags(&self) -> FileTagsPersistence;
    fn color(&self) -> FileColorOutputPersistence;
    fn rights(&self) -> FileAccessRightsPersistence;
    fn write_config(&self) -> FileWriteConfigPersistence;
    /// Empty storage of the same kind, for the round trip.
    fn fresh(&self) -> Self;
    /// What each of the format's files holds now, `None` for a missing one.
    fn contents(&self) -> Vec<Option<Vec<u8>>>;
}

/// A format's files at a base path.
pub struct Files {
    base: PathBuf,
    paths: Vec<PathBuf>,
}

impl Files {
    pub fn new(format: &Format, base: &Path) -> Self {
        Self {
            base: base.to_path_buf(),
            paths: format.files(base),
        }
    }
}

impl Medium for Files {
    fn log(&self) -> FileLogPersistence {
        FileLogPersistence::new(&self.base).unwrap()
    }
    fn audit(&self) -> FileAuditLogPersistence {
        FileAuditLogPersistence::new(&self.base).unwrap()
    }
    fn forwarder(&self) -> FileNotificationForwarderPersistence {
        FileNotificationForwarderPersistence::new(&self.base).unwrap()
    }
    fn class(&self) -> FileNotificationClassPersistence {
        FileNotificationClassPersistence::new(&self.base).unwrap()
    }
    fn tags(&self) -> FileTagsPersistence {
        FileTagsPersistence::new(&self.base).unwrap()
    }
    fn color(&self) -> FileColorOutputPersistence {
        FileColorOutputPersistence::new(&self.base).unwrap()
    }
    fn rights(&self) -> FileAccessRightsPersistence {
        FileAccessRightsPersistence::new(&self.base).unwrap()
    }
    fn write_config(&self) -> FileWriteConfigPersistence {
        FileWriteConfigPersistence::new(&self.base).unwrap()
    }
    /// A sibling base, with its files removed.
    fn fresh(&self) -> Self {
        let base = suffixed(&self.base, ".copy");
        let paths = if self.paths.len() == 2 {
            vec![suffixed(&base, ".slot0"), suffixed(&base, ".slot1")]
        } else {
            vec![base.clone()]
        };
        write_files(&paths, &vec![None; paths.len()]);
        Self { base, paths }
    }
    fn contents(&self) -> Vec<Option<Vec<u8>>> {
        read_files(&self.paths)
    }
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
        kind: Kind::TrendLog,
    },
    Format {
        name: "trend_log_multiple",
        two_slot: true,
        sample: samples::trend_log_multiple,
        kind: Kind::TrendLogMultiple,
    },
    Format {
        name: "event_log",
        two_slot: true,
        sample: samples::event_log,
        kind: Kind::EventLog,
    },
    Format {
        name: "audit_log",
        two_slot: true,
        sample: samples::audit_log,
        kind: Kind::AuditLog,
    },
    Format {
        name: "notification_forwarder",
        two_slot: false,
        sample: samples::notification_forwarder,
        kind: Kind::NotificationForwarder,
    },
    Format {
        name: "notification_class",
        two_slot: false,
        sample: samples::notification_class,
        kind: Kind::NotificationClass,
    },
    Format {
        name: "tags",
        two_slot: false,
        sample: samples::tags,
        kind: Kind::Tags,
    },
    Format {
        name: "color",
        two_slot: false,
        sample: samples::color,
        kind: Kind::Color,
    },
    Format {
        name: "color_temperature",
        two_slot: false,
        sample: samples::color_temperature,
        kind: Kind::ColorTemperature,
    },
    Format {
        name: "access_rights",
        two_slot: false,
        sample: samples::access_rights,
        kind: Kind::AccessRights,
    },
    Format {
        name: "timer_write_config",
        two_slot: false,
        sample: samples::timer_write_config,
        kind: Kind::TimerWriteConfig,
    },
    Format {
        name: "channel_write_config",
        two_slot: false,
        sample: samples::channel_write_config,
        kind: Kind::ChannelWriteConfig,
    },
    Format {
        name: "reporting_write_config",
        two_slot: false,
        sample: samples::reporting_write_config,
        kind: Kind::ReportingWriteConfig,
    },
    // Last, so the selectors of the formats before it stay as they were.
    Format {
        name: "alert_reporting_write_config",
        two_slot: false,
        sample: samples::alert_reporting_write_config,
        kind: Kind::AlertReportingWriteConfig,
    },
];

impl Format {
    /// Restore from what `medium` holds; see the module docs.
    pub fn check<M: Medium>(&self, medium: &M) -> Outcome {
        match self.kind {
            Kind::TrendLog => check_log(medium, oid(ObjectType::TREND_LOG), |size, storage| {
                TrendLogObject::with_persistence(1, "TL", size, storage)
                    .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
            }),
            Kind::TrendLogMultiple => check_log(
                medium,
                oid(ObjectType::TREND_LOG_MULTIPLE),
                |size, storage| {
                    TrendLogMultipleObject::with_persistence(1, "TLM", size, storage)
                        .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
                },
            ),
            Kind::EventLog => check_log(medium, oid(ObjectType::EVENT_LOG), |size, storage| {
                EventLogObject::with_persistence(1, "EL", size, storage)
                    .map(|log| (log.reopened(), Box::new(log) as Box<dyn BACnetObject>))
            }),
            Kind::AuditLog => check_audit(medium),
            Kind::NotificationForwarder => check_file(
                medium,
                oid(ObjectType::NOTIFICATION_FORWARDER),
                M::forwarder,
                |store| NotificationForwarderObject::with_persistence(1, "NF", store).map(drop),
            ),
            Kind::NotificationClass => check_file(
                medium,
                oid(ObjectType::NOTIFICATION_CLASS),
                M::class,
                |store| NotificationClass::with_persistence(1, "NC", store).map(drop),
            ),
            Kind::Tags => check_file(medium, oid(ObjectType::ANALOG_INPUT), M::tags, |store| {
                let units = EngineeringUnits::DEGREES_CELSIUS;
                AnalogInputObject::with_tags_persistence(1, "AI", units, store).map(drop)
            }),
            Kind::Color => check_file(medium, oid(ObjectType::COLOR), M::color, |store| {
                ColorObject::new(1, "CLR")?
                    .with_output_persistence(store)
                    .map(drop)
            }),
            Kind::ColorTemperature => check_file(
                medium,
                oid(ObjectType::COLOR_TEMPERATURE),
                M::color,
                |store| {
                    ColorTemperatureObject::new(1, "CT")?
                        .with_output_persistence(store)
                        .map(drop)
                },
            ),
            Kind::AccessRights => {
                check_file(medium, oid(ObjectType::ACCESS_RIGHTS), M::rights, |store| {
                    AccessRightsObject::with_persistence(1, "AR", store).map(drop)
                })
            }
            Kind::TimerWriteConfig => {
                check_file(medium, oid(ObjectType::TIMER), M::write_config, |store| {
                    TimerObject::with_persistence(1, "TMR", store).map(drop)
                })
            }
            Kind::ChannelWriteConfig => {
                check_file(medium, oid(ObjectType::CHANNEL), M::write_config, |store| {
                    ChannelObject::with_persistence(1, "CH", 1, store).map(drop)
                })
            }
            Kind::ReportingWriteConfig => check_file(
                medium,
                oid(ObjectType::ANALOG_INPUT),
                M::write_config,
                |store| {
                    let units = EngineeringUnits::DEGREES_CELSIUS;
                    AnalogInputObject::new(1, "AI", units)?
                        .with_reporting_persistence(store)
                        .map(drop)
                },
            ),
            // An Alert Enrollment refuses an ACK_NOTIFICATION Notify_Type
            // the format itself takes (#2331).
            Kind::AlertReportingWriteConfig => check_file(
                medium,
                oid(ObjectType::ALERT_ENROLLMENT),
                M::write_config,
                |store| {
                    AlertEnrollmentObject::new(1, "AE", oid(ObjectType::ANALOG_INPUT))?
                        .with_reporting_persistence(store)
                        .map(drop)
                },
            ),
        }
    }

    /// Restore from the files at `base`.
    pub fn check_files(&self, base: &Path) -> Outcome {
        self.check(&Files::new(self, base))
    }

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
/// from its definition: the table holds each octet's eight shift steps, so
/// the fuzz targets seal a slot with one lookup per octet (#2233).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut crc = n as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
                bit += 1;
            }
            table[n] = crc;
            n += 1;
        }
        table
    };
    !data.iter().fold(!0u32, |crc, &byte| {
        TABLE[usize::from(crc as u8 ^ byte)] ^ (crc >> 8)
    })
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

/// Whether `left` and `right` have the same `Debug` text. The fuzz targets
/// run this on every snapshot they load, so it formats `left` into a
/// buffer the thread keeps and matches `right`'s text against it as it is
/// written, stopping at the first difference (#2233).
fn same<T: Debug>(left: &T, right: &T) -> bool {
    use std::fmt::Write;

    /// The text still to match.
    struct Rest<'a>(&'a str);

    impl Write for Rest<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0 = self.0.strip_prefix(text).ok_or(std::fmt::Error)?;
            Ok(())
        }
    }

    thread_local! {
        static LEFT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }
    LEFT.with_borrow_mut(|text| {
        text.clear();
        write!(text, "{left:?}").expect("Debug text formats");
        let mut rest = Rest(text);
        write!(rest, "{right:?}").is_ok() && rest.0.is_empty()
    })
}

fn check_log<R: Debug + Send + Sync + 'static>(
    medium: &impl Medium,
    oid: ObjectIdentifier,
    build: impl Fn(u32, Arc<dyn LogPersistence<R>>) -> Result<(bool, Box<dyn BACnetObject>), Error>,
) -> Outcome
where
    FileLogPersistence: LogPersistence<R>,
{
    let store = medium.log();
    let before = medium.contents();
    let loaded = LogPersistence::<R>::load(&store, oid);
    if let Ok(Some(snapshot)) = &loaded {
        assert_eq!(
            snapshot.object_identifier, oid,
            "loaded another object's snapshot"
        );
        let copy = medium.fresh().log();
        // No backend documents this; the harness holds it as the round trip
        // a restore owes, so a failure here is a finding to triage.
        copy.commit(snapshot)
            .unwrap_or_else(|error| panic!("a loaded snapshot doesn't commit again: {error:?}"));
        let again = LogPersistence::<R>::load(&copy, oid).expect("the copy loads");
        assert!(
            same(&again.as_ref(), &Some(snapshot)),
            "the copy loads {again:?}"
        );
    }
    let size = match &loaded {
        Ok(Some(snapshot)) => snapshot.buffer_size.clamp(1, 64),
        _ => 8,
    };
    let built = build(size, Arc::new(store));
    match (&loaded, built) {
        (Err(_), Ok(_)) => panic!("the log built although its storage didn't load"),
        (_, Err(_)) => assert!(
            medium.contents() == before,
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
                if let Some(class) = snapshot.reporting.notification_class {
                    let served = log.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None);
                    assert_eq!(
                        served.ok(),
                        Some(PropertyValue::Unsigned(class.into())),
                        "the reopened log lost its written Notification_Class"
                    );
                }
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

fn check_audit(medium: &impl Medium) -> Outcome {
    let store = medium.audit();
    let before = medium.contents();
    let oid = oid(ObjectType::AUDIT_LOG);
    let loaded = store.load(oid);
    if let Ok(Some(snapshot)) = &loaded {
        assert_eq!(
            snapshot.object_identifier, oid,
            "loaded another object's snapshot"
        );
        let copy = medium.fresh().audit();
        // No backend documents this; the harness holds it as the round trip
        // a restore owes, so a failure here is a finding to triage.
        copy.commit(snapshot)
            .unwrap_or_else(|error| panic!("a loaded snapshot doesn't commit again: {error:?}"));
        let again = copy.load(oid).expect("the copy loads");
        assert!(
            same(&again.as_ref(), &Some(snapshot)),
            "the copy loads {again:?}"
        );
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
            medium.contents() == before,
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
trait OneFile: Send + Sync + 'static {
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
    FileWriteConfigPersistence => WriteConfigPersistence, WriteConfigSnapshot;
}

fn check_file<M: Medium, S: OneFile>(
    medium: &M,
    oid: ObjectIdentifier,
    open: fn(&M) -> S,
    build: impl Fn(Arc<S>) -> Result<(), Error>,
) -> Outcome {
    let before = medium.contents();
    let loaded = open(medium).load_for(oid);
    // Whether a loaded snapshot saves again; one that doesn't is one the
    // object must refuse, since it could never save it.
    let mut saves = true;
    if let Ok(Some(snapshot)) = &loaded {
        let copy = medium.fresh();
        let store = open(&copy);
        saves = store.save_for(oid, snapshot).is_ok();
        if saves {
            let again = store.load_for(oid).expect("the copy loads");
            assert!(
                same(&again.as_ref(), &Some(snapshot)),
                "the copy loads {again:?}"
            );
            // The saved octets are the canonical form: saving them again
            // changes nothing.
            let first = copy.contents();
            store
                .save_for(oid, again.as_ref().unwrap())
                .expect("the copy saves again");
            assert!(
                copy.contents() == first,
                "saving a loaded snapshot isn't stable"
            );
        }
    }
    let built = build(Arc::new(open(medium))).is_ok();
    if built {
        assert!(
            loaded.is_ok(),
            "the object built although its storage didn't load"
        );
        // Harness-invented, like the round trip above: a failure is a finding.
        assert!(saves, "the object took a snapshot its storage can't save");
    } else {
        assert!(
            medium.contents() == before,
            "a refused restore changed the file"
        );
    }
    Outcome {
        loaded: matches!(loaded, Ok(Some(_))),
        built,
    }
}
