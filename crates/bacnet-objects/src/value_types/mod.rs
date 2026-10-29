//! Extended value object types (ASHRAE 135-2020 Clause 12).
//!
//! The 12 "value" object types share a common structure: present_value, status_flags,
//! out_of_service, reliability, and (for commandable types) a 16-level priority array.
//!
//! A `define_value_object!` macro generates the struct + BACnetObject impl for each type.

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use crate::common::{self, read_common_properties};
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};
use crate::traits::BACnetObject;
use cov_increment::CovIncrement;

mod cov_increment;
mod metadata;

// ---------------------------------------------------------------------------
// Macro: define_value_object! (commandable variant)
// ---------------------------------------------------------------------------

/// Generate a commandable value object type with a 16-level priority array.
///
/// Copy types use the existing `recalculate_from_priority_array` helper;
/// non-Copy types (String, Vec, tuples containing Vec) use a Clone-based
/// inline recalculation.
///
/// `rd_validate` performs post-extraction validation for Relinquish_Default.
/// Every generated type exposes its validated local setter and network write;
/// required property metadata supplies property presence and writability.
///
/// The optional `units: EngineeringUnits` entry gives the type a Units
/// property (the numeric value types, whose tables require it): a field that
/// starts at NO_UNITS and reads as Enumerated, a `units` getter and a
/// validated `set_units` setter. Units has no network write route.
///
/// The optional `cov_increment` entry names the COV_Increment datatype of the
/// numeric value types (`u64` for Unsigned, `f64` for Double). It adds a
/// writable COV_Increment row starting at 0, a validated `set_cov_increment`
/// setter, and the `cov_increment` answer the server's COV change detection
/// compares Present_Value moves against.
///
/// Every generated type serves Current_Command_Priority from its priority
/// array: the 1-based slot Present_Value comes from, or NULL when
/// Relinquish_Default is in effect.
macro_rules! define_value_object_commandable {
    (
        name: $struct_name:ident,
        doc: $doc:expr_2021,
        object_type: $obj_type:expr_2021,
        value_type: $val_type:ty,
        default_value: $default:expr_2021,
        pv_to_property: $pv_to_prop:expr_2021,
        property_to_pv: $prop_to_pv:expr_2021,
        pa_wrap: $pa_wrap:expr_2021,
        rd_wrap: $rd_wrap:expr_2021,
        rd_validate: $rd_validate:expr_2021,
        copy_type: $is_copy:tt,
        $(units: $units_ty:ty,)?
        $(cov_increment: $cov_ty:ty,)?
        property_metadata: $property_metadata:expr_2021
        $(,)?
    ) => {
        #[doc = $doc]
        pub struct $struct_name {
            oid: ObjectIdentifier,
            name: String,
            description: String,
            present_value: $val_type,
            out_of_service: bool,
            status_flags: StatusFlags,
            reliability: Reliability,
            /// 16-level priority array. `None` = no command at that level.
            priority_array: [Option<$val_type>; 16],
            relinquish_default: $val_type,
            $(units: $units_ty,)?
            $(
            /// COV_Increment in the table's datatype.
            cov_increment: $cov_ty,
            )?
        }

        impl $struct_name {
            /// Create a new instance of this value object.
            pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
                let oid = ObjectIdentifier::new($obj_type, instance)?;
                Ok(Self {
                    oid,
                    name: name.into(),
                    description: String::new(),
                    present_value: $default,
                    out_of_service: false,
                    status_flags: StatusFlags::empty(),
                    reliability: Reliability::NO_FAULT_DETECTED,
                    priority_array: Default::default(),
                    relinquish_default: $default,
                    $(units: <$units_ty>::NO_UNITS,)?
                    $(cov_increment: <$cov_ty as CovIncrement>::ZERO,)?
                })
            }

            /// Recalculate present_value from the priority array.
            fn recalculate_present_value(&mut self) {
                define_value_object_commandable!(@recalc self, $is_copy);
            }

            /// Set the Relinquish_Default (#270).
            ///
            /// Validated the same way a commanded Present_Value is, then
            /// Present_Value is resolved anew from the priority array so an
            /// empty array falls back to the new default immediately. The
            /// standard permits Relinquish_Default writability on commandable
            /// types; the service decode's multi-element support (#182) makes
            /// the datetime-paired wire form (a date+time pair) deliverable,
            /// so every commandable value type takes network writes now.
            pub fn set_relinquish_default(&mut self, value: $val_type) -> Result<(), Error> {
                ($rd_validate)(&value)?;
                self.relinquish_default = value;
                self.recalculate_present_value();
                Ok(())
            }

            $(
            /// The engineering units of Present_Value, served as Units.
            pub fn units(&self) -> $units_ty {
                self.units
            }

            /// Set the engineering units of Present_Value. A new object uses
            /// NO_UNITS.
            ///
            /// Units is read-only over the network. A value above 65535,
            /// outside BACnetEngineeringUnits, is refused with
            /// VALUE_OUT_OF_RANGE and the property is left unchanged.
            pub fn set_units(&mut self, units: $units_ty) -> Result<(), Error> {
                if units.to_raw() > 65_535 {
                    return Err(common::value_out_of_range_error());
                }
                self.units = units;
                Ok(())
            }
            )?

            $(
            /// Set COV_Increment, the smallest Present_Value change that
            /// sends a COV notification. A new object uses 0, so any change
            /// notifies.
            ///
            /// WriteProperty goes through the same check. A Large Analog
            /// Value refuses a negative or non-finite increment with
            /// VALUE_OUT_OF_RANGE and keeps the old one; every Unsigned value
            /// is a valid increment.
            pub fn set_cov_increment(&mut self, increment: $cov_ty) -> Result<(), Error> {
                self.cov_increment = <$cov_ty as CovIncrement>::validate(increment)?;
                Ok(())
            }
            )?
        }

        impl BACnetObject for $struct_name {
            fn object_identifier(&self) -> ObjectIdentifier {
                self.oid
            }

            fn object_name(&self) -> &str {
                &self.name
            }

            fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
                Cow::Borrowed($property_metadata)
            }

            fn read_property(
                &self,
                property: PropertyIdentifier,
                array_index: Option<u32>,
            ) -> Result<PropertyValue, Error> {
                if let Some(result) = read_common_properties!(self, property, array_index) {
                    return result;
                }
                match property {
                    p if p == PropertyIdentifier::OBJECT_TYPE => {
                        Ok(PropertyValue::Enumerated($obj_type.to_raw()))
                    }
                    p if p == PropertyIdentifier::PRESENT_VALUE => {
                        Ok(($pv_to_prop)(&self.present_value))
                    }
                    p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                        define_value_object_commandable!(@read_pa self, array_index, $pa_wrap, $is_copy)
                    }
                    p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                        Ok(($rd_wrap)(&self.relinquish_default))
                    }
                    p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                        Ok(common::current_command_priority(&self.priority_array))
                    }
                    $(
                    p if p == PropertyIdentifier::UNITS => {
                        Ok(PropertyValue::Enumerated(<$units_ty>::to_raw(self.units)))
                    }
                    )?
                    $(
                    p if p == PropertyIdentifier::COV_INCREMENT => {
                        Ok(<$cov_ty as CovIncrement>::to_property(self.cov_increment))
                    }
                    )?
                    _ => Err(common::unknown_property_error()),
                }
            }

            fn write_property(
                &mut self,
                property: PropertyIdentifier,
                _array_index: Option<u32>,
                value: PropertyValue,
                priority: Option<u8>,
            ) -> Result<(), Error> {
                // Handle PRESENT_VALUE via priority array
                if property == PropertyIdentifier::PRESENT_VALUE {
                    let prio = priority.unwrap_or(16);
                    if !(1..=16).contains(&prio) {
                        return Err(common::value_out_of_range_error());
                    }
                    let idx = (prio - 1) as usize;
                    match value {
                        PropertyValue::Null => {
                            self.priority_array[idx] = None;
                        }
                        other => {
                            let extracted = ($prop_to_pv)(other)?;
                            self.priority_array[idx] = Some(extracted);
                        }
                    }
                    self.recalculate_present_value();
                    return Ok(());
                }
                // Extract through the per-type closure, then validate and recapture.
                if property == PropertyIdentifier::RELINQUISH_DEFAULT {
                    let extracted = ($prop_to_pv)(value)?;
                    return self.set_relinquish_default(extracted);
                }
                $(
                if property == PropertyIdentifier::COV_INCREMENT {
                    let increment = <$cov_ty as CovIncrement>::from_property(value)?;
                    return self.set_cov_increment(increment);
                }
                )?
                if let Some(result) =
                    common::write_out_of_service(&mut self.out_of_service, property, &value)
                {
                    return result;
                }
                if let Some(result) =
                    common::write_object_name(&mut self.name, property, &value)
                {
                    return result;
                }
                if let Some(result) =
                    common::write_description(&mut self.description, property, &value)
                {
                    return result;
                }
                Err(crate::common::unhandled_write_error(self.property_metadata().as_ref(), property, _array_index))
            }

            fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
                crate::property_metadata::property_list_from_metadata($property_metadata)
            }

            fn supports_cov(&self) -> bool {
                true
            }

            $(
            fn cov_increment(&self) -> Option<f64> {
                Some(<$cov_ty as CovIncrement>::as_f64(self.cov_increment))
            }
            )?

            fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
                crate::property_metadata::is_writable_in_metadata($property_metadata, property)
            }
        }
    };

    // --- Internal helper arms ---

    // Recalculate for Copy types: use the common helper.
    (@recalc $self:ident, copy) => {
        $self.present_value = common::recalculate_from_priority_array(
            &$self.priority_array,
            $self.relinquish_default,
        );
    };
    // Recalculate for Clone (non-Copy) types.
    (@recalc $self:ident, clone) => {
        $self.present_value = $self
            .priority_array
            .iter()
            .find_map(|slot| slot.as_ref().cloned())
            .unwrap_or_else(|| $self.relinquish_default.clone());
    };

    // Read priority array for Copy types: use the common macro.
    (@read_pa $self:ident, $array_index:ident, $wrap:expr_2021, copy) => {{
        common::read_priority_array!($self, $array_index, $wrap)
    }};
    // Read priority array for Clone (non-Copy) types.
    (@read_pa $self:ident, $array_index:ident, $wrap:expr_2021, clone) => {{
        let wrap_fn = $wrap;
        match $array_index {
            None => {
                let elements = $self
                    .priority_array
                    .iter()
                    .map(|slot| match slot {
                        Some(v) => wrap_fn(v),
                        None => PropertyValue::Null,
                    })
                    .collect();
                Ok(PropertyValue::List(elements))
            }
            Some(0) => Ok(PropertyValue::Unsigned(16)),
            Some(idx) if (1..=16).contains(&idx) => {
                match &$self.priority_array[(idx - 1) as usize] {
                    Some(v) => Ok(wrap_fn(v)),
                    None => Ok(PropertyValue::Null),
                }
            }
            _ => Err(common::invalid_array_index_error()),
        }
    }};
}

// ---------------------------------------------------------------------------
// Helper functions for value conversions
// ---------------------------------------------------------------------------

fn clone_string_to_pv(v: &str) -> PropertyValue {
    PropertyValue::CharacterString(v.to_owned())
}

fn clone_octetstring_to_pv(v: &[u8]) -> PropertyValue {
    PropertyValue::OctetString(v.to_owned())
}

fn clone_bitstring_to_pv(v: &(u8, Vec<u8>)) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: v.0,
        data: v.1.clone(),
    }
}

/// A set Priority_Array slot of a date-time value type: the date-time choice
/// of BACnetPriorityValue, context tag 1 around the Date and Time (#2098).
/// Present_Value and Relinquish_Default stay a bare BACnetDateTime; only the
/// array element is the CHOICE.
fn datetime_priority_value(v: (Date, Time)) -> PropertyValue {
    let mut buf = bytes::BytesMut::with_capacity(12);
    bacnet_encoding::primitives::encode_priority_date_time(&mut buf, &v.0, &v.1);
    PropertyValue::ApplicationData(buf.to_vec())
}

fn datetime_to_pv(dt: &(Date, Time)) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(dt.0), PropertyValue::Time(dt.1)])
}

fn pv_to_date(v: PropertyValue) -> Result<Date, Error> {
    if let PropertyValue::Date(d) = v {
        Ok(d)
    } else {
        Err(common::invalid_data_type_error())
    }
}

fn pv_to_time(v: PropertyValue) -> Result<Time, Error> {
    if let PropertyValue::Time(t) = v {
        Ok(t)
    } else {
        Err(common::invalid_data_type_error())
    }
}

fn pv_to_datetime(v: PropertyValue) -> Result<(Date, Time), Error> {
    if let PropertyValue::List(items) = v {
        if items.len() == 2 {
            let d = if let PropertyValue::Date(d) = &items[0] {
                *d
            } else {
                return Err(common::invalid_data_type_error());
            };
            let t = if let PropertyValue::Time(t) = &items[1] {
                *t
            } else {
                return Err(common::invalid_data_type_error());
            };
            Ok((d, t))
        } else {
            Err(common::invalid_data_type_error())
        }
    } else {
        Err(common::invalid_data_type_error())
    }
}

// ---------------------------------------------------------------------------
// 9 Commandable value objects
// ---------------------------------------------------------------------------

define_value_object_commandable! {
    name: IntegerValueObject,
    doc: "BACnet Integer Value object (type 45).",
    object_type: ObjectType::INTEGER_VALUE,
    value_type: i32,
    default_value: 0,
    pv_to_property: (|v: &i32| PropertyValue::Signed(*v)),
    property_to_pv: (|v: PropertyValue| -> Result<i32, Error> {
        if let PropertyValue::Signed(n) = v { Ok(n) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: PropertyValue::Signed,
    rd_wrap: (|v: &i32| PropertyValue::Signed(*v)),
    rd_validate: (|_: &i32| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    units: EngineeringUnits,
    cov_increment: u64,
    property_metadata: metadata::INTEGER_VALUE_BASE,
}

define_value_object_commandable! {
    name: PositiveIntegerValueObject,
    doc: "BACnet Positive Integer Value object (type 48).",
    object_type: ObjectType::POSITIVE_INTEGER_VALUE,
    value_type: u64,
    default_value: 0,
    pv_to_property: (|v: &u64| PropertyValue::Unsigned(*v)),
    property_to_pv: (|v: PropertyValue| -> Result<u64, Error> {
        if let PropertyValue::Unsigned(n) = v { Ok(n) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: PropertyValue::Unsigned,
    rd_wrap: (|v: &u64| PropertyValue::Unsigned(*v)),
    rd_validate: (|_: &u64| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    units: EngineeringUnits,
    cov_increment: u64,
    property_metadata: metadata::POSITIVE_INTEGER_VALUE_BASE,
}

define_value_object_commandable! {
    name: LargeAnalogValueObject,
    doc: "BACnet Large Analog Value object (type 46).",
    object_type: ObjectType::LARGE_ANALOG_VALUE,
    value_type: f64,
    default_value: 0.0,
    pv_to_property: (|v: &f64| PropertyValue::Double(*v)),
    property_to_pv: (|v: PropertyValue| -> Result<f64, Error> {
        if let PropertyValue::Double(n) = v { Ok(n) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: PropertyValue::Double,
    rd_wrap: (|v: &f64| PropertyValue::Double(*v)),
    rd_validate: (|v: &f64| -> Result<(), Error> {
        if v.is_finite() {
            Ok(())
        } else {
            Err(common::value_out_of_range_error())
        }
    }),
    copy_type: copy,
    units: EngineeringUnits,
    cov_increment: f64,
    property_metadata: metadata::LARGE_ANALOG_VALUE_BASE,
}

define_value_object_commandable! {
    name: CharacterStringValueObject,
    doc: "BACnet CharacterString Value object (type 40).",
    object_type: ObjectType::CHARACTERSTRING_VALUE,
    value_type: String,
    default_value: String::new(),
    pv_to_property: (|v: &String| PropertyValue::CharacterString(v.clone())),
    property_to_pv: (|v: PropertyValue| -> Result<String, Error> {
        if let PropertyValue::CharacterString(s) = v { Ok(s) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: clone_string_to_pv,
    rd_wrap: (|v: &String| PropertyValue::CharacterString(v.clone())),
    rd_validate: (|_: &String| -> Result<(), Error> { Ok(()) }),
    copy_type: clone,
    property_metadata: metadata::CHARACTERSTRING_VALUE_BASE,
}

define_value_object_commandable! {
    name: OctetStringValueObject,
    doc: "BACnet OctetString Value object (type 47).",
    object_type: ObjectType::OCTETSTRING_VALUE,
    value_type: Vec<u8>,
    default_value: Vec::new(),
    pv_to_property: (|v: &Vec<u8>| PropertyValue::OctetString(v.clone())),
    property_to_pv: (|v: PropertyValue| -> Result<Vec<u8>, Error> {
        if let PropertyValue::OctetString(b) = v { Ok(b) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: clone_octetstring_to_pv,
    rd_wrap: (|v: &Vec<u8>| PropertyValue::OctetString(v.clone())),
    rd_validate: (|_: &Vec<u8>| -> Result<(), Error> { Ok(()) }),
    copy_type: clone,
    property_metadata: metadata::OCTETSTRING_VALUE_BASE,
}

define_value_object_commandable! {
    name: BitStringValueObject,
    doc: "BACnet BitString Value object (type 39).",
    object_type: ObjectType::BITSTRING_VALUE,
    value_type: (u8, Vec<u8>),
    default_value: (0, Vec::new()),
    pv_to_property: (|v: &(u8, Vec<u8>)| PropertyValue::BitString {
        unused_bits: v.0,
        data: v.1.clone(),
    }),
    property_to_pv: (|v: PropertyValue| -> Result<(u8, Vec<u8>), Error> {
        if let PropertyValue::BitString { unused_bits, data } = v { Ok((unused_bits, data)) }
        else { Err(common::invalid_data_type_error()) }
    }),
    pa_wrap: clone_bitstring_to_pv,
    rd_wrap: (|v: &(u8, Vec<u8>)| PropertyValue::BitString {
        unused_bits: v.0,
        data: v.1.clone(),
    }),
    rd_validate: (|_: &(u8, Vec<u8>)| -> Result<(), Error> { Ok(()) }),
    copy_type: clone,
    property_metadata: metadata::BITSTRING_VALUE_BASE,
}

define_value_object_commandable! {
    name: DateValueObject,
    doc: "BACnet Date Value object (type 42).",
    object_type: ObjectType::DATE_VALUE,
    value_type: Date,
    default_value: Date { year: 0xFF, month: 0xFF, day: 0xFF, day_of_week: 0xFF },
    pv_to_property: (|v: &Date| PropertyValue::Date(*v)),
    property_to_pv: pv_to_date,
    pa_wrap: PropertyValue::Date,
    rd_wrap: (|v: &Date| PropertyValue::Date(*v)),
    rd_validate: (|_: &Date| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: metadata::DATE_VALUE_BASE,
}

const TIME_VALUE_PROPERTY_METADATA: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_NAME,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::DESCRIPTION,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRIORITY_ARRAY,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::Commandable),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::Commandable),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::Commandable),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
];

define_value_object_commandable! {
    name: TimeValueObject,
    doc: "BACnet Time Value object (type 50).",
    object_type: ObjectType::TIME_VALUE,
    value_type: Time,
    default_value: Time { hour: 0xFF, minute: 0xFF, second: 0xFF, hundredths: 0xFF },
    pv_to_property: (|v: &Time| PropertyValue::Time(*v)),
    property_to_pv: pv_to_time,
    pa_wrap: PropertyValue::Time,
    rd_wrap: (|v: &Time| PropertyValue::Time(*v)),
    rd_validate: (|_: &Time| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: TIME_VALUE_PROPERTY_METADATA,
}

define_value_object_commandable! {
    name: DateTimeValueObject,
    doc: "BACnet DateTime Value object (type 44).",
    object_type: ObjectType::DATETIME_VALUE,
    value_type: (Date, Time),
    default_value: (
        Date { year: 0xFF, month: 0xFF, day: 0xFF, day_of_week: 0xFF },
        Time { hour: 0xFF, minute: 0xFF, second: 0xFF, hundredths: 0xFF },
    ),
    pv_to_property: (|v: &(Date, Time)| datetime_to_pv(v)),
    property_to_pv: pv_to_datetime,
    pa_wrap: datetime_priority_value,
    rd_wrap: (|v: &(Date, Time)| datetime_to_pv(v)),
    rd_validate: (|_: &(Date, Time)| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: metadata::DATETIME_VALUE_BASE,
}

// ---------------------------------------------------------------------------
// 3 Commandable pattern value objects (with priority array)
// ---------------------------------------------------------------------------

define_value_object_commandable! {
    name: DatePatternValueObject,
    doc: "BACnet Date Pattern Value object (type 41).",
    object_type: ObjectType::DATEPATTERN_VALUE,
    value_type: Date,
    default_value: Date { year: 0xFF, month: 0xFF, day: 0xFF, day_of_week: 0xFF },
    pv_to_property: (|v: &Date| PropertyValue::Date(*v)),
    property_to_pv: pv_to_date,
    pa_wrap: PropertyValue::Date,
    rd_wrap: (|v: &Date| PropertyValue::Date(*v)),
    rd_validate: (|_: &Date| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: metadata::DATEPATTERN_VALUE_BASE,
}

define_value_object_commandable! {
    name: TimePatternValueObject,
    doc: "BACnet Time Pattern Value object (type 49).",
    object_type: ObjectType::TIMEPATTERN_VALUE,
    value_type: Time,
    default_value: Time { hour: 0xFF, minute: 0xFF, second: 0xFF, hundredths: 0xFF },
    pv_to_property: (|v: &Time| PropertyValue::Time(*v)),
    property_to_pv: pv_to_time,
    pa_wrap: PropertyValue::Time,
    rd_wrap: (|v: &Time| PropertyValue::Time(*v)),
    rd_validate: (|_: &Time| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: metadata::TIMEPATTERN_VALUE_BASE,
}

define_value_object_commandable! {
    name: DateTimePatternValueObject,
    doc: "BACnet DateTime Pattern Value object (type 43).",
    object_type: ObjectType::DATETIMEPATTERN_VALUE,
    value_type: (Date, Time),
    default_value: (
        Date { year: 0xFF, month: 0xFF, day: 0xFF, day_of_week: 0xFF },
        Time { hour: 0xFF, minute: 0xFF, second: 0xFF, hundredths: 0xFF },
    ),
    pv_to_property: (|v: &(Date, Time)| datetime_to_pv(v)),
    property_to_pv: pv_to_datetime,
    pa_wrap: datetime_priority_value,
    rd_wrap: (|v: &(Date, Time)| datetime_to_pv(v)),
    rd_validate: (|_: &(Date, Time)| -> Result<(), Error> { Ok(()) }),
    copy_type: copy,
    property_metadata: metadata::DATETIMEPATTERN_VALUE_BASE,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
