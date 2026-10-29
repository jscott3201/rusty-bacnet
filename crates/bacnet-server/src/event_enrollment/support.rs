use std::collections::HashMap;

use bacnet_objects::database::{LocalDevice, ObjectDatabase};
use bacnet_objects::event::EventTransition;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::EventEnrollmentReliabilityCause;
use super::LocalConfigurationReadError;
use super::algorithms::extract_real;
use super::commit::{EnrollmentUpdate, ReliabilityUpdate};
use crate::server::event_notification_payload::CapturedReferencedValue;

pub(super) enum SetpointRead {
    Value(f32),
    Unusable,
    Transient,
}

/// Read a FLOATING_LIMIT setpoint. The reference resolves as the monitored
/// one does: unqualified or naming this device reads `db`, and one naming
/// another device is unavailable here, like an unreachable local object.
pub(super) fn read_setpoint(
    db: &ObjectDatabase,
    local_device: LocalDevice,
    reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
) -> SetpointRead {
    if !local_device.is_local(reference.device_identifier) {
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
    transition_bit: EventTransitionBits,
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
            EventTransitionBits::from_bacnet(&data).intersects(transition_bit)
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
        Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY
                && matches!(
                    *code,
                    code if code == ErrorCode::INVALID_ARRAY_INDEX
                        || code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
                        || code == ErrorCode::UNKNOWN_PROPERTY
                )
    )
}

pub(super) fn classify_required_property_read_error(error: &Error) -> LocalConfigurationReadError {
    if matches!(
        error,
        Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY
                && *code == ErrorCode::UNKNOWN_PROPERTY
    ) {
        LocalConfigurationReadError::Malformed
    } else {
        LocalConfigurationReadError::Unavailable
    }
}

/// The enrollment under evaluation and the reliability, event state and
/// Event_Enable it held when the pass read it.
pub(super) struct ReliabilitySubject<'a> {
    pub(super) db: &'a ObjectDatabase,
    pub(super) enrollment: &'a dyn BACnetObject,
    pub(super) enrollment_oid: ObjectIdentifier,
    pub(super) previous: Reliability,
    pub(super) current_state: EventState,
    pub(super) event_enable: EventTransitionBits,
}

pub(super) fn queue_reliability_transition(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    subject: &ReliabilitySubject<'_>,
    monitored_oid: Option<ObjectIdentifier>,
    desired: Reliability,
    cause: EventEnrollmentReliabilityCause,
    referenced_value: CapturedReferencedValue,
) {
    let ReliabilitySubject {
        db,
        enrollment,
        enrollment_oid,
        previous,
        current_state,
        event_enable,
    } = *subject;
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
            distribute: event_enable.contains(transition_bit),
            ack_required: ack_required_for_transition(db, enrollment, transition_bit),
            cause,
            referenced_value,
        });
}

pub(super) fn queue_observation_unavailable(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
) {
    updates.entry(oid).or_default().observation_unavailable();
}

/// Apply the repository-local D4 continuity policy for a valid observation
/// that is temporarily unavailable. Coalesce every private coordinate to its
/// ownerless default before the typed terminal diagnostic is committed.
pub(super) fn queue_observation_gap(
    updates: &mut HashMap<ObjectIdentifier, EnrollmentUpdate>,
    oid: ObjectIdentifier,
    eval_state_supported: bool,
    eval_source: Option<Option<EventEnrollmentMonitoredSource>>,
) {
    let update = updates.entry(oid).or_default();
    if eval_state_supported {
        update.reset_eval_state();
    }
    update.reset_eval_source_for_observation_gap(eval_source);
    update.observation_unavailable();
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
