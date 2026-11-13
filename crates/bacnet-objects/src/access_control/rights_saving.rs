//! When an Access Rights object saves Positive_Access_Rules,
//! Negative_Access_Rules, Enable (#1392) and Accompaniment (#1393).
//!
//! An object built with [`AccessRightsObject::with_persistence`] saves each
//! write of those four properties before serving it: whole arrays, single
//! elements and index-0 resizes alike, since storage holds whole arrays. A
//! write it cannot save is refused with DEVICE / OPERATIONAL_PROBLEM, and the
//! old value stays. This is the Notification Class's scheme
//! ([`crate::durable`]), shared with it through `StagedSaves`: the bundled
//! server stages the write, the save runs on the object's writer thread while
//! the database guard is dropped, and the write then takes the saved state
//! without saving again. A WritePropertyMultiple's several writes to the
//! object stage one save of the state they leave together, and each takes
//! its own step of it in turn (#1423). A write that was not staged queues its
//! save and waits for it where it is.
//!
//! A staged write its request releases without making is dropped, and the
//! object at once queues a save of what it serves, so storage goes back to
//! that. A staged write whose request vanished is dropped the same way once
//! `STAGED_WRITE_LIFETIME` has
//! passed since its save finished, by the next write that stages or by the
//! server's once-a-second operation task. The server's `stop()` drops one
//! still held and waits for the correcting save, and an object dropped with
//! one still held saves its served state as it goes, unless the staged save
//! failed (#1363). Either way a restart serves what the object served.
//!
//! A written value wins over what the application configures: once a write
//! has set a property and it was saved, a rebuilt object serves the saved
//! value, and that property's setter checks what it is given without
//! storing it. Configuration alone is never saved, but a write saves the
//! whole array it leaves, so an element or index-0 write to an array no
//! write has set yet also saves the configured rules it didn't touch.
//!
//! A written Description is kept in the same file (#2364), saved and staged
//! as the others are. A NULL changes nothing, so it is neither saved nor
//! staged, and one past 16 KiB is refused with RESOURCES /
//! NO_SPACE_TO_WRITE_PROPERTY.

use crate::prelude::*;
use alloc::sync::Arc;
#[cfg(feature = "durable-writes")]
use core::time::Duration;

use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::persistence::{AccessRightsPersistence, AccessRightsSnapshot};
use super::{AccessRightsObject, check_accompaniment, checked_rules};
use crate::common;
use crate::durable::SaveWriter;
#[cfg(feature = "durable-writes")]
use crate::durable::staged;
use crate::durable::staged::StagedSaves;
#[cfg(feature = "durable-writes")]
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use crate::write_config::description;

/// The state a saved write leaves: the property it sets, as the object
/// holds it.
#[derive(Clone)]
pub(super) enum NextState {
    PositiveAccessRules(Vec<BACnetAccessRule>),
    NegativeAccessRules(Vec<BACnetAccessRule>),
    Enable(bool),
    Accompaniment(BACnetDeviceObjectReference),
    Description(String),
    /// A staged step's rule-array write with an index: the change it makes
    /// to the array served as it is taken (#2526). A staged step holds what
    /// its write sets; for a whole array, Enable, Accompaniment or a
    /// Description, that is the whole state it leaves.
    #[cfg(feature = "durable-writes")]
    Staged(RulesChange),
}

/// What a rule-array write with an index sets (#2526), in
/// Positive_Access_Rules (`true`) or Negative_Access_Rules.
#[cfg(feature = "durable-writes")]
#[derive(Clone)]
pub(super) enum RulesChange {
    /// The size an index-0 write gives the array.
    Size(bool, usize),
    /// The rule an element write puts at its zero-based slot.
    Rule(bool, usize, BACnetAccessRule),
}

#[cfg(feature = "durable-writes")]
impl RulesChange {
    /// The change a write at `index` to one of the arrays makes, given
    /// `rules`, the array it leaves: `None` for a whole write.
    fn of(positive: bool, index: Option<u32>, rules: &[BACnetAccessRule]) -> Option<Self> {
        match index? {
            0 => Some(Self::Size(positive, rules.len())),
            index => {
                let slot = usize::try_from(index.checked_sub(1)?).ok()?;
                Some(Self::Rule(positive, slot, rules.get(slot)?.clone()))
            }
        }
    }

    /// Whether the change is to Positive_Access_Rules.
    fn positive(&self) -> bool {
        match *self {
            Self::Size(positive, _) | Self::Rule(positive, ..) => positive,
        }
    }

    /// Make the change on `rules`, the array the write was worked out on. A
    /// rule's slot is inside it: the claim that took the step checked it
    /// ([`AccessRightsObject::makes`]).
    fn make(&self, rules: &mut Vec<BACnetAccessRule>) {
        match self {
            Self::Size(_, size) => rules.resize_with(*size, super::rights_writes::grown_rule),
            Self::Rule(_, slot, rule) => {
                debug_assert!(*slot < rules.len(), "a staged rule past the array served");
                if let Some(served) = rules.get_mut(*slot) {
                    *served = rule.clone();
                }
            }
        }
    }
}

/// The part of the served state a claimed write changed (#2526), so the
/// served state a staged write keeps follows it without being copied whole.
#[cfg(feature = "durable-writes")]
enum Touched {
    /// A rule array, and the change an indexed write made to it.
    Rules(bool, Option<RulesChange>),
    Enable,
    Accompaniment,
    Description,
}

#[cfg(feature = "durable-writes")]
impl Touched {
    fn of(next: &NextState) -> Self {
        match next {
            NextState::PositiveAccessRules(_) => Self::Rules(true, None),
            NextState::NegativeAccessRules(_) => Self::Rules(false, None),
            NextState::Staged(change) => Self::Rules(change.positive(), Some(change.clone())),
            NextState::Enable(_) => Self::Enable,
            NextState::Accompaniment(_) => Self::Accompaniment,
            NextState::Description(_) => Self::Description,
        }
    }
}

/// Why only a staged step holds a [`RulesChange`]: every other write is
/// worked out, and saved, as the whole state it leaves.
#[cfg(feature = "durable-writes")]
const WHOLE: &str = "a write is worked out as the whole state it leaves";

/// An object's writer and the write staged on it. A request's staged steps
/// each hold what their write sets, not the state it leaves (#2526): an
/// element or index-0 write of a rule array holds only its change, made on
/// the array served as the write is taken, so a request of such writes holds
/// the arrays a constant number of times however many it makes. The stage
/// works the steps out on one working copy and saves it once.
pub(super) type Storage = StagedSaves<AccessRightsSnapshot, NextState>;

/// Which saved properties a write has set, now or before a restart.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Written {
    positive: bool,
    negative: bool,
    enable: bool,
    accompaniment: bool,
    description: bool,
}

/// Whether an Access Rights object with storage saves a write of `value` to
/// `property`: a rule array, Enable, Accompaniment, or a Description other
/// than a NULL, which changes nothing (#2364).
pub(super) fn saved_write(property: PropertyIdentifier, value: &PropertyValue) -> bool {
    matches!(
        property,
        PropertyIdentifier::POSITIVE_ACCESS_RULES
            | PropertyIdentifier::NEGATIVE_ACCESS_RULES
            | PropertyIdentifier::ENABLE
            | PropertyIdentifier::ACCOMPANIMENT
    ) || (property == PropertyIdentifier::DESCRIPTION && !description::null_write(property, value))
}

impl AccessRightsObject {
    /// An Access Rights object that keeps written rule arrays, Enable and
    /// Accompaniment in `persistence`, starting from what is saved there for
    /// this object, if anything. A saved Accompaniment serves the optional
    /// row whether or not the application sets one.
    ///
    /// Each saved array and Accompaniment goes through the setters' checks,
    /// so this fails when a saved array holds more than
    /// [`MAX_ACCESS_RULES`](super::MAX_ACCESS_RULES) rules or a rule the
    /// setters refuse, or the saved Accompaniment is one
    /// [`set_accompaniment`](AccessRightsObject::set_accompaniment) refuses,
    /// or the saved Description is past 16 KiB, as well as when storage
    /// cannot be read. A written Description is kept there too (#2364).
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn AccessRightsPersistence>,
    ) -> Result<Self, Error> {
        let mut rights = Self::new(instance, name)?;
        let oid = rights.oid;
        if let Some(saved) = persistence.load(oid)? {
            if let Some(rules) = saved.positive_access_rules {
                rights.positive_access_rules = checked_rules(rules)?;
                rights.written.positive = true;
            }
            if let Some(rules) = saved.negative_access_rules {
                rights.negative_access_rules = checked_rules(rules)?;
                rights.written.negative = true;
            }
            if let Some(enable) = saved.enable {
                rights.enable = enable;
                rights.written.enable = true;
            }
            if let Some(reference) = saved.accompaniment {
                check_accompaniment(&reference)?;
                rights.accompaniment = Some(reference);
                rights.written.accompaniment = true;
            }
            description::check_saved(saved.description.as_deref())?;
            if let Some(text) = saved.description {
                rights.description = text;
                rights.written.description = true;
            }
        }
        let writer = SaveWriter::new(
            format!("bacnet-ar-{instance}-save"),
            move |snapshot: &AccessRightsSnapshot| persistence.save(oid, snapshot),
            move |_, result| {
                if let Err(error) = result {
                    log_warn!(rights = %oid, %error, "Failed to save Access Rights state");
                }
            },
        );
        rights.storage = Some(StagedSaves::new(writer));
        Ok(rights)
    }

    /// Whether storage holds a value a write set for `property`
    /// (Positive_Access_Rules, Negative_Access_Rules, Enable, property 133,
    /// Accompaniment or Description), now or before a restart. That
    /// property's setter then leaves it alone. Always false without
    /// persistence, and for any other property.
    pub fn property_saved(&self, property: PropertyIdentifier) -> bool {
        self.storage.is_some()
            && match property {
                PropertyIdentifier::POSITIVE_ACCESS_RULES => self.written.positive,
                PropertyIdentifier::NEGATIVE_ACCESS_RULES => self.written.negative,
                PropertyIdentifier::ENABLE => self.written.enable,
                PropertyIdentifier::ACCOMPANIMENT => self.written.accompaniment,
                PropertyIdentifier::DESCRIPTION => self.written.description,
                _ => false,
            }
    }

    /// Block until the saves queued so far have run. Dropping the object
    /// waits for them too.
    pub fn wait_for_saves(&self) {
        if let Some(storage) = &self.storage {
            storage.wait_idle();
        }
    }

    /// Whether a setter call for `property` leaves it alone, because a
    /// saved write set it.
    pub(super) fn keeps_saved(&self, property: PropertyIdentifier) -> bool {
        let kept = self.property_saved(property);
        if kept {
            log_debug!(
                rights = %self.oid,
                ?property,
                "Saved Access Rights value kept over a configured one"
            );
        }
        kept
    }

    /// Drop the Description a write saved (#2364): storage keeps none from
    /// now on, so a rebuilt object starts without one
    /// ([`property_saved`](Self::property_saved) is then false for Description).
    /// The text served now stays until a write changes it. Saves before it
    /// returns; a save that fails keeps the written Description and returns
    /// storage's error. Does nothing without storage or a saved Description.
    /// Call it before the object joins the database, as the setters are,
    /// not under the database guard: it waits for its save.
    pub fn clear_written_description(&mut self) -> Result<(), Error> {
        if !self.property_saved(PropertyIdentifier::DESCRIPTION) {
            return Ok(());
        }
        let mut cleared = self.snapshot();
        cleared.description = None;
        let storage = self.storage.as_mut().expect("checked above");
        storage.save_now(cleared)?;
        self.written.description = false;
        // A write staged before this gives way, and keeps the cleared state
        // as what is served.
        self.writes = self.writes.wrapping_add(1);
        self.with_storage(|_| ());
        Ok(())
    }

    /// What storage holds for the state the object serves.
    fn snapshot(&self) -> AccessRightsSnapshot {
        let served = |written: bool, rules: &Vec<BACnetAccessRule>| written.then(|| rules.clone());
        AccessRightsSnapshot {
            positive_access_rules: served(self.written.positive, &self.positive_access_rules),
            negative_access_rules: served(self.written.negative, &self.negative_access_rules),
            enable: self.written.enable.then_some(self.enable),
            accompaniment: self
                .written
                .accompaniment
                .then(|| self.accompaniment.clone())
                .flatten(),
            description: self.written.description.then(|| self.description.clone()),
        }
    }

    /// The state a write of `value` to `property` (at `array_index`) would
    /// leave on top of `staged_positive` and `staged_negative`, the rule
    /// arrays the earlier steps of its request leave (`None` for one they
    /// don't set), or the write's refusal. The checks are the ones every
    /// write makes, with or without persistence.
    fn next_state(
        &self,
        staged_positive: Option<&[BACnetAccessRule]>,
        staged_negative: Option<&[BACnetAccessRule]>,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<NextState, Error> {
        // An element or index-0 write edits the array an earlier step left.
        let current = match property {
            PropertyIdentifier::POSITIVE_ACCESS_RULES => {
                staged_positive.unwrap_or(&self.positive_access_rules)
            }
            PropertyIdentifier::NEGATIVE_ACCESS_RULES => {
                staged_negative.unwrap_or(&self.negative_access_rules)
            }
            // Only the application adds the optional row.
            PropertyIdentifier::ACCOMPANIMENT if self.accompaniment.is_none() => {
                return Err(common::unknown_property_error());
            }
            // Enable, Accompaniment and Description are no arrays: the
            // handlers refuse an index before the object sees it, and so
            // does a direct call.
            _ if array_index.is_some() => return Err(common::property_is_not_an_array_error()),
            PropertyIdentifier::DESCRIPTION => {
                return description::written(&value).map(NextState::Description);
            }
            PropertyIdentifier::ACCOMPANIMENT => {
                let reference: BACnetDeviceObjectReference =
                    crate::device_reference::decode_reference(&value)?;
                check_accompaniment(&reference)?;
                return Ok(NextState::Accompaniment(reference));
            }
            _ => {
                return match value {
                    PropertyValue::Boolean(enable) => Ok(NextState::Enable(enable)),
                    _ => Err(common::invalid_data_type_error()),
                };
            }
        };
        // An indexed write edits the array the object serves; a whole write
        // replaces it.
        let mut next = match array_index {
            Some(_) => current.to_vec(),
            None => Vec::new(),
        };
        super::super::rights_writes::write_rules(&mut next, array_index, value)?;
        Ok(if property == PropertyIdentifier::POSITIVE_ACCESS_RULES {
            NextState::PositiveAccessRules(next)
        } else {
            NextState::NegativeAccessRules(next)
        })
    }

    fn install(&mut self, next: NextState) {
        match next {
            NextState::PositiveAccessRules(rules) => {
                self.positive_access_rules = rules;
                self.written.positive = true;
            }
            NextState::NegativeAccessRules(rules) => {
                self.negative_access_rules = rules;
                self.written.negative = true;
            }
            NextState::Enable(enable) => {
                self.enable = enable;
                self.written.enable = true;
            }
            NextState::Accompaniment(reference) => {
                self.accompaniment = Some(reference);
                self.written.accompaniment = true;
            }
            NextState::Description(text) => {
                self.description = text;
                self.written.description = true;
            }
            // A staged step's change, made on the array served: the one the
            // stage made it on, since nothing came between (#2526).
            #[cfg(feature = "durable-writes")]
            NextState::Staged(change) => {
                let positive = change.positive();
                let served = if positive {
                    &mut self.positive_access_rules
                } else {
                    &mut self.negative_access_rules
                };
                let mut rules = core::mem::take(served);
                change.make(&mut rules);
                return self.install(if positive {
                    NextState::PositiveAccessRules(rules)
                } else {
                    NextState::NegativeAccessRules(rules)
                });
            }
        }
        self.writes = self.writes.wrapping_add(1);
    }

    /// Write Positive_Access_Rules, Negative_Access_Rules, Enable,
    /// Accompaniment or, with storage, Description, saving the result
    /// before the object serves it. A
    /// staged write takes the state already saved; any other saves now. A
    /// state that cannot be saved is refused with DEVICE /
    /// OPERATIONAL_PROBLEM and the old value stays.
    pub(super) fn write_saved(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let refused =
            |_| common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
        if let Some(taken) = self.take_staged(property, array_index, &value) {
            return taken.map_err(refused);
        }
        // A write the object refuses, such as one past the end of an array,
        // leaves a write staged for another request alone (#1424).
        let next = self.next_state(None, None, property, array_index, value)?;
        if self.storage.is_some() {
            let mut snapshot = self.snapshot();
            apply(&mut snapshot, &next);
            // This write will be made, so it supersedes a staged one: storage
            // goes back to the served state ahead of its save.
            self.with_storage(Storage::drop_staged);
            let storage = self.storage.as_mut().expect("checked above");
            storage.save_now(snapshot).map_err(refused)?;
        }
        self.install(next);
        Ok(())
    }

    /// Take the staged step for a write of `value` to `property` (at
    /// `array_index`), if it is the next one: the object then serves its
    /// state, or the save's error comes back. `None` when nothing staged is
    /// this write, or the step can't be made on the rules served
    /// ([`makes`](Self::makes)).
    #[cfg(feature = "durable-writes")]
    fn take_staged(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let mut storage = self.storage.take()?;
        let taken = storage
            .claim_unless(property, array_index, value, self.writes, |next| {
                !self.makes(next)
            })
            .map(|claimed| {
                claimed.map(|next| {
                    let touched = Touched::of(&next);
                    self.install(next);
                    touched
                })
            });
        // Steps the request has still to take keep the state served now:
        // the part this write changed follows it, rather than both arrays
        // being copied again at every write (#2526).
        match &taken {
            Some(Ok(touched)) => {
                storage.correct_with(|| self.snapshot(), |kept| self.follow(kept, touched));
            }
            _ => storage.correct(|| self.snapshot()),
        }
        self.storage = Some(storage);
        taken.map(|taken| taken.map(|_| ()))
    }

    /// Without durable-writes nothing is staged, so no write is.
    #[cfg(not(feature = "durable-writes"))]
    fn take_staged(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        None
    }

    /// Whether a staged step can be made on the rules served (#2526). Only a
    /// rule step can't: one whose slot lies past the end of its array; every
    /// other step is made whatever the arrays hold. Every change to what is
    /// served is counted, a write or a setter ([`changed`](Self::changed)),
    /// and a step is taken only while none came between, so nothing else can
    /// be stale and this holds no step back. Should one ever be, its write
    /// isn't taken and is refused as made on the rules served, rather than
    /// reported made.
    #[cfg(feature = "durable-writes")]
    fn makes(&self, next: &NextState) -> bool {
        match next {
            NextState::Staged(RulesChange::Rule(positive, slot, _)) => {
                let served = if *positive {
                    &self.positive_access_rules
                } else {
                    &self.negative_access_rules
                };
                *slot < served.len()
            }
            _ => true,
        }
    }

    /// Bring `kept`, what storage holds of the state served before a write
    /// just taken, up to date with that write, `touched`: the one part it
    /// changed, an indexed change made on the kept array itself (#2526).
    #[cfg(feature = "durable-writes")]
    fn follow(&self, kept: &mut AccessRightsSnapshot, touched: &Touched) {
        match touched {
            Touched::Rules(positive, change) => {
                let (rules, served) = if *positive {
                    (&mut kept.positive_access_rules, &self.positive_access_rules)
                } else {
                    (&mut kept.negative_access_rules, &self.negative_access_rules)
                };
                match (rules.as_mut(), change) {
                    (Some(rules), Some(change)) => change.make(rules),
                    // The array's first written state: storage keeps it from
                    // now on.
                    _ => *rules = Some(served.clone()),
                }
            }
            Touched::Enable => kept.enable = Some(self.enable),
            Touched::Accompaniment => kept.accompaniment = self.accompaniment.clone(),
            Touched::Description => kept.description = Some(self.description.clone()),
        }
        debug_assert!(
            *kept == self.snapshot(),
            "the kept state follows the object"
        );
    }

    /// The application changed a property storage keeps the written value
    /// of, while no write has set it: count it, so a staged step worked out
    /// on the old value gives way, and storage goes back to what is served
    /// (#2526).
    pub(super) fn changed(&mut self) {
        #[cfg(feature = "durable-writes")]
        {
            self.with_storage(Storage::drop_staged);
            self.writes = self.writes.wrapping_add(1);
        }
    }

    /// Run `f` on storage; then, if `f` dropped a staged write, queue the
    /// save that puts storage back to the served state. `None` without
    /// persistence.
    fn with_storage<R>(&mut self, f: impl FnOnce(&mut Storage) -> R) -> Option<R> {
        let mut storage = self.storage.take()?;
        let result = f(&mut storage);
        storage.correct(|| self.snapshot());
        self.storage = Some(storage);
        Some(result)
    }
}

/// Put the state `next` leaves into `snapshot`.
fn apply(snapshot: &mut AccessRightsSnapshot, next: &NextState) {
    match next {
        NextState::PositiveAccessRules(rules) => {
            snapshot.positive_access_rules = Some(rules.clone());
        }
        NextState::NegativeAccessRules(rules) => {
            snapshot.negative_access_rules = Some(rules.clone());
        }
        NextState::Enable(enable) => snapshot.enable = Some(*enable),
        NextState::Accompaniment(reference) => snapshot.accompaniment = Some(reference.clone()),
        NextState::Description(text) => snapshot.description = Some(text.clone()),
        #[cfg(feature = "durable-writes")]
        NextState::Staged(_) => unreachable!("{WHOLE}"),
    }
}

/// Put the state `next` leaves into `snapshot`, taking it.
#[cfg(feature = "durable-writes")]
fn put(snapshot: &mut AccessRightsSnapshot, next: NextState) {
    match next {
        NextState::PositiveAccessRules(rules) => snapshot.positive_access_rules = Some(rules),
        NextState::NegativeAccessRules(rules) => snapshot.negative_access_rules = Some(rules),
        NextState::Enable(enable) => snapshot.enable = Some(enable),
        NextState::Accompaniment(reference) => snapshot.accompaniment = Some(reference),
        NextState::Description(text) => snapshot.description = Some(text),
        NextState::Staged(_) => unreachable!("{WHOLE}"),
    }
}

#[cfg(feature = "durable-writes")]
impl DurableWrites for AccessRightsObject {
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep {
        self.stage_writes(&[PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }])
    }

    /// Fold the request's writes of the saved properties into one staged
    /// save (#1423), so a head end that provisions both rule arrays and
    /// Enable in one WritePropertyMultiple saves once, off the guard.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        let saved = |write: &PendingWrite| saved_write(write.property, &write.value);
        if self.storage.is_none() || !writes.iter().any(saved) {
            return StageStep::Skip;
        }
        if let Some(wait) = self.with_storage(Storage::busy).flatten() {
            return StageStep::Busy(wait);
        }
        let served = self.snapshot();
        // One working copy of what the steps so far leave, which the stage
        // saves; a step holds only what its write sets (#2526).
        let mut working = served.clone();
        let steps = staged::steps(writes, |_, write| {
            if !saved(write) {
                return Ok(None);
            }
            let PendingWrite {
                property,
                array_index,
                value,
            } = write;
            let (positive, negative) = (
                working.positive_access_rules.as_deref(),
                working.negative_access_rules.as_deref(),
            );
            let next =
                self.next_state(positive, negative, *property, *array_index, value.clone())?;
            let change = match &next {
                NextState::PositiveAccessRules(rules) => RulesChange::of(true, *array_index, rules),
                NextState::NegativeAccessRules(rules) => {
                    RulesChange::of(false, *array_index, rules)
                }
                _ => None,
            };
            let step = change.map_or_else(|| next.clone(), NextState::Staged);
            put(&mut working, next);
            Ok(Some(step))
        });
        if steps.is_empty() {
            return StageStep::Skip;
        }
        let base = self.writes;
        let storage = self.storage.as_mut().expect("checked above");
        storage.stage_built(steps, base, working, served)
    }

    fn release_staged_write(&mut self, staged: &SaveWait) {
        self.with_storage(|storage| storage.release(staged));
    }

    fn has_staged_write(&self) -> bool {
        self.storage.as_ref().is_some_and(Storage::is_staged)
    }

    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        let mut storage = self.storage.take()?;
        let wait = storage.drop_forgotten(|| self.snapshot());
        self.storage = Some(storage);
        Some(wait)
    }

    fn expire_staged(&mut self, now: Duration) {
        self.with_storage(|storage| storage.expire(now));
    }
}
