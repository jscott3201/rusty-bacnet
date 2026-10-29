//! Macros and helpers shared across object types to reduce duplication.
//!
//! These macros extract the common read/write property arms and error
//! construction patterns that are identical across analog, binary, and
//! multi-state object implementations.

/// Compute StatusFlags with all four bits dynamically set.
///
/// IN_ALARM: TRUE when event_state is not NORMAL.
/// FAULT: TRUE when reliability != NO_FAULT_DETECTED.
/// OUT_OF_SERVICE: from the object's out_of_service flag.
/// OVERRIDDEN: always FALSE for software-only (callers can set in base_flags).
pub fn compute_status_flags(
    base_flags: bacnet_types::primitives::StatusFlags,
    reliability: bacnet_types::enums::Reliability,
    out_of_service: bool,
    event_state: bacnet_types::enums::EventState,
) -> bacnet_types::primitives::PropertyValue {
    let mut flags = base_flags;
    if event_state != bacnet_types::enums::EventState::NORMAL {
        flags |= bacnet_types::primitives::StatusFlags::IN_ALARM;
    } else {
        flags -= bacnet_types::primitives::StatusFlags::IN_ALARM;
    }
    if reliability != bacnet_types::enums::Reliability::NO_FAULT_DETECTED {
        flags |= bacnet_types::primitives::StatusFlags::FAULT;
    } else {
        flags -= bacnet_types::primitives::StatusFlags::FAULT;
    }
    if out_of_service {
        flags |= bacnet_types::primitives::StatusFlags::OUT_OF_SERVICE;
    } else {
        flags -= bacnet_types::primitives::StatusFlags::OUT_OF_SERVICE;
    }
    bacnet_types::primitives::PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    }
}

/// Construct a protocol `Error` from an `ErrorClass` and `ErrorCode`.
#[inline]
pub(crate) fn protocol_error(
    class: bacnet_types::enums::ErrorClass,
    code: bacnet_types::enums::ErrorCode,
) -> bacnet_types::error::Error {
    bacnet_types::error::Error::protocol(class, code)
}

/// Classify an unhandled built-in write after its specific guards and routes.
/// Effective metadata includes PROPERTY_LIST and per-instance optional properties.
pub(crate) fn unhandled_write_error(
    metadata: &[crate::property_metadata::PropertyMetadata],
    property: bacnet_types::enums::PropertyIdentifier,
    array_index: Option<u32>,
) -> bacnet_types::error::Error {
    // Indexed fallback precedence is outside this unindexed correction.
    if array_index.is_some()
        || metadata
            .iter()
            .any(|row| row.property_identifier == property)
    {
        write_access_denied_error()
    } else {
        unknown_property_error()
    }
}

/// Read the PROPERTY_LIST property for any object that implements property_list().
/// Handles array_index variants: None = full list, Some(0) = length, Some(n) = nth element.
/// Object_Name, Object_Type, Object_Identifier, and Property_List itself are excluded.
pub fn read_property_list_property(
    props: &[bacnet_types::enums::PropertyIdentifier],
    array_index: Option<u32>,
) -> Result<bacnet_types::primitives::PropertyValue, bacnet_types::error::Error> {
    use bacnet_types::enums::PropertyIdentifier;

    // Filter out the four excluded properties
    let filtered: Vec<_> = props
        .iter()
        .copied()
        .filter(|p| {
            *p != PropertyIdentifier::OBJECT_IDENTIFIER
                && *p != PropertyIdentifier::OBJECT_NAME
                && *p != PropertyIdentifier::OBJECT_TYPE
                && *p != PropertyIdentifier::PROPERTY_LIST
        })
        .collect();

    match array_index {
        None => {
            let elements = filtered
                .iter()
                .map(|p| bacnet_types::primitives::PropertyValue::Enumerated(p.to_raw()))
                .collect();
            Ok(bacnet_types::primitives::PropertyValue::List(elements))
        }
        Some(0) => Ok(bacnet_types::primitives::PropertyValue::Unsigned(
            filtered.len() as u64,
        )),
        Some(idx) => {
            let i = (idx - 1) as usize;
            if i < filtered.len() {
                Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    filtered[i].to_raw(),
                ))
            } else {
                Err(invalid_array_index_error())
            }
        }
    }
}

/// Read arms for the properties every object type has: OBJECT_IDENTIFIER,
/// OBJECT_NAME, DESCRIPTION and PROPERTY_LIST. Any other property yields
/// `None`, leaving it to the caller's own arms and unknown-property fallback.
///
/// The caller must provide `self` which has fields: `oid`, `name`,
/// `description`.
macro_rules! read_identity_properties {
    ($self:expr_2021, $property:expr_2021, $array_index:expr_2021) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::OBJECT_IDENTIFIER => Some(Ok(
                bacnet_types::primitives::PropertyValue::ObjectIdentifier($self.oid),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::OBJECT_NAME => Some(Ok(
                bacnet_types::primitives::PropertyValue::CharacterString($self.name.clone()),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::DESCRIPTION => Some(Ok(
                bacnet_types::primitives::PropertyValue::CharacterString($self.description.clone()),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST => {
                let props = $self.property_list();
                Some($crate::common::read_property_list_property(
                    &props,
                    $array_index,
                ))
            }
            _ => None,
        }
    };
}
use crate::prelude::*;
pub(crate) use read_identity_properties;

/// Common read_property match arms shared by most object types.
///
/// Handles: the [`read_identity_properties!`] rows plus STATUS_FLAGS,
///          OUT_OF_SERVICE and RELIABILITY. Any other property yields `None`.
///
/// The caller must provide `self` which has fields: `oid`, `name`,
/// `description`, `status_flags`, `out_of_service`, `reliability`.
///
/// The `no_out_of_service` form is for object types whose property table has
/// Status_Flags and Reliability but no Out_Of_Service (#1064). It needs no
/// `out_of_service` field, leaves OUT_OF_SERVICE to the caller's
/// unknown-property fallback, and holds the OUT_OF_SERVICE flag FALSE.
macro_rules! read_common_properties {
    (@status $self:expr_2021, $property:expr_2021, $array_index:expr_2021, $out_of_service:expr_2021) => {{
        let out_of_service: Option<bool> = $out_of_service;
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::STATUS_FLAGS => {
                // Compute StatusFlags dynamically. Objects with event detection
                // should handle STATUS_FLAGS before calling this macro to include
                // IN_ALARM from their event_state; this default uses NORMAL.
                Some(Ok($crate::common::compute_status_flags(
                    $self.status_flags,
                    $self.reliability,
                    out_of_service.unwrap_or(false),
                    // default: no IN_ALARM (non-event objects)
                    bacnet_types::enums::EventState::NORMAL,
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE => out_of_service
                .map(|value| Ok(bacnet_types::primitives::PropertyValue::Boolean(value))),
            p if p == bacnet_types::enums::PropertyIdentifier::RELIABILITY => Some(Ok(
                bacnet_types::primitives::PropertyValue::Enumerated($self.reliability.to_raw()),
            )),
            _ => $crate::common::read_identity_properties!($self, $property, $array_index),
        }
    }};
    ($self:expr_2021, $property:expr_2021, $array_index:expr_2021, no_out_of_service) => {
        $crate::common::read_common_properties!(@status $self, $property, $array_index, None)
    };
    ($self:expr_2021, $property:expr_2021, $array_index:expr_2021) => {
        $crate::common::read_common_properties!(
            @status $self,
            $property,
            $array_index,
            Some($self.out_of_service)
        )
    };
}
pub(crate) use read_common_properties;

/// Return the unknown-property protocol error.
#[inline]
pub(crate) fn unknown_property_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
    )
}

/// Handle writing the OUT_OF_SERVICE property.
/// NULL relinquishment succeeds unchanged; it does not store NULL.
///
/// Returns `Some(Ok(()))` if the property was OUT_OF_SERVICE and successfully handled,
/// `Some(Err(...))` if the property was OUT_OF_SERVICE but the wrong type was provided,
/// or `None` if the property is not OUT_OF_SERVICE.
#[inline]
pub(crate) fn write_out_of_service(
    out_of_service: &mut bool,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property == bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE {
        if let bacnet_types::primitives::PropertyValue::Boolean(v) = value {
            *out_of_service = *v;
            Some(Ok(()))
        } else if matches!(value, bacnet_types::primitives::PropertyValue::Null) {
            Some(Ok(()))
        } else {
            Some(Err(invalid_data_type_error()))
        }
    } else {
        None
    }
}

/// Handle writing OUT_OF_SERVICE for objects that temporarily transfer
/// Reliability ownership to a client simulation.
///
/// The evaluated value is saved on the FALSE-to-TRUE edge and restored directly
/// on the TRUE-to-FALSE edge. If the entry edge was not observed, the restore
/// falls back to NO_FAULT_DETECTED. NULL relinquishment preserves all three
/// fields, including the saved value and its restoration ownership.
#[inline]
pub(crate) fn write_out_of_service_with_reliability_restore(
    out_of_service: &mut bool,
    reliability: &mut bacnet_types::enums::Reliability,
    saved_reliability: &mut Option<bacnet_types::enums::Reliability>,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    write_out_of_service_with_restore(
        out_of_service,
        reliability,
        saved_reliability,
        Some(bacnet_types::enums::Reliability::NO_FAULT_DETECTED),
        property,
        value,
    )
}

/// Handle writing OUT_OF_SERVICE for an object that serves a client's
/// simulated values while it is out of service.
///
/// On the FALSE-to-TRUE edge the object's own `served` value is copied into
/// `set_aside`. On the TRUE-to-FALSE edge `served` takes back the value set
/// aside, or `fallback` when the entry edge was not observed; with neither,
/// it keeps what it holds. NULL relinquishment changes none of the fields.
/// `None` for any property other than OUT_OF_SERVICE.
#[inline]
pub(crate) fn write_out_of_service_with_restore<T: Clone>(
    out_of_service: &mut bool,
    served: &mut T,
    set_aside: &mut Option<T>,
    fallback: Option<T>,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property != bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE {
        return None;
    }
    Some(match value {
        bacnet_types::primitives::PropertyValue::Boolean(v) => {
            match (*out_of_service, *v) {
                (false, true) => *set_aside = Some(served.clone()),
                (true, false) => {
                    if let Some(own) = set_aside.take().or(fallback) {
                        *served = own;
                    }
                }
                _ => {}
            }
            *out_of_service = *v;
            Ok(())
        }
        bacnet_types::primitives::PropertyValue::Null => Ok(()),
        _ => Err(invalid_data_type_error()),
    })
}

pub(crate) use crate::reliability_inhibit::ReliabilityInhibitState;

/// Handle writing the DESCRIPTION property. NULL relinquishment succeeds
/// without changing the stored string.
///
/// Returns `Some(Ok(()))` if the property was DESCRIPTION and successfully handled,
/// `Some(Err(...))` if the property was DESCRIPTION but the wrong type was provided,
/// or `None` if the property is not DESCRIPTION.
#[inline]
pub(crate) fn write_description(
    description: &mut String,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property == bacnet_types::enums::PropertyIdentifier::DESCRIPTION {
        Some(match value {
            bacnet_types::primitives::PropertyValue::CharacterString(s) => {
                *description = s.clone();
                Ok(())
            }
            bacnet_types::primitives::PropertyValue::Null => Ok(()),
            _ => Err(invalid_data_type_error()),
        })
    } else {
        None
    }
}

/// Write the OBJECT_NAME property.
///
/// Validates type and non-empty. Uniqueness must be checked by the caller
/// (ObjectDatabase) before calling this.
pub(crate) fn write_object_name(
    name: &mut String,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property == bacnet_types::enums::PropertyIdentifier::OBJECT_NAME {
        if let bacnet_types::primitives::PropertyValue::CharacterString(s) = value {
            if s.is_empty() {
                Some(Err(value_out_of_range_error()))
            } else {
                *name = s.clone();
                Some(Ok(()))
            }
        } else {
            Some(Err(invalid_data_type_error()))
        }
    } else {
        None
    }
}

/// Return the write-access-denied protocol error.
#[inline]
pub(crate) fn write_access_denied_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED,
    )
}

/// Return the optional-functionality-not-supported protocol error.
#[inline]
pub(crate) fn optional_functionality_not_supported_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::OBJECT,
        bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    )
}

/// Return the invalid-data-type protocol error.
#[inline]
pub(crate) fn invalid_data_type_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
    )
}

/// Return the value-out-of-range protocol error.
#[inline]
pub(crate) fn value_out_of_range_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
    )
}

mod command_write;
mod encoded_elements;
mod list_element;
pub(crate) use command_write::decode_command_write;
pub(crate) use encoded_elements::{chunks, decode_element, decode_elements, decode_single_element};
#[cfg(test)]
pub(crate) use list_element::assert_list_element_refused;
pub(crate) use list_element::at_list_element;

/// Return the invalid-data-encoding protocol error.
///
/// Clause 15.9.1.3 covers an encoding incompatible with the property's
/// datatype — the value is of the right BACnet datatype but its declared
/// shape does not match the property's production.
#[inline]
pub(crate) fn invalid_data_encoding_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::INVALID_DATA_ENCODING,
    )
}

/// Validate a wire `BitString` value against a fixed-width bit-string
/// production and return its single content octet (MSB-first).
///
/// A production with `named_bits` defined bits (e.g.
/// BACnetEventTransitionBits = 3, BACnetLimitEnable = 2, Clause 21) has
/// exactly one canonical shape: one content octet carrying the bits in its
/// high positions with `8 - named_bits` declared unused — the same form the
/// read path emits. A write declaring any other shape (full-octet bit string,
/// extra content octets, no content, a different unused-bit count) is refused
/// PROPERTY / INVALID_DATA_ENCODING rather than silently masked and
/// normalized. Objects-layer sibling of the decoding-layer checks in
/// `bacnet-encoding` (e.g. `check_fixed_bit_string`); kept here because this
/// layer reports protocol errors, not decoding errors.
#[inline]
pub(crate) fn check_fixed_width_bit_string(
    unused_bits: u8,
    data: &[u8],
    named_bits: u8,
) -> Result<u8, bacnet_types::error::Error> {
    if data.len() == 1 && unused_bits == 8 - named_bits {
        Ok(data[0])
    } else {
        Err(invalid_data_encoding_error())
    }
}

/// Return whether a BACnetReliability value is defined by ASHRAE or lies in
/// the vendor-proprietary range.
///
/// The named set is derived from `Reliability::ALL_NAMED` so the predicate
/// tracks the enum: when an addendum value lands as a constant in
/// `bacnet_types::enums::Reliability`, this write-path gate accepts it with no
/// second edit. The production's gaps stay explicit here: 11 is reserved for
/// a future addendum, 26..=63 are reserved for ASHRAE, and 64..=65535 is the
/// vendor-proprietary range (Clause 21 BACnetReliability).
#[inline]
pub(crate) fn is_reliability_value_valid(value: bacnet_types::enums::Reliability) -> bool {
    bacnet_types::enums::Reliability::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == value)
        || (64..=65_535).contains(&value.to_raw())
}

/// Return the invalid-array-index protocol error.
#[inline]
pub(crate) fn invalid_array_index_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::INVALID_ARRAY_INDEX,
    )
}

/// Read a BACnetARRAY held as its element values: the whole array with no
/// index, its size at index 0, or the element at a one-based index, which
/// fails with INVALID_ARRAY_INDEX past the end.
pub(crate) fn read_array(
    values: Vec<bacnet_types::primitives::PropertyValue>,
    array_index: Option<u32>,
) -> Result<bacnet_types::primitives::PropertyValue, bacnet_types::error::Error> {
    use bacnet_types::primitives::PropertyValue;

    match array_index {
        None => Ok(PropertyValue::List(values)),
        Some(0) => Ok(PropertyValue::Unsigned(values.len() as u64)),
        Some(index) => values
            .into_iter()
            .nth((index - 1) as usize)
            .ok_or_else(invalid_array_index_error),
    }
}

/// Return the property-is-not-an-array protocol error.
///
/// Clause 15.5.1.3 / 15.9.1.3: the request named an array index for a
/// property that isn't a BACnetARRAY. The RP/RPM/WP/WPM service handlers
/// gate on [`crate::traits::BACnetObject::is_array_property`]; object arms
/// mirror the classification for direct (non-service) calls.
#[inline]
pub(crate) fn property_is_not_an_array_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    )
}

/// Reject NaN and Infinity float values. Returns `Err(VALUE_OUT_OF_RANGE)` if not finite.
#[inline]
pub(crate) fn reject_non_finite(v: f32) -> Result<(), bacnet_types::error::Error> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(value_out_of_range_error())
    }
}

/// Convert a u64 BACnet Unsigned to u32, rejecting values that exceed u32::MAX.
#[inline]
pub(crate) fn u64_to_u32(v: u64) -> Result<u32, bacnet_types::error::Error> {
    u32::try_from(v).map_err(|_| value_out_of_range_error())
}

/// Recalculate present value from a 16-level priority array.
///
/// Picks the highest-priority (lowest index) non-None value, or falls
/// back to the relinquish default.
#[inline]
pub(crate) fn recalculate_from_priority_array<T: Copy>(
    priority_array: &[Option<T>; 16],
    relinquish_default: T,
) -> T {
    priority_array
        .iter()
        .flatten()
        .next()
        .copied()
        .unwrap_or(relinquish_default)
}

/// Compute the Current_Command_Priority property value.
///
/// Returns the 1-based index of the active priority array slot, or
/// Null if the relinquish default is in use.
pub(crate) fn current_command_priority<T>(
    priority_array: &[Option<T>; 16],
) -> bacnet_types::primitives::PropertyValue {
    for (i, slot) in priority_array.iter().enumerate() {
        if slot.is_some() {
            return bacnet_types::primitives::PropertyValue::Unsigned((i + 1) as u64);
        }
    }
    bacnet_types::primitives::PropertyValue::Null
}

/// Generic intrinsic-reporting read properties shared by every event detector.
macro_rules! read_generic_event_properties {
    ($self:expr_2021, $property:expr_2021) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_STATE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    $self.event_detector.event_state.to_raw(),
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![$self.event_detector.event_enable.to_bacnet()],
                }))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    $self.event_detector.notify_type.to_raw(),
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::NOTIFICATION_CLASS => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Unsigned(
                    $self.event_detector.notification_class as u64,
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::TIME_DELAY => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Unsigned(
                    $self.event_detector.time_delay as u64,
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::TIME_DELAY_NORMAL => {
                // Clause 13.3 supplies pTimeDelay as the fallback when
                // pTimeDelayNormal is absent —
                // so the read-back of an unwritten Time_Delay_Normal is
                // Time_Delay's value, matching the algorithm's behavior.
                Some(Ok(bacnet_types::primitives::PropertyValue::Unsigned(
                    $self
                        .event_detector
                        .time_delay_normal
                        .unwrap_or($self.event_detector.time_delay) as u64,
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::ACKED_TRANSITIONS => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![$self.event_detector.acked_transitions.to_bacnet()],
                }))
            }
            _ => None,
        }
    };
}
pub(crate) use read_generic_event_properties;

/// Analog-only intrinsic-reporting read properties for `OutOfRangeDetector`.
///
/// Event timestamps and message texts are served by `EventHistory::read`,
/// invoked at every analog call site immediately after this macro.
macro_rules! read_analog_event_properties {
    ($self:expr_2021, $property:expr_2021) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::HIGH_LIMIT => Some(Ok(
                bacnet_types::primitives::PropertyValue::Real($self.event_detector.high_limit),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::LOW_LIMIT => Some(Ok(
                bacnet_types::primitives::PropertyValue::Real($self.event_detector.low_limit),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::DEADBAND => Some(Ok(
                bacnet_types::primitives::PropertyValue::Real($self.event_detector.deadband),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::LIMIT_ENABLE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 6,
                    data: vec![$self.event_detector.limit_enable.to_bacnet()],
                }))
            }
            _ => None,
        }
    };
}
pub(crate) use read_analog_event_properties;

/// Analog-only intrinsic-reporting write_property arms, for objects whose event_detector is
/// an `OutOfRangeDetector`.
///
/// Handles: HIGH_LIMIT, LOW_LIMIT, DEADBAND, LIMIT_ENABLE.
///
/// This is the analog half of the split. The properties every detector carries —
/// EVENT_ENABLE, NOTIFICATION_CLASS, NOTIFY_TYPE, TIME_DELAY, TIME_DELAY_NORMAL
/// and the ACKED_TRANSITIONS denial — live in [`write_generic_event_properties!`],
/// and a call site that needs both must invoke both.
///
/// Returns `Some(Ok(()))` if the property was handled,
/// `Some(Err(...))` for type/validation errors,
/// or `None` if the property is not an event property.
macro_rules! write_analog_event_properties {
    ($self:expr_2021, $property:expr_2021, $value:expr_2021) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::HIGH_LIMIT => {
                if let bacnet_types::primitives::PropertyValue::Real(v) = $value {
                    if let Err(e) = $crate::common::reject_non_finite(v) {
                        Some(Err(e))
                    } else {
                        $self.event_detector.high_limit = v;
                        Some(Ok(()))
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::LOW_LIMIT => {
                if let bacnet_types::primitives::PropertyValue::Real(v) = $value {
                    if let Err(e) = $crate::common::reject_non_finite(v) {
                        Some(Err(e))
                    } else {
                        $self.event_detector.low_limit = v;
                        Some(Ok(()))
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::DEADBAND => {
                if let bacnet_types::primitives::PropertyValue::Real(v) = $value {
                    if v < 0.0 || !v.is_finite() {
                        Some(Err($crate::common::value_out_of_range_error()))
                    } else {
                        $self.event_detector.deadband = v;
                        Some(Ok(()))
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::LIMIT_ENABLE => {
                // BACnetLimitEnable is a 2-bit production (Clause 21): the
                // written BitString must declare its canonical shape.
                if let bacnet_types::primitives::PropertyValue::BitString { unused_bits, data } =
                    &$value
                {
                    match $crate::common::check_fixed_width_bit_string(*unused_bits, data, 2) {
                        Ok(byte) => {
                            $self.event_detector.limit_enable =
                                bacnet_types::bitstring::LimitEnable::from_bacnet(&[byte]);
                            Some(Ok(()))
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            _ => None,
        }
    };
}
pub(crate) use write_analog_event_properties;

/// Generic intrinsic-reporting write properties shared by every event detector.
macro_rules! write_generic_event_properties {
    ($self:expr_2021, $property:expr_2021, $value:expr_2021) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE => {
                // BACnetEventTransitionBits is a 3-bit production (Clause 21):
                // the written BitString must declare its canonical shape.
                if let bacnet_types::primitives::PropertyValue::BitString { unused_bits, data } =
                    &$value
                {
                    match $crate::common::check_fixed_width_bit_string(*unused_bits, data, 3) {
                        Ok(byte) => {
                            $self.event_detector.event_enable =
                                bacnet_types::bitstring::EventTransitionBits::from_bacnet(&[byte]);
                            Some(Ok(()))
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::NOTIFICATION_CLASS => {
                if let bacnet_types::primitives::PropertyValue::Unsigned(v) = $value {
                    match $crate::common::u64_to_u32(v) {
                        Ok(v32) => {
                            $self.event_detector.notification_class = v32;
                            Some(Ok(()))
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE => {
                // BACnetNotifyType is a closed three-value production
                // {alarm, event, ack-notification} (Clause 21); membership is
                // derived from NotifyType::ALL_NAMED so a future addendum
                // constant widens the gate without a second edit.
                if let bacnet_types::primitives::PropertyValue::Enumerated(v) = $value {
                    let notify_type = bacnet_types::enums::NotifyType::from_raw(v);
                    let named = bacnet_types::enums::NotifyType::ALL_NAMED
                        .iter()
                        .any(|&(_, n)| n == notify_type);
                    if !named {
                        Some(Err($crate::common::value_out_of_range_error()))
                    } else {
                        $self.event_detector.notify_type = notify_type;
                        Some(Ok(()))
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::TIME_DELAY => {
                if let bacnet_types::primitives::PropertyValue::Unsigned(v) = $value {
                    match $crate::common::u64_to_u32(v) {
                        Ok(v32) => {
                            $self.event_detector.time_delay = v32;
                            Some(Ok(()))
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::TIME_DELAY_NORMAL => {
                if let bacnet_types::primitives::PropertyValue::Unsigned(v) = $value {
                    match $crate::common::u64_to_u32(v) {
                        Ok(v32) => {
                            $self.event_detector.time_delay_normal = Some(v32);
                            Some(Ok(()))
                        }
                        Err(e) => Some(Err(e)),
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::ACKED_TRANSITIONS => {
                // Read-only: maintained by the alarm-acknowledgment process
                // (Clause 13.2.3) from event-state transitions and
                // acknowledgment indications — the latter arriving from
                // AcknowledgeAlarm or a local means — never by property write.
                //
                // This denial predates the generic/analog split and must survive it. The
                // service path (`BACnetObject::acknowledge_alarm`) deliberately ORs the
                // acknowledged bit in; a property write would assign, so it could both
                // fabricate an acknowledgment and erase one. GetAlarmSummary and
                // GetEventInformation read this field straight off the object, so an
                // assignable arm would let a client mark an unacknowledged alarm
                // acknowledged with a plain WriteProperty.
                //
                // It also carries the Clause 12.7 / 12.19 invariant that while
                // Event_Detection_Enable is FALSE, Acked_Transitions must retain its
                // initial value — an ungated write arm is the one route that
                // could break that between detection-enable writes.
                Some(Err($crate::common::write_access_denied_error()))
            }
            _ => None,
        }
    };
}
pub(crate) use write_generic_event_properties;

/// Read a priority array property (handles array_index=None, Some(0), Some(1..=16)).
///
/// `$wrap` is a closure/function that converts `T` into a `PropertyValue`.
macro_rules! read_priority_array {
    ($self:expr_2021, $array_index:expr_2021, $wrap:expr_2021) => {{
        let wrap_fn = $wrap;
        match $array_index {
            None => {
                let elements = $self
                    .priority_array
                    .iter()
                    .map(|slot| match slot {
                        Some(v) => wrap_fn(*v),
                        None => bacnet_types::primitives::PropertyValue::Null,
                    })
                    .collect();
                Ok(bacnet_types::primitives::PropertyValue::List(elements))
            }
            Some(0) => Ok(bacnet_types::primitives::PropertyValue::Unsigned(16)),
            Some(idx) if (1..=16).contains(&idx) => {
                match $self.priority_array[(idx - 1) as usize] {
                    Some(v) => Ok(wrap_fn(v)),
                    None => Ok(bacnet_types::primitives::PropertyValue::Null),
                }
            }
            _ => Err($crate::common::invalid_array_index_error()),
        }
    }};
}
pub(crate) use read_priority_array;

/// Write COV_INCREMENT with non-negative validation.
///
/// Returns `Some(Ok(()))` if handled, `Some(Err(...))` for type/range errors,
/// or `None` if property is not COV_INCREMENT.
#[inline]
pub(crate) fn write_cov_increment(
    cov_increment: &mut f32,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property == bacnet_types::enums::PropertyIdentifier::COV_INCREMENT {
        if let bacnet_types::primitives::PropertyValue::Real(v) = value {
            if *v < 0.0 || !v.is_finite() {
                Some(Err(value_out_of_range_error()))
            } else {
                *cov_increment = *v;
                Some(Ok(()))
            }
        } else {
            Some(Err(invalid_data_type_error()))
        }
    } else {
        None
    }
}

// ──────────────────────────────────────────────────────────────────────────
// PICS writability helpers
// ──────────────────────────────────────────────────────────────────────────
//
// Both predicates are test-only. Property-metadata tests use
// `is_common_writable` to check the core object types against their
// out-of-service, name and description write arms, and the commandable
// predicate to check commandable objects against their
// `write_priority_array!` arms.

/// Writable commandable-object properties shared by all commandable types
/// (AnalogOutput, AnalogValue, BinaryOutput, BinaryValue, MultiStateOutput,
/// MultiStateValue): commandable `PRESENT_VALUE` writes and the validated
/// `RELINQUISH_DEFAULT` write arm
/// (#270 — the standard permits Relinquish_Default to be writable; the
/// conformance tables carry it R or O, and the writability implemented here
/// is permitted, not required).
///
/// `PRIORITY_ARRAY` and derived `CURRENT_COMMAND_PRIORITY` stay read-only.
#[cfg(test)]
#[inline]
pub(crate) fn is_commandable_property_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    matches!(
        property,
        bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE
            | bacnet_types::enums::PropertyIdentifier::RELINQUISH_DEFAULT
    )
}

/// Writable common properties shared by all core I/O/V object types (accepted
/// via `write_out_of_service` or
/// `write_out_of_service_with_reliability_restore`, plus `write_object_name`
/// and `write_description`).
#[cfg(test)]
#[inline]
pub(crate) fn is_common_writable(property: bacnet_types::enums::PropertyIdentifier) -> bool {
    matches!(
        property,
        bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE
            | bacnet_types::enums::PropertyIdentifier::OBJECT_NAME
            | bacnet_types::enums::PropertyIdentifier::DESCRIPTION
    )
}
