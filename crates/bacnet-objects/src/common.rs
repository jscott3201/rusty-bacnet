//! Macros and helpers shared across object types to reduce duplication.
//!
//! These macros extract the common read/write property arms and error
//! construction patterns that are identical across analog, binary, and
//! multi-state object implementations.

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
///
/// Per Clause 12.1.1.4.1, Object_Name, Object_Type, Object_Identifier, and
/// Property_List itself are NOT included in the returned list.
pub fn read_property_list_property(
    props: &[bacnet_types::enums::PropertyIdentifier],
    array_index: Option<u32>,
) -> Result<bacnet_types::primitives::PropertyValue, bacnet_types::error::Error> {
    use bacnet_types::enums::PropertyIdentifier;

    // Clause 12.1.1.4.1: filter out the four excluded properties
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
                // Compute StatusFlags dynamically per Clause 12:
                //   Bit 0 (IN_ALARM): from status_flags field (set by event detection)
                //   Bit 1 (FAULT): reliability != NO_FAULT_DETECTED (0)
                //   Bit 2 (OVERRIDDEN): false (no local override mechanism)
                //   Bit 3 (OUT_OF_SERVICE): from out_of_service field
                let mut flags = $self.status_flags;
                if $self.reliability != 0 {
                    flags |= bacnet_types::primitives::StatusFlags::FAULT;
                } else {
                    flags -= bacnet_types::primitives::StatusFlags::FAULT;
                }
                if $self.out_of_service {
                    flags |= bacnet_types::primitives::StatusFlags::OUT_OF_SERVICE;
                } else {
                    flags -= bacnet_types::primitives::StatusFlags::OUT_OF_SERVICE;
                }
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![flags.bits() << 4],
                }))
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

/// Return the invalid-array-index protocol error.
#[inline]
pub(crate) fn invalid_array_index_error() -> bacnet_types::error::Error {
    protocol_error(
        bacnet_types::enums::ErrorClass::PROPERTY,
        bacnet_types::enums::ErrorCode::INVALID_ARRAY_INDEX,
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

/// Value source tracking for commandable objects (Clause 19.5).
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
/// Per Clause 19.2.1, required for AO, BO, MSO.
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

/// Common intrinsic-reporting read_property arms for objects with an
/// `OutOfRangeDetector` event_detector field.
///
/// Handles: HIGH_LIMIT, LOW_LIMIT, DEADBAND, LIMIT_ENABLE, EVENT_ENABLE,
///          NOTIFY_TYPE, NOTIFICATION_CLASS, TIME_DELAY, EVENT_STATE.
macro_rules! read_event_properties {
    ($self:expr, $property:expr) => {
        match $property {
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_STATE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::Enumerated(
                    $self.event_detector.event_state.to_raw(),
                )))
            }
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
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![$self.event_detector.event_enable << 5],
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
            p if p == bacnet_types::enums::PropertyIdentifier::ACKED_TRANSITIONS => {
                Some(Ok(bacnet_types::primitives::PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![$self.event_detector.acked_transitions << 5],
                }))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_TIME_STAMPS => {
                Some(Ok(bacnet_types::primitives::PropertyValue::List(vec![
                    bacnet_types::primitives::PropertyValue::Unsigned(
                        match $self.event_time_stamps[0] {
                            bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(n) => {
                                n as u64
                            }
                            _ => 0,
                        },
                    ),
                    bacnet_types::primitives::PropertyValue::Unsigned(
                        match $self.event_time_stamps[1] {
                            bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(n) => {
                                n as u64
                            }
                            _ => 0,
                        },
                    ),
                    bacnet_types::primitives::PropertyValue::Unsigned(
                        match $self.event_time_stamps[2] {
                            bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(n) => {
                                n as u64
                            }
                            _ => 0,
                        },
                    ),
                ])))
            }
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_MESSAGE_TEXTS => {
                Some(Ok(bacnet_types::primitives::PropertyValue::List(vec![
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        $self.event_message_texts[0].clone(),
                    ),
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        $self.event_message_texts[1].clone(),
                    ),
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        $self.event_message_texts[2].clone(),
                    ),
                ])))
            }
            _ => None,
        }
    };
}
pub(crate) use read_event_properties;

/// Common intrinsic-reporting write_property arms for objects with an
/// `OutOfRangeDetector` event_detector field.
///
/// Handles: HIGH_LIMIT, LOW_LIMIT, DEADBAND, LIMIT_ENABLE,
///          NOTIFICATION_CLASS, NOTIFY_TYPE.
///
/// Returns `Some(Ok(()))` if the property was handled,
/// `Some(Err(...))` for type/validation errors,
/// or `None` if the property is not an event property.
macro_rules! write_event_properties {
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
                if let bacnet_types::primitives::PropertyValue::BitString { data, .. } = &$value {
                    if let Some(&byte) = data.first() {
                        $self.event_detector.limit_enable =
                            $crate::event::LimitEnable::from_bits(byte);
                        Some(Ok(()))
                    } else {
                        Some(Err($crate::common::invalid_data_type_error()))
                    }
                } else {
                    Some(Err($crate::common::invalid_data_type_error()))
                }
            }
            p if p == bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE => {
                if let bacnet_types::primitives::PropertyValue::BitString { data, .. } = &$value {
                    if let Some(&byte) = data.first() {
                        // BACnet bitstring: 3 bits used (5 unused), MSB-first
                        // Bit 0 = TO_OFFNORMAL, Bit 1 = TO_FAULT, Bit 2 = TO_NORMAL
                        $self.event_detector.event_enable = byte >> 5;
                        Some(Ok(()))
                    } else {
                        Some(Err($crate::common::invalid_data_type_error()))
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
                if let bacnet_types::primitives::PropertyValue::Enumerated(v) = $value {
                    $self.event_detector.notify_type = v;
                    Some(Ok(()))
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
            p if p == bacnet_types::enums::PropertyIdentifier::ACKED_TRANSITIONS => {
                // Read-only: modified only by AcknowledgeAlarm service (Clause 12.13.9)
                Some(Err($crate::common::write_access_denied_error()))
            }
            _ => None,
        }
    };
}
pub(crate) use write_event_properties;

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

/// Handle direct writes to PRIORITY_ARRAY[index] per Clause 15.9.1.1.3.
///
/// If `property` is PRIORITY_ARRAY and `array_index` is Some(1..=16),
/// writes to that priority slot. Null relinquishes; otherwise `$extract`
/// converts the value. Calls `recalculate_present_value()` after write.
///
/// Returns early with `Ok(())` or `Err(...)` if the property is PRIORITY_ARRAY.
/// Falls through (does nothing) if the property is not PRIORITY_ARRAY.
macro_rules! write_priority_array_direct {
    ($self:expr, $property:expr, $array_index:expr, $value:expr, $extract:expr) => {
        if $property == bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY {
            let idx = match $array_index {
                Some(n) if (1..=16).contains(&n) => (n - 1) as usize,
                Some(_) => return Err($crate::common::invalid_array_index_error()),
                None => {
                    return Err(bacnet_types::error::Error::Encoding(
                        "PRIORITY_ARRAY requires array_index (1-16)".into(),
                    ))
                }
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
