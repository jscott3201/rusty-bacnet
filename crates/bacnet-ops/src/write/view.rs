//! What the write calls return: [`Written`], [`PriorityView`] and
//! [`WritePreview`].

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{DecodingKind, Error};
use bacnet_types::primitives::ObjectIdentifier;

use super::command::elements;
use super::{Commandability, Effect, Occupancy, Priority, WriteDenied};
use crate::device::DeviceAddress;
use crate::error::OpsError;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat};
use crate::reading::PropertyRef;

/// What a priority view reads of an object, in one ReadPropertyMultiple.
pub(crate) const PRIORITY_PROPERTIES: [PropertyIdentifier; 3] = [
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::PRESENT_VALUE,
];

/// A write the device acknowledged ([`Device::write`](crate::Device::write)).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Written {
    /// Where it went.
    pub device: DeviceAddress,
    /// The property written.
    pub property: PropertyRef,
    /// The value as sent: its octets (`raw`), typed data and text.
    pub value: Value,
    /// The priority sent: the one asked for, or 16 for a property the
    /// standard makes commandable ([`Commandability::Always`]) when none
    /// was. `None` when none was sent: a property that isn't commandable,
    /// or one that may be ([`Commandability::Optional`]) written without
    /// one, which the device then takes at 16 if it is (Clause 19.2.1).
    pub priority: Option<Priority>,
    /// Whether the property takes priorities, as its object type and
    /// identifier say.
    pub commandable: Commandability,
}

/// An object's commands as read: its Priority_Array's sixteen slots, the
/// slot in control, Relinquish_Default and Present_Value
/// ([`Device::priority_array`](crate::Device::priority_array)). Values
/// name no units, as [`Device::read`](crate::Device::read) names none.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PriorityView {
    /// The object read.
    pub object: ObjectIdentifier,
    /// Each slot's command, `slots[0]` for priority 1
    /// ([`Priority::index`]); `None` for an empty slot (NULL). Boxed: a
    /// view is sixteen values.
    pub slots: Box<[Option<Value>; Priority::COUNT]>,
    /// The slot in control: the lowest-numbered one holding a command
    /// (Clause 19.2.1). `None` when every slot is empty and Present_Value
    /// follows Relinquish_Default.
    pub in_control: Option<Priority>,
    /// Relinquish_Default; `None` when the device gave none, which a
    /// commandable object should (Clause 19.2).
    pub relinquish_default: Option<Value>,
    /// Present_Value.
    pub present_value: Value,
}

impl PriorityView {
    /// The command at `priority`, `None` when its slot is empty.
    pub fn slot(&self, priority: Priority) -> Option<&Value> {
        self.slots[priority.index()].as_ref()
    }

    /// The view from the rows of a read of [`PRIORITY_PROPERTIES`] of
    /// `object` at `at`.
    ///
    /// # Errors
    ///
    /// [`OpsError::NoPriorityArray`] when the device says the object has
    /// no Priority_Array; its other refusals of Priority_Array or
    /// Present_Value as [`OpsError::Refused`]; and a Priority_Array that
    /// isn't sixteen values, an answer that left one out, or one whose
    /// value didn't decode, as [`OpsError::Client`]. A request that failed
    /// never reaches here as a row: the read fails with its own error
    /// first (#2610).
    pub(crate) fn from_rows(
        object: ObjectIdentifier,
        rows: &[RawRow],
        at: &DeviceAddress,
    ) -> Result<Self, OpsError> {
        let texts = StateTexts::default();
        let format = ValueFormat::new(object.object_type(), &texts, Floats::Exact).whole_lists();
        let none = rows.iter().any(|(property, value)| {
            *property == PropertyIdentifier::PRIORITY_ARRAY
                && value.as_ref().is_err_and(unknown_property)
        });
        if none {
            return Err(OpsError::NoPriorityArray {
                device: at.clone(),
                object,
                relinquished: None,
            });
        }
        let priority_array = row(rows, PropertyIdentifier::PRIORITY_ARRAY, at)?;
        let present_value = row(rows, PropertyIdentifier::PRESENT_VALUE, at)?;
        let relinquish_default = row(rows, PropertyIdentifier::RELINQUISH_DEFAULT, at).ok();
        let octets = elements(priority_array)
            .filter(|slots| slots.len() == Priority::COUNT)
            .ok_or_else(|| {
                OpsError::Client(Error::decoding(0, "the Priority_Array is not 16 values"))
            })?;
        let slots: [Option<Value>; Priority::COUNT] = std::array::from_fn(|i| {
            let slot = octets[i];
            // NULL, application tag 0 with no length, is an empty slot.
            (slot != [0x00]).then(|| {
                let index = Priority::at_index(i).map(|p| u32::from(p.get()));
                let property = PropertyIdentifier::PRIORITY_ARRAY;
                Value::new(&format, property, index, slot.to_vec(), None)
            })
        });
        let whole =
            |property, data: &[u8]| Value::new(&format, property, None, data.to_vec(), None);
        Ok(Self {
            object,
            in_control: slots
                .iter()
                .position(Option::is_some)
                .and_then(Priority::at_index),
            slots: Box::new(slots),
            relinquish_default: relinquish_default
                .map(|data| whole(PropertyIdentifier::RELINQUISH_DEFAULT, data)),
            present_value: whole(PropertyIdentifier::PRESENT_VALUE, present_value),
        })
    }
}

/// Whether `error` is the device saying the property doesn't exist.
pub(crate) fn unknown_property(error: &PropertyError) -> bool {
    error.class_code() == Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
}

/// The value of `property` among `rows`, or why there is none.
fn row<'r>(
    rows: &'r [RawRow],
    property: PropertyIdentifier,
    at: &DeviceAddress,
) -> Result<&'r [u8], OpsError> {
    let missing = || {
        Error::decoding_kind(
            DecodingKind::Missing,
            0,
            "the answer left out a property it was asked for",
        )
    };
    match rows.iter().find(|(p, _)| *p == property) {
        Some((_, Ok(data))) => Ok(data),
        Some((_, Err(PropertyError::Remote(error)))) => Err(OpsError::Refused {
            device: at.clone(),
            error: *error,
            detail: None,
        }),
        // The answer arrived and couldn't be read: a request that was
        // never sent, or never answered, fails the read before any row.
        Some((_, Err(PropertyError::Other(_)))) => Err(OpsError::Client(Error::decoding(
            0,
            "a property of the priority read did not decode",
        ))),
        Some((_, Err(_))) | None => Err(OpsError::Client(missing())),
    }
}

/// What a write would do, read before it is sent
/// ([`Device::preview_write`](crate::Device::preview_write)).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WritePreview {
    /// Where the read went, and the write would go.
    pub device: DeviceAddress,
    /// The property it would write.
    pub property: PropertyRef,
    /// The value as it would be sent.
    pub value: Value,
    /// The slot it would take: the priority asked for, or 16, on a
    /// property the read found commandable; `None` on one it found isn't.
    pub priority: Option<Priority>,
    /// Who holds that slot now.
    pub occupancy: Occupancy,
    /// What the write replaces: the slot's command when one holds it, or
    /// the property's value as read for one that isn't commandable (`None`
    /// when the device refused that read).
    pub before: Option<Value>,
    /// What it does to Present_Value.
    pub effect: Effect,
    /// The object's commands as read, for a commandable property.
    pub priorities: Option<PriorityView>,
    /// The client's [`WritePolicy`](super::WritePolicy) would refuse the
    /// write: [`Device::write`](crate::Device::write) would fail with this
    /// and send nothing.
    pub denied: Option<WriteDenied>,
}
