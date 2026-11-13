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
/// [`Reporter::update`] or [`Reporter::command`], typed by the datatype it
/// is written as: a REAL, an Enumerated, an Unsigned or a Signed. The
/// object takes or refuses it by that datatype, as it would a write of it,
/// not by the family a variant is named for: a Life Safety Point takes a
/// `Binary` as the Enumerated it is, and a Positive Integer Value a
/// `State`. A datatype the object's Present_Value doesn't have is refused
/// with PROPERTY / INVALID_DATA_TYPE.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum LocalValue {
    /// A REAL, as an analog object takes.
    Real(f32),
    /// An Enumerated, as a binary object takes for its state.
    Binary(BinaryPV),
    /// An Unsigned, as a multi-state object takes for its state, from 1 to
    /// its Number_Of_States.
    State(u32),
    /// A Signed, as an Integer Value takes.
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
    /// is. `changed` takes every capture pending, not only `oid`'s, and a
    /// change that wrote nothing forgets `oid`'s capture even when the host
    /// made it before this call.
    ///
    /// The reporter sends COV notifications only, and runs no event pass
    /// yet. With `bacnet-objects`' `event-reporting` on, a fault set here
    /// leaves an intrinsic reporter's Event_State as it was, so Status_Flags
    /// and the COV notification carry FAULT without the IN_ALARM the
    /// server's event pass would add. Event notifications come with #2303's
    /// third part, from the same entry ahead of COV, as `bacnet-server`
    /// orders them, so a change made here will reach them too.
    pub fn apply_local(
        &mut self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        change: LocalChange,
        now: Instant,
    ) -> Result<Applied, Error> {
        self.capture(db, &[oid]);
        // Without a clock, no Recipient_List entry the device can't honour,
        // as the `Responder`'s writes refuse (decision 14).
        #[cfg(feature = "event-reporting")]
        let mut clockless = crate::write::ClocklessRecipients(None);
        #[cfg(feature = "event-reporting")]
        let observed = self.responder.clock().is_none().then_some(local::Observed {
            observer: &mut clockless,
            octets: &[],
        });
        #[cfg(not(feature = "event-reporting"))]
        let observed = None;
        let applied = local::apply(db, oid, change, observed);
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
    ///
    /// Until the reporter runs the event pass (#2303), a fault leaves an
    /// intrinsic reporter's Event_State as it was, so the notification
    /// carries no IN_ALARM where `bacnet-server`'s would; see
    /// [`apply_local`](Self::apply_local).
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

    /// Relinquish `oid`'s priority slot `priority` as the local Device,
    /// whatever holds it, and report what follows:
    /// [`command`](Self::command) with a NULL.
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
