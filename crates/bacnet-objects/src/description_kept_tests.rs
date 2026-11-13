//! A written Description kept in the snapshot formats of their own (#2364):
//! the three logs, Notification Class, Notification Forwarder and Access
//! Rights, and (in `tags_and_colour`) the Device and the two lighting
//! outputs in the Tags format and the two colour objects in theirs. Each
//! comes back on a rebuild; a write whose save fails is refused
//! with the old value kept; a NULL changes nothing and saves nothing; one
//! past the cap is refused before anything is saved, but only while it
//! would be kept; a request writing it beside another kept property saves
//! once; and a saved one wins over the setter.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::access_control::{AccessRightsObject, AccessRightsPersistence, AccessRightsSnapshot};
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::event_log::EventLogObject;
use crate::log_persistence::{LogPersistence, LogSnapshot};
use crate::notification_class::{
    NotificationClass, NotificationClassPersistence, NotificationClassSnapshot,
};
use crate::notification_forwarder::{
    ForwarderSnapshot, NotificationForwarderObject, NotificationForwarderPersistence,
};
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

mod clear;
mod tags_and_colour;

/// One snapshot in memory, saves counted, and a switch that fails them.
pub(crate) struct Memory<S> {
    saved: Mutex<Option<S>>,
    saves: AtomicUsize,
    fail: AtomicBool,
}

impl<S: Clone> Memory<S> {
    pub(crate) fn holding(snapshot: Option<S>) -> Arc<Self> {
        Arc::new(Self {
            saved: Mutex::new(snapshot),
            saves: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        })
    }

    pub(crate) fn saved(&self) -> Option<S> {
        self.saved.lock().unwrap().clone()
    }

    fn store(&self, snapshot: &S) -> Result<(), Error> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl NotificationClassPersistence for Memory<NotificationClassSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        Ok(self.saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &NotificationClassSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

impl NotificationForwarderPersistence for Memory<ForwarderSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        Ok(self.saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

impl AccessRightsPersistence for Memory<AccessRightsSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        Ok(self.saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

impl<R: Clone + Send + Sync + 'static> LogPersistence<R> for Memory<LogSnapshot<R>> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LogSnapshot<R>>, Error> {
        Ok(self.saved())
    }
    fn commit(&self, snapshot: &LogSnapshot<R>) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// An object with storage, as the server reaches it.
pub(crate) trait KeptObject: BACnetObject {
    fn object(&self) -> &dyn BACnetObject;
    fn object_mut(&mut self) -> &mut dyn BACnetObject;
    /// The staging the server reaches. `None` on an object that lends it
    /// only through `durable_writes()`, in a build without that feature.
    fn staging(&mut self) -> Option<&mut dyn DurableWrites>;
    /// The application's `set_description`, on a type that has one.
    fn configure(&mut self, text: &str);
    /// The type's `clear_written_description`. `None` on a log, whose
    /// `clear_written_reporting` drops a saved Description.
    fn clear(&mut self) -> Option<Result<(), Error>>;
}

/// [`KeptObject`] for a type, from how it lends its staging, takes a
/// configured Description and clears a saved one.
macro_rules! kept_object {
    ($object:ty, |$staged:ident| $staging:expr, |$set:ident, $text:ident| $configure:expr, |$cleared:ident| $clear:expr) => {
        impl KeptObject for $object {
            fn object(&self) -> &dyn BACnetObject {
                self
            }
            fn object_mut(&mut self) -> &mut dyn BACnetObject {
                self
            }
            fn staging(&mut self) -> Option<&mut dyn DurableWrites> {
                #[cfg(feature = "durable-writes")]
                {
                    let $staged = self;
                    $staging
                }
                #[cfg(not(feature = "durable-writes"))]
                {
                    None
                }
            }
            fn configure(&mut self, $text: &str) {
                let $set = self;
                $configure
            }
            fn clear(&mut self) -> Option<Result<(), Error>> {
                let $cleared = self;
                $clear
            }
        }
    };
}
pub(crate) use kept_object;

kept_object!(
    NotificationClass,
    |class| Some(class),
    |class, text| class.set_description(text),
    |class| Some(class.clear_written_description())
);
kept_object!(
    NotificationForwarderObject,
    |forwarder| Some(forwarder),
    |forwarder, text| forwarder.set_description(text),
    |forwarder| Some(forwarder.clear_written_description())
);
// It has no Description setter.
kept_object!(
    AccessRightsObject,
    |rights| Some(rights),
    |_rights, _text| (),
    |rights| Some(rights.clear_written_description())
);
kept_object!(
    TrendLogObject,
    |log| Some(log),
    |log, text| log.set_description(text),
    |_log| None
);
kept_object!(
    TrendLogMultipleObject,
    |log| Some(log),
    |log, text| log.set_description(text),
    |_log| None
);
kept_object!(
    EventLogObject,
    |log| Some(log),
    |log, text| log.set_description(text),
    |_log| None
);

/// Build an object on storage holding snapshots `S`.
type Open<S> = fn(Arc<Memory<S>>) -> Result<Box<dyn KeptObject>, Error>;

/// One object type on its storage.
struct Case<S> {
    storage: Arc<Memory<S>>,
    open: Open<S>,
    description: fn(&S) -> Option<&String>,
    /// Another property the type keeps, and a value a write gives it.
    other: (P, PropertyValue),
    /// The tag of the file a snapshot saves as, on a type whose saved
    /// Description `clear_written_description` drops.
    tag: Option<fn(&S) -> [u8; 8]>,
}

/// What the checks need of a [`Case`], whatever its snapshot.
trait Kept {
    fn open(&self) -> Box<dyn KeptObject>;
    fn try_open(&self) -> Result<Box<dyn KeptObject>, Error>;
    fn saves(&self) -> usize;
    fn fail(&self, fail: bool);
    fn saved_description(&self) -> Option<String>;
    fn other(&self) -> (P, PropertyValue);
    fn has_setter(&self) -> bool;
    /// Whether the type has `clear_written_description`.
    fn clears(&self) -> bool;
    /// The tag of the file storage's snapshot saves as, on such a type.
    fn saved_tag(&self) -> Option<[u8; 8]>;
}

impl<S: Clone + 'static> Kept for Case<S> {
    fn open(&self) -> Box<dyn KeptObject> {
        self.try_open().unwrap()
    }
    fn try_open(&self) -> Result<Box<dyn KeptObject>, Error> {
        (self.open)(Arc::clone(&self.storage))
    }
    fn saves(&self) -> usize {
        self.storage.saves.load(Ordering::SeqCst)
    }
    fn fail(&self, fail: bool) {
        self.storage.fail.store(fail, Ordering::SeqCst);
    }
    fn saved_description(&self) -> Option<String> {
        self.storage
            .saved()
            .and_then(|saved| (self.description)(&saved).cloned())
    }
    fn other(&self) -> (P, PropertyValue) {
        self.other.clone()
    }
    fn has_setter(&self) -> bool {
        !matches!(
            self.other.0,
            P::ENABLE | P::DEFAULT_COLOR | P::DEFAULT_COLOR_TEMPERATURE
        )
    }
    fn clears(&self) -> bool {
        self.tag.is_some()
    }
    fn saved_tag(&self) -> Option<[u8; 8]> {
        let saved = self.storage.saved()?;
        self.tag.map(|tag| tag(&saved))
    }
}

/// The eight octets a format's `encode` result opens with.
fn tag_of(octets: Result<Vec<u8>, crate::snapshot::SnapshotError>) -> [u8; 8] {
    octets.unwrap()[..8].try_into().unwrap()
}

fn oid(kind: bacnet_types::enums::ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 1).unwrap()
}

/// Every covered type on fresh storage, or on storage holding a snapshot
/// with `saved` as its Description.
fn cases(saved: Option<&str>) -> Vec<Box<dyn Kept>> {
    let saved = saved.map(String::from);
    let class = (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9));
    let empty_list = (
        P::RECIPIENT_LIST,
        PropertyValue::ApplicationData(Vec::new()),
    );
    let mut cases: Vec<Box<dyn Kept>> = vec![
        Box::new(Case {
            storage: Memory::holding(saved.clone().map(|text| NotificationClassSnapshot {
                recipient_list: None,
                description: Some(text),
            })),
            open: |storage| {
                Ok(Box::new(NotificationClass::with_persistence(1, "NC", storage)?) as _)
            },
            description: |saved: &NotificationClassSnapshot| saved.description.as_ref(),
            other: empty_list.clone(),
            tag: Some(|saved| {
                let class = oid(bacnet_types::enums::ObjectType::NOTIFICATION_CLASS);
                tag_of(crate::snapshot::notification_class::encode(class, saved))
            }),
        }),
        Box::new(Case {
            storage: Memory::holding(saved.clone().map(|text| ForwarderSnapshot {
                description: Some(text),
                ..ForwarderSnapshot::default()
            })),
            open: |storage| {
                Ok(Box::new(NotificationForwarderObject::with_persistence(
                    1, "NF", storage,
                )?) as _)
            },
            description: |saved: &ForwarderSnapshot| saved.description.as_ref(),
            other: empty_list,
            tag: Some(|saved| {
                let forwarder = oid(bacnet_types::enums::ObjectType::NOTIFICATION_FORWARDER);
                tag_of(crate::snapshot::notification_forwarder::encode(
                    forwarder, saved,
                ))
            }),
        }),
        Box::new(Case {
            storage: Memory::holding(saved.clone().map(|text| AccessRightsSnapshot {
                description: Some(text),
                ..AccessRightsSnapshot::default()
            })),
            open: |storage| {
                Ok(Box::new(AccessRightsObject::with_persistence(1, "AR", storage)?) as _)
            },
            description: |saved: &AccessRightsSnapshot| saved.description.as_ref(),
            other: (P::ENABLE, PropertyValue::Boolean(false)),
            tag: Some(|saved| {
                let rights = oid(bacnet_types::enums::ObjectType::ACCESS_RIGHTS);
                tag_of(crate::snapshot::access_rights::encode(rights, saved))
            }),
        }),
        Box::new(Case::<LogSnapshot<BACnetLogRecord>> {
            storage: Memory::holding(saved.clone().map(|text| log_holding(text, trend_log))),
            open: trend_log,
            description: |saved| saved.reporting.description.as_ref(),
            other: class.clone(),
            tag: None,
        }),
        Box::new(Case::<LogSnapshot<BACnetLogMultipleRecord>> {
            storage: Memory::holding(
                saved
                    .clone()
                    .map(|text| log_holding(text, trend_log_multiple)),
            ),
            open: trend_log_multiple,
            description: |saved| saved.reporting.description.as_ref(),
            other: class.clone(),
            tag: None,
        }),
        Box::new(Case::<LogSnapshot<BACnetEventLogRecord>> {
            storage: Memory::holding(saved.clone().map(|text| log_holding(text, event_log))),
            open: event_log,
            description: |saved| saved.reporting.description.as_ref(),
            other: class,
            tag: None,
        }),
    ];
    cases.extend(tags_and_colour::cases(saved.as_deref()));
    cases
}

/// A snapshot of a log that `fresh` builds and commits, with `saved` as its
/// Description.
fn log_holding<R: Clone + Send + Sync + 'static>(
    saved: String,
    fresh: Open<LogSnapshot<R>>,
) -> LogSnapshot<R> {
    let storage = Memory::holding(None);
    let mut log = fresh(Arc::clone(&storage)).unwrap();
    write(log.object_mut(), P::DESCRIPTION, text("x")).unwrap();
    drop(log);
    let mut snapshot = storage.saved().unwrap();
    snapshot.reporting.description = Some(saved);
    snapshot
}

fn trend_log(
    storage: Arc<Memory<LogSnapshot<BACnetLogRecord>>>,
) -> Result<Box<dyn KeptObject>, Error> {
    Ok(Box::new(TrendLogObject::with_persistence(
        1, "TL", 8, storage,
    )?))
}

fn trend_log_multiple(
    storage: Arc<Memory<LogSnapshot<BACnetLogMultipleRecord>>>,
) -> Result<Box<dyn KeptObject>, Error> {
    Ok(Box::new(TrendLogMultipleObject::with_persistence(
        1, "TLM", 8, storage,
    )?))
}

fn event_log(
    storage: Arc<Memory<LogSnapshot<BACnetEventLogRecord>>>,
) -> Result<Box<dyn KeptObject>, Error> {
    Ok(Box::new(EventLogObject::with_persistence(
        1, "EL", 8, storage,
    )?))
}

fn text(text: &str) -> PropertyValue {
    PropertyValue::CharacterString(text.into())
}

/// One past the cap a kept Description has.
fn too_long() -> PropertyValue {
    PropertyValue::CharacterString("d".repeat(16 * 1024 + 1))
}

fn description(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::DESCRIPTION, None).unwrap()
}

fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn refused_with(result: &Result<(), Error>, class: ErrorClass, code: ErrorCode) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
        if *c == class && *e == code)
}

fn pending(property: P, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: None,
        value,
    }
}

#[test]
fn a_written_description_survives_a_rebuild() {
    for case in cases(None) {
        let mut object = case.open();
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        assert_eq!(case.saves(), 1);
        assert_eq!(description(object.object()), text("lobby"));
        drop(object);
        assert_eq!(case.saved_description().as_deref(), Some("lobby"));
        assert_eq!(description(case.open().object()), text("lobby"));
    }
}

#[test]
fn a_description_whose_save_fails_is_refused_with_the_old_value_kept() {
    for case in cases(None) {
        let mut object = case.open();
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        case.fail(true);
        let result = write(object.object_mut(), P::DESCRIPTION, text("hall"));
        assert!(refused_with(
            &result,
            ErrorClass::DEVICE,
            ErrorCode::OPERATIONAL_PROBLEM
        ));
        assert_eq!(description(object.object()), text("lobby"));
        case.fail(false);
        drop(object);
        assert_eq!(case.saved_description().as_deref(), Some("lobby"));
    }
}

/// A NULL changes nothing and saves nothing; one past the cap, or of
/// another type, is refused before anything is saved.
#[test]
fn a_null_or_a_refused_description_saves_nothing() {
    for case in cases(None) {
        let mut object = case.open();
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        write(object.object_mut(), P::DESCRIPTION, PropertyValue::Null).unwrap();
        assert_eq!(description(object.object()), text("lobby"));
        if let Some(staging) = object.staging() {
            let null = pending(P::DESCRIPTION, PropertyValue::Null);
            assert!(matches!(staging.stage_writes(&[null]), StageStep::Skip));
            let long = staging.stage_writes(&[pending(P::DESCRIPTION, too_long())]);
            assert!(matches!(long, StageStep::Skip));
        }
        let result = write(object.object_mut(), P::DESCRIPTION, too_long());
        assert!(refused_with(
            &result,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ));
        let result = write(
            object.object_mut(),
            P::DESCRIPTION,
            PropertyValue::Unsigned(1),
        );
        assert!(refused_with(
            &result,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE
        ));
        assert_eq!(case.saves(), 1);
        assert_eq!(description(object.object()), text("lobby"));
    }
}

/// A request writing Description and another kept property stages one
/// save, and each write takes it without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_writing_description_and_another_property_saves_once() {
    for case in cases(None) {
        let (property, value) = case.other();
        let mut object = case.open();
        let writes = [
            pending(P::DESCRIPTION, text("lobby")),
            pending(property, value.clone()),
        ];
        let Some(staging) = object.staging() else {
            continue;
        };
        let StageStep::Staged(wait) = staging.stage_writes(&writes) else {
            panic!("{property:?}: expected a staged write");
        };
        wait.block();
        assert_eq!(case.saves(), 1);
        for write in writes {
            object
                .object_mut()
                .write_property(write.property, None, write.value, None)
                .unwrap();
        }
        let staging = object.staging().expect("lent above");
        staging.release_staged_write(&wait);
        assert_eq!(case.saves(), 1, "{property:?}");
        // Dropping the object waits for every save it queued: taking the
        // steps queued none behind the staged one.
        drop(object);
        assert_eq!(case.saves(), 1, "{property:?}, once its saves ran");
        let rebuilt = case.open();
        assert_eq!(description(rebuilt.object()), text("lobby"));
        let served = rebuilt.object().read_property(property, None).unwrap();
        if property == P::RECIPIENT_LIST {
            assert_eq!(served, PropertyValue::ApplicationData(Vec::new()));
        } else if property == P::TAGS {
            // The written empty array, over the configured tag.
            assert_eq!(served, PropertyValue::List(Vec::new()));
        } else {
            assert_eq!(served, value);
        }
    }
}

/// A saved Description wins over the setter; without one saved the setter
/// applies.
#[test]
fn a_saved_description_wins_over_its_setter() {
    for case in cases(Some("lobby")) {
        let mut object = case.open();
        assert_eq!(description(object.object()), text("lobby"));
        object.configure("configured");
        assert_eq!(description(object.object()), text("lobby"));
    }
    for case in cases(None).into_iter().filter(|case| case.has_setter()) {
        let (property, value) = case.other();
        let mut object = case.open();
        write(object.object_mut(), property, value).unwrap();
        object.configure("configured");
        assert_eq!(description(object.object()), text("configured"));
    }
}

/// Storage holding a Description past the cap fails the build.
#[test]
fn a_saved_description_past_its_cap_fails_the_build() {
    let long = "d".repeat(16 * 1024 + 1);
    for case in cases(Some(&long)) {
        assert!(case.try_open().is_err());
    }
}

/// Without storage a Description write is made where it is, a long one
/// included.
#[test]
fn without_storage_nothing_is_kept_or_capped() {
    let mut objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(crate::device::DeviceObject::new(Default::default()).unwrap()),
        Box::new(crate::lighting::LightingOutputObject::new(1, "LO").unwrap()),
        Box::new(crate::lighting::BinaryLightingOutputObject::new(1, "BLO").unwrap()),
        Box::new(crate::color::ColorObject::new(1, "CLR").unwrap()),
        Box::new(crate::color::ColorTemperatureObject::new(1, "CT").unwrap()),
        Box::new(NotificationClass::new(1, "NC").unwrap()),
        Box::new(NotificationForwarderObject::new(1, "NF").unwrap()),
        Box::new(AccessRightsObject::new(1, "AR").unwrap()),
        Box::new(TrendLogObject::new(1, "TL", 8).unwrap()),
        Box::new(TrendLogMultipleObject::new(1, "TLM", 8).unwrap()),
        Box::new(EventLogObject::new(1, "EL", 8).unwrap()),
    ];
    for object in &mut objects {
        write(object.as_mut(), P::DESCRIPTION, too_long()).unwrap();
        assert_eq!(description(object.as_ref()), too_long());
    }
}

/// A log's saved Description goes with `clear_written_reporting`: the text
/// served stays, the setter applies again, and storage keeps none.
#[test]
fn clear_written_reporting_drops_a_logs_description() {
    let storage = Memory::<LogSnapshot<BACnetLogRecord>>::holding(None);
    let mut log = TrendLogObject::with_persistence(1, "TL", 8, storage.clone()).unwrap();
    write(&mut log, P::DESCRIPTION, text("lobby")).unwrap();
    assert_eq!(
        log.written_reporting().description.as_deref(),
        Some("lobby")
    );
    log.set_description("configured");
    assert_eq!(description(&log), text("lobby"));
    log.clear_written_reporting().unwrap();
    assert_eq!(description(&log), text("lobby"));
    assert_eq!(storage.saved().unwrap().reporting.description, None);
    log.set_description("configured");
    assert_eq!(description(&log), text("configured"));
}
