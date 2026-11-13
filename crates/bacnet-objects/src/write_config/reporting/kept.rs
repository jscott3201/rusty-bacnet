//! The reporting persistence itself, compiled with the `event-reporting`
//! feature; see the [parent module](super).

use alloc::sync::Arc;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use core::time::Duration;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::parameters;
use crate::common;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::staged::{self, StagedSaves};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{PendingWrite, SaveWait, StageStep};
use crate::event::options::{MAX_KEPT_TEXT_OCTETS, ReportingOptions};
use crate::prelude::{Box, String};
use crate::snapshot::write_config::parameters as parameter_format;
use crate::traits::BACnetObject;
use crate::write_config::description;
use crate::write_config::{
    AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};

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
    /// The event algorithm parameters the object holds, whichever of them
    /// its type keeps (#2331).
    pub(crate) algorithm: AlgorithmWriteConfig,
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
    /// The written algorithm parameters, as the bits of the file's presence
    /// mask.
    parameters: u32,
}

impl Written {
    /// The flag of `property`, if it is a row every reporter keeps.
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
            parameters: parameter_format::mask(&saved.algorithm),
        }
    }

    /// Whether a write set `property` on an object of `object_type`.
    fn has(mut self, object_type: ObjectType, property: P) -> bool {
        match self.flag(property) {
            Some(flag) => *flag,
            None => parameters::kept_bit(object_type, property)
                .is_some_and(|bit| self.parameters & bit != 0),
        }
    }

    /// Note a write of `property`, a kept row of an object of
    /// `object_type`.
    fn set(&mut self, object_type: ObjectType, property: P) {
        if let Some(flag) = self.flag(property) {
            *flag = true;
        } else if let Some(bit) = parameters::kept_bit(object_type, property) {
            self.parameters |= bit;
        }
    }
}

/// Whether `property` is one of the rows an object of `object_type` keeps:
/// the reporting rows, Description, and the type's algorithm parameters.
pub(crate) fn kept_row(object_type: ObjectType, property: P) -> bool {
    Written::default().flag(property).is_some()
        || parameters::kept_bit(object_type, property).is_some()
}

/// Whether storage keeps a write of `value` to `property` of an object of
/// `object_type`: one of a kept row, but not a NULL to Description, which
/// changes nothing (#2364).
fn kept_write(object_type: ObjectType, property: P, value: &PropertyValue) -> bool {
    kept_row(object_type, property) && !description::null_write(property, value)
}

/// The rows and which of them a write set: what one staged write leaves.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    rows: ReportingRows,
    written: Written,
    /// The object's type: which algorithm parameters it keeps.
    object_type: ObjectType,
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
    /// [`into_after`](Self::into_after) on a copy, for a stage, which keeps
    /// these rows for its next step.
    // durable-writes: only a stage works a write out on a copy.
    #[cfg(feature = "durable-writes")]
    fn after(&self, write: &PendingWrite, object: &dyn KeepsReporting) -> Result<Self, Error> {
        self.clone().into_after(write, object)
    }

    /// What `write` leaves on top of this, or the object's refusal of it;
    /// `object` answers for what is its own: its Notify_Type rule
    /// ([`KeepsReporting::takes_notify_type`]) and its algorithm parameters
    /// and State_Text ([`KeepsReporting::parameter_after`]), checked against
    /// these rows, so against the state count they hold. A message text past what
    /// storage keeps is refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY,
    /// before anything is saved, so every backend gives that answer (#1952),
    /// once the texts are written and so kept.
    ///
    /// Worked out on these rows themselves, not a copy (#2526): a write made
    /// as it is admitted needs the rows before it no more. A refused write
    /// leaves them part changed, so they go with the refusal.
    fn into_after(
        mut self,
        write: &PendingWrite,
        object: &dyn KeepsReporting,
    ) -> Result<Self, Error> {
        let object_type = self.object_type;
        let rows = &mut self.rows;
        let value = &write.value;
        match write.property {
            P::NOTIFICATION_CLASS | P::EVENT_ENABLE | P::NOTIFY_TYPE => {
                // The objects' own event-row macro, so the step refuses
                // exactly what the object will.
                let mut scratch = DetectorRows::of(rows);
                common::write_generic_event_properties!(scratch, write.property, value.clone())
                    .unwrap_or_else(|| Err(common::unknown_property_error()))?;
                scratch.event_detector.apply_to(rows);
                if write.property == P::NOTIFY_TYPE && !object.takes_notify_type(rows.notify_type) {
                    return Err(common::value_out_of_range_error());
                }
            }
            P::EVENT_DETECTION_ENABLE => match *value {
                PropertyValue::Boolean(enable) => rows.event_detection_enable = enable,
                _ => return Err(common::invalid_data_type_error()),
            },
            // Past its cap it is refused before anything is saved (#2364).
            P::DESCRIPTION => rows.description = description::written(value)?,
            property if parameters::kept_bit(object_type, property).is_some() => {
                object.parameter_after(&mut rows.algorithm, write)?;
            }
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
        self.written.set(object_type, write.property);
        // Only texts storage will keep count: a long one the object took
        // before it had storage is no reason to refuse another row.
        let texts = self.rows.options.message_texts_config();
        if self.written.texts && texts.iter().any(|text| text.len() > MAX_KEPT_TEXT_OCTETS) {
            return Err(common::protocol_error(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ));
        }
        Ok(self)
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
            algorithm: parameter_format::only(&rows.algorithm, written.parameters),
        }
    }

    /// Whether `saved` is what storage keeps of this ([`saved`](Self::saved)),
    /// compared without the copy (#2431). Debug builds check that the two
    /// agree.
    // durable-writes: only a staged step is checked (#2532).
    #[cfg(feature = "durable-writes")]
    fn is_in(&self, saved: &ReportingWriteConfig) -> bool {
        let is_in = self.is_in_rows(saved);
        debug_assert_eq!(is_in, self.saved() == *saved, "is_in and saved disagree");
        is_in
    }

    #[cfg(feature = "durable-writes")]
    fn is_in_rows(&self, saved: &ReportingWriteConfig) -> bool {
        let (rows, written) = (&self.rows, self.written);
        let options = &rows.options;
        saved.notification_class == written.class.then_some(rows.notification_class)
            && saved.event_enable == written.enable.then_some(rows.event_enable)
            && saved.notify_type == written.notify_type.then_some(rows.notify_type)
            && saved.event_detection_enable
                == written.detection.then_some(rows.event_detection_enable)
            && saved.event_message_texts_config.as_ref()
                == written.texts.then(|| options.message_texts_config())
            && saved.event_algorithm_inhibit_ref.as_ref()
                == written
                    .inhibit_ref
                    .then(|| options.inhibit_reference())
                    .flatten()
            && saved.event_algorithm_inhibit == written.inhibit.then(|| options.inhibited())
            && saved.description.as_deref()
                == written.description.then_some(rows.description.as_str())
            && parameter_format::is_only(&saved.algorithm, &rows.algorithm, written.parameters)
    }

    fn snapshot(&self) -> WriteConfigSnapshot {
        WriteConfigSnapshot::Reporting(self.saved())
    }

    /// [`snapshot`](Self::snapshot), as storage is handed it.
    fn stored(&self) -> Saved {
        Arc::new(self.snapshot())
    }
}

/// A staged step keeps nothing but its write: what the write leaves is
/// worked out again from the object when the write is made. So a request's
/// stage holds the rows a constant number of times, however many writes it
/// makes (#2414). The request's last step also keeps what the stage saved
/// ([`Saved`]).
type Storage = StagedSaves<Saved, Option<Saved>>;

/// What a request's stage saved, kept with its last step (#2431). The
/// object can change between the stage and the writes: a setter such as
/// `set_number_of_states` resizes the labels a State_Text element write
/// keeps the rest of, and the server's inhibit follower changes a written
/// Event_Algorithm_Inhibit. So the request's last write checks that it
/// leaves the rows storage holds. If it doesn't, its step isn't taken, and
/// the write saves where it is, as one nobody staged. A change to what a
/// write is only checked against needs no such check: a write refused
/// then leaves the request's release to put storage back, and one the
/// stage refused has no step and saves where it is.
///
/// Storage is handed every snapshot shared, so the step keeps the one the
/// stage's save holds rather than a copy (once the save has run, only the
/// step does), and the writer's queue moves a pointer, not the rows.
type Saved = Arc<WriteConfigSnapshot>;

/// Whether `saved`, what a stage saved for `oid`, is what storage keeps of
/// `next`, the rows its last write leaves as it is made: equal, or, since a
/// limit that is no number isn't equal to itself, encoded as the same file
/// octets.
// durable-writes: only a staged step is checked (#2532).
#[cfg(feature = "durable-writes")]
fn still_saved(oid: ObjectIdentifier, saved: &WriteConfigSnapshot, next: &Kept) -> bool {
    use crate::snapshot::write_config::encode;
    matches!(saved, WriteConfigSnapshot::Reporting(saved) if next.is_in(saved))
        || matches!(
            (encode(oid, saved), encode(oid, &next.snapshot())),
            (Ok(saved), Ok(next)) if saved == next
        )
}

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
    pub(crate) fn kept_over_setter(&self, object_type: ObjectType, property: P) -> bool {
        self.attached() && self.written.has(object_type, property)
    }

    /// Block until the saves queued so far have run.
    pub(crate) fn wait_idle(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
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
    fn takes_notify_type(&self, _notify_type: NotifyType) -> bool {
        true
    }
    /// What `write`, of an algorithm parameter the object's type keeps or
    /// of a multi-state object's State_Text, leaves of `rows`, or the
    /// object's refusal of it: the object's own write path, run on a
    /// detector of its kind ([`parameters::after`]), or for State_Text on
    /// the texts `rows` hold. Alarm_Values entries meet the count of the
    /// State_Text in `rows`: the object's own, or at a stage the one the
    /// request's earlier writes leave. The default is for a type that keeps
    /// none.
    fn parameter_after(
        &self,
        _rows: &mut AlgorithmWriteConfig,
        _write: &PendingWrite,
    ) -> Result<(), Error> {
        Err(common::unknown_property_error())
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

fn store_snapshot<T: KeepsReporting>(written: &Written, object: &T) -> Saved {
    kept(*written, object).stored()
}

/// `object`'s rows as it serves them, `written` those a write set.
fn kept<T: KeepsReporting>(written: Written, object: &T) -> Kept {
    kept_of(written, object)
}

/// [`kept`], for an object seen through its trait.
fn kept_of(written: Written, object: &dyn KeepsReporting) -> Kept {
    Kept {
        rows: object.reporting_rows(),
        written,
        object_type: object.object_identifier().object_type(),
    }
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
    // A stored parameter the object would refuse on the wire, or doesn't
    // have, is refused here as the codec refuses it: a backend of the
    // application's own may hand over anything. One wire rule is left out:
    // an Alarm_Values entry past Number_Of_States is installed as stored
    // (see `parameters`), so an application's smaller count can't stop the
    // object from opening.
    let invalid = crate::snapshot::write_config::reporting_invalid(&saved)
        .or_else(|| parameter_format::invalid(oid.object_type(), &saved.algorithm));
    if let Some(reason) = invalid {
        return Err(Error::Encoding(format!(
            "saved reporting settings {reason}"
        )));
    }
    if saved
        .notify_type
        .is_some_and(|notify_type| !object.takes_notify_type(notify_type))
    {
        return Err(Error::Encoding(
            "saved reporting settings hold a Notify_Type this object refuses".into(),
        ));
    }
    object.restore_reporting_rows(&saved);
    // A multi-state object works its Reliability out again when State_Text
    // or (on a Value) Alarm_Values is written, so it does when a saved one
    // goes over the configured one: a fault only the configured value
    // raised doesn't outlive it, and a saved entry past the object's states
    // shows as CONFIGURATION_ERROR from the start.
    if (saved.algorithm.alarm_values.is_some() || saved.algorithm.state_text.is_some())
        && let Some(reliability) = object.reliability_eval()
    {
        let _ = reliability.evaluate_reliability();
    }
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
    // Storage first: an object without it needs no look at the property.
    if !object.reporting_store_ref().attached()
        || !kept_write(object.object_identifier().object_type(), property, value)
    {
        return Ok(());
    }
    admit_kept(object, property, array_index, value)
}

/// [`admit`] of a write the object keeps. Out of line, so the write path
/// of an object without storage, which passes through `admit` on every
/// write, doesn't carry it (#2431).
#[inline(never)]
fn admit_kept<T: KeepsReporting>(
    object: &mut T,
    property: P,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    let mut store = core::mem::take(object.reporting_store());
    let current = kept(store.written, object);
    let write = PendingWrite {
        property,
        array_index,
        value: value.clone(),
    };
    let result = admit_to(&mut store, current, &write, object);
    *object.reporting_store() = store;
    result
}

/// [`admit`] on the store taken out of its object, whose rows are `current`.
/// The write is worked out on `current` itself (#2526); should storage go
/// back to the rows before it, they are worked out again from `object`,
/// which makes the write only once this returns.
fn admit_to(
    store: &mut ReportingStore,
    current: Kept,
    write: &PendingWrite,
    object: &dyn KeepsReporting,
) -> Result<(), Error> {
    let storage = store.storage.as_mut().expect("checked by the caller");
    // What the write leaves, worked out from the rows the object serves
    // now; the object's refusal comes first and takes no staged step. A
    // check can turn on the object itself, which a write the store doesn't
    // count may have changed since the stage: a State_Text shrink on a
    // Present_Value, Relinquish_Default or command written earlier in the
    // same request. The stage took such a shrink, so storage briefly holds
    // labels the device then refuses; the request's release puts it back
    // (a crash inside that window leaves a file that loads, with that
    // State_Text). A write the object refuses also leaves another
    // request's staged write alone (#1424).
    let written = current.written;
    let next = current.into_after(write, object)?;
    let current = || kept_of(written, object).stored();
    #[cfg(feature = "durable-writes")]
    let claimed = claim(storage, write, store.writes, &next, object);
    // Without durable-writes nothing stages: the built-in objects don't
    // implement `DurableWrites` there, and the stand-in `claim` finds no
    // step, so there is no stale stage to check for. The check's encoding
    // fallback would link the write-configuration encoder into images that
    // never stage (#2532).
    #[cfg(not(feature = "durable-writes"))]
    let claimed = storage.claim(
        write.property,
        write.array_index,
        &write.value,
        store.writes,
    );
    match claimed {
        Some(Ok(_)) => {
            // Steps the request has still to take keep, as the state served,
            // the rows this write leaves: should the object go away before
            // the next one, storage goes back to them, this write included.
            storage.correct(|| next.stored());
        }
        Some(Err(_)) => {
            storage.correct(current);
            return Err(operational_problem());
        }
        None => {
            storage.drop_staged();
            // Storage goes back to the served rows first, should the save
            // below fail.
            storage.correct(current);
            storage
                .save_now(next.stored())
                .map_err(|_| operational_problem())?;
        }
    }
    store.written = next.written;
    store.writes = store.writes.wrapping_add(1);
    Ok(())
}

/// The staged step `write`, made at write count `writes`, takes, if it is
/// the next one: what [`StagedSaves::claim_unless`] gives, with the last
/// step stale when a change since the stage leaves the object making it
/// with rows other than those saved, `next` (#2431).
#[cfg(feature = "durable-writes")]
fn claim(
    storage: &mut Storage,
    write: &PendingWrite,
    writes: u64,
    next: &Kept,
    object: &dyn KeepsReporting,
) -> Option<Result<Option<Saved>, Error>> {
    let stale = |saved: &Option<Saved>| {
        let oid = object.object_identifier();
        saved
            .as_deref()
            .is_some_and(|saved| !still_saved(oid, saved, next))
    };
    storage.claim_unless(
        write.property,
        write.array_index,
        &write.value,
        writes,
        stale,
    )
}

/// Stage the kept rows a request writes to `object`
/// ([`DurableWrites::stage_writes`](crate::durable::DurableWrites::stage_writes)):
/// one save of what they leave together.
#[cfg(feature = "durable-writes")]
pub(crate) fn stage_writes<T: KeepsReporting>(
    object: &mut T,
    writes: &[PendingWrite],
) -> StageStep {
    let object_type = object.object_identifier().object_type();
    let kept_one = |write: &PendingWrite| kept_write(object_type, write.property, &write.value);
    if !object.reporting_store_ref().attached() || !writes.iter().any(kept_one) {
        return StageStep::Skip;
    }
    with_store(object, |store, object| {
        let storage = store.storage.as_mut().expect("checked above");
        if let Some(wait) = storage.busy() {
            return StageStep::Busy(wait);
        }
        let current = kept(store.written, object);
        // One working copy of the rows the steps so far leave; a step keeps
        // only its write.
        let mut working: Option<Kept> = None;
        let mut steps = staged::steps(writes, |_, write| {
            if !kept_one(write) {
                return Ok(None);
            }
            crate::property_metadata::check_write_array_index(
                object,
                write.property,
                write.array_index,
            )?;
            working = Some(working.as_ref().unwrap_or(&current).after(write, object)?);
            Ok(Some(None))
        });
        // A step was taken exactly when the working copy was set.
        let Some(last) = working else {
            return StageStep::Skip;
        };
        let saved = last.stored();
        if let Some(step) = steps.last_mut() {
            step.next = Some(Arc::clone(&saved));
        }
        storage.stage_built(steps, store.writes, saved, current.stored())
    })
}

/// See [`DurableWrites::release_staged_write`](crate::durable::DurableWrites::release_staged_write).
#[cfg(feature = "durable-writes")]
pub(crate) fn release<T: KeepsReporting>(object: &mut T, staged: &SaveWait) {
    with_store(object, |store, _| {
        if let Some(storage) = &mut store.storage {
            storage.release(staged);
        }
    });
}

/// See [`DurableWrites::has_staged_write`](crate::durable::DurableWrites::has_staged_write).
#[cfg(feature = "durable-writes")]
pub(crate) fn has_staged(store: &ReportingStore) -> bool {
    store.storage.as_deref().is_some_and(Storage::is_staged)
}

/// See [`DurableWrites::settle_forgotten_writes`](crate::durable::DurableWrites::settle_forgotten_writes).
#[cfg(feature = "durable-writes")]
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
#[cfg(feature = "durable-writes")]
pub(crate) fn expire<T: KeepsReporting>(object: &mut T, now: Duration) {
    with_store(object, |store, _| {
        if let Some(storage) = &mut store.storage {
            storage.expire(now);
        }
    });
}

/// Drop every kept row, the algorithm parameters with the rest: save the
/// rows with none written and wait for the save, then let the setters apply
/// again. The values served stay until a
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
        let current = kept(store.written, object);
        let cleared = Kept {
            written: Written::default(),
            ..current.clone()
        };
        // A staged step worked out from the written rows gives way.
        storage.drop_staged();
        storage.correct(|| current.stored());
        storage.save_now(cleared.stored())?;
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
    kept(store.written, object).saved()
}

/// Two stages of one request joined, as the colour objects join Tags and
/// their default (#1956): a request busy on either waits without holding the
/// other.
#[cfg(feature = "durable-writes")]
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
    let kept = store.kept_over_setter(object.object_type(), property);
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
