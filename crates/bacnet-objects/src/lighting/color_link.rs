//! The colour links of the two lighting output types (#1527,
//! Addendum 135-2020ca part 4): Color_Reference, Color_Override and
//! Override_Color_Reference.
//!
//! A lighting output sets its own luminance, but its colour comes from a
//! companion colour object (Color or Color Temperature) in the same device:
//! the one Color_Reference names or, while Color_Override is TRUE, the one
//! Override_Color_Reference names (Clauses 12.54.X to 12.54.Z and 12.55.X to
//! 12.55.Z). Turning the override on or off only changes which object the
//! colour comes from. Neither the output's own fade nor the referenced
//! object's is touched, so once the override ends the colour is wherever the
//! Color_Reference object has got to by then. Nothing here writes a colour
//! object: the outputs store and serve the references, and
//! [`ObjectDatabase::lighting_color`] follows them to the colour shown.
//!
//! A reference names a colour object of either type. Instance 4194303 names
//! none, whatever the type, which leaves the colour to the application: the
//! clauses give the instance alone that meaning, so any type is taken with it
//! for interoperability. A reference to an
//! object the database doesn't hold is stored and served but not followed:
//! the clauses put the companion in the same device, and an object
//! identifier can't name another one.

use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::common;
use crate::database::ObjectDatabase;
use crate::property_metadata::{
    PropertyConformance::Optional, PropertyMetadata, PropertyPresenceCondition,
    PropertyWriteCapability::Always,
};

/// A lighting output's link to the colour objects that set its colour
/// (Addendum 135-2020ca part 4, #1527).
///
/// Each reference must name a colour object, Color or Color Temperature, or
/// have instance 4194303, which names none whatever its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorLink {
    /// Color_Reference: the object that sets the colour.
    pub reference: ObjectIdentifier,
    /// Color_Override and Override_Color_Reference, present when the output
    /// supports colour override.
    pub color_override: Option<ColorOverride>,
}

/// A lighting output's colour override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorOverride {
    /// Color_Override: whether Override_Color_Reference sets the colour now.
    pub active: bool,
    /// Override_Color_Reference: the object that sets the colour while
    /// overridden.
    pub reference: ObjectIdentifier,
}

impl ColorLink {
    /// A link to `reference` with no colour override.
    pub const fn new(reference: ObjectIdentifier) -> Self {
        Self {
            reference,
            color_override: None,
        }
    }

    /// The reference that sets the colour now: Override_Color_Reference
    /// while overridden, otherwise Color_Reference. `None` when it names no
    /// object (instance 4194303).
    pub fn active_reference(&self) -> Option<ObjectIdentifier> {
        let reference = match self.color_override {
            Some(ColorOverride {
                active: true,
                reference,
            }) => reference,
            _ => self.reference,
        };
        (reference.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE).then_some(reference)
    }

    /// Check both references: VALUE_OUT_OF_RANGE unless each names a colour
    /// object of either type or has instance 4194303.
    pub(super) fn checked(self) -> Result<Self, Error> {
        checked_reference(self.reference)?;
        if let Some(color_override) = self.color_override {
            checked_reference(color_override.reference)?;
        }
        Ok(self)
    }
}

fn checked_reference(reference: ObjectIdentifier) -> Result<ObjectIdentifier, Error> {
    if reference.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        || matches!(
            reference.object_type(),
            ObjectType::COLOR | ObjectType::COLOR_TEMPERATURE
        )
    {
        Ok(reference)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// The metadata rows `link` brings, in the order Tables 12-64 and 12-69 give
/// them. Each is required while present (the tables' footnotes), and each
/// takes writes: Color_Override must, and the references take one that
/// names a colour object.
pub(super) fn metadata(link: Option<&ColorLink>) -> impl Iterator<Item = PropertyMetadata> {
    let overridable = link.is_some_and(|link| link.color_override.is_some());
    [
        (P::COLOR_REFERENCE, link.is_some()),
        (P::COLOR_OVERRIDE, overridable),
        (P::OVERRIDE_COLOR_REFERENCE, overridable),
    ]
    .into_iter()
    .filter(|&(_, present)| present)
    .map(|(property, _)| {
        PropertyMetadata::new(
            property,
            Optional,
            Some(PropertyPresenceCondition::LightingColor),
            Always,
        )
    })
}

/// Read a colour-link property, if `property` is one; absent ones are
/// UNKNOWN_PROPERTY.
pub(super) fn read(link: Option<&ColorLink>, property: P) -> Option<Result<PropertyValue, Error>> {
    let color_override = link.and_then(|link| link.color_override);
    let value = match property {
        P::COLOR_REFERENCE => link.map(|link| PropertyValue::ObjectIdentifier(link.reference)),
        P::COLOR_OVERRIDE => color_override.map(|o| PropertyValue::Boolean(o.active)),
        P::OVERRIDE_COLOR_REFERENCE => {
            color_override.map(|o| PropertyValue::ObjectIdentifier(o.reference))
        }
        _ => return None,
    };
    Some(value.ok_or_else(common::unknown_property_error))
}

/// Write a colour-link property, if `property` is one and present.
pub(super) fn write(
    link: &mut Option<ColorLink>,
    property: P,
    value: &PropertyValue,
) -> Option<Result<(), Error>> {
    if let Err(error) = read(link.as_ref(), property)? {
        return Some(Err(error));
    }
    let link = link.as_mut()?;
    Some(match (property, value) {
        (P::COLOR_REFERENCE, PropertyValue::ObjectIdentifier(reference)) => {
            checked_reference(*reference).map(|reference| link.reference = reference)
        }
        (P::COLOR_OVERRIDE, PropertyValue::Boolean(active)) => {
            if let Some(color_override) = &mut link.color_override {
                color_override.active = *active;
            }
            Ok(())
        }
        (P::OVERRIDE_COLOR_REFERENCE, PropertyValue::ObjectIdentifier(reference)) => {
            checked_reference(*reference).map(|reference| {
                if let Some(color_override) = &mut link.color_override {
                    color_override.reference = reference;
                }
            })
        }
        _ => Err(common::invalid_data_type_error()),
    })
}

/// The colour link `output` serves through its properties, if it has one.
fn read_link(output: &dyn crate::traits::BACnetObject) -> Option<ColorLink> {
    let read = |property| output.read_property(property, None).ok();
    let Some(PropertyValue::ObjectIdentifier(reference)) = read(P::COLOR_REFERENCE) else {
        return None;
    };
    let color_override = match (read(P::COLOR_OVERRIDE), read(P::OVERRIDE_COLOR_REFERENCE)) {
        (
            Some(PropertyValue::Boolean(active)),
            Some(PropertyValue::ObjectIdentifier(reference)),
        ) => Some(ColorOverride { active, reference }),
        _ => None,
    };
    Some(ColorLink {
        reference,
        color_override,
    })
}

/// The colour a lighting output shows, from the colour object it follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingColor {
    /// The colour object the colour comes from.
    pub source: ObjectIdentifier,
    /// That object's Tracking_Value.
    pub color: OutputColor,
}

/// A colour as a colour object's Tracking_Value gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputColor {
    /// A Color object's CIE 1931 xy colour.
    Xy(BACnetXyColor),
    /// A Color Temperature object's correlated colour temperature, in
    /// kelvin.
    Kelvin(u32),
}

impl ObjectDatabase {
    /// The colour the lighting output `lighting_output` shows now (#1527):
    /// the Tracking_Value of the colour object its Color_Reference names, or
    /// its Override_Color_Reference while Color_Override is TRUE, so a
    /// colour fade in progress shows as it runs.
    ///
    /// `None` when the object has no Color_Reference, the reference in use
    /// names no object (instance 4194303), or this database holds no Color
    /// or Color Temperature object by that identifier. It reads the
    /// properties, so it follows any object that serves them.
    pub fn lighting_color(&self, lighting_output: &ObjectIdentifier) -> Option<LightingColor> {
        let source = read_link(self.get(lighting_output)?)?.active_reference()?;
        let tracking = self
            .get(&source)?
            .read_property(P::TRACKING_VALUE, None)
            .ok()?;
        let color = match (source.object_type(), tracking) {
            (ObjectType::COLOR, PropertyValue::List(xy)) => match xy.as_slice() {
                [PropertyValue::Real(x), PropertyValue::Real(y)] => {
                    OutputColor::Xy(BACnetXyColor::new(*x, *y))
                }
                _ => return None,
            },
            (ObjectType::COLOR_TEMPERATURE, PropertyValue::Unsigned(kelvin)) => {
                OutputColor::Kelvin(u32::try_from(kelvin).ok()?)
            }
            _ => return None,
        };
        Some(LightingColor { source, color })
    }
}
