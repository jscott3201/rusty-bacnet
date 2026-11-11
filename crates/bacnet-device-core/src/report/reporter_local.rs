//! The [`Reporter`]'s local writes (#2446): the application's own change to
//! an object of its device, made with [`local::apply`] and then reported as
//! [`Reporter::changed`] reports the host's other changes.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::LocalInputValue;
use bacnet_time::Instant;
use bacnet_types::enums::{BinaryPV, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::Reporter;
use crate::write::Applied;
use crate::write::local::{self, LocalChange, LocalCommandSource};

/// A Present_Value the application gives a point through
/// [`Reporter::update`] or [`Reporter::command`], typed by the point's
/// family. A value of another family than the object's is still refused
/// with PROPERTY / INVALID_DATA_TYPE, as a write of it would be.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum LocalValue {
    /// An analog object's REAL.
    Real(f32),
    /// A binary object's state, an Enumerated.
    Binary(BinaryPV),
    /// A multi-state object's state, an Unsigned from 1 to its
    /// Number_Of_States.
    State(u32),
    /// An Integer Value's INTEGER.
    Integer(i32),
}

impl From<f32> for LocalValue {
    fn from(value: f32) -> Self {
        Self::Real(value)
    }
}

impl From<BinaryPV> for LocalValue {
    fn from(value: BinaryPV) -> Self {
        Self::Binary(value)
    }
}

impl From<i32> for LocalValue {
    fn from(value: i32) -> Self {
        Self::Integer(value)
    }
}

/// The application-tagged value a [`LocalValue`] is written as: a REAL, an
/// Enumerated, an Unsigned or a Signed.
impl From<LocalValue> for PropertyValue {
    fn from(value: LocalValue) -> Self {
        match value {
            LocalValue::Real(value) => Self::Real(value),
            LocalValue::Binary(value) => Self::Enumerated(value.to_raw()),
            LocalValue::State(value) => Self::Unsigned(u64::from(value)),
            LocalValue::Integer(value) => Self::Signed(value),
        }
    }
}

impl Reporter {
    /// Make `change` to `oid` in `db`, the application's own change, and
    /// queue what it reports, as [`handle`](Self::handle) does for a
    /// client's write.
    ///
    /// A Life Safety object's COV values are captured first
    /// ([`capture`](Self::capture)), then [`local::apply`] checks and
    /// commits the change, with no observer. One that wrote is reported
    /// through [`changed`](Self::changed). One that changed nothing
    /// ([`Applied::Unchanged`]) or was refused reports nothing, and the
    /// capture is dropped. The refusal is the object's or `apply`'s, as it
    /// is.
    ///
    /// The reporter sends COV notifications only. Event notifications come
    /// with #2303's third part, from the same entry ahead of COV, as
    /// `bacnet-server` orders them, so a change made here will reach them
    /// too.
    pub fn apply_local(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        change: LocalChange,
        now: Instant,
    ) -> Result<Applied, Error> {
        self.capture(db, &[oid]);
        let applied = local::apply(db, oid, change, None);
        if let Ok(Applied::Written) = applied {
            self.changed(db, &[oid], now);
        } else {
            self.snapshots.forget(oid);
        }
        applied
    }

    /// Give `oid` the Present_Value the application measured or works out,
    /// through its [`LocalInput`](bacnet_objects::traits::LocalInput), and
    /// report it: [`LocalChange::Input`]. An Input or a noncommandable
    /// Value takes it while in service; a commandable one refuses it with
    /// OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    pub fn update(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        value: LocalValue,
        now: Instant,
    ) -> Result<Applied, Error> {
        let input = LocalInputValue::present_value(value.into());
        self.apply_local(db, oid, LocalChange::Input(input), now)
    }

    /// Give `oid` the Reliability the application works out, and report
    /// the change, Status_Flags' FAULT with it: [`LocalChange::Reliability`].
    pub fn set_reliability(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        reliability: Reliability,
        now: Instant,
    ) -> Result<Applied, Error> {
        self.apply_local(db, oid, LocalChange::Reliability(reliability), now)
    }

    /// Command `oid`'s Present_Value to `value` at `priority` as the local
    /// Device, and report it: exactly a [`LocalChange::Property`] write of
    /// Present_Value made as [`LocalCommandSource::ServerDevice`]. An
    /// object that isn't commandable takes or refuses it as it would that
    /// write, with no refusal of its own here.
    pub fn command(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        priority: u8,
        value: LocalValue,
        now: Instant,
    ) -> Result<Applied, Error> {
        self.apply_local(db, oid, present_value(value.into(), priority), now)
    }

    /// Relinquish the local Device's command of `oid` at `priority`, and
    /// report what follows: [`command`](Self::command) with a NULL.
    pub fn relinquish(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        priority: u8,
        now: Instant,
    ) -> Result<Applied, Error> {
        let change = present_value(PropertyValue::Null, priority);
        self.apply_local(db, oid, change, now)
    }
}

/// A write of `value` to Present_Value at `priority`, as the local Device.
fn present_value(value: PropertyValue, priority: u8) -> LocalChange {
    LocalChange::Property {
        property: PropertyIdentifier::PRESENT_VALUE,
        array_index: None,
        value,
        priority: Some(priority),
        source: LocalCommandSource::ServerDevice,
    }
}
