//! Extended value object types (ASHRAE 135-2020 Clause 12).
//!
//! The 12 "value" object types share a common structure: present_value, status_flags,
//! out_of_service, reliability, and (for commandable types) a 16-level priority array.
//!
//! A `define_value_object!` macro generates the struct + BACnetObject impl for each type.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// Macro: define_value_object! (commandable variant)
// ---------------------------------------------------------------------------

/// Generate a commandable value object type with a 16-level priority array.
///
/// Copy types use the existing `recalculate_from_priority_array` helper;
/// non-Copy types (String, Vec, tuples containing Vec) use a Clone-based
/// inline recalculation.
macro_rules! define_value_object_commandable {
    (
        name: $struct_name:ident,
        doc: $doc:expr,
        object_type: $obj_type:expr,
        value_type: $val_type:ty,
        default_value: $default:expr,
        pv_to_property: $pv_to_prop:expr,
        property_to_pv: $prop_to_pv:expr,
        pa_wrap: $pa_wrap:expr,
        rd_wrap: $rd_wrap:expr,
        copy_type: $is_copy:tt
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
            reliability: u32,
            /// 16-level priority array. `None` = no command at that level.
            priority_array: [Option<$val_type>; 16],
            relinquish_default: $val_type,
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
                    reliability: 0,
                    priority_array: Default::default(),
                    relinquish_default: $default,
                })
            }

            /// Recalculate present_value from the priority array.
            fn recalculate_present_value(&mut self) {
                define_value_object_commandable!(@recalc self, $is_copy);
            }
        }

        impl BACnetObject for $struct_name {
            fn object_identifier(&self) -> ObjectIdentifier {
                self.oid
            }

            fn object_name(&self) -> &str {
                &self.name
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
                    _ => Err(common::unknown_property_error()),
                }
            }

            fn write_property(
                &mut self,
                property: PropertyIdentifier,
                array_index: Option<u32>,
                value: PropertyValue,
                priority: Option<u8>,
            ) -> Result<(), Error> {
                // Handle PRIORITY_ARRAY direct writes
                if property == PropertyIdentifier::PRIORITY_ARRAY {
                    let idx = match array_index {
                        Some(n) if (1..=16).contains(&n) => (n - 1) as usize,
                        Some(_) => return Err(common::invalid_array_index_error()),
                        None => {
                            return Err(Error::Encoding(
                                "PRIORITY_ARRAY requires array_index (1-16)".into(),
                            ))
                        }
                    };
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
                Err(common::write_access_denied_error())
            }

            fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
                static PROPS: &[PropertyIdentifier] = &[
                    PropertyIdentifier::OBJECT_IDENTIFIER,
                    PropertyIdentifier::OBJECT_NAME,
                    PropertyIdentifier::DESCRIPTION,
                    PropertyIdentifier::OBJECT_TYPE,
                    PropertyIdentifier::PRESENT_VALUE,
                    PropertyIdentifier::STATUS_FLAGS,
                    PropertyIdentifier::OUT_OF_SERVICE,
                    PropertyIdentifier::RELIABILITY,
                    PropertyIdentifier::PRIORITY_ARRAY,
                    PropertyIdentifier::RELINQUISH_DEFAULT,
                ];
                Cow::Borrowed(PROPS)
            }

            fn supports_cov(&self) -> bool {
                true
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
    (@read_pa $self:ident, $array_index:ident, $wrap:expr, copy) => {{
        common::read_priority_array!($self, $array_index, $wrap)
    }};
    // Read priority array for Clone (non-Copy) types.
    (@read_pa $self:ident, $array_index:ident, $wrap:expr, clone) => {{
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
// Macro: define_value_object! (non-commandable variant)
// ---------------------------------------------------------------------------

/// Generate a non-commandable value object type (simple read/write PV).
/// Currently unused — all value types are commandable.
#[allow(unused_macros)]
macro_rules! define_value_object_simple {
    (
        name: $struct_name:ident,
        doc: $doc:expr,
        object_type: $obj_type:expr,
        value_type: $val_type:ty,
        default_value: $default:expr,
        pv_to_property: $pv_to_prop:expr,
        property_to_pv: $prop_to_pv:expr
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
            reliability: u32,
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
                    reliability: 0,
                })
            }
        }

        impl BACnetObject for $struct_name {
            fn object_identifier(&self) -> ObjectIdentifier {
                self.oid
            }

            fn object_name(&self) -> &str {
                &self.name
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
                    _ => Err(common::unknown_property_error()),
                }
            }

            fn write_property(
                &mut self,
                property: PropertyIdentifier,
                _array_index: Option<u32>,
                value: PropertyValue,
                _priority: Option<u8>,
            ) -> Result<(), Error> {
                if property == PropertyIdentifier::PRESENT_VALUE {
                    let extracted = ($prop_to_pv)(value)?;
                    self.present_value = extracted;
                    return Ok(());
                }
                if let Some(result) =
                    common::write_out_of_service(&mut self.out_of_service, property, &value)
                {
                    return result;
                }
                if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
                    return result;
                }
                if let Some(result) =
                    common::write_description(&mut self.description, property, &value)
                {
                    return result;
                }
                Err(common::write_access_denied_error())
            }

            fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
                static PROPS: &[PropertyIdentifier] = &[
                    PropertyIdentifier::OBJECT_IDENTIFIER,
                    PropertyIdentifier::OBJECT_NAME,
                    PropertyIdentifier::DESCRIPTION,
                    PropertyIdentifier::OBJECT_TYPE,
                    PropertyIdentifier::PRESENT_VALUE,
                    PropertyIdentifier::STATUS_FLAGS,
                    PropertyIdentifier::OUT_OF_SERVICE,
                    PropertyIdentifier::RELIABILITY,
                ];
                Cow::Borrowed(PROPS)
            }

            fn supports_cov(&self) -> bool {
                true
            }
        }
    };
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

fn datetime_copy_to_pv(v: (Date, Time)) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(v.0), PropertyValue::Time(v.1)])
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
    copy_type: copy,
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
    copy_type: copy,
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
    copy_type: copy,
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
    copy_type: clone,
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
    copy_type: clone,
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
    copy_type: clone,
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
    copy_type: copy,
}

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
    copy_type: copy,
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
    pa_wrap: datetime_copy_to_pv,
    rd_wrap: (|v: &(Date, Time)| datetime_to_pv(v)),
    copy_type: copy,
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
    copy_type: copy,
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
    copy_type: copy,
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
    pa_wrap: datetime_copy_to_pv,
    rd_wrap: (|v: &(Date, Time)| datetime_to_pv(v)),
    copy_type: copy,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
