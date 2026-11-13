//! Closed projection of committed intrinsic and Event Enrollment transitions.
//!
//! Every value is selected explicitly from the evaluated source while the
//! server still owns the database write guard. The resulting private wrapper
//! is the immutable payload carried to all recipients and confirmed retries.

use alloc::vec::Vec;
use bacnet_encoding::constructed::{
    encode_bacnet_property_value, notification_forms, validate_tlv_sequence,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

/// One validated notification-parameter value captured for a committed event.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedNotificationPayload(NotificationParameters);

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

/// The event-value forms [`project_intrinsic_payload`] builds, as a mask
/// for `bacnet_encoding`'s `encode_event_notification_of`: a host that
/// distributes only intrinsic transitions links no other form's encoder
/// (#2524).
pub const INTRINSIC_FORMS: u32 = notification_forms(&[
    EventType::OUT_OF_RANGE,
    EventType::CHANGE_OF_STATE,
    EventType::COMMAND_FAILURE,
    EventType::BUFFER_READY,
    EventType::CHANGE_OF_RELIABILITY,
]);

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

fn status_flags(value: &PropertyValue) -> Option<StatusFlags> {
    let PropertyValue::BitString { unused_bits, data } = value else {
        return None;
    };
    (*unused_bits == 4 && data.len() == 1 && data[0] & 0x0f == 0)
        .then(|| StatusFlags::from_bits_retain(data[0] >> 4))
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

mod enrollment;
pub use enrollment::{
    CapturedReferencedValue, CapturedStatusFlags, EventEnrollmentProjectionSnapshot,
    capture_status_flags, decode_reference_value, project_event_enrollment_payload,
};

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
