//! The Event Enrollment half of the projection: what an enrollment's
//! evaluation captured, and the Event Values its committed transition
//! reports. The evaluator itself stays in `bacnet-server` until #2319.

use alloc::vec::Vec;
use bacnet_encoding::constructed::{
    decode_device_object_property_reference, encode_property_state,
};
use bacnet_encoding::tags;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::{ChangeOfValueChoice, NotificationParameters};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};
use bacnet_types::enums::{EventState, EventType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

use super::{
    CommittedNotificationPayload, append_property_value, required_status_flags, selected_limit,
    status_flags,
};

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

fn validate_bitstring(unused_bits: u8, data: &[u8]) -> Option<()> {
    if unused_bits > 7 || (data.is_empty() && unused_bits != 0) {
        return None;
    }
    let trailing_mask = (1u8.checked_shl(u32::from(unused_bits))?).wrapping_sub(1);
    data.last()
        .is_none_or(|last| last & trailing_mask == 0)
        .then_some(())
}
