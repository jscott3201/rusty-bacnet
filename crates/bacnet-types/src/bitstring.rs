//! Named BACnet bit-string types (ASHRAE 135-2020 Clause 21).
//!
//! Several properties carry a `BitString` whose bits have a fixed, named
//! meaning defined by the standard. This module models those `BACnetXxx` BIT
//! STRING productions so a decoded [`crate::primitives::PropertyValue::BitString`]
//! can be promoted from raw `(unused_bits, data)` bytes to named flags, the same
//! way [`crate::enums::ResolvedEnum`] promotes an `Enumerated`.
//!
//! ## Bit order
//!
//! BACnet bit strings are transmitted most-significant-bit-first: bit 0 of the
//! string is the top bit (`0x80`) of the first content byte, bit 8 is the top
//! bit of the second byte, and so on. Every type here reads bit *N* of the
//! string as "feature N", matching how the device object packs these fields
//! (see `bacnet-objects`'s `compute_object_types_supported`).
//!
//! [`crate::primitives::StatusFlags`] predates this module and keeps its own
//! right-aligned representation; it is re-exported and mapped by the resolver as
//! well.

use crate::enums::{ObjectType, ServiceSupported};

pub use crate::primitives::StatusFlags;

/// Read bit `n` of a BACnet bit string (MSB-first) from its content bytes.
///
/// `data` is the bit-string payload *without* the leading unused-bits count
/// (i.e. `PropertyValue::BitString::data`). Bits past the end read as `false`.
fn wire_bit(data: &[u8], n: usize) -> bool {
    let mask = 0x80u8 >> (n % 8);
    data.get(n / 8).is_some_and(|b| b & mask != 0)
}

/// Pack a bit0-first value into its Clause 20.2.10 wire octet.
///
/// Every ≤8-bit BACnet bit string in this stack keeps its internal layout as
/// `bit0 = first defined bit` (`TO_OFFNORMAL`, `monday`, `low-limit-enable`,
/// …), while the wire wants the first defined bit in the most significant bit
/// of the octet. Reversing the byte is that whole conversion: bit 0 lands at
/// `0x80`, bit 1 at `0x40`, and the result is left-aligned for any width.
pub fn pack_octet(bits_lsb0: u8) -> u8 {
    bits_lsb0.reverse_bits()
}

/// Inverse of [`pack_octet`]: read the first octet of a bit-string payload
/// back into bit0-first form, masked to the string's `defined_bits`.
///
/// Masking (rather than trusting the peer's declared unused-bit count) keeps
/// nonconformant padding out of the value; an empty payload reads as zero.
pub fn unpack_octet(data: &[u8], defined_bits: u32) -> u8 {
    let mask = if defined_bits >= 8 {
        u8::MAX
    } else {
        (1u8 << defined_bits) - 1
    };
    data.first().copied().unwrap_or(0).reverse_bits() & mask
}

/// Decode a `BACnetStatusFlags` bit-string payload (MSB-first) into
/// [`StatusFlags`], which keeps its right-aligned in-memory layout.
///
/// Reads wire bits 0–3 by position, so a peer that declares a wrong string
/// length still decodes correctly — Clause 20.2.10 puts bit 0 in the most
/// significant bit of the first octet regardless of length.
pub fn status_flags_from_bacnet(data: &[u8]) -> StatusFlags {
    let mut flags = StatusFlags::empty();
    flags.set(StatusFlags::IN_ALARM, wire_bit(data, 0));
    flags.set(StatusFlags::FAULT, wire_bit(data, 1));
    flags.set(StatusFlags::OVERRIDDEN, wire_bit(data, 2));
    flags.set(StatusFlags::OUT_OF_SERVICE, wire_bit(data, 3));
    flags
}

/// Render a bitflags value as its set Clause-21 names (` NAME | NAME `), or `()`
/// when none are set — for both `Display` and `Debug`, never a raw number.
macro_rules! impl_named_bit_display {
    ($t:ty) => {
        impl core::fmt::Display for $t {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut first = true;
                for (name, _) in self.iter_names() {
                    if !first {
                        f.write_str(" | ")?;
                    }
                    f.write_str(name)?;
                    first = false;
                }
                if first {
                    f.write_str("()")?;
                }
                Ok(())
            }
        }

        impl core::fmt::Debug for $t {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(self, f)
            }
        }
    };
}

/// Write `Display` items as ` A | B | C ` (or `()` when empty).
fn write_joined<T: core::fmt::Display>(
    f: &mut core::fmt::Formatter<'_>,
    items: impl Iterator<Item = T>,
) -> core::fmt::Result {
    let mut first = true;
    for item in items {
        if !first {
            f.write_str(" | ")?;
        }
        write!(f, "{item}")?;
        first = false;
    }
    if first {
        f.write_str("()")?;
    }
    Ok(())
}

bitflags::bitflags! {
    /// `BACnetEventTransitionBits` — 3-bit string for `event-enable`,
    /// `acked-transitions`, and `ack-required` (Clause 21).
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct EventTransitionBits: u8 {
        /// Transition to an off-normal event state (bit 0).
        const TO_OFFNORMAL = 1 << 0;
        /// Transition to the fault event state (bit 1).
        const TO_FAULT = 1 << 1;
        /// Transition to the normal event state (bit 2).
        const TO_NORMAL = 1 << 2;
    }
}

impl EventTransitionBits {
    /// Decode from a BACnet bit-string payload (MSB-first).
    pub fn from_bacnet(data: &[u8]) -> Self {
        let mut bits = Self::empty();
        bits.set(Self::TO_OFFNORMAL, wire_bit(data, 0));
        bits.set(Self::TO_FAULT, wire_bit(data, 1));
        bits.set(Self::TO_NORMAL, wire_bit(data, 2));
        bits
    }

    /// Encode to the single Clause 20.2.10 wire octet (`unused_bits: 5`):
    /// `TO_OFFNORMAL` at `0x80`, `TO_FAULT` at `0x40`, `TO_NORMAL` at `0x20`.
    pub fn to_bacnet(self) -> u8 {
        pack_octet(self.bits())
    }
}

impl_named_bit_display!(EventTransitionBits);

bitflags::bitflags! {
    /// `BACnetLimitEnable` — 2-bit string for the `limit-enable` property
    /// (Clause 21).
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct LimitEnable: u8 {
        /// Low-limit event detection is enabled (bit 0).
        const LOW_LIMIT_ENABLE = 1 << 0;
        /// High-limit event detection is enabled (bit 1).
        const HIGH_LIMIT_ENABLE = 1 << 1;
    }
}

impl LimitEnable {
    /// Decode from a BACnet bit-string payload (MSB-first).
    pub fn from_bacnet(data: &[u8]) -> Self {
        let mut bits = Self::empty();
        bits.set(Self::LOW_LIMIT_ENABLE, wire_bit(data, 0));
        bits.set(Self::HIGH_LIMIT_ENABLE, wire_bit(data, 1));
        bits
    }

    /// Encode to the single Clause 20.2.10 wire octet (`unused_bits: 6`):
    /// `LOW_LIMIT_ENABLE` at `0x80`, `HIGH_LIMIT_ENABLE` at `0x40`.
    pub fn to_bacnet(self) -> u8 {
        pack_octet(self.bits())
    }
}

impl_named_bit_display!(LimitEnable);

/// `BACnetServicesSupported` — the `protocol-services-supported` bit string,
/// one bit per protocol service (Clause 21).
///
/// Stored as a `u64` bitset: bit *N* is set iff [`ServiceSupported`] value *N*
/// is supported. `protocol-services-supported` is a closed enumeration whose
/// highest defined bit is 48 (with no vendor-proprietary range), so a `u64`
/// covers every standard service with headroom. Any nonstandard bit ≥ 64 — a
/// protocol violation — is ignored on decode. The public API still yields named
/// [`ServiceSupported`] values, and `Debug` prints names, never a raw number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ServicesSupported {
    bits: u64,
}

impl ServicesSupported {
    /// Decode a bit-string payload (the `data` of `PropertyValue::BitString`)
    /// into the bitset of services it advertises.
    pub fn from_bacnet(data: &[u8]) -> Self {
        let mut bits = 0u64;
        for (i, &byte) in data.iter().take(8).enumerate() {
            bits |= (byte.reverse_bits() as u64) << (i * 8);
        }
        Self { bits }
    }

    /// Whether `service` is marked supported.
    pub fn contains(&self, service: ServiceSupported) -> bool {
        let n = service.to_raw() as u32;
        n < 64 && self.bits & (1u64 << n) != 0
    }

    /// Iterate the supported services in ascending bit order.
    pub fn iter(&self) -> impl Iterator<Item = ServiceSupported> + '_ {
        let bits = self.bits;
        (0..64)
            .filter(move |&n| bits & (1u64 << n) != 0)
            .map(|n| ServiceSupported::from_raw(n as u8))
    }
}

impl core::fmt::Display for ServicesSupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write_joined(f, self.iter())
    }
}

impl core::fmt::Debug for ServicesSupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

/// `BACnetObjectTypesSupported` — the `protocol-object-types-supported` bit
/// string, one bit per object type (Clause 21).
///
/// Stored as a `[u64; 16]` bitset (1024 bits): bit *N* is set iff [`ObjectType`]
/// value *N* is supported. The property covers only standardized object types
/// (Clause 12.11.15) and the bit-string production is closed, but
/// `BACnetObjectType` values run to 1023, so decode tolerates the enumeration's
/// full 10-bit range instead of discarding a nonconformant peer's extra bits —
/// hence the 16-word array, which is allocation-free and `Copy`. The public API
/// still yields named
/// [`ObjectType`] values (reusing that table rather than duplicating it), and
/// `Debug` prints names, never raw numbers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ObjectTypesSupported {
    bits: [u64; 16],
}

impl ObjectTypesSupported {
    /// Decode a bit-string payload (the `data` of `PropertyValue::BitString`)
    /// into the bitset of object types it advertises.
    pub fn from_bacnet(data: &[u8]) -> Self {
        let mut bits = [0u64; 16];
        for (i, &byte) in data.iter().take(128).enumerate() {
            bits[i / 8] |= (byte.reverse_bits() as u64) << ((i % 8) * 8);
        }
        Self { bits }
    }

    /// Whether `object_type` is marked supported.
    pub fn contains(&self, object_type: ObjectType) -> bool {
        let n = object_type.to_raw();
        n < 1024 && self.bits[(n / 64) as usize] & (1u64 << (n % 64)) != 0
    }

    /// Iterate the supported object types in ascending bit order.
    pub fn iter(&self) -> impl Iterator<Item = ObjectType> + '_ {
        (0..1024u32)
            .filter(move |&n| self.bits[(n / 64) as usize] & (1u64 << (n % 64)) != 0)
            .map(ObjectType::from_raw)
    }
}

impl core::fmt::Display for ObjectTypesSupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write_joined(f, self.iter())
    }
}

impl core::fmt::Debug for ObjectTypesSupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::StatusFlags;

    #[test]
    fn pack_unpack_octet_spec_vectors() {
        // Clause 20.2.10: first defined bit -> MSB. Deliberately asymmetric
        // values — round trips alone cannot catch a symmetric encode/decode
        // inversion, only literal wire bytes can.
        assert_eq!(pack_octet(0b001), 0x80); // TO_OFFNORMAL / monday
        assert_eq!(pack_octet(0b100), 0x20); // TO_NORMAL
        assert_eq!(pack_octet(0b0100_0000), 0x02); // sunday (7-bit valid_days)
        assert_eq!(unpack_octet(&[0x80], 3), 0b001);
        assert_eq!(unpack_octet(&[0x20], 3), 0b100);
        assert_eq!(unpack_octet(&[0xFE], 7), 0x7F); // all seven days
        assert_eq!(unpack_octet(&[], 3), 0);
        // Nonconformant junk past the defined bits is masked off.
        assert_eq!(unpack_octet(&[0xFF], 3), 0b111);
        // The octet helpers and the typed codecs agree.
        assert_eq!(
            EventTransitionBits::from_bacnet(&[pack_octet(0b001)]),
            EventTransitionBits::TO_OFFNORMAL
        );
        assert_eq!(EventTransitionBits::TO_OFFNORMAL.to_bacnet(), 0x80);
        assert_eq!(LimitEnable::LOW_LIMIT_ENABLE.to_bacnet(), 0x80);
    }

    #[test]
    fn event_transition_bits_msb_first() {
        // Wire byte 0b1010_0000 => bit0 (to-offnormal) and bit2 (to-normal).
        let bits = EventTransitionBits::from_bacnet(&[0b1010_0000]);
        assert_eq!(
            bits,
            EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL
        );
        assert_eq!(bits.to_string(), "TO_OFFNORMAL | TO_NORMAL");
    }

    #[test]
    fn limit_enable_msb_first() {
        assert_eq!(
            LimitEnable::from_bacnet(&[0x80]),
            LimitEnable::LOW_LIMIT_ENABLE
        );
        assert_eq!(
            LimitEnable::from_bacnet(&[0xC0]),
            LimitEnable::LOW_LIMIT_ENABLE | LimitEnable::HIGH_LIMIT_ENABLE
        );
        assert_eq!(LimitEnable::from_bacnet(&[]).to_string(), "()");
    }

    #[test]
    fn object_types_supported_matches_device_layout() {
        // Byte 8 = 0x80 sets bit 64 => ColorTemperature (per device object test).
        let mut data = vec![0u8; 9];
        data[8] = 0x80;
        let ots = ObjectTypesSupported::from_bacnet(&data);
        assert!(ots.contains(ObjectType::COLOR_TEMPERATURE));
        assert!(!ots.contains(ObjectType::ANALOG_INPUT));
        assert_eq!(
            ots.iter().collect::<Vec<_>>(),
            vec![ObjectType::COLOR_TEMPERATURE]
        );

        // Byte 4 = 0xFD: types 32-37,39 set, 38 (NetworkSecurity) unset.
        let ss = ObjectTypesSupported::from_bacnet(&[0, 0, 0, 0, 0xFD]);
        assert!(ss.contains(ObjectType::from_raw(32)));
        assert!(!ss.contains(ObjectType::NETWORK_SECURITY)); // type 38
        assert!(ss.contains(ObjectType::from_raw(39)));
    }

    #[test]
    fn resolved_value_holds_names_not_bytes() {
        // The whole point: the decoded value contains named variants, and even
        // its Debug shows names rather than the raw wire bytes.
        let ots = ObjectTypesSupported::from_bacnet(&[0b1110_0000]); // types 0,1,2
        assert_eq!(
            ots.iter().collect::<Vec<_>>(),
            [
                ObjectType::ANALOG_INPUT,
                ObjectType::ANALOG_OUTPUT,
                ObjectType::ANALOG_VALUE
            ]
        );
        let dbg = format!("{ots:?}");
        assert!(
            dbg.contains("ANALOG_INPUT"),
            "Debug should name types: {dbg}"
        );
        assert!(
            !dbg.contains("224"),
            "Debug must not show raw byte 0xE0: {dbg}"
        );
    }

    #[test]
    fn services_supported_follows_clause_21() {
        // Device object's basic set: byte0=0xA4, byte1=0x0B, byte4=0x80.
        let ss = ServicesSupported::from_bacnet(&[0xA4, 0x0B, 0x80, 0x35, 0x80, 0x00]);
        assert!(ss.contains(ServiceSupported::ACKNOWLEDGE_ALARM)); // bit 0
        assert!(ss.contains(ServiceSupported::SUBSCRIBE_COV)); // bit 5
        assert!(ss.contains(ServiceSupported::READ_PROPERTY)); // bit 12
        assert!(ss.contains(ServiceSupported::I_AM)); // bit 26
        assert!(ss.contains(ServiceSupported::TIME_SYNCHRONIZATION)); // bit 32 (NOT who-is)
        assert!(!ss.contains(ServiceSupported::WHO_IS)); // bit 34
    }

    #[test]
    fn status_flags_render_names_not_numbers() {
        assert_eq!(
            (StatusFlags::IN_ALARM | StatusFlags::FAULT).to_string(),
            "IN_ALARM | FAULT"
        );
        // Empty renders as "()", and Debug matches Display — never a raw number.
        assert_eq!(StatusFlags::empty().to_string(), "()");
        assert_eq!(format!("{:?}", StatusFlags::empty()), "()");
        assert_eq!(format!("{:?}", StatusFlags::IN_ALARM), "IN_ALARM");
    }

    #[test]
    fn small_types_debug_is_named_never_a_number() {
        // bitflags' derived Debug prints 0x0 for an empty set; ours prints "()".
        assert_eq!(format!("{:?}", EventTransitionBits::empty()), "()");
        assert_eq!(format!("{:?}", LimitEnable::empty()), "()");
        assert_eq!(format!("{:?}", EventTransitionBits::TO_FAULT), "TO_FAULT");
    }
}
