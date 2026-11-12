//! Closed projection of committed intrinsic and Event Enrollment transitions.
//!
//! Every value is selected explicitly from the evaluated source while the
//! server still owns the database write guard. The resulting private wrapper
//! is the immutable payload carried to all recipients and confirmed retries.

use alloc::vec::Vec;
use bacnet_encoding::constructed::{
    decode_device_object_property_reference, encode_bacnet_property_value, encode_property_state,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::{constructed::validate_tlv_sequence, tags};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::{ChangeOfValueChoice, NotificationParameters};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

/// One validated notification-parameter value captured for a committed event.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedNotificationPayload(NotificationParameters);

/// The monitored object's Status_Flags as an Event Enrollment read them.
#[derive(Clone, Copy)]
pub enum CapturedStatusFlags {
    /// A well-formed value.
    Value(StatusFlags),
    /// The object serves none.
    Unavailable,
    /// The value isn't a four-bit BitString.
    Malformed,
}

/// The monitored property's value an Event Enrollment evaluated, for a
/// CHANGE_OF_RELIABILITY report.
#[derive(Clone)]
pub enum CapturedReferencedValue {
    /// Not evaluated: the projection reads it.
    NotEvaluated,
    /// The value evaluated.
    Value(PropertyValue),
    /// It couldn't be read: the report leaves it out.
    Unavailable,
}

impl CapturedReferencedValue {
    /// The value an evaluation read, or [`Self::NotEvaluated`] for none.
    pub fn from_evaluated(value: Option<&PropertyValue>) -> Self {
        value.cloned().map_or(Self::NotEvaluated, Self::Value)
    }
}

/// What an Event Enrollment's evaluation read, for its committed
/// transition's Event Values.
#[derive(Clone)]
pub struct EventEnrollmentProjectionSnapshot {
    reference: MonitoredReference,
    monitored_value: PropertyValue,
    parameters: BACnetEventParameter,
    status_flags: CapturedStatusFlags,
    /// The FLOATING_LIMIT setpoint the evaluator read. It is present only
    /// when the evaluator resolved the setpoint reference to this device, so
    /// the projection never resolves that reference a second way (#1184).
    setpoint_value: Option<f32>,
}

impl EventEnrollmentProjectionSnapshot {
    /// Capture the monitored reference, its value, the event parameters, the
    /// monitored Status_Flags and a FLOATING_LIMIT setpoint.
    pub fn new(
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        array_index: Option<u32>,
        monitored_value: PropertyValue,
        parameters: BACnetEventParameter,
        status_flags: CapturedStatusFlags,
        setpoint_value: Option<f32>,
    ) -> Self {
        Self {
            reference: MonitoredReference {
                object_identifier,
                property_identifier,
                array_index,
            },
            monitored_value,
            parameters,
            status_flags,
            setpoint_value,
        }
    }
}

impl CommittedNotificationPayload {
    /// The notification parameters it carries.
    pub fn into_parameters(self) -> NotificationParameters {
        self.0
    }

    /// Wrap parameters without projecting them, for hosts' tests only.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn for_test(parameters: NotificationParameters) -> Self {
        Self(parameters)
    }
}

#[derive(Clone, Copy)]
struct MonitoredReference {
    object_identifier: ObjectIdentifier,
    property_identifier: PropertyIdentifier,
    array_index: Option<u32>,
}

enum OptionalProjectionValue {
    Unavailable,
    Value(PropertyValue),
    Malformed,
}

/// Project an intrinsic source after its transition commit. `device` is this
/// device's identifier, which a log's BUFFER_READY report names its buffer
/// with.
pub fn project_intrinsic_payload(
    object: &dyn BACnetObject,
    change: &EventStateChange,
    event_type: EventType,
    device: Option<ObjectIdentifier>,
) -> Option<CommittedNotificationPayload> {
    let object_type = object.object_identifier().object_type();
    let params = if change.from == EventState::FAULT || change.to == EventState::FAULT {
        (event_type == EventType::CHANGE_OF_RELIABILITY)
            .then(|| project_builtin_reliability(object, object_type))??
    } else {
        match object_type {
            ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE
                if event_type == EventType::OUT_OF_RANGE =>
            {
                project_builtin_out_of_range(object, change)?
            }
            ObjectType::BINARY_INPUT
            | ObjectType::BINARY_VALUE
            | ObjectType::MULTI_STATE_INPUT
            | ObjectType::MULTI_STATE_VALUE
            | ObjectType::ACCESS_ZONE
            | ObjectType::ACCESS_DOOR
                if event_type == EventType::CHANGE_OF_STATE =>
            {
                project_builtin_change_of_state(object, object_type)?
            }
            ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT
                if event_type == EventType::COMMAND_FAILURE =>
            {
                project_builtin_command_failure(object, object_type)?
            }
            ObjectType::TREND_LOG | ObjectType::EVENT_LOG | ObjectType::TREND_LOG_MULTIPLE
                if event_type == EventType::BUFFER_READY =>
            {
                project_buffer_ready(object, device)?
            }
            _ => return None,
        }
    };
    Some(CommittedNotificationPayload(params))
}

/// A log's BUFFER_READY report (Clause 13.3.7): its own Log_Buffer in this
/// device, with the counts its committed transition reported.
fn project_buffer_ready(
    object: &dyn BACnetObject,
    device: Option<ObjectIdentifier>,
) -> Option<NotificationParameters> {
    let report = object.event_reporting()?.buffer_ready_report()?;
    Some(NotificationParameters::BufferReady {
        buffer_property: BACnetDeviceObjectPropertyReference {
            object_identifier: object.object_identifier(),
            property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
            property_array_index: None,
            device_identifier: device,
        },
        previous_notification: report.previous_notification,
        current_notification: report.current_notification,
    })
}

/// Project an Event Enrollment source after its transition commit.
pub fn project_event_enrollment_payload(
    db: &ObjectDatabase,
    enrollment_oid: ObjectIdentifier,
    expected_monitored_oid: Option<ObjectIdentifier>,
    change: &EventStateChange,
    event_type: EventType,
    snapshot: Option<&EventEnrollmentProjectionSnapshot>,
    reliability_value: Option<&CapturedReferencedValue>,
) -> Option<CommittedNotificationPayload> {
    let params = if change.from == EventState::FAULT || change.to == EventState::FAULT {
        if event_type != EventType::CHANGE_OF_RELIABILITY {
            return None;
        }
        let enrollment = db.get(&enrollment_oid)?;
        let (reference_value, reference) = read_monitored_reference(enrollment)?;
        if expected_monitored_oid.is_some_and(|expected| expected != reference.object_identifier) {
            return None;
        }
        project_event_enrollment_reliability(
            db,
            enrollment,
            reference_value,
            reference,
            reliability_value?,
        )?
    } else {
        if event_type == EventType::NONE {
            return None;
        }
        let snapshot = snapshot?;
        if expected_monitored_oid
            .is_some_and(|expected| expected != snapshot.reference.object_identifier)
        {
            return None;
        }
        project_event_enrollment_normal(snapshot, change, event_type)?
    };
    Some(CommittedNotificationPayload(params))
}

fn project_builtin_out_of_range(
    object: &dyn BACnetObject,
    change: &EventStateChange,
) -> Option<NotificationParameters> {
    let exceeding_value = read_real(object, PropertyIdentifier::PRESENT_VALUE)?;
    let deadband = read_real(object, PropertyIdentifier::DEADBAND)?;
    if deadband < 0.0 {
        return None;
    }
    let exceeded_limit = selected_limit(
        change,
        read_real(object, PropertyIdentifier::LOW_LIMIT)?,
        read_real(object, PropertyIdentifier::HIGH_LIMIT)?,
    )?;
    Some(NotificationParameters::OutOfRange {
        exceeding_value,
        status_flags: required_status_flags(object)?,
        deadband,
        exceeded_limit,
    })
}

/// The property a built-in CHANGE_OF_STATE source watches, whose value
/// New_State carries (Clause 13.3.2): Present_Value, except on an Access
/// Zone, which watches Occupancy_State (Clause 12.32.6), and an Access Door,
/// which watches Door_Alarm_State (Clause 12.26.20).
fn watched_property(object_type: ObjectType) -> PropertyIdentifier {
    match object_type {
        ObjectType::ACCESS_ZONE => PropertyIdentifier::OCCUPANCY_STATE,
        ObjectType::ACCESS_DOOR => PropertyIdentifier::DOOR_ALARM_STATE,
        _ => PropertyIdentifier::PRESENT_VALUE,
    }
}

fn project_builtin_change_of_state(
    object: &dyn BACnetObject,
    object_type: ObjectType,
) -> Option<NotificationParameters> {
    let watched = object
        .read_property(watched_property(object_type), None)
        .ok()?;
    let new_state = match (object_type, watched) {
        (ObjectType::BINARY_INPUT | ObjectType::BINARY_VALUE, PropertyValue::Enumerated(value))
            if value <= 1 =>
        {
            BACnetPropertyStates::BinaryValue(value)
        }
        (
            ObjectType::MULTI_STATE_INPUT | ObjectType::MULTI_STATE_VALUE,
            PropertyValue::Unsigned(value),
        ) if value > 0 => BACnetPropertyStates::UnsignedValue(u32::try_from(value).ok()?),
        (ObjectType::ACCESS_ZONE, PropertyValue::Enumerated(value)) => {
            BACnetPropertyStates::ZoneOccupancyState(value)
        }
        (ObjectType::ACCESS_DOOR, PropertyValue::Enumerated(value)) => {
            BACnetPropertyStates::DoorAlarmState(value)
        }
        _ => return None,
    };
    Some(NotificationParameters::ChangeOfState {
        new_state,
        status_flags: required_status_flags(object)?,
    })
}

fn project_builtin_command_failure(
    object: &dyn BACnetObject,
    object_type: ObjectType,
) -> Option<NotificationParameters> {
    let command = object
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .ok()?;
    let feedback = object
        .read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
        .ok()?;
    match (&command, &feedback, object_type) {
        (PropertyValue::Enumerated(a), PropertyValue::Enumerated(b), ObjectType::BINARY_OUTPUT)
            if *a <= 1 && *b <= 1 => {}
        (
            PropertyValue::Unsigned(a),
            PropertyValue::Unsigned(b),
            ObjectType::MULTI_STATE_OUTPUT,
        ) if *a > 0 && *b > 0 && u32::try_from(*a).is_ok() && u32::try_from(*b).is_ok() => {}
        _ => return None,
    }
    Some(NotificationParameters::CommandFailure {
        command_value: encode_abstract_value(&command)?,
        status_flags: required_status_flags(object)?,
        feedback_value: encode_abstract_value(&feedback)?,
    })
}

fn project_builtin_reliability(
    object: &dyn BACnetObject,
    object_type: ObjectType,
) -> Option<NotificationParameters> {
    let PropertyValue::Enumerated(reliability) = object
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .ok()?
    else {
        return None;
    };
    let mut property_values = Vec::new();
    for &property in reliability_report_properties(object_type)? {
        let value = object.read_property(property, None).ok()?;
        match property {
            PropertyIdentifier::PRESENT_VALUE => {
                validate_builtin_present_value(object_type, &value)?
            }
            PropertyIdentifier::FEEDBACK_VALUE => {
                validate_builtin_feedback_value(object_type, &value)?
            }
            _ => matches!(value, PropertyValue::Enumerated(_)).then_some(())?,
        }
        append_property_value(&mut property_values, property, None, &value)?;
    }

    Some(NotificationParameters::ChangeOfReliability {
        reliability: Reliability::from_raw(reliability),
        status_flags: required_status_flags(object)?,
        property_values,
    })
}

/// The properties a built-in source's CHANGE_OF_RELIABILITY notification
/// carries, in the order Table 13-5 lists them for its object type.
fn reliability_report_properties(object_type: ObjectType) -> Option<&'static [PropertyIdentifier]> {
    match object_type {
        ObjectType::ANALOG_INPUT
        | ObjectType::ANALOG_OUTPUT
        | ObjectType::ANALOG_VALUE
        | ObjectType::BINARY_INPUT
        | ObjectType::BINARY_VALUE
        | ObjectType::MULTI_STATE_INPUT
        | ObjectType::MULTI_STATE_VALUE => Some(&[PropertyIdentifier::PRESENT_VALUE]),
        ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT => Some(&[
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::FEEDBACK_VALUE,
        ]),
        ObjectType::ACCESS_ZONE => Some(&[PropertyIdentifier::OCCUPANCY_STATE]),
        ObjectType::ACCESS_DOOR => Some(&[
            PropertyIdentifier::DOOR_ALARM_STATE,
            PropertyIdentifier::PRESENT_VALUE,
        ]),
        _ => None,
    }
}

fn project_event_enrollment_normal(
    snapshot: &EventEnrollmentProjectionSnapshot,
    change: &EventStateChange,
    event_type: EventType,
) -> Option<NotificationParameters> {
    let reference = snapshot.reference;
    let monitored_value = snapshot.monitored_value.clone();
    let status_flags = match snapshot.status_flags {
        CapturedStatusFlags::Value(value) => value,
        CapturedStatusFlags::Unavailable => StatusFlags::empty(),
        CapturedStatusFlags::Malformed => return None,
    };
    let parameters = snapshot.parameters.clone();

    match (event_type, parameters) {
        (EventType::CHANGE_OF_BITSTRING, BACnetEventParameter::ChangeOfBitstring { .. }) => {
            let PropertyValue::BitString { unused_bits, data } = monitored_value else {
                return None;
            };
            validate_bitstring(unused_bits, &data)?;
            Some(NotificationParameters::ChangeOfBitstring {
                referenced_bitstring: (unused_bits, data),
                status_flags,
            })
        }
        (
            EventType::CHANGE_OF_STATE,
            BACnetEventParameter::ChangeOfState { list_of_values, .. },
        ) => Some(NotificationParameters::ChangeOfState {
            new_state: property_state_for_value(
                &monitored_value,
                reference.object_identifier.object_type(),
                reference.property_identifier,
                &list_of_values,
            )?,
            status_flags,
        }),
        (EventType::CHANGE_OF_VALUE, BACnetEventParameter::ChangeOfValue { criteria, .. }) => {
            let new_value = match (criteria, monitored_value) {
                (
                    bacnet_types::constructed::ChangeOfValueCriteria::ReferencedPropertyIncrement(
                        _,
                    ),
                    PropertyValue::Real(value),
                ) if value.is_finite() => ChangeOfValueChoice::ChangedValue(value),
                (
                    bacnet_types::constructed::ChangeOfValueCriteria::Bitmask { .. },
                    PropertyValue::BitString { unused_bits, data },
                ) => {
                    validate_bitstring(unused_bits, &data)?;
                    ChangeOfValueChoice::ChangedBits { unused_bits, data }
                }
                _ => return None,
            };
            Some(NotificationParameters::ChangeOfValue {
                new_value,
                status_flags,
            })
        }
        (
            EventType::FLOATING_LIMIT,
            BACnetEventParameter::FloatingLimit {
                low_diff_limit,
                high_diff_limit,
                ..
            },
        ) => {
            let PropertyValue::Real(reference_value) = monitored_value else {
                return None;
            };
            if !reference_value.is_finite()
                || !low_diff_limit.is_finite()
                || !high_diff_limit.is_finite()
            {
                return None;
            }
            let setpoint_value = snapshot.setpoint_value?;
            if !setpoint_value.is_finite() {
                return None;
            }
            Some(NotificationParameters::FloatingLimit {
                reference_value,
                status_flags,
                setpoint_value,
                error_limit: selected_limit(change, low_diff_limit, high_diff_limit)?,
            })
        }
        (
            EventType::OUT_OF_RANGE,
            BACnetEventParameter::OutOfRange {
                low_limit,
                high_limit,
                deadband,
                ..
            },
        ) => {
            let PropertyValue::Real(exceeding_value) = monitored_value else {
                return None;
            };
            if !exceeding_value.is_finite()
                || !low_limit.is_finite()
                || !high_limit.is_finite()
                || !deadband.is_finite()
                || deadband < 0.0
            {
                return None;
            }
            Some(NotificationParameters::OutOfRange {
                exceeding_value,
                status_flags,
                deadband,
                exceeded_limit: selected_limit(change, low_limit, high_limit)?,
            })
        }
        _ => None,
    }
}

fn project_event_enrollment_reliability(
    db: &ObjectDatabase,
    enrollment: &dyn BACnetObject,
    reference_value: PropertyValue,
    reference: MonitoredReference,
    captured_value: &CapturedReferencedValue,
) -> Option<NotificationParameters> {
    let PropertyValue::Enumerated(reliability) = enrollment
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .ok()?
    else {
        return None;
    };
    let mut property_values = Vec::new();
    append_property_value(
        &mut property_values,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        None,
        &reference_value,
    )?;

    if let Some(monitored) = db.get(&reference.object_identifier) {
        let referenced_value = match captured_value {
            CapturedReferencedValue::Value(value) => Some(value.clone()),
            CapturedReferencedValue::Unavailable => None,
            // An index on a scalar reads the whole value, as before objects
            // refused that index themselves (#2274).
            CapturedReferencedValue::NotEvaluated => monitored
                .read_property(
                    reference.property_identifier,
                    reference.array_index.filter(|_| {
                        bacnet_objects::property_metadata::is_array_property(
                            monitored,
                            reference.property_identifier,
                        )
                    }),
                )
                .ok(),
        };
        if let Some(value) = referenced_value {
            append_property_value(
                &mut property_values,
                reference.property_identifier,
                reference.array_index,
                &value,
            )?;
        }
        match optional_reliability(monitored) {
            OptionalProjectionValue::Value(value) => append_property_value(
                &mut property_values,
                PropertyIdentifier::RELIABILITY,
                None,
                &value,
            )?,
            OptionalProjectionValue::Unavailable => {}
            OptionalProjectionValue::Malformed => return None,
        }
        match optional_status_flags(monitored) {
            OptionalProjectionValue::Value(value) => append_property_value(
                &mut property_values,
                PropertyIdentifier::STATUS_FLAGS,
                None,
                &value,
            )?,
            OptionalProjectionValue::Unavailable => {}
            OptionalProjectionValue::Malformed => return None,
        }
    }

    Some(NotificationParameters::ChangeOfReliability {
        reliability: Reliability::from_raw(reliability),
        status_flags: required_status_flags(enrollment)?,
        property_values,
    })
}

fn read_monitored_reference(
    enrollment: &dyn BACnetObject,
) -> Option<(PropertyValue, MonitoredReference)> {
    let value = enrollment
        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
        .ok()?;
    let reference = decode_reference_value(&value)?;
    Some((
        value,
        MonitoredReference {
            object_identifier: reference.object_identifier,
            property_identifier: PropertyIdentifier::from_raw(reference.property_identifier),
            array_index: reference.property_array_index,
        },
    ))
}

/// The reference an Event Enrollment's Object_Property_Reference read holds:
/// exactly one BACnetDeviceObjectPropertyReference in its Clause 21 encoding
/// (#1182). Any property identifier counts, ASHRAE's above 4194303 included
/// (#887). Anything else is `None`, and so is the unset form an enrollment
/// without a reference reads as, its object or Device at the reserved
/// instance 4194303 (#1417).
pub fn decode_reference_value(
    value: &PropertyValue,
) -> Option<BACnetDeviceObjectPropertyReference> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return None;
    };
    let (reference, end) = decode_device_object_property_reference(bytes, 0).ok()?;
    (end == bytes.len() && !reference.is_unset()).then_some(reference)
}

fn property_state_for_value(
    value: &PropertyValue,
    object_type: ObjectType,
    property: PropertyIdentifier,
    alarm_values: &[BACnetPropertyStates],
) -> Option<BACnetPropertyStates> {
    match value {
        PropertyValue::Boolean(value) => Some(BACnetPropertyStates::BooleanValue(*value)),
        PropertyValue::Signed(value) => Some(BACnetPropertyStates::IntegerValue(*value)),
        PropertyValue::Unsigned(value) => Some(BACnetPropertyStates::UnsignedValue(
            u32::try_from(*value).ok()?,
        )),
        PropertyValue::Enumerated(value)
            if property == PropertyIdentifier::PRESENT_VALUE
                && matches!(
                    object_type,
                    ObjectType::BINARY_INPUT | ObjectType::BINARY_OUTPUT | ObjectType::BINARY_VALUE
                )
                && *value <= 1 =>
        {
            Some(BACnetPropertyStates::BinaryValue(*value))
        }
        PropertyValue::Enumerated(value) => {
            let mut tag = None;
            for alarm in alarm_values {
                let mut encoded = BytesMut::new();
                encode_property_state(&mut encoded, alarm).ok()?;
                let (candidate, _) = tags::decode_tag(&encoded, 0).ok()?;
                if candidate.is_opening || candidate.is_closing || candidate.number >= 63 {
                    return None;
                }
                match tag {
                    Some(existing) if existing != candidate.number => return None,
                    None => tag = Some(candidate.number),
                    _ => {}
                }
            }
            let tag = tag?;
            let mut encoded = BytesMut::new();
            bacnet_encoding::primitives::encode_ctx_enumerated(&mut encoded, tag, *value);
            let (state, consumed) =
                bacnet_encoding::constructed::decode_property_state(&encoded, 0).ok()?;
            (consumed == encoded.len()).then_some(state)
        }
        _ => None,
    }
}

fn selected_limit(change: &EventStateChange, low: f32, high: f32) -> Option<f32> {
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    if change.to == EventState::LOW_LIMIT
        || (change.from == EventState::LOW_LIMIT && change.to == EventState::NORMAL)
    {
        Some(low)
    } else if change.to == EventState::HIGH_LIMIT
        || (change.from == EventState::HIGH_LIMIT && change.to == EventState::NORMAL)
    {
        Some(high)
    } else {
        None
    }
}

fn read_real(object: &dyn BACnetObject, property: PropertyIdentifier) -> Option<f32> {
    let PropertyValue::Real(value) = object.read_property(property, None).ok()? else {
        return None;
    };
    value.is_finite().then_some(value)
}

fn required_status_flags(object: &dyn BACnetObject) -> Option<StatusFlags> {
    let value = object
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .ok()?;
    status_flags(&value)
}

/// Read an object's Status_Flags for an Event Enrollment's projection.
pub fn capture_status_flags(object: &dyn BACnetObject) -> CapturedStatusFlags {
    match object.read_property(PropertyIdentifier::STATUS_FLAGS, None) {
        Ok(value) => status_flags(&value)
            .map(CapturedStatusFlags::Value)
            .unwrap_or(CapturedStatusFlags::Malformed),
        Err(_) => CapturedStatusFlags::Unavailable,
    }
}

fn optional_status_flags(object: &dyn BACnetObject) -> OptionalProjectionValue {
    match object.read_property(PropertyIdentifier::STATUS_FLAGS, None) {
        Ok(value) if status_flags(&value).is_some() => OptionalProjectionValue::Value(value),
        Ok(_) => OptionalProjectionValue::Malformed,
        Err(_) => OptionalProjectionValue::Unavailable,
    }
}

fn optional_reliability(object: &dyn BACnetObject) -> OptionalProjectionValue {
    match object.read_property(PropertyIdentifier::RELIABILITY, None) {
        Ok(value @ PropertyValue::Enumerated(_)) => OptionalProjectionValue::Value(value),
        Ok(_) => OptionalProjectionValue::Malformed,
        Err(_) => OptionalProjectionValue::Unavailable,
    }
}

fn status_flags(value: &PropertyValue) -> Option<StatusFlags> {
    let PropertyValue::BitString { unused_bits, data } = value else {
        return None;
    };
    (*unused_bits == 4 && data.len() == 1 && data[0] & 0x0f == 0)
        .then(|| StatusFlags::from_bits_retain(data[0] >> 4))
}

fn validate_bitstring(unused_bits: u8, data: &[u8]) -> Option<()> {
    if unused_bits > 7 || (data.is_empty() && unused_bits != 0) {
        return None;
    }
    let trailing_mask = (1u8.checked_shl(u32::from(unused_bits))?).wrapping_sub(1);
    data.last()
        .is_none_or(|last| last & trailing_mask == 0)
        .then_some(())
}

fn validate_builtin_present_value(object_type: ObjectType, value: &PropertyValue) -> Option<()> {
    match (object_type, value) {
        (
            ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE,
            PropertyValue::Real(value),
        ) if value.is_finite() => Some(()),
        (
            ObjectType::BINARY_INPUT | ObjectType::BINARY_OUTPUT | ObjectType::BINARY_VALUE,
            PropertyValue::Enumerated(value),
        ) if *value <= 1 => Some(()),
        (
            ObjectType::MULTI_STATE_INPUT
            | ObjectType::MULTI_STATE_OUTPUT
            | ObjectType::MULTI_STATE_VALUE,
            PropertyValue::Unsigned(value),
        ) if *value > 0 && u32::try_from(*value).is_ok() => Some(()),
        // A BACnetDoorValue: LOCK to EXTENDED_PULSE_UNLOCK.
        (ObjectType::ACCESS_DOOR, PropertyValue::Enumerated(value)) if *value <= 3 => Some(()),
        _ => None,
    }
}

fn validate_builtin_feedback_value(object_type: ObjectType, value: &PropertyValue) -> Option<()> {
    match (object_type, value) {
        (ObjectType::BINARY_OUTPUT, PropertyValue::Enumerated(value)) if *value <= 1 => Some(()),
        (ObjectType::MULTI_STATE_OUTPUT, PropertyValue::Unsigned(value))
            if *value > 0 && u32::try_from(*value).is_ok() =>
        {
            Some(())
        }
        _ => None,
    }
}

fn encode_abstract_value(value: &PropertyValue) -> Option<Vec<u8>> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).ok()?;
    validate_tlv_sequence(&encoded, "committed notification abstract value").ok()?;
    Some(encoded.to_vec())
}

fn append_property_value(
    encoded: &mut Vec<u8>,
    property_identifier: PropertyIdentifier,
    property_array_index: Option<u32>,
    value: &PropertyValue,
) -> Option<()> {
    let value = encode_abstract_value(value)?;
    let property_value = BACnetPropertyValue {
        property_identifier,
        property_array_index,
        value,
        priority: None,
    };
    let mut entry = BytesMut::new();
    encode_bacnet_property_value(&property_value, &mut entry).ok()?;
    validate_tlv_sequence(&entry, "committed reliability property value").ok()?;
    encoded.extend_from_slice(&entry);
    Some(())
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
