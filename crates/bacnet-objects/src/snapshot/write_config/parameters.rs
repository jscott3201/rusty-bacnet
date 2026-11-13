//! The event algorithm parameters that end a
//! [`KIND_REPORTING_PARAMETERS`] body
//! (#2331). The parent module documents the layout.
//!
//! One table, `rows!`, is all there is to a parameter: its field of
//! [`AlgorithmWriteConfig`], its bit of the presence mask (and with it its
//! property), how its value is stored (a module of `stored`) and the object
//! types that keep it (a list of `on`). The mask, the octets written, the
//! octets read back, the values refused and [`kept`] all come from a row, so
//! a parameter can't be written and not read, or read and not kept. A
//! parameter added later is one more row with the next bit; the kind stays,
//! and a build that doesn't keep the bit for the object's type refuses the
//! file as an unknown kind.

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bytes::BytesMut;

use super::{KIND_REPORTING_PARAMETERS, Reader, SnapshotError, SnapshotErrorKind};
use crate::write_config::AlgorithmWriteConfig;

/// What is wrong with a parameter outside [`kept`] for the object's type:
/// one a later build keeps for it, or one no build does.
const UNKEPT: &str = "holds an algorithm parameter this build doesn't keep for this object type";

/// The object types a row is kept for.
mod on {
    use bacnet_types::enums::ObjectType as T;

    /// OUT_OF_RANGE's own parameters: the analog Input, Output and Value.
    pub(super) const ANALOG: &[T] = &[T::ANALOG_INPUT, T::ANALOG_OUTPUT, T::ANALOG_VALUE];
    /// The one alarm value of CHANGE_OF_STATE on a Binary Input or Value.
    pub(super) const BINARY: &[T] = &[T::BINARY_INPUT, T::BINARY_VALUE];
    /// The two lists only an Access Door has: Fault_Values and
    /// Masked_Alarm_Values.
    pub(super) const DOOR: &[T] = &[T::ACCESS_DOOR];
    /// Alarm_Values as a list of states: a Multi-state Input or Value's
    /// state numbers, an Access Door's door alarm states and an Access
    /// Zone's occupancy states. One property and one row; what an entry
    /// may be is the type's own rule (`stored::states`).
    pub(super) const LISTED: &[T] = &[
        T::MULTI_STATE_INPUT,
        T::MULTI_STATE_VALUE,
        T::ACCESS_DOOR,
        T::ACCESS_ZONE,
    ];
    /// An Event Enrollment's own: Event_Parameters and Fault_Parameters.
    pub(super) const ENROLLMENT: &[T] = &[T::EVENT_ENROLLMENT];
    /// State_Text: the multi-state Input, Output and Value (#2414).
    pub(super) const MULTI_STATE: &[T] = &[
        T::MULTI_STATE_INPUT,
        T::MULTI_STATE_OUTPUT,
        T::MULTI_STATE_VALUE,
    ];
    /// The types with a Time_Delay_Normal property: those of [`DELAYED`],
    /// and an Event Enrollment, whose Time_Delay is a member of its
    /// Event_Parameters and so no row of its own.
    pub(super) const DELAYED_NORMAL: &[T] = &[
        T::ANALOG_INPUT,
        T::ANALOG_OUTPUT,
        T::ANALOG_VALUE,
        T::BINARY_INPUT,
        T::BINARY_OUTPUT,
        T::BINARY_VALUE,
        T::MULTI_STATE_INPUT,
        T::MULTI_STATE_OUTPUT,
        T::MULTI_STATE_VALUE,
        T::ACCESS_DOOR,
        T::ACCESS_ZONE,
        T::EVENT_ENROLLMENT,
    ];
    /// The types whose two time delays are properties of their own: those
    /// above, and the Binary and Multi-state Output, which run
    /// COMMAND_FAILURE.
    pub(super) const DELAYED: &[T] = &[
        T::ANALOG_INPUT,
        T::ANALOG_OUTPUT,
        T::ANALOG_VALUE,
        T::BINARY_INPUT,
        T::BINARY_OUTPUT,
        T::BINARY_VALUE,
        T::MULTI_STATE_INPUT,
        T::MULTI_STATE_OUTPUT,
        T::MULTI_STATE_VALUE,
        T::ACCESS_DOOR,
        T::ACCESS_ZONE,
    ];
}

/// How a parameter's value is stored. Each module has `put`, which appends
/// a value's octets, `get`, which reads them back, and `invalid`, which
/// says why a value is one no write to an object of the given type leaves.
mod stored {
    use bacnet_types::enums::ObjectType;
    use bytes::BytesMut;

    use super::{Reader, SnapshotError};

    /// An Unsigned of 32 bits, as the objects hold a time delay: four
    /// octets.
    pub(super) mod unsigned {
        use super::{BytesMut, ObjectType, Reader, SnapshotError};

        pub(in super::super) fn put(value: &u32, body: &mut BytesMut) {
            body.extend_from_slice(&value.to_be_bytes());
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<u32, SnapshotError> {
            reader.u32()
        }

        pub(in super::super) fn invalid(_: &u32, _: ObjectType) -> Option<&'static str> {
            None
        }
    }

    /// A limit: the four octets of an IEEE 754 single, finite.
    pub(super) mod limit {
        use super::{BytesMut, ObjectType, Reader, SnapshotError};

        pub(in super::super) fn put(value: &f32, body: &mut BytesMut) {
            body.extend_from_slice(&value.to_bits().to_be_bytes());
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<f32, SnapshotError> {
            Ok(f32::from_bits(reader.u32()?))
        }

        pub(in super::super) fn invalid(value: &f32, _: ObjectType) -> Option<&'static str> {
            (!value.is_finite()).then_some("holds a limit or deadband that isn't finite")
        }
    }

    /// Deadband: a limit that isn't negative either.
    pub(super) mod deadband {
        use super::ObjectType;
        pub(in super::super) use super::limit::{get, put};

        pub(in super::super) fn invalid(value: &f32, kind: ObjectType) -> Option<&'static str> {
            super::limit::invalid(value, kind)
                .or_else(|| (*value < 0.0).then_some("holds a negative Deadband"))
        }
    }

    /// Limit_Enable: one octet, bit 0 the low limit and bit 1 the high.
    pub(super) mod limit_enable {
        use bacnet_types::bitstring::LimitEnable;

        use super::{BytesMut, ObjectType, Reader, SnapshotError};

        pub(in super::super) fn put(value: &LimitEnable, body: &mut BytesMut) {
            body.extend_from_slice(&[value.bits()]);
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<LimitEnable, SnapshotError> {
            Ok(LimitEnable::from_bits_retain(reader.u8()?))
        }

        pub(in super::super) fn invalid(
            value: &LimitEnable,
            _: ObjectType,
        ) -> Option<&'static str> {
            LimitEnable::from_bits(value.bits())
                .is_none()
                .then_some("holds unknown Limit_Enable bits")
        }
    }

    /// A BACnetBinaryPV: one octet, 0 for INACTIVE or 1 for ACTIVE.
    pub(super) mod binary_pv {
        use bacnet_types::enums::BinaryPV;

        use super::{BytesMut, ObjectType, Reader, SnapshotError};

        pub(in super::super) fn put(value: &BinaryPV, body: &mut BytesMut) {
            // `invalid` has refused a value past one octet.
            body.extend_from_slice(&[value.to_raw() as u8]);
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<BinaryPV, SnapshotError> {
            Ok(BinaryPV::from_raw(u32::from(reader.u8()?)))
        }

        pub(in super::super) fn invalid(value: &BinaryPV, _: ObjectType) -> Option<&'static str> {
            (value.to_raw() > BinaryPV::ACTIVE.to_raw())
                .then_some("holds an Alarm_Value that is neither INACTIVE nor ACTIVE")
        }
    }

    /// A list of states: a count in four octets, at most the 1,024 a write
    /// of such a list may hold
    /// ([`MAX_ALARM_VALUES`](crate::multistate::MAX_ALARM_VALUES), the cap
    /// of every object type that has one), then each state in four octets.
    /// The count says how long the row is, so a row cut short doesn't read
    /// as a shorter list, and one going on past its count leaves octets the
    /// body's end refuses.
    ///
    /// The octets are the same on every type; what a state may be is the
    /// rule of the object's type, which [`invalid`](states::invalid) is
    /// handed:
    ///
    /// - a Multi-state Input's or Value's state number is any but zero
    ///   (the object's Number_Of_States is no part of the rule: a list
    ///   opens on an object built with fewer states);
    /// - an Access Door's or Access Zone's is a member of its enumeration,
    ///   named or proprietary, other than NORMAL, by the check its writes
    ///   run ([`listed_states`](crate::access_control::listed_states));
    /// - any other type has no rule, and every list is refused for it. No
    ///   such type keeps a list row.
    ///
    /// The type is the one in the file's header: a file is opened for one
    /// object identifier and refused for any other, so a list is never
    /// read under another type's rule.
    pub(super) mod states {
        use super::{BytesMut, ObjectType, Reader, SnapshotError};
        use crate::multistate::MAX_ALARM_VALUES;
        use crate::prelude::Vec;

        pub(in super::super) fn put(values: &[u32], body: &mut BytesMut) {
            // `invalid` has refused a list past the cap.
            body.extend_from_slice(&(values.len() as u32).to_be_bytes());
            for state in values {
                body.extend_from_slice(&state.to_be_bytes());
            }
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<Vec<u32>, SnapshotError> {
            // Nothing is allocated until the count is within the cap and
            // the body holds every state it promises.
            let count = reader.count(MAX_ALARM_VALUES)?;
            let octets = reader.take(count * 4)?;
            let (states, _) = octets.as_chunks::<4>();
            Ok(states
                .iter()
                .map(|state| u32::from_be_bytes(*state))
                .collect())
        }

        pub(in super::super) fn invalid(values: &[u32], kind: ObjectType) -> Option<&'static str> {
            if values.len() > MAX_ALARM_VALUES {
                return Some("holds a longer list of states than an object takes");
            }
            if matches!(
                kind,
                ObjectType::MULTI_STATE_INPUT | ObjectType::MULTI_STATE_VALUE
            ) {
                return values
                    .contains(&0)
                    .then_some("holds an Alarm_Values state of zero");
            }
            match crate::access_control::listed_states(kind) {
                Some(listed) => values
                    .iter()
                    .any(|state| !listed.admits(*state))
                    .then_some("holds a state its list doesn't take on this object type"),
                // No type gets here: every type a list row is kept for has
                // a rule above. One given a row without one has every list
                // refused, an empty one too, rather than taken unchecked.
                None => Some("holds a list of states for an object type with no rule for one"),
            }
        }
    }

    /// A value held in its BACnet encoding: a length in four octets, at
    /// most the 64 KiB an enrollment with storage takes in a write
    /// ([`MAX_KEPT_OCTETS`](crate::event_enrollment::parameters::MAX_KEPT_OCTETS)),
    /// then that many octets. The `event_parameters` and `fault_parameters`
    /// modules below are made from it.
    ///
    /// The octets are one whole value in the encoding a save writes, by
    /// the decoder a write of the property runs
    /// ([`reloaded`](crate::event_enrollment::parameters::reloaded), the
    /// rule a write to an enrollment with storage is held to as well): a
    /// row that doesn't decode, goes on past its value, or decodes to a
    /// value that encodes otherwise (a limit in another numeric form, say)
    /// is damage. So a file a load took is the file the next save writes.
    ///
    /// The length is held to the cap and to the body before an octet is
    /// read. The decoder then walks those octets once: it nests at most 32
    /// tags deep, takes at most 10,000 entries in a list, and allocates
    /// only for what the octets hold.
    macro_rules! encoded {
        ($name:ident, $value:ty, $decode:path, $encode:path, $what:literal) => {
            pub(super) mod $name {
                use super::{BytesMut, ObjectType, Reader, SnapshotError};
                use crate::event_enrollment::parameters::{MAX_KEPT_OCTETS, NotKept, reloaded};

                pub(in super::super) fn put(value: &$value, body: &mut BytesMut) {
                    // `invalid` has refused a value with no encoding, and
                    // one past the cap.
                    let octets = $encode(value).unwrap_or_default();
                    body.extend_from_slice(&(octets.len() as u32).to_be_bytes());
                    body.extend_from_slice(&octets);
                }

                pub(in super::super) fn get(
                    reader: &mut Reader<'_>,
                ) -> Result<$value, SnapshotError> {
                    let octets = reader.octets(MAX_KEPT_OCTETS)?;
                    reloaded(octets, $decode, $encode).map_err(|why| match why {
                        NotKept::Undecodable(_) => {
                            SnapshotError::damaged(concat!("holds ", $what, " that don't decode"))
                        }
                        NotKept::Recoded => SnapshotError::damaged(concat!(
                            "holds ",
                            $what,
                            " in an encoding no save writes"
                        )),
                    })
                }

                pub(in super::super) fn invalid(
                    value: &$value,
                    _: ObjectType,
                ) -> Option<&'static str> {
                    match $encode(value) {
                        Ok(octets) if octets.len() <= MAX_KEPT_OCTETS => None,
                        Ok(_) => Some(concat!("holds ", $what, " past their cap")),
                        Err(_) => Some(concat!("holds ", $what, " with no encoding")),
                    }
                }
            }
        };
    }

    encoded!(
        event_parameters,
        bacnet_types::constructed::BACnetEventParameter,
        crate::event_enrollment::parameters::framed_event_parameters,
        crate::event_enrollment::parameters::event_parameter_octets,
        "Event_Parameters"
    );
    /// State_Text: a count in four octets, from 1 to the 1,024 states an
    /// object takes
    /// ([`MAX_NUMBER_OF_STATES`](crate::multistate::MAX_NUMBER_OF_STATES)),
    /// then each label as a length in four octets, at most
    /// [`MAX_KEPT_STATE_TEXT_OCTETS`](crate::multistate::MAX_KEPT_STATE_TEXT_OCTETS),
    /// and that many octets of UTF-8. A count past the cap, or one the
    /// body's octets can't hold at four octets a label, is refused before
    /// anything is allocated, and a length past the cap before its octets
    /// are read. What a write leaves storage holding is one rule
    /// ([`state_text_fits`](crate::multistate::state_text_fits)), for a
    /// write and for a load.
    pub(super) mod texts {
        use super::{BytesMut, ObjectType, Reader, SnapshotError};
        use crate::multistate::{MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES};
        use crate::prelude::{String, Vec};

        pub(in super::super) fn put(texts: &[String], body: &mut BytesMut) {
            // `invalid` has refused a count or a label past its cap.
            body.extend_from_slice(&(texts.len() as u32).to_be_bytes());
            for text in texts {
                body.extend_from_slice(&(text.len() as u32).to_be_bytes());
                body.extend_from_slice(text.as_bytes());
            }
        }

        pub(in super::super) fn get(reader: &mut Reader<'_>) -> Result<Vec<String>, SnapshotError> {
            let count = reader.count(MAX_NUMBER_OF_STATES as usize)?;
            // Each label takes at least its four-octet length.
            if count * 4 > reader.body.len() - reader.at {
                return Err(SnapshotError::truncated(
                    "holds fewer octets than its State_Text count needs",
                ));
            }
            let mut texts = Vec::with_capacity(count);
            for _ in 0..count {
                let octets = reader.octets(MAX_KEPT_STATE_TEXT_OCTETS)?;
                let text = core::str::from_utf8(octets).map_err(|_| {
                    SnapshotError::damaged("holds a State_Text label that isn't UTF-8")
                })?;
                texts.push(String::from(text));
            }
            Ok(texts)
        }

        pub(in super::super) fn invalid(texts: &[String], _: ObjectType) -> Option<&'static str> {
            (!crate::multistate::state_text_fits(texts))
                .then_some("holds a State_Text no write leaves storage holding")
        }
    }

    encoded!(
        fault_parameters,
        bacnet_types::constructed::FaultParameters,
        crate::event_enrollment::parameters::framed_fault_parameters,
        crate::event_enrollment::parameters::fault_parameter_octets,
        "Fault_Parameters"
    );
}

/// The parameter table. A row is
/// `field: NAME = bit, how it is stored, the types that keep it;`, where
/// `NAME` names both the bit and the property. Rows are in bit order,
/// which is the order a file holds them in.
macro_rules! rows {
    ($($(#[$doc:meta])* $field:ident: $name:ident = $bit:expr, $stored:ident, $on:expr;)*) => {
        /// Each parameter's bit of the presence mask. A bit keeps its
        /// meaning for good: files hold it.
        pub(crate) mod bit {
            $($(#[$doc])* pub(crate) const $name: u32 = $bit;)*
        }

        // A file holds the rows in bit order, and they are written and read
        // in the table's: each row is one bit, above the row before.
        const _: () = {
            let bits = [$(bit::$name),*];
            let mut row = 0;
            while row < bits.len() {
                assert!(bits[row].count_ones() == 1);
                assert!(row == 0 || bits[row - 1] < bits[row]);
                row += 1;
            }
        };

        /// The presence mask of `config`: the bits of the parameters it
        /// holds.
        pub(crate) fn mask(config: &AlgorithmWriteConfig) -> u32 {
            let mut mask = 0;
            $(if config.$field.is_some() {
                mask |= bit::$name;
            })*
            mask
        }

        /// The parameters of `config` whose bits are in `mask`.
        // event-reporting: only the reporting store picks rows.
        #[cfg(feature = "event-reporting")]
        pub(crate) fn only(config: &AlgorithmWriteConfig, mask: u32) -> AlgorithmWriteConfig {
            AlgorithmWriteConfig {
                $($field: if mask & bit::$name != 0 {
                    config.$field.clone()
                } else {
                    None
                },)*
            }
        }

        /// Whether `saved` is [`only`] of `config` and `mask`, compared
        /// without the copy (#2431).
        // A staged step's check alone uses it (#2532).
        #[cfg(all(feature = "event-reporting", feature = "durable-writes"))]
        pub(crate) fn is_only(
            saved: &AlgorithmWriteConfig,
            config: &AlgorithmWriteConfig,
            mask: u32,
        ) -> bool {
            true $(&& if mask & bit::$name != 0 {
                saved.$field == config.$field
            } else {
                saved.$field.is_none()
            })*
        }

        /// Put the parameters `saved` holds over those of `onto`.
        #[cfg(feature = "event-reporting")]
        pub(crate) fn overlay(onto: &mut AlgorithmWriteConfig, saved: &AlgorithmWriteConfig) {
            $(if saved.$field.is_some() {
                onto.$field.clone_from(&saved.$field);
            })*
        }

        /// Every row: its bit and its property, in bit order. A test that
        /// walks the table (the detector side isn't made from it) reads it
        /// here.
        #[cfg(test)]
        pub(crate) const ROWS: &[(u32, P)] = &[$((bit::$name, P::$name)),*];

        /// The mask bit of `property`, if it is a parameter this build
        /// keeps for some object type. A property has one bit on every
        /// type that keeps it; where the types differ in what its value
        /// may be, the row's `stored` module is handed the type.
        pub(crate) fn bit_of(property: P) -> Option<u32> {
            match property {
                $(P::$name => Some(bit::$name),)*
                _ => None,
            }
        }

        /// The parameters objects of `object_type` keep, as mask bits; no
        /// bit for a type that keeps none.
        pub(crate) fn kept(object_type: ObjectType) -> u32 {
            let mut mask = 0;
            $(if $on.contains(&object_type) {
                mask |= bit::$name;
            })*
            mask
        }

        /// Why a parameter of `config` holds a value no write to an object
        /// of `object_type` leaves.
        fn invalid_value(
            config: &AlgorithmWriteConfig,
            object_type: ObjectType,
        ) -> Option<&'static str> {
            $(if let Some(reason) = config
                .$field
                .as_ref()
                .and_then(|value| stored::$stored::invalid(value, object_type))
            {
                return Some(reason);
            })*
            None
        }

        /// Append each parameter `config` holds, in bit order.
        fn put(body: &mut BytesMut, config: &AlgorithmWriteConfig) {
            $(if let Some(value) = &config.$field {
                stored::$stored::put(value, body);
            })*
        }

        impl Reader<'_> {
            /// The parameters `mask` names, in bit order.
            fn rows(&mut self, mask: u32) -> Result<AlgorithmWriteConfig, SnapshotError> {
                Ok(AlgorithmWriteConfig {
                    $($field: if mask & bit::$name != 0 {
                        Some(stored::$stored::get(self)?)
                    } else {
                        None
                    },)*
                })
            }
        }
    };
}

rows! {
    /// Time_Delay: four octets.
    time_delay: TIME_DELAY = 1 << 0, unsigned, on::DELAYED;
    /// Time_Delay_Normal: four octets.
    time_delay_normal: TIME_DELAY_NORMAL = 1 << 1, unsigned, on::DELAYED_NORMAL;
    /// High_Limit: the four octets of an IEEE 754 single.
    high_limit: HIGH_LIMIT = 1 << 2, limit, on::ANALOG;
    /// Low_Limit, as High_Limit.
    low_limit: LOW_LIMIT = 1 << 3, limit, on::ANALOG;
    /// Deadband, as High_Limit.
    deadband: DEADBAND = 1 << 4, deadband, on::ANALOG;
    /// Limit_Enable: one octet, bit 0 the low limit and bit 1 the high.
    limit_enable: LIMIT_ENABLE = 1 << 5, limit_enable, on::ANALOG;
    /// Alarm_Value of a binary object: one octet, 0 or 1.
    alarm_value: ALARM_VALUE = 1 << 6, binary_pv, on::BINARY;
    /// Alarm_Values of a multi-state object, an Access Door or an Access
    /// Zone: a count in four octets, then each state in four.
    alarm_values: ALARM_VALUES = 1 << 7, states, on::LISTED;
    /// Fault_Values of an Access Door, as Alarm_Values.
    fault_values: FAULT_VALUES = 1 << 8, states, on::DOOR;
    /// Masked_Alarm_Values of an Access Door, as Alarm_Values.
    masked_alarm_values: MASKED_ALARM_VALUES = 1 << 9, states, on::DOOR;
    /// Event_Parameters of an Event Enrollment: a length in four octets,
    /// then the value's BACnet encoding.
    event_parameters: EVENT_PARAMETERS = 1 << 10, event_parameters, on::ENROLLMENT;
    /// Fault_Parameters of an Event Enrollment, as Event_Parameters.
    fault_parameters: FAULT_PARAMETERS = 1 << 11, fault_parameters, on::ENROLLMENT;
    /// State_Text of a multi-state object, no event algorithm parameter
    /// but kept beside them (#2414): a count in four octets, then each
    /// label as its length in four octets and its UTF-8.
    state_text: STATE_TEXT = 1 << 12, texts, on::MULTI_STATE;
}

// The largest file a multi-state object can save fits the format's cap:
// every label at its cap, beside a Description and three message texts at
// theirs, a full Alarm_Values list, and room to spare for the rest.
const _: () = {
    use crate::multistate::{MAX_ALARM_VALUES, MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES};
    let texts = 4 + MAX_NUMBER_OF_STATES as usize * (4 + MAX_KEPT_STATE_TEXT_OCTETS);
    let others = 4 * (4 + crate::event::options::MAX_KEPT_TEXT_OCTETS) + 4 + 4 * MAX_ALARM_VALUES;
    assert!(texts + others + 64 * 1024 <= super::MAX_LEN);
};

/// Why a list row holding `values` is one no write to an object of
/// `object_type` leaves: the rule itself, for its tests.
#[cfg(test)]
pub(crate) fn invalid_list(values: &[u32], object_type: ObjectType) -> Option<&'static str> {
    stored::states::invalid(values, object_type)
}

/// Whether objects of `object_type` keep a written `property` as an event
/// algorithm parameter in a
/// [`KIND_REPORTING_PARAMETERS`] file
/// (#2331):
///
/// - Time_Delay and Time_Delay_Normal on the analog, binary and multi-state
///   Input, Output and Value objects, Access Door and Access Zone;
/// - Time_Delay_Normal, Event_Parameters and Fault_Parameters on an Event
///   Enrollment, whose Time_Delay is a member of its Event_Parameters;
/// - High_Limit, Low_Limit, Deadband and Limit_Enable on the three analog
///   ones;
/// - Alarm_Value on a Binary Input or Value;
/// - Alarm_Values on a Multi-state Input or Value, an Access Door and an
///   Access Zone;
/// - Fault_Values and Masked_Alarm_Values on an Access Door;
/// - State_Text on a Multi-state Input, Output or Value (#2414), which is
///   no event algorithm parameter but is kept in the same table, and with
///   it Number_Of_States.
///
/// An Alert Enrollment keeps none: it has no algorithm parameter a peer
/// writes. A file holding a parameter the object's type doesn't keep here is
/// refused as an unknown kind: a later build may keep it.
pub fn keeps_parameter(object_type: ObjectType, property: P) -> bool {
    bit_of(property).is_some_and(|bit| kept(object_type) & bit != 0)
}

/// Why `config` holds what no write to an object of `object_type` leaves: a
/// parameter the type doesn't keep, a limit or deadband that isn't finite,
/// a negative deadband, Limit_Enable bits past its two, an Alarm_Value past
/// ACTIVE, or a list of states past the 1,024 a write may hold or naming a
/// state the type's lists don't take (zero on a multi-state object; NORMAL
/// or a value outside the enumeration on an Access Door or Zone), or
/// Event_Parameters or Fault_Parameters with no encoding or one past the
/// 64 KiB an enrollment with storage takes, or a State_Text of no label,
/// of more than 1,024 or with a label past 512 octets. The
/// object refuses each on the wire, so storage holding one is refused too.
/// A state past a multi-state object's own Number_Of_States is no such
/// value: on storage holding no State_Text the count is the one the
/// application builds the object with, so the object takes the list as it
/// was written.
pub(crate) fn invalid(
    object_type: ObjectType,
    config: &AlgorithmWriteConfig,
) -> Option<&'static str> {
    if mask(config) & !kept(object_type) != 0 {
        return Some(UNKEPT);
    }
    invalid_value(config, object_type)
}

/// Append `config`'s parameters for an object of `object_type`: the mask,
/// then each parameter it holds in bit order. Refuses one [`invalid`]
/// names.
pub(super) fn encode(
    body: &mut BytesMut,
    object_type: ObjectType,
    config: &AlgorithmWriteConfig,
) -> Result<(), SnapshotError> {
    if let Some(reason) = invalid(object_type, config) {
        return Err(SnapshotError::out_of_range(reason));
    }
    body.extend_from_slice(&mask(config).to_be_bytes());
    put(body, config);
    Ok(())
}

impl Reader<'_> {
    /// The parameters after a reporting body, for an object of
    /// `object_type`. A bit outside [`kept`] for that type, whether this
    /// build knows it for another type or not at all, is a newer build's
    /// sound file ([`UnknownKind`](SnapshotErrorKind::UnknownKind)), refused
    /// before a row is read, and never damage: a backend with two copies
    /// mustn't pass it over for an older one. No bit at all, or a kept
    /// parameter holding what [`invalid`] names, is damage.
    pub(super) fn parameters(
        &mut self,
        object_type: ObjectType,
    ) -> Result<AlgorithmWriteConfig, SnapshotError> {
        let mask = self.u32()?;
        if mask & !kept(object_type) != 0 {
            return Err(SnapshotError::new(
                SnapshotErrorKind::UnknownKind(KIND_REPORTING_PARAMETERS),
                UNKEPT,
            ));
        }
        if mask == 0 {
            return Err(SnapshotError::damaged(
                "is of the kind with algorithm parameters and holds none",
            ));
        }
        let config = self.rows(mask)?;
        match invalid_value(&config, object_type) {
            Some(reason) => Err(SnapshotError::damaged(reason)),
            None => Ok(config),
        }
    }
}
