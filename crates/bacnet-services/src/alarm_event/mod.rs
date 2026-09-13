//! Alarm acknowledgment, event notification, and event information services.
//!
//! - AcknowledgeAlarm acknowledges an event transition.
//! - ConfirmedEventNotification / UnconfirmedEventNotification report events.
//! - GetEventInformation retrieves event summaries.

use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetPropertyStates,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};

mod acknowledge_alarm;
mod event_notification;
mod get_event_information;
mod notification_parameters;
mod property_states;

pub use acknowledge_alarm::AcknowledgeAlarmRequest;
pub use event_notification::EventNotificationRequest;
pub use get_event_information::{EventSummary, GetEventInformationAck, GetEventInformationRequest};
pub use notification_parameters::{ChangeOfValueChoice, NotificationParameters};

#[cfg(test)]
mod tests;
