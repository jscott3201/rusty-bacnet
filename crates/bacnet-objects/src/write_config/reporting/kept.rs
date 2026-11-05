//! The reporting persistence itself, compiled with the `event-reporting`
//! feature; see the [parent module](super).

use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::durable::staged::{self, StagedSaves};
use crate::durable::{PendingWrite, SaveWait, StageStep};
use crate::event::options::{MAX_KEPT_TEXT_OCTETS, ReportingOptions};
use crate::prelude::{Box, String};
use crate::traits::BACnetObject;
use crate::write_config::description;
use crate::write_config::{ReportingWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};

/// The kept rows' values as an object serves them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReportingRows {
    pub(crate) notification_class: u32,
    pub(crate) event_enable: EventTransitionBits,
    pub(crate) notify_type: NotifyType,
    pub(crate) event_detection_enable: bool,
    pub(crate) options: ReportingOptions,
    /// Description, kept beside the reporting settings (#2364).
    pub(crate) description: String,
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
    description: bool,
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
            P::DESCRIPTION => &mut self.description,
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
            description: saved.description.is_some(),
        }
    }
}

/// Whether `property` is one of the kept rows.
pub(crate) fn kept_row(property: P) -> bool {
    Written::default().flag(property).is_some()
}

/// Whether storage keeps a write of `value` to `property`: one of a kept
/// row, but not a NULL to Description, which changes nothing (#2364).
fn kept_write(property: P, value: &PropertyValue) -> bool {
    kept_row(property) && !description::null_write(property, value)
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
    /// What `write` leaves on top of this, or the object's refusal of it;
    /// `takes_notify_type` is the object type's own Notify_Type rule
    /// ([`KeepsReporting::takes_notify_type`]). A message text past what
    /// storage keeps is refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY,
    /// before anything is saved, so every backend gives that answer (#1952),
    /// once the texts are written and so kept.
    fn after(
        &self,
        write: &PendingWrite,
        takes_notify_type: fn(NotifyType) -> bool,
    ) -> Result<Self, Error> {
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
                if write.property == P::NOTIFY_TYPE && !takes_notify_type(rows.notify_type) {
                    return Err(common::value_out_of_range_error());
                }
            }
            P::EVENT_DETECTION_ENABLE => match *value {
                PropertyValue::Boolean(enable) => rows.event_detection_enable = enable,
                _ => return Err(common::invalid_data_type_error()),
            },
            // Past its cap it is refused before anything is saved (#2364).
            P::DESCRIPTION => rows.description = description::written(value)?,
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
            description: written.description.then(|| rows.description.clone()),
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

/// A copy holds no storage, as a copy of an object's Tags doesn't: an Access
/// Door's COV snapshot is a frozen copy for reading, which saves nothing.
impl Clone for ReportingStore {
    fn clone(&self) -> Self {
        Self {
            written: self.written,
            writes: self.writes,
            storage: None,
        }
    }
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
    /// Whether the object takes `notify_type`, a named value, as its
    /// Notify_Type: most take every one, and an Alert Enrollment only ALARM
    /// and EVENT. A staged write is refused as the object refuses it, and
    /// storage holding another fails the build.
    fn takes_notify_type(_notify_type: NotifyType) -> bool {
        true
    }
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
    description::check_saved(saved.description.as_deref())?;
    if let Some(reason) = crate::snapshot::write_config::reporting_invalid(&saved) {
        return Err(Error::Encoding(format!(
            "saved reporting settings {reason}"
        )));
    }
    if saved
        .notify_type
        .is_some_and(|notify_type| !T::takes_notify_type(notify_type))
    {
        return Err(Error::Encoding(
            "saved reporting settings hold a Notify_Type this object refuses".into(),
        ));
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
    if !kept_write(property, value) || !object.reporting_store_ref().attached() {
        return Ok(());
    }
    let mut store = core::mem::take(object.reporting_store());
    let current = store.kept(object.reporting_rows());
    let write = PendingWrite {
        property,
        array_index,
        value: value.clone(),
    };
    let result = admit_to(&mut store, current, &write, T::takes_notify_type);
    *object.reporting_store() = store;
    result
}

/// [`admit`] on the store taken out of its object, whose rows are `current`.
fn admit_to(
    store: &mut ReportingStore,
    current: Kept,
    write: &PendingWrite,
    takes_notify_type: fn(NotifyType) -> bool,
) -> Result<(), Error> {
    let storage = store.storage.as_mut().expect("checked by the caller");
    let claimed = storage.claim(
        write.property,
        write.array_index,
        &write.value,
        store.writes,
    );
    let next = match claimed {
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
            // A write the object refuses leaves another request's staged
            // write alone (#1424).
            let next = current.after(write, takes_notify_type)?;
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
    let kept = |write: &PendingWrite| kept_write(write.property, &write.value);
    if !object.reporting_store_ref().attached() || !writes.iter().any(kept) {
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
            if !kept(write) {
                return Ok(None);
            }
            crate::property_metadata::check_write_array_index(
                object,
                write.property,
                write.array_index,
            )?;
            let from = earlier.last().map_or(&current, |step| &step.next);
            from.after(write, T::takes_notify_type).map(Some)
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

/// Whether a setter of `property` on `object` leaves it alone: the object
/// keeps its reporting settings and a write saved the row.
pub(crate) fn setter_gives_way(
    store: &ReportingStore,
    object: ObjectIdentifier,
    property: P,
) -> bool {
    let kept = store.kept_over_setter(property);
    if kept {
        log_debug!(object = %object, ?property, "A written reporting setting kept over a setter");
    }
    kept
}

/// Where an object holds the kept rows, borrowed to put saved rows back.
pub(crate) struct RowFields<'a> {
    pub(crate) notification_class: &'a mut u32,
    pub(crate) event_enable: &'a mut EventTransitionBits,
    pub(crate) notify_type: &'a mut NotifyType,
    pub(crate) event_detection_enable: &'a mut bool,
    pub(crate) options: &'a mut ReportingOptions,
    pub(crate) description: &'a mut String,
}

impl RowFields<'_> {
    /// Serve the rows `saved` holds, leaving the others as they are.
    pub(crate) fn restore(self, saved: &ReportingWriteConfig) {
        if let Some(class) = saved.notification_class {
            *self.notification_class = class;
        }
        if let Some(enable) = saved.event_enable {
            *self.event_enable = enable;
        }
        if let Some(notify_type) = saved.notify_type {
            *self.notify_type = notify_type;
        }
        if let Some(enable) = saved.event_detection_enable {
            *self.event_detection_enable = enable;
        }
        if let Some(texts) = &saved.event_message_texts_config {
            self.options.restore_message_texts_config(texts.clone());
        }
        if let Some(reference) = &saved.event_algorithm_inhibit_ref {
            self.options
                .restore_inhibit_reference(Some(reference.clone()));
        }
        if let Some(inhibit) = saved.event_algorithm_inhibit {
            self.options.restore_inhibit(inhibit);
        }
        if let Some(text) = &saved.description {
            self.description.clone_from(text);
        }
    }
}
