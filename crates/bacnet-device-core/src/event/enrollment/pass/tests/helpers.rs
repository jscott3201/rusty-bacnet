//! Stand-ins for what these tests used from bacnet-server before they moved
//! here with the pass (#2319): its public wrappers over the pass, and its
//! test helpers (the metadata rows are the shared `crate::test_rows`).
//! Test-only; none is a server concept.

use bacnet_objects::DeviceIdentity;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};

use super::super::{
    EventEnrollmentEvaluationReport, EventEnrollmentTransition,
    evaluate_event_enrollments_for_delivery,
};

/// The server's `evaluate_event_enrollments`, without its log lines.
pub(crate) fn evaluate_event_enrollments(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> Vec<EventEnrollmentTransition> {
    evaluate_event_enrollments_report(db, interval_secs).transitions
}

/// The server's `evaluate_event_enrollments_report`, without its log lines.
pub(crate) fn evaluate_event_enrollments_report(
    db: &mut ObjectDatabase,
    interval_secs: u64,
) -> EventEnrollmentEvaluationReport {
    evaluate_event_enrollments_for_delivery(db, interval_secs).report
}

/// The server's `command_source::test_origin`: a local origin at Device 1.
pub(crate) fn test_origin() -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        initiating_object: None,
    }
}

/// The server's `test_event_state::commit_event_state`: commit `object`'s
/// transition to `to` from its Event_State, stamped with sequence 0.
pub(crate) fn commit_event_state(
    object: &mut dyn BACnetObject,
    to: EventState,
    ack_required: bool,
) {
    let from = match object.read_property(P::EVENT_STATE, None) {
        Ok(PropertyValue::Enumerated(raw)) => EventState::from_raw(raw),
        other => panic!("expected an Event_State, got {other:?}"),
    };
    object
        .event_reporting_mut()
        .expect("the object reports events")
        .commit_event_transition(EventTransitionCommit {
            change: EventStateChange { from, to },
            coordinate: EventTransition::for_target_state(to),
            ack_required,
            timestamp: BACnetTimeStamp::SequenceNumber(0),
            message_text: None,
        })
        .expect("the transition commits");
}

/// The server's `local_device::bind_database`: bind `db` to `identity`'s
/// Device for good.
pub(crate) fn bind_database(db: &mut ObjectDatabase, identity: &DeviceIdentity) {
    db.bind_local_device_internal(Some(identity.device_oid()));
}

/// The server's `local_device::lower_device`: Device `instance`, named
/// after it.
pub(crate) fn lower_device(instance: u32) -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: DeviceIdentity::new(instance, 0)
                .unwrap()
                .with_name(format!("Lower-Device-{instance}")),
            ..DeviceConfig::default()
        })
        .unwrap(),
    )
}
