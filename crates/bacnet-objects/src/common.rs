//! Macros and helpers shared across object types to reduce duplication.
//!
//! These macros extract the common read/write property arms and error
//! construction patterns that are identical across analog, binary, and
//! multi-state object implementations.

/// Compute StatusFlags with all four bits dynamically set.
///
/// IN_ALARM: TRUE when event_state != NORMAL (0).
/// FAULT: TRUE when reliability != NO_FAULT_DETECTED (0).
/// OUT_OF_SERVICE: from the object's out_of_service flag.
/// OVERRIDDEN: always FALSE for software-only (callers can set in base_flags).
pub fn compute_status_flags(
    base_flags: bacnet_types::primitives::StatusFlags,
    reliability: u32,
    out_of_service: bool,
    event_state: u32,
) -> bacnet_types::primitives::PropertyValue {
    let mut flags = base_flags;
    if event_state != 0 {
        flags |= bacnet_types::primitives::StatusFlags::IN_ALARM;
    } else {
        flags -= bacnet_types::primitives::StatusFlags::IN_ALARM;
    }
    if reliability != 0 {
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
    bacnet_types::error::Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
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

/// Common read_property match arms shared by all object types.
///
/// Handles: OBJECT_IDENTIFIER, OBJECT_NAME, DESCRIPTION, STATUS_FLAGS,
///          OUT_OF_SERVICE, RELIABILITY, PROPERTY_LIST, and the
///          unknown-property fallback.
///
/// The caller must provide `self` which has fields: `oid`, `name`,
/// `description`, `status_flags`, `out_of_service`, `reliability`.
macro_rules! read_common_properties {
    ($self:expr, $property:expr, $array_index:expr) => {
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
            p if p == bacnet_types::enums::PropertyIdentifier::STATUS_FLAGS => {
                // Compute StatusFlags dynamically. Objects with event detection
                // should handle STATUS_FLAGS before calling this macro to include
                // IN_ALARM from their event_state; this default uses event_state=0.
                Some(Ok(common::compute_status_flags(
                    $self.status_flags,
                    $self.reliability,
                    $self.out_of_service,
                    0, // default: no IN_ALARM (non-event objects)
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE => Some(Ok(
                bacnet_types::primitives::PropertyValue::Boolean($self.out_of_service),
            )),
            p if p == bacnet_types::enums::PropertyIdentifier::RELIABILITY => Some(Ok(
                bacnet_types::primitives::PropertyValue::Enumerated($self.reliability),
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
        } else {
            Some(Err(protocol_error(
                bacnet_types::enums::ErrorClass::PROPERTY,
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            )))
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
/// falls back to NO_FAULT_DETECTED.
#[inline]
pub(crate) fn write_out_of_service_with_reliability_restore(
    out_of_service: &mut bool,
    reliability: &mut u32,
    saved_reliability: &mut Option<u32>,
    property: bacnet_types::enums::PropertyIdentifier,
    value: &bacnet_types::primitives::PropertyValue,
) -> Option<Result<(), bacnet_types::error::Error>> {
    if property == bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE {
        if let bacnet_types::primitives::PropertyValue::Boolean(v) = value {
            if !*out_of_service && *v {
                *saved_reliability = Some(*reliability);
            } else if *out_of_service && !*v {
                *reliability = saved_reliability
                    .take()
                    .unwrap_or(bacnet_types::enums::Reliability::NO_FAULT_DETECTED.to_raw());
            }
            *out_of_service = *v;
            Some(Ok(()))
        } else {
            Some(Err(protocol_error(
                bacnet_types::enums::ErrorClass::PROPERTY,
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
            )))
        }
    } else {
        None
    }
}

/// Handle writing the DESCRIPTION property.
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
        if let bacnet_types::primitives::PropertyValue::CharacterString(s) = value {
            *description = s.clone();
            Some(Ok(()))
        } else {
            Some(Err(invalid_data_type_error()))
        }
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

/// Return the invalid-data-encoding protocol error.
///
/// Clause 15.9.1.3: "The encoding is not valid for the datatype of the
/// property" — the value is of the right BACnet datatype but its declared
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

/// Return whether a raw BACnetReliability value is defined by ASHRAE or lies
/// in the vendor-proprietary range.
///
/// The named set is derived from `Reliability::ALL_NAMED` so the predicate
/// tracks the enum: when an addendum value lands as a constant in
/// `bacnet_types::enums::Reliability`, this write-path gate accepts it with no
/// second edit. The production's gaps stay explicit here: 11 is reserved for
/// a future addendum, 26..=63 are reserved for ASHRAE, and 64..=65535 is the
/// vendor-proprietary range (Clause 21 BACnetReliability).
#[inline]
pub(crate) fn is_reliability_value_valid(value: u32) -> bool {
    bacnet_types::enums::Reliability::ALL_NAMED
        .iter()
        .any(|&(_, named)| named.to_raw() == value)
        || (64..=65_535).contains(&value)
}

/// Return the invalid-array-index protocol error.
#[inline]
pub(crate) fn invalid_array_index_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::INVALID_ARRAY_INDEX,
    )
}

/// Return the property-is-not-an-array protocol error.
///
/// Clause 15.5.1.3 / 15.9.1.3: an array index was provided but the property
/// is not an array. The RP/RPM/WP/WPM service handlers gate on
/// [`crate::traits::BACnetObject::is_array_property`]; object arms mirror the
/// classification for direct (non-service) calls.
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

/// Value source tracking for commandable objects.
///
/// Stores the source that last wrote to each priority array slot.
#[derive(Debug, Clone)]
pub struct ValueSourceTracking {
    /// Value_Source: the source of the current present_value.
    /// Null if no command is active (relinquish default).
    pub value_source: bacnet_types::primitives::PropertyValue,
    /// Value_Source_Array[16]: source per priority slot.
    #[allow(dead_code)]
    pub value_source_array: [bacnet_types::primitives::PropertyValue; 16],
    /// Last_Command_Time: timestamp of the last write.
    pub last_command_time: bacnet_types::primitives::BACnetTimeStamp,
    /// Command_Time_Array[16]: timestamp per priority slot.
    #[allow(dead_code)]
    pub command_time_array: [bacnet_types::primitives::BACnetTimeStamp; 16],
}

impl Default for ValueSourceTracking {
    fn default() -> Self {
        Self {
            value_source: bacnet_types::primitives::PropertyValue::Null,
            value_source_array: std::array::from_fn(|_| {
                bacnet_types::primitives::PropertyValue::Null
            }),
            last_command_time: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
            command_time_array: std::array::from_fn(|_| {
                bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0)
            }),
        }
    }
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
    ($self:expr, $property:expr) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_STATE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    $self.event_detector.event_state.to_raw(),
                )))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![bacnet_types::bitstring::pack_octet(
                        $self.event_detector.event_enable,
                    )],
                }))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    $self.event_detector.notify_type,
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
                // Clause 13.3: "If no value is available for this parameter,
                // then it takes on the value of the pTimeDelay parameter" —
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
                    data: vec![bacnet_types::bitstring::pack_octet(
                        $self.event_detector.acked_transitions,
                    )],
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
    ($self:expr, $property:expr) => {
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
                    data: vec![$self.event_detector.limit_enable.to_bits()],
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
    ($self:expr, $property:expr, $value:expr) => {
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
                                $crate::event::LimitEnable::from_bits(byte);
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
    ($self:expr, $property:expr, $value:expr) => {
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
                                bacnet_types::bitstring::unpack_octet(&[byte], 3);
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
                    let named = bacnet_types::enums::NotifyType::ALL_NAMED
                        .iter()
                        .any(|&(_, n)| n.to_raw() == v);
                    if !named {
                        Some(Err($crate::common::value_out_of_range_error()))
                    } else {
                        $self.event_detector.notify_type = v;
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
                // Event_Detection_Enable is FALSE, Acked_Transitions "shall be equal to
                // [its] initial condition" — an ungated write arm is the one route that
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
    ($self:expr, $array_index:expr, $wrap:expr) => {{
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

/// Validate priority index and write to a priority array slot.
///
/// Handles priority validation, Null (relinquish), and delegates value
/// extraction/validation to the caller's `$extract` block.
///
/// `$extract` receives the `value` and must return `Result<T, Error>`.
/// After a successful write, calls `$self.recalculate_present_value()`.
macro_rules! write_priority_array {
    ($self:expr, $value:expr, $priority:expr, $extract:expr) => {{
        let prio = $priority.unwrap_or(16);
        if !(1..=16).contains(&prio) {
            return Err($crate::common::value_out_of_range_error());
        }
        let idx = (prio - 1) as usize;
        match $value {
            bacnet_types::primitives::PropertyValue::Null => {
                $self.priority_array[idx] = None;
            }
            other => {
                let extracted = ($extract)(other)?;
                $self.priority_array[idx] = Some(extracted);
            }
        }
        $self.recalculate_present_value();
        Ok(())
    }};
}
pub(crate) use write_priority_array;

/// Handle direct writes to PRIORITY_ARRAY[index].
///
/// If `property` is PRIORITY_ARRAY and `array_index` is Some(1..=16),
/// writes to that priority slot. Null relinquishes; otherwise `$extract`
/// converts the value. Calls `recalculate_present_value()` after write.
///
/// Index validation follows Clause 12.1.5.1: an out-of-range index is
/// PROPERTY / INVALID_ARRAY_INDEX; an omitted index means whole-array
/// access, and whole-array writes are not supported on commandable objects,
/// so it is PROPERTY / WRITE_ACCESS_DENIED — a protocol error that the
/// service layer can return as Result(-) (Clause 15.9.1.3).
///
/// Returns early with `Ok(())` or `Err(...)` if the property is PRIORITY_ARRAY.
/// Falls through (does nothing) if the property is not PRIORITY_ARRAY.
macro_rules! write_priority_array_direct {
    ($self:expr, $property:expr, $array_index:expr, $value:expr, $extract:expr) => {
        if $property == bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY {
            let idx = match $array_index {
                Some(n) if (1..=16).contains(&n) => (n - 1) as usize,
                Some(_) => return Err($crate::common::invalid_array_index_error()),
                None => return Err($crate::common::write_access_denied_error()),
            };
            match $value {
                bacnet_types::primitives::PropertyValue::Null => {
                    $self.priority_array[idx] = None;
                }
                other => {
                    let extracted = ($extract)(other)?;
                    $self.priority_array[idx] = Some(extracted);
                }
            }
            $self.recalculate_present_value();
            return Ok(());
        }
    };
}
pub(crate) use write_priority_array_direct;

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
// Shared property-set predicates used by the `is_writable_property` overrides
// on the core object types. Each predicate mirrors the arms of the matching
// `write_property` implementation (via the `write_generic_event_properties!` and
// `write_analog_event_properties!` macros and
// the `write_priority_array!` / `write_priority_array_direct!` macros) so PICS
// and runtime dispatch share one truth source. Keep these in lock-step with
// the macros below.

/// Generic writable event-detection properties shared by every detector.
///
/// `TIME_DELAY_NORMAL` mirrors `TIME_DELAY`: every Clause 12 conformance
/// table carries both as O-coded (present-only-if-intrinsic-reporting), so
/// writability is permitted rather than required — and accepting the write
/// is what makes the Clause 13.3 delay asymmetry commissionable at all.
#[inline]
pub(crate) fn is_generic_event_property_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    matches!(
        property,
        bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE
            | bacnet_types::enums::PropertyIdentifier::NOTIFICATION_CLASS
            | bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE
            | bacnet_types::enums::PropertyIdentifier::TIME_DELAY
            | bacnet_types::enums::PropertyIdentifier::TIME_DELAY_NORMAL
    )
    // ACKED_TRANSITIONS is deliberately absent: the generic write arm denies it, and this
    // predicate is what PICS reports, so listing it would advertise a write dispatch rejects.
}

/// Writable generic and analog event properties exposed by analog objects.
#[inline]
pub(crate) fn is_event_property_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    is_generic_event_property_writable(property)
        || matches!(
            property,
            bacnet_types::enums::PropertyIdentifier::HIGH_LIMIT
                | bacnet_types::enums::PropertyIdentifier::LOW_LIMIT
                | bacnet_types::enums::PropertyIdentifier::DEADBAND
                | bacnet_types::enums::PropertyIdentifier::LIMIT_ENABLE
        )
}

/// Writable commandable-object properties shared by all commandable types
/// (AnalogOutput, AnalogValue, BinaryOutput, BinaryValue, MultiStateOutput,
/// MultiStateValue): `PRIORITY_ARRAY` direct writes, commandable
/// `PRESENT_VALUE` writes, and the validated `RELINQUISH_DEFAULT` write arm
/// (#270 — the standard permits Relinquish_Default to be writable; the
/// conformance tables carry it R or O, and the writability implemented here
/// is permitted, not required).
///
/// `CURRENT_COMMAND_PRIORITY` stays read-only: it is derived from the
/// priority array, so no `write_property` arm accepts it.
#[inline]
pub(crate) fn is_commandable_property_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    matches!(
        property,
        bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY
            | bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE
            | bacnet_types::enums::PropertyIdentifier::RELINQUISH_DEFAULT
    )
}

/// Writable common properties shared by all core I/O/V object types (accepted
/// via `write_out_of_service` or
/// `write_out_of_service_with_reliability_restore`, plus `write_object_name`
/// and `write_description`).
#[inline]
pub(crate) fn is_common_writable(property: bacnet_types::enums::PropertyIdentifier) -> bool {
    matches!(
        property,
        bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE
            | bacnet_types::enums::PropertyIdentifier::OBJECT_NAME
            | bacnet_types::enums::PropertyIdentifier::DESCRIPTION
    )
}

/// Writable properties for commandable Multi-State objects (MSO, MSV):
/// commandable (PRIORITY_ARRAY + PRESENT_VALUE) + common + STATE_TEXT.
/// Mirrors the `write_property` arms of MultiStateOutput/Value.
#[inline]
pub(crate) fn is_multistate_commandable_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    is_commandable_property_writable(property)
        || is_common_writable(property)
        || property == bacnet_types::enums::PropertyIdentifier::STATE_TEXT
}

/// Writable properties for Multi-State Input (MSI): PRESENT_VALUE (when out
/// of service) + common + STATE_TEXT. Mirrors the `write_property` arms of
/// MultiStateInput (commandable `PRESENT_VALUE` is not accepted — inputs are
/// not commandable — so this excludes `is_commandable_property_writable`).
#[inline]
pub(crate) fn is_multistate_input_writable(
    property: bacnet_types::enums::PropertyIdentifier,
) -> bool {
    is_common_writable(property)
        || property == bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE
        || property == bacnet_types::enums::PropertyIdentifier::STATE_TEXT
}
