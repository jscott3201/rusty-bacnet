//! Alarm and event services per ASHRAE 135-2020 Clauses 13.2–13.9.
//!
//! - AcknowledgeAlarm (Clause 13.3)
//! - ConfirmedEventNotification / UnconfirmedEventNotification (Clause 13.5/13.6)
//! - GetEventInformation (Clause 13.9)

use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetPropertyStates};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

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
