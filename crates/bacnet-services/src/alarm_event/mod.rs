//! Alarm acknowledgment, event notification, and event information services.
//!
//! - AcknowledgeAlarm acknowledges an event transition (Clause 13.5).
//! - ConfirmedEventNotification / UnconfirmedEventNotification report events (Clauses 13.8, 13.9).
//! - GetEventInformation retrieves event summaries (Clause 13.12).
//!
//! The event notification request and its event values are bacnet-types
//! types, since an Event Log record holds a notification too; their codec is
//! in `bacnet_encoding::constructed` (`encode_event_notification`,
//! `decode_event_notification`).

use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{EventState, NotifyType};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

mod acknowledge_alarm;
mod event_forwarding;
mod get_event_information;

pub use acknowledge_alarm::AcknowledgeAlarmRequest;
pub use bacnet_types::constructed::{
    ChangeOfValueChoice, EventNotificationRequest, NotificationParameters,
};
pub use event_forwarding::ForwardedEventNotification;
pub use get_event_information::{EventSummary, GetEventInformationAck, GetEventInformationRequest};

#[cfg(test)]
mod tests;
