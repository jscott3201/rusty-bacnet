use std::fmt;

use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::decode_destination_list_pv;
use crate::database::ObjectDatabase;
use crate::event::EventTransition;

/// Strict Notification Class values needed by GetEnrollmentSummary.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentSummaryClassProjection {
    pub priority: u8,
    pub enrollment_member: bool,
}

/// Failure to produce an exact Notification Class summary projection.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentSummaryClassProjectionError {
    NotificationClassMissing,
    PriorityUnreadable,
    PriorityMalformed,
    RecipientListUnreadable,
    RecipientListMalformed,
}

impl fmt::Display for EnrollmentSummaryClassProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::NotificationClassMissing => "Notification Class object instance is missing",
            Self::PriorityUnreadable => "Priority is unreadable",
            Self::PriorityMalformed => "Priority is not three Unsigned8 values",
            Self::RecipientListUnreadable => "Recipient_List is unreadable",
            Self::RecipientListMalformed => "Recipient_List is malformed",
        };
        f.write_str(detail)
    }
}

/// Resolve one exact Notification Class and its current summary values.
///
/// The event-initiating object's `Notification_Class` value is the instance
/// number of the Notification Class object to resolve. The resolved object's
/// own `Notification_Class` property is unrelated to that identity and is not
/// read. Recipient membership intentionally ignores delivery-time eligibility,
/// transition bits, and confirmed mode.
#[doc(hidden)]
pub fn resolve_enrollment_summary_class_internal(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
    enrollment_filter: Option<(&BACnetRecipient, u32)>,
) -> Result<EnrollmentSummaryClassProjection, EnrollmentSummaryClassProjectionError> {
    let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, notification_class)
        .map_err(|_| EnrollmentSummaryClassProjectionError::NotificationClassMissing)?;
    let object = db
        .get(&oid)
        .ok_or(EnrollmentSummaryClassProjectionError::NotificationClassMissing)?;

    let value = object
        .read_property(PropertyIdentifier::PRIORITY, None)
        .map_err(|_| EnrollmentSummaryClassProjectionError::PriorityUnreadable)?;
    let PropertyValue::List(values) = value else {
        return Err(EnrollmentSummaryClassProjectionError::PriorityMalformed);
    };
    if values.len() != 3 {
        return Err(EnrollmentSummaryClassProjectionError::PriorityMalformed);
    }
    let mut priorities = [0u8; 3];
    for (slot, value) in priorities.iter_mut().zip(values) {
        let PropertyValue::Unsigned(value) = value else {
            return Err(EnrollmentSummaryClassProjectionError::PriorityMalformed);
        };
        *slot = u8::try_from(value)
            .map_err(|_| EnrollmentSummaryClassProjectionError::PriorityMalformed)?;
    }

    let enrollment_member = if let Some((recipient, process_identifier)) = enrollment_filter {
        let value = object
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .map_err(|_| EnrollmentSummaryClassProjectionError::RecipientListUnreadable)?;
        decode_destination_list_pv(&value)
            .map_err(|_| EnrollmentSummaryClassProjectionError::RecipientListMalformed)?
            .iter()
            .any(|destination| {
                &destination.recipient == recipient
                    && destination.process_identifier == process_identifier
            })
    } else {
        true
    };

    Ok(EnrollmentSummaryClassProjection {
        priority: priorities[transition.index()],
        enrollment_member,
    })
}
