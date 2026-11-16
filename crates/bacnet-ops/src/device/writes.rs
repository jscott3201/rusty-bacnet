//! A [`Device`]'s writes (#2560): [`write`](Device::write),
//! [`relinquish`](Device::relinquish), [`priority_array`](Device::priority_array)
//! and [`preview_write`](Device::preview_write).
//!
//! Everything a write is checked for is checked before anything is sent:
//! the text, the priority (one, in range, on a property that takes one),
//! NULL where it would empty a slot, a write to Priority_Array itself, and
//! the client's [`WritePolicy`](crate::WritePolicy). A write goes at the
//! device's gate, like a read. One that found no route was not sent and
//! goes again where a following handle's device went; one that timed out
//! may have been applied, so it is never sent again.

use bacnet_encoding::constructed::check_property_value;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::{Device, DeviceAddress};
use crate::client::Wanted;
use crate::error::OpsError;
use crate::format::{Floats, StateTexts, Value, ValueFormat, decode_all};
use crate::label::{kebab, object_label};
use crate::reading::{IntoObject, IntoProperty, PropertyRef, one_property};
use crate::write::{
    Commandability, Effect, IntoWriteValue, Occupancy, PRIORITY_PROPERTIES, Priority, PriorityView,
    WriteDenied, WritePreview, WriteValue, Written, commands, effect, keeps_priorities,
    one_priority, unknown_property,
};

/// A write checked and encoded, before anything is sent.
struct Order {
    property: PropertyRef,
    /// The value as it goes on the wire.
    value: Value,
    /// The priority it goes with, if any.
    priority: Option<Priority>,
    commandable: Commandability,
}

impl Order {
    /// The write as sent to `device`.
    fn written(self, device: DeviceAddress) -> Written {
        Written {
            device,
            property: self.property,
            value: self.value,
            priority: self.priority,
            commandable: self.commandable,
        }
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// Write one property: one WriteProperty, acknowledged.
    ///
    /// `property` is a [`PropertyRef`], its text (`"av:1 pv"`) or a tuple
    /// ([`IntoProperty`]); `value` is text in the value grammar, read for
    /// the property's datatype (`"72.5"`, `"active"`, `"72.5@8"` with a
    /// priority), or a typed [`PropertyValue`] ([`IntoWriteValue`]).
    ///
    /// The priority is `priority`, or the one the text names, or both when
    /// they are the same. Which one goes, and whether one does, depends on
    /// the property ([`Commandability`]):
    ///
    /// - one the standard makes commandable (an Analog Output's
    ///   Present_Value, say) goes at the priority given, or at 16 when none
    ///   is, which [`Written::priority`] says;
    /// - one that may be commandable (an Analog Value's Present_Value,
    ///   which is when the object has a Priority_Array) goes at the priority
    ///   given, or with none, which the device takes at 16 if it is
    ///   commandable (Clause 19.2.1); nothing is read to find out, and with
    ///   no priority sent the [`WritePolicy`](crate::WritePolicy) has none
    ///   to check;
    /// - one that isn't goes with no priority.
    ///
    /// # Errors
    ///
    /// With nothing sent: [`OpsError::Parse`] for text that doesn't read
    /// (`what` says which part), for two different priorities (the text's
    /// and `priority`), for a priority on a property that isn't commandable,
    /// for NULL to a Present_Value with a Priority_Array (that empties a
    /// slot: use [`relinquish`](Self::relinquish)), and for a write to
    /// Priority_Array, which is read-only; [`OpsError::Denied`] for a
    /// priority the client's [`WritePolicy`](crate::WritePolicy) refuses
    /// (1, 2, 5 and 6 by default). Then the device's refusal as
    /// [`OpsError::Refused`], and [`OpsError::Unreachable`] for no answer,
    /// in which case the write may or may not have been applied.
    pub async fn write(
        &self,
        property: impl IntoProperty,
        value: impl IntoWriteValue,
        priority: Option<Priority>,
    ) -> Result<Written, OpsError> {
        let order = order(property, value, priority)?;
        if let Some(priority) = order.priority {
            let policy = self.client.write_policy();
            policy.check(order.property, priority, false)?;
        }
        let device = self.send_write(&order).await?;
        Ok(order.written(device))
    }

    /// Release `object`'s command at `priority`: NULL written to its
    /// Present_Value in that slot (Clause 19.2.1.3), so the next slot down,
    /// or Relinquish_Default, takes over. Then its priorities as read just
    /// after, in one ReadPropertyMultiple: two requests in all, or four
    /// with ReadProperty each once the device has refused RPM.
    ///
    /// `object` is an [`ObjectIdentifier`] or its text (`"av:1"`).
    ///
    /// # Errors
    ///
    /// With nothing sent: [`OpsError::Parse`] for an object type that keeps
    /// no Priority_Array (an input; a Channel, which passes a relinquish
    /// written to it on to its members: [`write`](Self::write) NULL there);
    /// [`OpsError::Denied`] for a priority the client's
    /// [`WritePolicy`](crate::WritePolicy) refuses. Then the device's
    /// refusal of the write, or no answer to it, as for
    /// [`write`](Self::write). A relinquish the device acknowledged whose
    /// read after it failed is [`OpsError::ReadBack`]: the slot is empty
    /// now, so it needs no second send. One whose read says the object has
    /// no Priority_Array (a value object built without one) is
    /// [`OpsError::NoPriorityArray`]: the device took the NULL and it
    /// changed nothing. Both hold the write ([`OpsError::written`]).
    pub async fn relinquish(
        &self,
        object: impl IntoObject,
        priority: Priority,
    ) -> Result<PriorityView, OpsError> {
        let object = object.into_object()?;
        keeping_priorities(object)?;
        let property = PropertyRef::new(object, PropertyIdentifier::PRESENT_VALUE);
        self.client.write_policy().check(property, priority, true)?;
        let order = Order {
            property,
            value: encoded(property, &PropertyValue::Null)?,
            priority: Some(priority),
            commandable: Commandability::of(object.object_type(), property.property),
        };
        let at = self.send_write(&order).await?;
        let read = self
            .rows_at(&at, object, Wanted::List(&PRIORITY_PROPERTIES))
            .await
            .map_err(|error| OpsError::at(&at, error))
            .and_then(|(rows, _)| PriorityView::from_rows(object, &rows, &at));
        read.map_err(|error| match error {
            // The device took the NULL as a write to a property that isn't
            // commandable, which changes nothing (Clause 19.2.1).
            OpsError::NoPriorityArray { device, object, .. } => OpsError::NoPriorityArray {
                device,
                object,
                relinquished: Some(Box::new(order.written(at))),
            },
            source => OpsError::ReadBack {
                written: Box::new(order.written(at)),
                source: Box::new(source),
            },
        })
    }

    /// `object`'s commands: its Priority_Array, Relinquish_Default and
    /// Present_Value in one ReadPropertyMultiple (ReadProperty each once the
    /// device has refused RPM), as a [`PriorityView`].
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`], with nothing sent, for an object type that
    /// keeps no Priority_Array; [`OpsError::NoPriorityArray`] when the
    /// device says the object has none; its other refusals of
    /// Priority_Array or Present_Value as [`OpsError::Refused`]; and one
    /// that isn't sixteen values as [`OpsError::Client`].
    pub async fn priority_array(&self, object: impl IntoObject) -> Result<PriorityView, OpsError> {
        let object = object.into_object()?;
        keeping_priorities(object)?;
        let (rows, at) = super::sent!(self, |at| self
            .rows_at(at, object, Wanted::List(&PRIORITY_PROPERTIES))
            .await
            .map(|(rows, _)| (rows, at.clone())))?;
        PriorityView::from_rows(object, &rows, &at)
    }

    /// What [`write`](Self::write) of the same arguments would do, read
    /// first and sent nowhere: the slot it takes and who holds it now,
    /// whether a stronger slot keeps Present_Value where it is, and what
    /// Present_Value becomes. Opt-in, and one request: a ReadPropertyMultiple
    /// of the object's priorities for a Present_Value that may be
    /// commandable, or one ReadProperty of the property for any other.
    ///
    /// Where a property's commandability rests on the object
    /// ([`Commandability::Optional`]), the read settles it: a Priority_Array
    /// read makes it commandable, an unknown property makes it not. A
    /// Priority_Array refused any other way, or on an object the standard
    /// makes commandable, leaves the slots unknown ([`Occupancy::Unknown`],
    /// [`Effect::Unknown`]), with the priority the write would carry.
    ///
    /// # Errors
    ///
    /// What [`write`](Self::write) refuses before sending, but for the
    /// [`WritePolicy`](crate::WritePolicy), which
    /// [`WritePreview::denied`] reports instead; a Channel, whose commands
    /// its members keep ([`OpsError::Parse`]); a device that doesn't answer
    /// the read; and a Priority_Array that isn't sixteen values. A device
    /// that refuses to read a property that isn't commandable leaves
    /// [`WritePreview::before`] empty.
    pub async fn preview_write(
        &self,
        property: impl IntoProperty,
        value: impl IntoWriteValue,
        priority: Option<Priority>,
    ) -> Result<WritePreview, OpsError> {
        let order = order(property, value, priority)?;
        let target = order.property;
        let object = target.object;
        let present_value = target.property == PropertyIdentifier::PRESENT_VALUE;
        if present_value && object.object_type() == ObjectType::CHANNEL {
            keeping_priorities(object)?;
        }
        let denied = order.priority.and_then(|priority| {
            let policy = self.client.write_policy();
            policy.check(target, priority, false).err()
        });
        if present_value && target.index.is_none() && keeps_priorities(object.object_type()) {
            let (rows, at) = super::sent!(self, |at| self
                .rows_at(at, object, Wanted::List(&PRIORITY_PROPERTIES))
                .await
                .map(|(rows, _)| (rows, at.clone())))?;
            let priority_array = rows
                .iter()
                .find(|(p, _)| *p == PropertyIdentifier::PRIORITY_ARRAY)
                .map(|(_, value)| value.as_deref());
            match priority_array {
                Some(Ok(priority_array)) => {
                    let slots = commands(priority_array).ok_or_else(|| {
                        OpsError::Client(Error::decoding(0, "the Priority_Array is not 16 values"))
                    })?;
                    let view = PriorityView::from_rows(object, &rows, &at)?;
                    return Ok(commanded(order, view, &slots, at, denied));
                }
                // A value object without a Priority_Array: Present_Value is
                // written as it is.
                Some(Err(error))
                    if order.commandable == Commandability::Optional && unknown_property(error) =>
                {
                    let now = rows
                        .into_iter()
                        .find(|(p, _)| *p == PropertyIdentifier::PRESENT_VALUE)
                        .and_then(|(_, value)| value.ok());
                    let before = now.map(|raw| encoded_raw(target, raw));
                    return Ok(uncommanded(order, before, at, denied));
                }
                // Refused, or left out, otherwise: the write goes as it
                // would, but what the slots hold isn't known.
                _ => return Ok(unknown(order, at, denied)),
            }
        }
        let before = match self.read(target).await {
            Ok(value) => Some(value),
            Err(refused) if refused.remote().is_some() => None,
            Err(error) => return Err(error),
        };
        let at = self.address();
        if order.commandable == Commandability::Optional && !present_value {
            // A vendor's own commandable property: its slots go by other
            // names, so who holds them isn't read.
            return Ok(WritePreview {
                before,
                effect: Effect::NotPresentValue,
                ..unknown(order, at, denied)
            });
        }
        Ok(uncommanded(order, before, at, denied))
    }

    /// Send `order` as one WriteProperty at the device's gate: the address
    /// it went to.
    async fn send_write(&self, order: &Order) -> Result<DeviceAddress, OpsError> {
        // The one check of the WriteProperty encoding `order` hasn't had
        // (a legacy EventParameters value that wouldn't decode back), made
        // before the device is looked up, as the encoding itself refuses it.
        check_property_value(order.property.property, &order.value.raw, 3)
            .map_err(|refused| OpsError::Client(refused.within("WriteProperty").into()))?;
        let address = self.target().await?;
        let result = self.write_at(&address, order).await;
        // No route: nothing was sent. A timeout is never sent again.
        if matches!(result, Err(Error::UnknownDevice { .. }))
            && let Some(to) = self.retry(&address, &result).await?
        {
            let result = self.write_at(&to, order).await;
            return result
                .map(|()| to.clone())
                .map_err(|e| OpsError::at(&to, e));
        }
        result
            .map(|()| address.clone())
            .map_err(|error| OpsError::at(&address, error))
    }

    /// One WriteProperty of `order` to `at`, in the device's turn, encoded
    /// once by the client. Its priority is a checked one and
    /// [`Self::send_write`] has checked its value, so the encoding refuses
    /// nothing here.
    async fn write_at(&self, at: &DeviceAddress, order: &Order) -> Result<(), Error> {
        let (path, _turn) = self.turn_at(at).await?;
        let PropertyRef {
            object,
            property,
            index,
        } = order.property;
        self.client
            .low_level()
            .write_property(
                &path,
                object,
                property,
                index,
                order.value.raw.clone(),
                order.priority.map(Priority::get),
            )
            .await
    }
}

/// `property`, `value` and `priority` checked and encoded as a write;
/// nothing is sent. See [`Device::write`] for what is refused.
fn order(
    property: impl IntoProperty,
    value: impl IntoWriteValue,
    priority: Option<Priority>,
) -> Result<Order, OpsError> {
    let property = one_property(property.into_property()?)?;
    let PropertyRef {
        object,
        property: id,
        index,
    } = property;
    let object_type = object.object_type();
    if id == PropertyIdentifier::PRIORITY_ARRAY {
        let named = index.map_or(kebab(id), |i| format!("{}[{i}]", kebab(id)));
        return Err(OpsError::parse(
            "property",
            &named,
            format!(
                "{named} is read-only (Clause 19.2.1): write present-value at that priority, \
                 or relinquish the slot"
            ),
        ));
    }
    let WriteValue {
        value,
        priority: named,
    } = value.into_write_value(&property)?;
    let asked = one_priority(named, priority)?;
    // The octets as sent, however the value was given: a typed NULL, or
    // raw octets that hold one, empty a slot all the same.
    let value = encoded(property, &value)?;
    if id == PropertyIdentifier::PRESENT_VALUE
        && keeps_priorities(object_type)
        && all_null(&value.raw)
    {
        return Err(OpsError::parse(
            "value",
            "null",
            format!(
                "NULL in a slot of {}'s Priority_Array empties it: relinquish that slot instead",
                object_label(object)
            ),
        ));
    }
    let commandable = Commandability::of(object_type, id);
    let priority = match (commandable, asked) {
        (Commandability::Always, asked) => Some(asked.unwrap_or(Priority::LOWEST)),
        (Commandability::Never, Some(asked)) => {
            return Err(OpsError::parse(
                "priority",
                &asked.to_string(),
                format!(
                    "{} {} isn't commandable, so a priority means nothing to it \
                     (Clause 19.2.1): write it without one",
                    object_label(object),
                    kebab(id)
                ),
            ));
        }
        (_, asked) => asked,
    };
    Ok(Order {
        value,
        property,
        priority,
        commandable,
    })
}

/// Whether `raw` is one or more application NULLs and nothing else.
fn all_null(raw: &[u8]) -> bool {
    matches!(
        decode_all(raw).as_deref(),
        Some(values) if !values.is_empty() && values.iter().all(|v| *v == PropertyValue::Null)
    )
}

/// `value` encoded for a write of `property`, as a [`Value`].
fn encoded(property: PropertyRef, value: &PropertyValue) -> Result<Value, OpsError> {
    let mut raw = BytesMut::new();
    encode_property_value(&mut raw, value).map_err(OpsError::Client)?;
    Ok(encoded_raw(property, raw.to_vec()))
}

/// `raw`, a value of `property`, as the write calls show one: every digit,
/// no units.
fn encoded_raw(property: PropertyRef, raw: Vec<u8>) -> Value {
    let texts = StateTexts::default();
    let format = ValueFormat::new(property.object.object_type(), &texts, Floats::Exact);
    Value::new(
        &format.whole_lists(),
        property.property,
        property.index,
        raw,
        None,
    )
}

/// Refuse `object` unless its type keeps a Priority_Array.
fn keeping_priorities(object: ObjectIdentifier) -> Result<(), OpsError> {
    let object_type = object.object_type();
    if keeps_priorities(object_type) {
        return Ok(());
    }
    let label = object_label(object);
    let reason = if object_type == ObjectType::CHANNEL {
        format!(
            "{label} keeps no Priority_Array: a Channel passes each command on to its members, \
             so read or relinquish theirs, or write NULL to the Channel at the priority"
        )
    } else {
        format!("{label} keeps no Priority_Array: its Present_Value isn't commandable")
    };
    Err(OpsError::parse("object", &label, reason))
}

/// The preview of `order` to a commandable Present_Value whose priorities
/// read as `view`, its slots as [`commands`] reads them, at `device`.
fn commanded(
    order: Order,
    view: PriorityView,
    slots: &[PropertyValue],
    device: DeviceAddress,
    denied: Option<WriteDenied>,
) -> WritePreview {
    let priority = order.priority.unwrap_or(Priority::LOWEST);
    let before = view.slot(priority).cloned();
    let occupancy = match &before {
        Some(_) => Occupancy::Held,
        None => Occupancy::Empty,
    };
    let texts = StateTexts::default();
    let format = ValueFormat::new(view.object.object_type(), &texts, Floats::Exact);
    let default = view.relinquish_default.as_ref().map(|d| d.text.as_str());
    let effect = effect(&format, slots, priority, &order.value.raw, default);
    WritePreview {
        device,
        property: order.property,
        value: order.value,
        priority: Some(priority),
        occupancy,
        before,
        effect,
        priorities: Some(view),
        denied,
    }
}

/// The preview of `order` at `device` to a property that may take a
/// priority, whose slots weren't read: the priority it would go at, and
/// nothing known of its slots or of Present_Value.
fn unknown(order: Order, device: DeviceAddress, denied: Option<WriteDenied>) -> WritePreview {
    WritePreview {
        device,
        property: order.property,
        value: order.value,
        priority: order.priority,
        occupancy: Occupancy::Unknown,
        before: None,
        effect: Effect::Unknown,
        priorities: None,
        denied,
    }
}

/// The preview of `order` to a property that takes no priority, whose
/// value reads as `before`, at `device`.
fn uncommanded(
    order: Order,
    before: Option<Value>,
    device: DeviceAddress,
    denied: Option<WriteDenied>,
) -> WritePreview {
    let whole = order.property.index.is_none();
    let effect = if order.property.property == PropertyIdentifier::PRESENT_VALUE && whole {
        Effect::Changes {
            from: before
                .as_ref()
                .map_or_else(|| "?".into(), |b| b.text.clone()),
            to: order.value.text.clone(),
        }
    } else {
        Effect::NotPresentValue
    };
    WritePreview {
        device,
        property: order.property,
        value: order.value,
        priority: None,
        occupancy: Occupancy::NotCommandable,
        before,
        effect,
        priorities: None,
        denied,
    }
}
