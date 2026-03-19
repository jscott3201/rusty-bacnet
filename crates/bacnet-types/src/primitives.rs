//! BACnet primitive data types per ASHRAE 135-2020 Clause 20.2.
//!
//! Core types used throughout the protocol: [`ObjectIdentifier`], [`Date`],
//! [`Time`], and the [`PropertyValue`] sum type.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::enums::ObjectType;
use crate::error::Error;

// ---------------------------------------------------------------------------
// ObjectIdentifier (Clause 20.2.14)
// ---------------------------------------------------------------------------

/// BACnet Object Identifier: 10-bit type + 22-bit instance number.
///
/// Uniquely identifies a BACnet object within a device. Encoded as a
/// 4-byte big-endian value: `(object_type << 22) | instance_number`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectIdentifier {
    object_type: ObjectType,
    instance_number: u32,
}

impl ObjectIdentifier {
    /// Maximum valid instance number (2^22 - 1 = 4,194,303).
    pub const MAX_INSTANCE: u32 = 0x3F_FFFF;

    /// The "wildcard" instance number used in WhoIs/IAm (4,194,303).
    pub const WILDCARD_INSTANCE: u32 = Self::MAX_INSTANCE;

    /// Create a new ObjectIdentifier.
    ///
    /// # Errors
    /// Returns `Err` if `instance_number` exceeds [`MAX_INSTANCE`](Self::MAX_INSTANCE).
    pub fn new(object_type: ObjectType, instance_number: u32) -> Result<Self, Error> {
        if instance_number > Self::MAX_INSTANCE {
            return Err(Error::OutOfRange(alloc_or_std_format!(
                "instance number {} exceeds max {}",
                instance_number,
                Self::MAX_INSTANCE
            )));
        }
        Ok(Self {
            object_type,
            instance_number,
        })
    }

    /// Create without validation. Caller must ensure instance <= MAX_INSTANCE.
    pub const fn new_unchecked(object_type: ObjectType, instance_number: u32) -> Self {
        Self {
            object_type,
            instance_number,
        }
    }

    /// The object type.
    pub const fn object_type(&self) -> ObjectType {
        self.object_type
    }

    /// The instance number (0 to 4,194,303).
    pub const fn instance_number(&self) -> u32 {
        self.instance_number
    }

    /// Encode to the 4-byte BACnet wire format (big-endian).
    pub fn encode(&self) -> [u8; 4] {
        debug_assert!(
            self.object_type.to_raw() <= 0x3FF,
            "ObjectType {} exceeds 10-bit field",
            self.object_type.to_raw()
        );
        debug_assert!(
            self.instance_number <= Self::MAX_INSTANCE,
            "Instance {} exceeds MAX_INSTANCE",
            self.instance_number
        );
        let value = ((self.object_type.to_raw() & 0x3FF) << 22)
            | (self.instance_number & Self::MAX_INSTANCE);
        value.to_be_bytes()
    }

    /// Decode from the 4-byte BACnet wire format (big-endian).
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::buffer_too_short(4, data.len()));
        }
        let value = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let type_raw = (value >> 22) & 0x3FF;
        let instance = value & Self::MAX_INSTANCE;
        Ok(Self {
            object_type: ObjectType::from_raw(type_raw),
            instance_number: instance,
        })
    }
}

impl core::fmt::Debug for ObjectIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ObjectIdentifier({:?}, {})",
            self.object_type, self.instance_number
        )
    }
}

impl core::fmt::Display for ObjectIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{}", self.object_type, self.instance_number)
    }
}

// ---------------------------------------------------------------------------
// Date (Clause 20.2.12)
// ---------------------------------------------------------------------------

/// BACnet Date: year, month, day, day-of-week.
///
/// - Year: 0-254 relative to 1900 (0xFF = unspecified)
/// - Month: 1-14 (13=odd, 14=even, 0xFF=unspecified)
/// - Day: 1-34 (32=last, 33=odd, 34=even, 0xFF=unspecified)
/// - Day of week: 1=Monday..7=Sunday (0xFF=unspecified)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Date {
    /// Year minus 1900 (0-254, or 0xFF for unspecified).
    pub year: u8,
    /// Month (1-14, or 0xFF for unspecified).
    pub month: u8,
    /// Day of month (1-34, or 0xFF for unspecified).
    pub day: u8,
    /// Day of week (1=Monday..7=Sunday, or 0xFF for unspecified).
    pub day_of_week: u8,
}

impl Date {
    /// Value indicating "unspecified" for any date field.
    pub const UNSPECIFIED: u8 = 0xFF;

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        [self.year, self.month, self.day, self.day_of_week]
    }

    /// Decode from 4 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::buffer_too_short(4, data.len()));
        }
        Ok(Self {
            year: data[0],
            month: data[1],
            day: data[2],
            day_of_week: data[3],
        })
    }

    /// Get the actual year (1900 + year field), or None if unspecified.
    pub fn actual_year(&self) -> Option<u16> {
        if self.year == Self::UNSPECIFIED {
            None
        } else {
            Some(1900 + self.year as u16)
        }
    }
}

// ---------------------------------------------------------------------------
// Time (Clause 20.2.13)
// ---------------------------------------------------------------------------

/// BACnet Time: hour, minute, second, hundredths.
///
/// Each field can be 0xFF for "unspecified".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Time {
    /// Hour (0-23, or 0xFF for unspecified).
    pub hour: u8,
    /// Minute (0-59, or 0xFF for unspecified).
    pub minute: u8,
    /// Second (0-59, or 0xFF for unspecified).
    pub second: u8,
    /// Hundredths of a second (0-99, or 0xFF for unspecified).
    pub hundredths: u8,
}

impl Time {
    /// Value indicating "unspecified" for any time field.
    pub const UNSPECIFIED: u8 = 0xFF;

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        [self.hour, self.minute, self.second, self.hundredths]
    }

    /// Decode from 4 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::buffer_too_short(4, data.len()));
        }
        Ok(Self {
            hour: data[0],
            minute: data[1],
            second: data[2],
            hundredths: data[3],
        })
    }
}

// ---------------------------------------------------------------------------
// BACnetTimeStamp (Clause 20.2.1.5)
// ---------------------------------------------------------------------------

/// BACnet timestamp -- a CHOICE of Time, sequence number, or DateTime.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetTimeStamp {
    /// Context tag 0: Time
    Time(Time),
    /// Context tag 1: Unsigned (sequence number)
    SequenceNumber(u64),
    /// Context tag 2: BACnetDateTime (Date + Time)
    DateTime { date: Date, time: Time },
}

// ---------------------------------------------------------------------------
// StatusFlags (Clause 12.X -- used by many object types)
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// BACnet StatusFlags -- 4-bit bitstring present on most objects.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct StatusFlags: u8 {
        const IN_ALARM = 0b1000;
        const FAULT = 0b0100;
        const OVERRIDDEN = 0b0010;
        const OUT_OF_SERVICE = 0b0001;
    }
}

// ---------------------------------------------------------------------------
// PropertyValue -- sum type for BACnet property values
// ---------------------------------------------------------------------------

/// A BACnet application-layer value.
///
/// This enum covers all primitive value types that can appear as property
/// values in BACnet objects. Constructed types (lists, sequences) are
/// represented as nested structures.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// Null value.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// Unsigned integer (up to 64-bit for BACnet Unsigned64).
    Unsigned(u64),
    /// Signed integer.
    Signed(i32),
    /// IEEE 754 single-precision float.
    Real(f32),
    /// IEEE 754 double-precision float.
    Double(f64),
    /// Octet string (raw bytes).
    OctetString(Vec<u8>),
    /// Character string (UTF-8).
    CharacterString(String),
    /// Bit string (variable length).
    BitString {
        /// Number of unused bits in the last byte.
        unused_bits: u8,
        /// The bit data bytes.
        data: Vec<u8>,
    },
    /// Enumerated value.
    Enumerated(u32),
    /// Date value.
    Date(Date),
    /// Time value.
    Time(Time),
    /// Object identifier.
    ObjectIdentifier(ObjectIdentifier),
    /// A sequence (array) of property values.
    ///
    /// Used when reading an entire array property with `arrayIndex` absent
    /// (Clause 15.5.1). Each element is encoded as its own application-tagged
    /// value, concatenated in order.
    List(Vec<PropertyValue>),
}

// ---------------------------------------------------------------------------
// Formatting helper macro (works in both std and no_std+alloc)
// ---------------------------------------------------------------------------

/// Format a string using either std or alloc.
#[cfg(feature = "std")]
macro_rules! alloc_or_std_format {
    ($($arg:tt)*) => { format!($($arg)*) }
}

#[cfg(not(feature = "std"))]
macro_rules! alloc_or_std_format {
    ($($arg:tt)*) => { alloc::format!($($arg)*) }
}

use alloc_or_std_format;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_identifier_encode_decode_round_trip() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let bytes = oid.encode();
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(oid, decoded);
    }

    #[test]
    fn object_identifier_wire_format() {
        // AnalogInput (type=0) instance=1: (0 << 22) | 1 = 0x00000001
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(oid.encode(), [0x00, 0x00, 0x00, 0x01]);

        // Device (type=8) instance=1234: (8 << 22) | 1234 = 0x020004D2
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        let expected = ((8u32 << 22) | 1234u32).to_be_bytes();
        assert_eq!(oid.encode(), expected);
    }

    #[test]
    fn object_identifier_max_instance() {
        let oid =
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap();
        assert_eq!(oid.instance_number(), 0x3F_FFFF);
        let bytes = oid.encode();
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(decoded.instance_number(), 0x3F_FFFF);
    }

    #[test]
    fn object_identifier_invalid_instance() {
        let result = ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE + 1);
        assert!(result.is_err());
    }

    #[test]
    fn object_identifier_buffer_too_short() {
        let result = ObjectIdentifier::decode(&[0x00, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn date_encode_decode_round_trip() {
        let date = Date {
            year: 124, // 2024
            month: 6,
            day: 15,
            day_of_week: 6, // Saturday
        };
        let bytes = date.encode();
        let decoded = Date::decode(&bytes).unwrap();
        assert_eq!(date, decoded);
        assert_eq!(decoded.actual_year(), Some(2024));
    }

    #[test]
    fn date_unspecified_year() {
        let date = Date {
            year: Date::UNSPECIFIED,
            month: 1,
            day: 1,
            day_of_week: Date::UNSPECIFIED,
        };
        assert_eq!(date.actual_year(), None);
    }

    #[test]
    fn time_encode_decode_round_trip() {
        let time = Time {
            hour: 14,
            minute: 30,
            second: 45,
            hundredths: 50,
        };
        let bytes = time.encode();
        let decoded = Time::decode(&bytes).unwrap();
        assert_eq!(time, decoded);
    }

    #[test]
    fn status_flags_operations() {
        let flags = StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE;
        assert!(flags.contains(StatusFlags::IN_ALARM));
        assert!(flags.contains(StatusFlags::OUT_OF_SERVICE));
        assert!(!flags.contains(StatusFlags::FAULT));
        assert!(!flags.contains(StatusFlags::OVERRIDDEN));
    }

    // --- OID edge cases ---

    #[test]
    fn object_identifier_instance_zero() {
        // Instance 0 is valid and commonly used (e.g., Device,0)
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap();
        assert_eq!(oid.instance_number(), 0);
        let bytes = oid.encode();
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(decoded.instance_number(), 0);
        assert_eq!(decoded.object_type(), ObjectType::DEVICE);
    }

    #[test]
    fn object_identifier_all_types_instance_zero() {
        // Instance 0 for each common type should round-trip correctly
        for type_raw in [0u32, 1, 2, 3, 4, 5, 6, 8, 10, 13, 14, 17, 19] {
            let obj_type = ObjectType::from_raw(type_raw);
            let oid = ObjectIdentifier::new(obj_type, 0).unwrap();
            let bytes = oid.encode();
            let decoded = ObjectIdentifier::decode(&bytes).unwrap();
            assert_eq!(decoded.object_type(), obj_type, "type {type_raw} failed");
            assert_eq!(
                decoded.instance_number(),
                0,
                "type {type_raw} instance failed"
            );
        }
    }

    #[test]
    fn object_identifier_wildcard_instance() {
        let oid =
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE).unwrap();
        assert_eq!(oid.instance_number(), ObjectIdentifier::MAX_INSTANCE);
        let bytes = oid.encode();
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(decoded.instance_number(), ObjectIdentifier::MAX_INSTANCE);
    }

    #[test]
    fn object_identifier_decode_extra_bytes_ignored() {
        // If we have more than 4 bytes, only the first 4 are used
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 42).unwrap();
        let mut bytes = oid.encode().to_vec();
        bytes.extend_from_slice(&[0xFF, 0xFF]); // extra garbage
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(decoded, oid);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "exceeds 10-bit field"))]
    fn object_identifier_type_overflow_round_trip() {
        // In debug builds, encode() asserts type <= 1023.
        // In release builds, types > 1023 are silently masked to 10 bits.
        let oid = ObjectIdentifier::new_unchecked(ObjectType::from_raw(1024), 0);
        let bytes = oid.encode();
        let decoded = ObjectIdentifier::decode(&bytes).unwrap();
        assert_eq!(decoded.object_type(), ObjectType::from_raw(0));
    }

    #[test]
    fn property_value_variants() {
        let null = PropertyValue::Null;
        let boolean = PropertyValue::Boolean(true);
        let real = PropertyValue::Real(72.5);
        let string = PropertyValue::CharacterString("test".into());

        assert_eq!(null, PropertyValue::Null);
        assert_eq!(boolean, PropertyValue::Boolean(true));
        assert_ne!(real, PropertyValue::Real(73.0));
        assert_eq!(string, PropertyValue::CharacterString("test".into()));
    }
}
