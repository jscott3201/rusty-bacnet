//! The reporting persistence itself, compiled with the `event-reporting`
//! feature; see the [parent module](super).

use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;
use crate::durable::staged::{self, StagedSaves};
use crate::durable::{PendingWrite, SaveWait, StageStep};
use crate::event::options::{MAX_KEPT_TEXT_OCTETS, ReportingOptions};
use crate::prelude::Box;
use crate::traits::BACnetObject;
use crate::write_config::{ReportingWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};

/// The kept rows' values as an object serves them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReportingRows {
    pub(crate) notification_class: u32,
    pub(crate) event_enable: EventTransitionBits,
    pub(crate) notify_type: NotifyType,
    pub(crate) event_detection_enable: bool,
    pub(crate) options: ReportingOptions,
}

/// The rows a write has set, now or before a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Written {
    class: bool,
    enable: bool,
    notify_type: bool,
    detection: bool,
    texts: bool,
    inhibit_ref: bool,
    inhibit: bool,
}

impl Written {
    /// The flag of `property`, if it is a kept row.
    fn flag(&mut self, property: P) -> Option<&mut bool> {
        Some(match property {
            P::NOTIFICATION_CLASS => &mut self.class,
            P::EVENT_ENABLE => &mut self.enable,
            P::NOTIFY_TYPE => &mut self.notify_type,
            P::EVENT_DETECTION_ENABLE => &mut self.detection,
            P::EVENT_MESSAGE_TEXTS_CONFIG => &mut self.texts,
            P::EVENT_ALGORITHM_INHIBIT_REF => &mut self.inhibit_ref,
            P::EVENT_ALGORITHM_INHIBIT => &mut self.inhibit,
            _ => return None,
        })
    }

    /// The rows `saved` holds.
    fn of(saved: &ReportingWriteConfig) -> Self {
        Self {
            class: saved.notification_class.is_some(),
            enable: saved.event_enable.is_some(),
            notify_type: saved.notify_type.is_some(),
            detection: saved.event_detection_enable.is_some(),
            texts: saved.event_message_texts_config.is_some(),
            inhibit_ref: saved.event_algorithm_inhibit_ref.is_some(),
            inhibit: saved.event_algorithm_inhibit.is_some(),
        }
    }
}

/// Whether `property` is one of the kept rows.
pub(crate) fn kept_row(property: P) -> bool {
    Written::default().flag(property).is_some()
}

/// The rows and which of them a write set: what one staged write leaves.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    rows: ReportingRows,
    written: Written,
}

/// The detector fields `write_generic_event_properties!` writes, around
/// the three kept rows it takes.
struct DetectorRows {
    event_detector: DetectorFields,
}

struct DetectorFields {
    notification_class: u32,
    event_enable: EventTransitionBits,
    notify_type: NotifyType,
    time_delay: u32,
    time_delay_normal: Option<u32>,
}

impl DetectorRows {
    fn of(rows: &ReportingRows) -> Self {
        Self {
            event_detector: DetectorFields {
                notification_class: rows.notification_class,
                event_enable: rows.event_enable,
                notify_type: rows.notify_type,
                time_delay: 0,
                time_delay_normal: None,
            },
        }
    }
}

impl DetectorFields {
    fn apply_to(self, rows: &mut ReportingRows) {
        rows.notification_class = self.notification_class;
        rows.event_enable = self.event_enable;
        rows.notify_type = self.notify_type;
    }
}

impl Kept {
    /// What `write` leaves on top of this, or the object's refusal of it.
    /// A message text past what storage keeps is refused with RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY, before anything is saved, so every
    /// backend gives that answer (#1952), once the texts are written and
    /// so kept.
    fn after(&self, write: &PendingWrite) -> Result<Self, Error> {
        let mut next = self.clone();
        let rows = &mut next.rows;
        let value = &write.value;
        match write.property {
            P::NOTIFICATION_CLASS | P::EVENT_ENABLE | P::NOTIFY_TYPE => {
                // The objects' own event-row macro, so the step refuses
                // exactly what the object will.
                let mut scratch = DetectorRows::of(rows);
                common::write_generic_event_properties!(scratch, write.property, value.clone())
                    .unwrap_or_else(|| Err(common::unknown_property_error()))?;
                scratch.event_detector.apply_to(rows);
            }
            P::EVENT_DETECTION_ENABLE => match *value {
                PropertyValue::Boolean(enable) => rows.event_detection_enable = enable,
                _ => return Err(common::invalid_data_type_error()),
            },
            property => rows
                .options
                .write(
                    property,
                    write.array_index,
                    value,
                    rows.event_detection_enable,
                )
                .unwrap_or_else(|| Err(common::unknown_property_error()))?,
        }
        if let Some(flag) = next.written.flag(write.property) {
            *flag = true;
        }
        // Only texts storage will keep count: a long one the object took
        // before it had storage is no reason to refuse another row.
        let texts = next.rows.options.message_texts_config();
        if next.written.texts && texts.iter().any(|text| text.len() > MAX_KEPT_TEXT_OCTETS) {
            return Err(common::protocol_error(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ));
        }
        Ok(next)
    }

    /// What storage keeps of this: the rows a write set.
    fn saved(&self) -> ReportingWriteConfig {
        let (rows, written) = (&self.rows, self.written);
        ReportingWriteConfig {
            notification_class: written.class.then_some(rows.notification_class),
            event_enable: written.enable.then_some(rows.event_enable),
            notify_type: written.notify_type.then_some(rows.notify_type),
            event_detection_enable: written.detection.then_some(rows.event_detection_enable),
            event_message_texts_config: written
                .texts
                .then(|| rows.options.message_texts_config().clone()),
            event_algorithm_inhibit_ref: written
                .inhibit_ref
                .then(|| rows.options.inhibit_reference().cloned())
                .flatten(),
            event_algorithm_inhibit: written.inhibit.then(|| rows.options.inhibited()),
        }
    }

    fn snapshot(&self) -> WriteConfigSnapshot {
        WriteConfigSnapshot::Reporting(self.saved())
    }
}

type Storage = StagedSaves<WriteConfigSnapshot, Kept>;

/// An object's reporting storage: none until it opts in.
#[derive(Default)]
pub(crate) struct ReportingStore {
    written: Written,
    /// Writes of the kept rows made so far, so a staged step is taken only
    /// if no other came between.
    writes: u64,
    /// Boxed: every object with intrinsic reporting holds a store, and most
    /// never attach storage, so an unattached object pays one pointer, not
    /// the staged-save state (the firmware `reporting` scenario's arena,
    /// #2331).
    storage: Option<Box<Storage>>,
}

impl ReportingStore {
    /// Whether the object keeps its reporting settings in storage.
    pub(crate) fn attached(&self) -> bool {
        self.storage.is_some()
    }

    /// Whether a setter of `property` leaves it alone: the object has
    /// storage and a write has set the row.
    pub(crate) fn kept_over_setter(&self, property: P) -> bool {
        let mut written = self.written;
        self.attached() && written.flag(property).is_some_and(|flag| *flag)
    }

    /// Block until the saves queued so far have run.
    pub(crate) fn wait_idle(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    fn kept(&self, rows: ReportingRows) -> Kept {
        Kept {
            rows,
            written: self.written,
        }
    }
}

/// An object that keeps its reporting settings: where its store is, and
/// how its rows are read and restored.
pub(crate) trait KeepsReporting: BACnetObject {
    fn reporting_store(&mut self) -> &mut ReportingStore;
    fn reporting_store_ref(&self) -> &ReportingStore;
    /// The kept rows' values as served now.
    fn reporting_rows(&self) -> ReportingRows;
    /// Serve the rows `saved` holds.
    fn restore_reporting_rows(&mut self, saved: &ReportingWriteConfig);
}

/// Run `f` on `object`'s store, which is taken out meanwhile so `f` can look
/// at the object; then, if `f` dropped a staged write, queue the save that
/// puts storage back to what the object serves.
fn with_store<T: KeepsReporting, R>(
    object: &mut T,
    f: impl FnOnce(&mut ReportingStore, &T) -> R,
) -> R {
    let mut store = core::mem::take(object.reporting_store());
    let result = f(&mut store, object);
    if let Some(storage) = &mut store.storage {
        storage.correct(|| store_snapshot(&store.written, object));
    }
    *object.reporting_store() = store;
    result
}

fn store_snapshot<T: KeepsReporting>(written: &Written, object: &T) -> WriteConfigSnapshot {
    Kept {
        rows: object.reporting_rows(),
        written: *written,
    }
    .snapshot()
}

fn operational_problem() -> Error {
    common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
}

/// Keep `object`'s reporting settings in `persistence`, serving what it saved
/// for the object, if anything. Fails when it can't be loaded, holds another
/// kind of configuration, or holds what no write leaves.
pub(crate) fn attach<T: KeepsReporting>(
    mut object: T,
    persistence: Arc<dyn WriteConfigPersistence>,
) -> Result<T, Error> {
    let oid = object.object_identifier();
    let saved = match persistence.load(oid)? {
        None => ReportingWriteConfig::default(),
        Some(WriteConfigSnapshot::Reporting(saved)) => saved,
        Some(_) => {
            return Err(Error::Encoding(
                "saved write configuration is not reporting settings".into(),
            ));
        }
    };
    if let Some(reason) = crate::snapshot::write_config::reporting_invalid(&saved) {
        return Err(Error::Encoding(format!(
            "saved reporting settings {reason}"
        )));
    }
    object.restore_reporting_rows(&saved);
    let store = object.reporting_store();
    store.written = Written::of(&saved);
    store.storage = Some(Box::new(StagedSaves::new(
        crate::write_config::saving::writer(oid, persistence),
    )));
    Ok(object)
}

/// Before `object` makes a write of `property`: when it keeps the row,
/// take the write's staged step, or save what the write leaves where it is.
/// `Ok` lets the object make the write as usual; an error refuses it: the
/// object's own refusal, or DEVICE / OPERATIONAL_PROBLEM when the save
/// failed. The caller has checked the array index.
pub(crate) fn admit<T: KeepsReporting>(
    object: &mut T,
    property: P,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    if !kept_row(property) || !object.reporting_store_ref().attached() {
        return Ok(());
    }
    let mut store = core::mem::take(object.reporting_store());
    let current = store.kept(object.reporting_rows());
    let result = admit_to(&mut store, current, property, array_index, value);
    *object.reporting_store() = store;
    result
}

/// [`admit`] on the store taken out of its object, whose rows are `current`.
fn admit_to(
    store: &mut ReportingStore,
    current: Kept,
    property: P,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    let storage = store.storage.as_mut().expect("checked by the caller");
    let next = match storage.claim(property, array_index, value, store.writes) {
        Some(Ok(next)) => {
            // Steps the request has still to take keep, as the state served,
            // the rows this write leaves.
            storage.correct(|| next.snapshot());
            next
        }
        Some(Err(_)) => {
            storage.correct(|| current.snapshot());
            return Err(operational_problem());
        }
        None => {
            let write = PendingWrite {
                property,
                array_index,
                value: value.clone(),
            };
            // A write the object refuses leaves another request's staged
            // write alone (#1424).
            let next = current.after(&write)?;
            storage.drop_staged();
            // Storage goes back to the served rows first, should the save
            // below fail.
            storage.correct(|| current.snapshot());
            storage
                .save_now(next.snapshot())
                .map_err(|_| operational_problem())?;
            next
        }
    };
    store.written = next.written;
    store.writes = store.writes.wrapping_add(1);
    Ok(())
}

/// Stage the kept rows a request writes to `object`
/// ([`DurableWrites::stage_writes`](crate::durable::DurableWrites::stage_writes)):
/// one save of what they leave together.
pub(crate) fn stage_writes<T: KeepsReporting>(
    object: &mut T,
    writes: &[PendingWrite],
) -> StageStep {
    if !object.reporting_store_ref().attached() || !writes.iter().any(|w| kept_row(w.property)) {
        return StageStep::Skip;
    }
    with_store(object, |store, object| {
        let storage = store.storage.as_mut().expect("checked above");
        if let Some(wait) = storage.busy() {
            return StageStep::Busy(wait);
        }
        let current = Kept {
            rows: object.reporting_rows(),
            written: store.written,
        };
        let steps = staged::steps(writes, |earlier, write| {
            if !kept_row(write.property) {
                return Ok(None);
            }
            crate::property_metadata::check_write_array_index(
                object,
                write.property,
                write.array_index,
            )?;
            let from = earlier.last().map_or(&current, |step| &step.next);
            from.after(write).map(Some)
        });
        storage.stage(steps, store.writes, current.snapshot(), |snapshot, kept| {
            *snapshot = kept.snapshot();
        })
    })
}

/// See [`DurableWrites::release_staged_write`](crate::durable::DurableWrites::release_staged_write).
pub(crate) fn release<T: KeepsReporting>(object: &mut T, staged: &SaveWait) {
    with_store(object, |store, _| {
        if let Some(storage) = &mut store.storage {
            storage.release(staged);
        }
    });
}

/// See [`DurableWrites::has_staged_write`](crate::durable::DurableWrites::has_staged_write).
pub(crate) fn has_staged(store: &ReportingStore) -> bool {
    store.storage.as_deref().is_some_and(Storage::is_staged)
}

/// See [`DurableWrites::settle_forgotten_writes`](crate::durable::DurableWrites::settle_forgotten_writes).
pub(crate) fn settle<T: KeepsReporting>(object: &mut T) -> Option<SaveWait> {
    let mut store = core::mem::take(object.reporting_store());
    let written = store.written;
    let wait = store
        .storage
        .as_mut()
        .map(|storage| storage.drop_forgotten(|| store_snapshot(&written, object)));
    *object.reporting_store() = store;
    wait
}

/// See [`DurableWrites::expire_staged`](crate::durable::DurableWrites::expire_staged).
pub(crate) fn expire<T: KeepsReporting>(object: &mut T, now: Duration) {
    with_store(object, |store, _| {
        if let Some(storage) = &mut store.storage {
            storage.expire(now);
        }
    });
}

/// Drop every kept row: save the rows with none written and wait for the
/// save, then let the setters apply again. The values served stay until a
/// setter or a write changes them. A save that fails keeps the rows written
/// and returns storage's error. Does nothing without storage.
pub(crate) fn clear<T: KeepsReporting>(object: &mut T) -> Result<(), Error> {
    with_store(object, |store, object| {
        let Some(storage) = store.storage.as_mut() else {
            return Ok(());
        };
        if store.written == Written::default() {
            return Ok(());
        }
        let current = Kept {
            rows: object.reporting_rows(),
            written: store.written,
        };
        let cleared = Kept {
            rows: current.rows.clone(),
            written: Written::default(),
        };
        // A staged step worked out from the written rows gives way.
        storage.drop_staged();
        storage.correct(|| current.snapshot());
        storage.save_now(cleared.snapshot())?;
        store.written = Written::default();
        store.writes = store.writes.wrapping_add(1);
        Ok(())
    })
}

/// The rows storage keeps because a write set them; all `None` without
/// storage.
pub(crate) fn written<T: KeepsReporting>(object: &T) -> ReportingWriteConfig {
    let store = object.reporting_store_ref();
    if !store.attached() {
        return ReportingWriteConfig::default();
    }
    store.kept(object.reporting_rows()).saved()
}

/// Two stages of one request joined, as the colour objects join Tags and
/// their default (#1956): a request busy on either waits without holding the
/// other.
pub(crate) fn joined(
    tags: StageStep,
    reporting: StageStep,
    release_tags: impl FnOnce(&SaveWait),
) -> StageStep {
    match (tags, reporting) {
        (StageStep::Staged(tags), StageStep::Busy(wait)) => {
            release_tags(&tags);
            StageStep::Busy(wait)
        }
        (StageStep::Staged(tags), StageStep::Staged(reporting)) => {
            StageStep::Staged(tags.and(reporting))
        }
        (StageStep::Skip, step) | (step, StageStep::Skip) => step,
        (_, step) => step,
    }
}

/// Put `saved`'s Event_Message_Texts_Config and Event_Algorithm_Inhibit pair
/// into `options`.
pub(crate) fn restore_options(options: &mut ReportingOptions, saved: &ReportingWriteConfig) {
    if let Some(texts) = &saved.event_message_texts_config {
        options.restore_message_texts_config(texts.clone());
    }
    if let Some(reference) = &saved.event_algorithm_inhibit_ref {
        options.restore_inhibit_reference(Some(reference.clone()));
    }
    if let Some(inhibit) = saved.event_algorithm_inhibit {
        options.restore_inhibit(inhibit);
    }
}

/// Implement reporting persistence for `$object`, which holds the kept rows
/// in the usual fields (`event_detector`, `event_detection_enable`,
/// `event_history`), its Tags storage in `profile`, and a [`ReportingStore`]
/// in `reporting_store`. It gets `with_reporting_persistence`,
/// `written_reporting`, `clear_written_reporting`, the three setters, and
/// [`DurableWrites`](crate::durable::DurableWrites) staging both its Tags and
/// its reporting settings; its `durable_writes` returns
/// `self.durable_capability()`, and its `write_property` offers each write
/// to [`admit`] once the array index is checked.
macro_rules! impl_reporting_persistence {
    ($object:ty) => {
        impl $object {
            /// Keep the reporting settings peers write (Notification_Class,
            /// Event_Enable, Notify_Type, Event_Detection_Enable,
            /// Event_Message_Texts_Config and the Event_Algorithm_Inhibit
            /// pair) in `persistence`, serving what is saved there for this
            /// object, if anything (#2331).
            ///
            /// A write of one of them is then saved before the object serves
            /// it, and refused with DEVICE / OPERATIONAL_PROBLEM, the old
            /// value kept, when the save fails. A row a write saved wins over
            /// its setter until [`clear_written_reporting`](Self::clear_written_reporting).
            /// Fails when storage can't be loaded, holds another kind of
            /// configuration, or holds what no write leaves.
            pub fn with_reporting_persistence(
                self,
                persistence: alloc::sync::Arc<dyn $crate::write_config::WriteConfigPersistence>,
            ) -> Result<Self, bacnet_types::error::Error> {
                $crate::write_config::reporting::attach(self, persistence)
            }

            /// The reporting settings storage keeps because a write set them,
            /// now or before a restart; all `None` without storage.
            pub fn written_reporting(&self) -> $crate::write_config::ReportingWriteConfig {
                $crate::write_config::reporting::written(self)
            }

            /// Drop every reporting setting a write saved, so the setters
            /// apply again. The values served stay until a setter or a write
            /// changes them. Saves before it returns; a save that fails keeps
            /// the written settings and returns storage's error. Does nothing
            /// without storage. Call it before the object joins the database,
            /// as the setters are: it waits for its save.
            pub fn clear_written_reporting(&mut self) -> Result<(), bacnet_types::error::Error> {
                $crate::write_config::reporting::clear(self)
            }

            /// Block until the reporting saves queued so far have run.
            /// Dropping the object waits for them too.
            pub fn wait_for_reporting_saves(&self) {
                self.reporting_store.wait_idle();
            }

            /// Set Notification_Class. On an object keeping its reporting
            /// settings, a value a write saved wins, as for the next two.
            pub fn set_notification_class(&mut self, class: u32) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::NOTIFICATION_CLASS) {
                    self.event_detector.notification_class = class;
                }
            }

            /// Set Event_Enable.
            pub fn set_event_enable(&mut self, enable: bacnet_types::bitstring::EventTransitionBits) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE) {
                    self.event_detector.event_enable = enable;
                }
            }

            /// Set Notify_Type.
            pub fn set_notify_type(&mut self, notify_type: bacnet_types::enums::NotifyType) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE) {
                    self.event_detector.notify_type = notify_type;
                }
            }

            fn kept_over_setter(&self, property: bacnet_types::enums::PropertyIdentifier) -> bool {
                let kept = self.reporting_store.kept_over_setter(property);
                if kept {
                    log_debug!(object = %self.oid, ?property, "A written reporting setting kept over a setter");
                }
                kept
            }

            /// The staging this object lends: its Tags and its reporting
            /// settings, whichever it keeps.
            #[cfg(feature = "durable-writes")]
            fn durable_capability(&mut self) -> Option<&mut dyn $crate::durable::DurableWrites> {
                let keeps = self.profile.capability().is_some() || self.reporting_store.attached();
                keeps.then_some(self as _)
            }
        }

        impl $crate::write_config::reporting::KeepsReporting for $object {
            fn reporting_store(&mut self) -> &mut $crate::write_config::reporting::ReportingStore {
                &mut self.reporting_store
            }

            fn reporting_store_ref(&self) -> &$crate::write_config::reporting::ReportingStore {
                &self.reporting_store
            }

            fn reporting_rows(&self) -> $crate::write_config::reporting::ReportingRows {
                $crate::write_config::reporting::ReportingRows {
                    notification_class: self.event_detector.notification_class,
                    event_enable: self.event_detector.event_enable,
                    notify_type: self.event_detector.notify_type,
                    event_detection_enable: self.event_detection_enable,
                    options: self.event_history.options.clone(),
                }
            }

            fn restore_reporting_rows(&mut self, saved: &$crate::write_config::ReportingWriteConfig) {
                if let Some(class) = saved.notification_class {
                    self.event_detector.notification_class = class;
                }
                if let Some(enable) = saved.event_enable {
                    self.event_detector.event_enable = enable;
                }
                if let Some(notify_type) = saved.notify_type {
                    self.event_detector.notify_type = notify_type;
                }
                if let Some(enable) = saved.event_detection_enable {
                    self.event_detection_enable = enable;
                }
                $crate::write_config::reporting::restore_options(&mut self.event_history.options, saved);
            }
        }

        impl $crate::durable::DurableWrites for $object {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.stage_writes(&[$crate::durable::PendingWrite {
                    property,
                    array_index,
                    value: value.clone(),
                }])
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                let tags = $crate::durable::DurableWrites::stage_writes(&mut self.profile, writes);
                if matches!(tags, $crate::durable::StageStep::Busy(_)) {
                    return tags;
                }
                let reporting = $crate::write_config::reporting::stage_writes(self, writes);
                $crate::write_config::reporting::joined(tags, reporting, |tags| {
                    $crate::durable::DurableWrites::release_staged_write(&mut self.profile, tags);
                })
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                $crate::durable::DurableWrites::release_staged_write(&mut self.profile, staged);
                $crate::write_config::reporting::release(self, staged);
            }

            fn has_staged_write(&self) -> bool {
                $crate::durable::DurableWrites::has_staged_write(&self.profile)
                    || $crate::write_config::reporting::has_staged(&self.reporting_store)
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                let tags = $crate::durable::DurableWrites::settle_forgotten_writes(&mut self.profile);
                match (tags, $crate::write_config::reporting::settle(self)) {
                    (Some(tags), Some(reporting)) => Some(tags.and(reporting)),
                    (tags, reporting) => tags.or(reporting),
                }
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::durable::DurableWrites::expire_staged(&mut self.profile, now);
                $crate::write_config::reporting::expire(self, now);
            }
        }
    };
}

pub(crate) use impl_reporting_persistence;
