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
// 3 Non-commandable pattern value objects
// ---------------------------------------------------------------------------

define_value_object_simple! {
    name: DatePatternValueObject,
    doc: "BACnet Date Pattern Value object (type 41).",
    object_type: ObjectType::DATEPATTERN_VALUE,
    value_type: Date,
    default_value: Date { year: 0xFF, month: 0xFF, day: 0xFF, day_of_week: 0xFF },
    pv_to_property: (|v: &Date| PropertyValue::Date(*v)),
    property_to_pv: pv_to_date,
}

define_value_object_simple! {
    name: TimePatternValueObject,
    doc: "BACnet Time Pattern Value object (type 49).",
    object_type: ObjectType::TIMEPATTERN_VALUE,
    value_type: Time,
    default_value: Time { hour: 0xFF, minute: 0xFF, second: 0xFF, hundredths: 0xFF },
    pv_to_property: (|v: &Time| PropertyValue::Time(*v)),
    property_to_pv: pv_to_time,
}

define_value_object_simple! {
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    // -----------------------------------------------------------------------
    // IntegerValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn integer_value_construct_and_read_object_type() {
        let obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::INTEGER_VALUE.to_raw())
        );
    }

    #[test]
    fn integer_value_read_write_pv() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        // Default PV is 0
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(0));

        // Write via priority 8
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Signed(-42),
            Some(8),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(-42));
    }

    #[test]
    fn integer_value_priority_array() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        // Write at priority 10
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Signed(100),
            Some(10),
        )
        .unwrap();
        // Write at priority 5 (should win)
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Signed(50),
            Some(5),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(50));

        // Relinquish priority 5 — priority 10 takes over
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(5),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(100));

        // Read priority array size via array_index 0
        let pa_size = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(pa_size, PropertyValue::Unsigned(16));
    }

    #[test]
    fn integer_value_invalid_data_type() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let result = obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString("bad".into()),
            Some(16),
        );
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // PositiveIntegerValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn positive_integer_value_read_write() {
        let mut obj = PositiveIntegerValueObject::new(1, "PIV-1").unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Unsigned(0));

        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(9999),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Unsigned(9999));
    }

    #[test]
    fn positive_integer_value_object_type() {
        let obj = PositiveIntegerValueObject::new(1, "PIV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::POSITIVE_INTEGER_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // LargeAnalogValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn large_analog_value_read_write() {
        let mut obj = LargeAnalogValueObject::new(1, "LAV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Double(1.23456789012345),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Double(1.23456789012345));
    }

    #[test]
    fn large_analog_value_object_type() {
        let obj = LargeAnalogValueObject::new(1, "LAV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::LARGE_ANALOG_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // CharacterStringValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn characterstring_value_read_write() {
        let mut obj = CharacterStringValueObject::new(1, "CSV-1").unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::CharacterString(String::new()));

        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString("hello world".into()),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::CharacterString("hello world".into()));
    }

    #[test]
    fn characterstring_value_priority_array() {
        let mut obj = CharacterStringValueObject::new(1, "CSV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString("low".into()),
            Some(16),
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString("high".into()),
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::CharacterString("high".into()));

        // Relinquish priority 1 — low takes over
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::CharacterString("low".into()));
    }

    // -----------------------------------------------------------------------
    // OctetStringValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn octetstring_value_read_write() {
        let mut obj = OctetStringValueObject::new(1, "OSV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::OctetString(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::OctetString(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn octetstring_value_object_type() {
        let obj = OctetStringValueObject::new(1, "OSV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::OCTETSTRING_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // BitStringValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn bitstring_value_read_write() {
        let mut obj = BitStringValueObject::new(1, "BSV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::BitString {
                unused_bits: 3,
                data: vec![0b11010000],
            },
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            pv,
            PropertyValue::BitString {
                unused_bits: 3,
                data: vec![0b11010000],
            }
        );
    }

    #[test]
    fn bitstring_value_object_type() {
        let obj = BitStringValueObject::new(1, "BSV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::BITSTRING_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // DateValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn date_value_read_write() {
        let mut obj = DateValueObject::new(1, "DV-1").unwrap();
        let d = Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Date(d),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Date(d));
    }

    #[test]
    fn date_value_priority_array() {
        let mut obj = DateValueObject::new(1, "DV-1").unwrap();
        let d1 = Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        };
        let d2 = Date {
            year: 124,
            month: 12,
            day: 25,
            day_of_week: 3,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Date(d1),
            Some(16),
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Date(d2),
            Some(8),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Date(d2));
    }

    // -----------------------------------------------------------------------
    // TimeValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn time_value_read_write() {
        let mut obj = TimeValueObject::new(1, "TV-1").unwrap();
        let t = Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Time(t),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Time(t));
    }

    #[test]
    fn time_value_object_type() {
        let obj = TimeValueObject::new(1, "TV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::TIME_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // DateTimeValueObject
    // -----------------------------------------------------------------------

    #[test]
    fn datetime_value_read_write() {
        let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
        let d = Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 6,
        };
        let t = Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            pv,
            PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
        );
    }

    #[test]
    fn datetime_value_object_type() {
        let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::DATETIME_VALUE.to_raw())
        );
    }

    #[test]
    fn datetime_value_priority_array() {
        let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
        let d1 = Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        };
        let t1 = Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        let d2 = Date {
            year: 124,
            month: 12,
            day: 31,
            day_of_week: 2,
        };
        let t2 = Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::List(vec![PropertyValue::Date(d1), PropertyValue::Time(t1)]),
            Some(16),
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)]),
            Some(4),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            pv,
            PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)])
        );
    }

    // -----------------------------------------------------------------------
    // DatePatternValueObject (non-commandable)
    // -----------------------------------------------------------------------

    #[test]
    fn date_pattern_value_read_write() {
        let mut obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
        let d = Date {
            year: 0xFF,
            month: 0xFF,
            day: 25,
            day_of_week: 0xFF,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Date(d),
            None,
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Date(d));
    }

    #[test]
    fn date_pattern_value_object_type() {
        let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::DATEPATTERN_VALUE.to_raw())
        );
    }

    #[test]
    fn date_pattern_value_no_priority_array() {
        let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
        let props = obj.property_list();
        assert!(!props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(!props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    }

    // -----------------------------------------------------------------------
    // TimePatternValueObject (non-commandable)
    // -----------------------------------------------------------------------

    #[test]
    fn time_pattern_value_read_write() {
        let mut obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
        let t = Time {
            hour: 12,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Time(t),
            None,
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Time(t));
    }

    #[test]
    fn time_pattern_value_object_type() {
        let obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::TIMEPATTERN_VALUE.to_raw())
        );
    }

    // -----------------------------------------------------------------------
    // DateTimePatternValueObject (non-commandable)
    // -----------------------------------------------------------------------

    #[test]
    fn datetime_pattern_value_read_write() {
        let mut obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
        let d = Date {
            year: 0xFF,
            month: 12,
            day: 25,
            day_of_week: 0xFF,
        };
        let t = Time {
            hour: 0xFF,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        };
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
            None,
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(
            pv,
            PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
        );
    }

    #[test]
    fn datetime_pattern_value_object_type() {
        let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::DATETIMEPATTERN_VALUE.to_raw())
        );
    }

    #[test]
    fn datetime_pattern_value_no_priority_array() {
        let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
        let props = obj.property_list();
        assert!(!props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(!props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    }

    // -----------------------------------------------------------------------
    // Common property tests (using IntegerValue as representative)
    // -----------------------------------------------------------------------

    #[test]
    fn value_object_read_common_properties() {
        let obj = IntegerValueObject::new(42, "TestObj").unwrap();

        // OBJECT_NAME
        let name = obj
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        assert_eq!(name, PropertyValue::CharacterString("TestObj".into()));

        // OBJECT_IDENTIFIER
        let oid = obj
            .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .unwrap();
        assert!(matches!(oid, PropertyValue::ObjectIdentifier(_)));

        // STATUS_FLAGS
        let sf = obj
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap();
        assert!(matches!(sf, PropertyValue::BitString { .. }));

        // OUT_OF_SERVICE
        let oos = obj
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(oos, PropertyValue::Boolean(false));

        // RELIABILITY
        let rel = obj
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(rel, PropertyValue::Enumerated(0));
    }

    #[test]
    fn value_object_write_description() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("A test integer".into()),
            None,
        )
        .unwrap();
        let desc = obj
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(
            desc,
            PropertyValue::CharacterString("A test integer".into())
        );
    }

    #[test]
    fn value_object_write_out_of_service() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let oos = obj
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(oos, PropertyValue::Boolean(true));
    }

    #[test]
    fn value_object_relinquish_default() {
        let obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let rd = obj
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap();
        assert_eq!(rd, PropertyValue::Signed(0));
    }

    #[test]
    fn value_object_priority_array_direct_write() {
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();

        // Write directly to priority array slot 5
        obj.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Signed(77),
            None,
        )
        .unwrap();

        // Read back slot 5
        let slot = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap();
        assert_eq!(slot, PropertyValue::Signed(77));

        // PV should reflect it
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(77));

        // Relinquish slot 5
        obj.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Null,
            None,
        )
        .unwrap();

        // PV falls back to relinquish default
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Signed(0));
    }

    #[test]
    fn value_object_unknown_property() {
        let obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let result = obj.read_property(PropertyIdentifier::UNITS, None);
        assert!(result.is_err());
    }

    #[test]
    fn value_object_write_object_name() {
        // Clause 12.1.1.2: Object_Name shall be writable
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let result = obj.write_property(
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("new-name".into()),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(obj.object_name(), "new-name");
    }

    #[test]
    fn value_object_write_access_denied() {
        // OBJECT_TYPE is never writable
        let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
        let result = obj.write_property(
            PropertyIdentifier::OBJECT_TYPE,
            None,
            PropertyValue::Enumerated(0),
            None,
        );
        assert!(result.is_err());
    }
}
