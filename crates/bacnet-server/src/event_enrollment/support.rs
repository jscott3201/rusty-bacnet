use std::collections::HashMap;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventTransition;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::algorithms::extract_real;
use super::commit::{EnrollmentUpdate, ReliabilityUpdate};
use super::EventEnrollmentReliabilityCause;
use super::LocalConfigurationReadError;

pub(super) enum SetpointRead {
    Value(f32),
    Unusable,
    Transient,
}

pub(super) fn read_setpoint(
    db: &ObjectDatabase,
    reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
) -> SetpointRead {
    if reference.device_identifier.is_some() {
        return SetpointRead::Transient;
    }
    let Some(object) = db.get(&reference.object_identifier) else {
        return SetpointRead::Transient;
    };
    let property = PropertyIdentifier::from_raw(reference.property_identifier);
    if reference.property_array_index.is_some() && !object.is_array_property(property) {
        return SetpointRead::Unusable;
    }
    match object.read_property(property, reference.property_array_index) {
        Ok(value) => extract_real(&value)
            .map(SetpointRead::Value)
            .unwrap_or(SetpointRead::Unusable),
        Err(error)
            if reference.property_array_index.is_some() && invalid_indexed_target_error(&error) =>
        {
            SetpointRead::Unusable
        }
        Err(_) => SetpointRead::Transient,
    }
}

pub(super) fn passes_for_delay(delay_secs: u32, interval_secs: u64) -> u32 {
    let passes = (delay_secs as u64).div_ceil(interval_secs.max(1));
    u32::try_from(passes).unwrap_or(u32::MAX)
}

pub(super) fn ack_required_for_transition(
    db: &ObjectDatabase,
    enrollment: &dyn BACnetObject,
    transition_bit: u8,
) -> bool {
    let Ok(PropertyValue::Unsigned(instance)) =
        enrollment.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
    else {
        return false;
    };
    let Ok(oid) = ObjectIdentifier::new(
        ObjectType::NOTIFICATION_CLASS,
        u32::try_from(instance).unwrap_or(u32::MAX),
    ) else {
        return false;
    };
    let Some(notification_class) = db.get(&oid) else {
        return false;
    };
    match notification_class.read_property(PropertyIdentifier::ACK_REQUIRED, None) {
        Ok(PropertyValue::BitString { data, .. }) => {
            bacnet_types::bitstring::unpack_octet(&data, 3) & transition_bit != 0
        }
        _ => false,
    }
}

pub(super) fn queue_eval_state_reset(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    supported: bool,
    state: &EventEnrollmentEvalState,
    force: bool,
) {
    if supported && (force || *state != EventEnrollmentEvalState::default()) {
        updates.entry(oid).or_default().reset_eval_state();
    }
}

pub(super) fn queue_pending_cancellation(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    supported: bool,
    state: &mut EventEnrollmentEvalState,
) {
    if state.pending.take().is_some() && supported {
        updates
            .entry(oid)
            .or_default()
            .cancel_pending(state.clone());
    }
}

pub(super) fn queue_eval_source_reset(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    source: Option<Option<EventEnrollmentMonitoredSource>>,
) {
    if source.flatten().is_some() {
        updates.entry(oid).or_default().set_eval_source(None);
    }
}

pub(super) fn queue_invalid_reference(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    eval_state_supported: bool,
    eval_source: Option<Option<EventEnrollmentMonitoredSource>>,
) {
    if eval_state_supported {
        updates.entry(oid).or_default().reset_eval_state();
    }
    queue_eval_source_reset(updates, oid, eval_source);
}

pub(super) fn invalid_indexed_target_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Protocol { class, code }
            if *class == ErrorClass::PROPERTY.to_raw() as u32
                && matches!(
                    *code,
                    code if code == ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32
                        || code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32
                        || code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
                )
    )
}

pub(super) fn classify_required_property_read_error(error: &Error) -> LocalConfigurationReadError {
    if matches!(
        error,
        Error::Protocol { class, code }
            if *class == ErrorClass::PROPERTY.to_raw() as u32
                && *code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
    ) {
        LocalConfigurationReadError::Malformed
    } else {
        LocalConfigurationReadError::Unavailable
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn queue_reliability_transition(
    db: &ObjectDatabase,
    enrollment: &dyn BACnetObject,
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    enrollment_oid: ObjectIdentifier,
    monitored_oid: Option<ObjectIdentifier>,
    previous: Reliability,
    desired: Reliability,
    current_state: EventState,
    event_enable: u8,
    cause: EventEnrollmentReliabilityCause,
) {
    let target = if desired == Reliability::NO_FAULT_DETECTED {
        EventState::NORMAL
    } else {
        EventState::FAULT
    };
    if previous == desired && current_state == target {
        return;
    }
    let transition_bit = EventTransition::for_target_state(target).bit_mask();
    updates
        .entry(enrollment_oid)
        .or_default()
        .set_reliability(ReliabilityUpdate {
            monitored_oid,
            previous,
            desired,
            from: current_state,
            to: target,
            distribute: event_enable & transition_bit != 0,
            ack_required: ack_required_for_transition(db, enrollment, transition_bit),
            cause,
        });
}

pub(super) fn queue_observation_unavailable(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
) {
    updates.entry(oid).or_default().observation_unavailable();
}

pub(super) fn queue_evaluation_ownership(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    eval_state_supported: bool,
    reset_state: bool,
    eval_source: Option<Option<EventEnrollmentMonitoredSource>>,
    monitored_reference: EventEnrollmentMonitoredSource,
) {
    if eval_state_supported && reset_state {
        updates.entry(oid).or_default().reset_eval_state();
    }
    if eval_source.is_some_and(|current| current != Some(monitored_reference)) {
        updates
            .entry(oid)
            .or_default()
            .set_eval_source(Some(monitored_reference));
    }
}
