//! Event Log and Audit Log records' notifications as typed data (#2103):
//! an event notification (Clause 12.27.13: the notification's parameters,
//! its event values through [`EventValues`]) and an audit notification
//! (Clause 12.64.10), every field a [`Datum`] with its enumerations named,
//! objects as objects and timestamps rendered.
//!
//! Device text stays a [`Datum::Text`] (an event's message, an audit's
//! comments), and the event values' element is not plain, so a front end
//! that shows device text as data (the MCP server's envelopes) finds every
//! string a device wrote by the datum's form alone.

use bacnet_types::constructed::{
    BACnetAuditNotification, BACnetRecipient, EventNotificationRequest,
};
use bacnet_types::enums::{NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde::Serialize;

use super::alarms::timestamp;
use super::notification::EventValues;
use super::primitive::hex;
use super::{Datum, Floats, StateTexts, ValueFormat};
use crate::label::kebab;

/// An Event Log record's event notification, decoded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventNotificationData {
    /// The notification's process identifier, a number.
    pub process_identifier: Datum,
    /// The device that sent it, a [`Datum::Object`].
    pub initiating_device: Datum,
    /// The object whose event it reports.
    pub event_object: Datum,
    /// When the transition happened, a `time-stamp` element.
    pub timestamp: Datum,
    /// Its notification class, a number.
    pub notification_class: Datum,
    /// Its priority, a number.
    pub priority: Datum,
    /// The event algorithm, named.
    pub event_type: Datum,
    /// The message, as the device wrote it; [`Datum::Null`] when absent.
    pub message_text: Datum,
    /// Alarm, event or ack-notification.
    pub notify_type: Datum,
    /// Whether the recipient was asked to acknowledge it; [`Datum::Null`]
    /// for an ack-notification, which doesn't carry it.
    pub ack_required: Datum,
    /// The state before the transition, named; [`Datum::Null`] for an
    /// ack-notification, which doesn't carry it either.
    pub from_state: Datum,
    /// The state after it, named.
    pub to_state: Datum,
    /// The event values, a `notification-parameters` element decoded
    /// field by field ([`EventValues`]); [`Datum::Null`] when absent.
    pub event_values: Datum,
}

/// An Audit Log record's audit notification, decoded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditNotificationData {
    /// The audited operation, named.
    pub operation: Datum,
    /// Who asked for it: a device as an object, or a `recipient` address.
    pub source_device: Datum,
    /// The object that asked, when given.
    pub source_object: Datum,
    /// When the source sent it, a `time-stamp` element, when given.
    pub source_timestamp: Datum,
    /// The source's comment, as the device wrote it.
    pub source_comment: Datum,
    /// The source's user identifier, a number, when given.
    pub source_user_id: Datum,
    /// The source's user role, a number, when given.
    pub source_user_role: Datum,
    /// The invoke ID of the audited request, when given.
    pub invoke_id: Datum,
    /// The device the operation targeted.
    pub target_device: Datum,
    /// The object it targeted, when given.
    pub target_object: Datum,
    /// The property it targeted, named, when given.
    pub target_property: Datum,
    /// The array index it targeted, when given.
    pub target_array_index: Datum,
    /// The priority it wrote at, when given.
    pub target_priority: Datum,
    /// When the target logged it, a `time-stamp` element, when given.
    pub target_timestamp: Datum,
    /// The target's comment, as the device wrote it.
    pub target_comment: Datum,
    /// The value written, decoded as the target property's.
    pub target_value: Datum,
    /// The value the target held, decoded the same way.
    pub current_value: Datum,
    /// The error the operation ended in, when it failed: `error_class` and
    /// `error_code`, named.
    pub result: Option<AuditResult>,
}

/// An audited operation's error.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditResult {
    /// The error class, named.
    pub error_class: Datum,
    /// The error code, named.
    pub error_code: Datum,
}

/// The formatter the records name values with: no state texts, every
/// digit, for objects of `object_type`.
fn named<R>(object_type: ObjectType, f: impl FnOnce(&ValueFormat<'_>) -> R) -> R {
    let texts = StateTexts::default();
    f(&ValueFormat::new(object_type, &texts, Floats::Exact).whole_lists())
}

fn object(format: &ValueFormat<'_>, object: ObjectIdentifier) -> Datum {
    format.element_data(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        &PropertyValue::ObjectIdentifier(object),
    )
}

fn enumerated(format: &ValueFormat<'_>, property: PropertyIdentifier, raw: u32) -> Datum {
    format.element_data(property, &PropertyValue::Enumerated(raw))
}

/// An enumeration named by its constant, kebab case, or bare when unknown.
fn by_name(value: u32, name: Option<&str>) -> Datum {
    Datum::Enumerated {
        value,
        name: name.map(kebab),
    }
}

fn number(value: Option<impl Into<u64>>) -> Datum {
    value.map_or(Datum::Null, |n| Datum::Unsigned(n.into()))
}

fn text(text: Option<&String>) -> Datum {
    text.map_or(Datum::Null, |text| Datum::Text(text.clone()))
}

fn recipient(format: &ValueFormat<'_>, recipient: &BACnetRecipient) -> Datum {
    match recipient {
        BACnetRecipient::Device(device) => object(format, *device),
        BACnetRecipient::Address(address) => Datum::Constructed {
            element: "recipient".into(),
            text: format!(
                "network {} mac {}",
                address.network_number,
                hex(&address.mac_address, ":")
            ),
            fields: None,
        },
    }
}

impl EventNotificationData {
    /// One event notification, decoded.
    pub fn new(notification: &EventNotificationRequest) -> Self {
        // An acknowledgment's record leaves these out on the wire, so the
        // decoded request holds placeholders (`EventLogDatum::Notification`).
        let ack = notification.notify_type == NotifyType::ACK_NOTIFICATION;
        named(ObjectType::DEVICE, |format| Self {
            process_identifier: Datum::Unsigned(u64::from(notification.process_identifier)),
            initiating_device: object(format, notification.initiating_device_identifier),
            event_object: object(format, notification.event_object_identifier),
            timestamp: timestamp(&notification.timestamp),
            notification_class: Datum::Unsigned(u64::from(notification.notification_class)),
            priority: Datum::Unsigned(u64::from(notification.priority)),
            event_type: enumerated(
                format,
                PropertyIdentifier::EVENT_TYPE,
                notification.event_type.to_raw(),
            ),
            message_text: text(notification.message_text.as_ref()),
            notify_type: enumerated(
                format,
                PropertyIdentifier::NOTIFY_TYPE,
                notification.notify_type.to_raw(),
            ),
            ack_required: if ack {
                Datum::Null
            } else {
                Datum::Boolean(notification.ack_required)
            },
            from_state: if ack {
                Datum::Null
            } else {
                enumerated(
                    format,
                    PropertyIdentifier::EVENT_STATE,
                    notification.from_state.to_raw(),
                )
            },
            to_state: enumerated(
                format,
                PropertyIdentifier::EVENT_STATE,
                notification.to_state.to_raw(),
            ),
            event_values: notification
                .event_values
                .as_ref()
                .map_or(Datum::Null, |values| EventValues::new(values).data),
        })
    }
}

impl AuditNotificationData {
    /// One audit notification, decoded. The values written and held decode
    /// as the target property's, of the target object's type.
    pub fn new(notification: &BACnetAuditNotification) -> Self {
        let object_type = notification
            .target_object
            .map_or(ObjectType::DEVICE, |target| target.object_type());
        let property = notification.target_property.as_ref();
        named(object_type, |format| {
            let value = |data: Option<&Vec<u8>>| match (data, property) {
                (Some(data), Some(property)) => format.data(property.property_identifier, data),
                (Some(data), None) => Datum::Octets {
                    octets: hex(data, ""),
                },
                (None, _) => Datum::Null,
            };
            let maybe_object = |target: Option<ObjectIdentifier>| {
                target.map_or(Datum::Null, |target| object(format, target))
            };
            Self {
                operation: by_name(
                    notification.operation.to_raw(),
                    notification.operation.name(),
                ),
                source_device: recipient(format, &notification.source_device),
                source_object: maybe_object(notification.source_object),
                source_timestamp: notification
                    .source_timestamp
                    .as_ref()
                    .map_or(Datum::Null, timestamp),
                source_comment: text(notification.source_comment.as_ref()),
                source_user_id: number(notification.source_user_id),
                source_user_role: number(notification.source_user_role),
                invoke_id: number(notification.invoke_id),
                target_device: recipient(format, &notification.target_device),
                target_object: maybe_object(notification.target_object),
                target_property: property.map_or(Datum::Null, |property| {
                    by_name(
                        property.property_identifier.to_raw(),
                        property.property_identifier.name(),
                    )
                }),
                target_array_index: number(property.and_then(|p| p.property_array_index)),
                target_priority: number(notification.target_priority),
                target_timestamp: notification
                    .target_timestamp
                    .as_ref()
                    .map_or(Datum::Null, timestamp),
                target_comment: text(notification.target_comment.as_ref()),
                target_value: value(notification.target_value.as_ref()),
                current_value: value(notification.current_value.as_ref()),
                result: notification.result.map(|(class, code)| AuditResult {
                    error_class: by_name(u32::from(class.to_raw()), class.name()),
                    error_code: by_name(u32::from(code.to_raw()), code.name()),
                }),
            }
        })
    }
}

#[cfg(test)]
#[path = "log_records_tests.rs"]
mod tests;
