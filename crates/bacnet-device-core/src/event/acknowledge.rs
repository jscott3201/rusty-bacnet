//! AcknowledgeAlarm (Clause 13.5): the acknowledgement of one committed
//! transition (Clause 13.2.3), and the ACK_NOTIFICATION it may send.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::traits::no_alarm_configured;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::NotificationTransition;

/// Exact context the bundled server can use for ACK_NOTIFICATION distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcknowledgmentNotificationContext {
    /// Exact historical transition correlated by the request.
    pub change: EventStateChange,
    /// Effective Event Type for the acknowledged transition.
    pub event_type: EventType,
    /// Current Event_Enable decision for the transition coordinate.
    pub distribute: bool,
}

/// A successfully correlated and applied acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedAcknowledgeAlarm {
    /// Object whose acknowledgment bit was accepted.
    pub event_object_identifier: ObjectIdentifier,
    /// Exact notification context, when the object can provide it.
    ///
    /// `None` when the object's acknowledgment returns no exact transition
    /// (`Ok(None)`) or no Event Type; the acknowledgment remains accepted.
    pub notification: Option<AcknowledgmentNotificationContext>,
}

/// Handle an AcknowledgeAlarm request.
///
/// Correlates the request with one committed transition before acknowledging it.
pub fn handle_acknowledge_alarm(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<AcceptedAcknowledgeAlarm, Error> {
    let request =
        AcknowledgeAlarmRequest::decode(service_data).map_err(Error::into_request_reject)?;

    let object = db
        .get_mut(&request.event_object_identifier)
        .ok_or(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ))?;

    let reporting = object
        .event_reporting_mut()
        .ok_or_else(no_alarm_configured)?;
    let change =
        reporting.acknowledge_alarm(request.event_state_acknowledged, &request.timestamp)?;
    // The notification's Event Type comes from the object's own algorithm.
    let correlated = change.and_then(|change| {
        let algorithm = reporting.enrollment_summary_capability()?.event_type;
        Some((change, algorithm))
    });

    let notification = correlated.and_then(|(change, algorithm)| {
        let PropertyValue::BitString { unused_bits, data } = object
            .read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .ok()?
        else {
            return None;
        };
        if unused_bits != 5 || data.len() != 1 || data[0] & 0x1f != 0 {
            return None;
        }
        let distribute =
            EventTransitionBits::from_bacnet(&data).contains(change.transition().bit_mask());
        let event_type = change.event_type(algorithm);
        Some(AcknowledgmentNotificationContext {
            change,
            event_type,
            distribute,
        })
    });

    // These request fields are validated by decoding but intentionally are not
    // persisted. ACK_NOTIFICATION construction uses object history and a fresh
    // send-time timestamp rather than either caller timestamp.
    let _ = (
        request.acknowledging_process_identifier,
        request.acknowledgment_source,
        request.time_of_acknowledgment,
    );

    Ok(AcceptedAcknowledgeAlarm {
        event_object_identifier: request.event_object_identifier,
        notification,
    })
}

impl AcceptedAcknowledgeAlarm {
    /// The ACK_NOTIFICATION to distribute for it, with its object: `None`
    /// when the object gave no exact transition or Event_Enable leaves the
    /// transition out. A host sends it after the SimpleACK.
    pub fn into_notification(self) -> Option<(ObjectIdentifier, NotificationTransition)> {
        let notification = self.notification?;
        if !notification.distribute {
            return None;
        }
        Some((
            self.event_object_identifier,
            NotificationTransition::acknowledgment(notification.change, notification.event_type),
        ))
    }
}
