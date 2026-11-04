//! Generated `constructed_decode` seeds: one value through each codec that
//! no harvested seed reaches.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bacnet_encoding::constructed::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits, LogStatus};
use bacnet_types::constructed::*;
use bacnet_types::enums::{
    AccessAuthenticationFactorDisable, AuthenticationFactorType, EventState, EventType, NotifyType,
    ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue, StatusFlags};

use super::{Seed, date, encoded, object, selected, time};
use crate::constructed::CODECS;

fn device_recipient() -> BACnetRecipient {
    BACnetRecipient::Device(object(ObjectType::DEVICE, 1001))
}

fn address_recipient() -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 5,
        mac_address: [10, 0, 0, 7, 0xBA, 0xC0][..].into(),
    })
}

fn property_reference() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference {
        object_identifier: object(ObjectType::ANALOG_VALUE, 2),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: Some(4),
    }
}

fn device_property_reference() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: object(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: Some(object(ObjectType::DEVICE, 1001)),
    }
}

fn device_object_reference() -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(object(ObjectType::DEVICE, 1001)),
        object_identifier: object(ObjectType::ACCESS_POINT, 3),
    }
}

fn time_value() -> BACnetTimeValue {
    BACnetTimeValue {
        time: time(),
        value: PropertyValue::Real(72.5),
    }
}

fn special_event() -> BACnetSpecialEvent {
    BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(date())),
        list_of_time_values: vec![time_value()],
        event_priority: 10,
    }
}

fn authentication_factor() -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::from_raw(1),
        format_class: 2,
        value: vec![0x12, 0x34, 0x56],
    }
}

fn status_flags() -> StatusFlags {
    StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE
}

/// A notification with each optional field present.
pub(crate) fn event_notification() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 9,
        initiating_device_identifier: object(ObjectType::DEVICE, 1001),
        event_object_identifier: object(ObjectType::ANALOG_INPUT, 1),
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        notification_class: 3,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("over the limit".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: Some(out_of_range()),
    }
}

fn out_of_range() -> NotificationParameters {
    NotificationParameters::OutOfRange {
        exceeding_value: 81.5,
        status_flags: status_flags(),
        deadband: 1.0,
        exceeded_limit: 80.0,
    }
}

fn action_command() -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: Some(object(ObjectType::DEVICE, 1001)),
        object_identifier: object(ObjectType::BINARY_OUTPUT, 4),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Enumerated(1),
        priority: Some(8),
        post_delay: Some(5),
        quit_on_failure: true,
        write_successful: false,
    }
}

fn host_n_port() -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))),
        port: 47808,
    }
}

/// The seeds, by codec label.
pub(super) fn seeds() -> Vec<Seed> {
    let recipient_process = BACnetRecipientProcess {
        recipient: device_recipient(),
        process_identifier: 77,
    };
    let destination = BACnetDestination {
        valid_days: DaysOfWeek::MONDAY | DaysOfWeek::FRIDAY,
        from_time: time(),
        to_time: time(),
        recipient: address_recipient(),
        process_identifier: 12,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    };
    let entries: Vec<(&str, Vec<u8>)> = vec![
        (
            "BACnetRecipient",
            encoded(|buf| encode_recipient(buf, &address_recipient()).unwrap()),
        ),
        (
            "BACnetDestination",
            encoded(|buf| encode_destination(buf, &destination).unwrap()),
        ),
        (
            "BACnetDestination list",
            encoded(|buf| {
                encode_destination_list(buf, &[destination.clone(), destination.clone()]).unwrap()
            }),
        ),
        (
            "BACnetPropertyAccessResult",
            encoded(|buf| {
                encode_property_access_result(
                    buf,
                    &BACnetPropertyAccessResult {
                        reference: device_property_reference(),
                        access_result: AccessResult::Value(PropertyValue::Real(3.5)),
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetxyColor",
            encoded(|buf| {
                encode_xy_color(
                    buf,
                    &BACnetXyColor {
                        x: 0.3127,
                        y: 0.329,
                    },
                )
            }),
        ),
        (
            "BACnetAuditLogRecord",
            encoded(|buf| {
                encode_audit_log_record(
                    &BACnetAuditLogRecord {
                        timestamp: (date(), time()),
                        datum: BACnetAuditLogDatum::TimeChange(1.5),
                    },
                    buf,
                )
                .unwrap()
            }),
        ),
        (
            "BACnetCOVSubscription",
            encoded(|buf| {
                encode_cov_subscription(
                    buf,
                    &BACnetCOVSubscription {
                        recipient: recipient_process.clone(),
                        monitored_property_reference: property_reference(),
                        issue_confirmed_notifications: true,
                        time_remaining: 300,
                        cov_increment: Some(0.5),
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetCOVMultipleSubscription",
            encoded(|buf| {
                encode_cov_multiple_subscription(
                    buf,
                    &BACnetCOVMultipleSubscription {
                        recipient: recipient_process.clone(),
                        issue_confirmed_notifications: false,
                        time_remaining: 60,
                        max_notification_delay: 5,
                        list_of_cov_subscription_specifications: vec![
                            BACnetCOVSubscriptionSpecification {
                                monitored_object_identifier: object(ObjectType::ANALOG_INPUT, 1),
                                list_of_cov_references: vec![BACnetCOVReference {
                                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                                    property_array_index: None,
                                    cov_increment: Some(0.1),
                                    timestamped: true,
                                }],
                            },
                        ],
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetAuthenticationPolicy",
            encoded(|buf| {
                encode_authentication_policy(
                    buf,
                    &BACnetAuthenticationPolicy {
                        policy: vec![BACnetAuthenticationPolicyEntry {
                            credential_data_input: device_object_reference(),
                            index: 1,
                        }],
                        order_enforced: true,
                        timeout: 30,
                    },
                )
            }),
        ),
        (
            "BACnetHostAddress",
            encoded(|buf| {
                encode_host_address(buf, &BACnetHostAddress::Name("bms.example".into())).unwrap()
            }),
        ),
        (
            "BACnetHostNPort",
            encoded(|buf| encode_host_n_port(buf, &host_n_port()).unwrap()),
        ),
        (
            "BACnetBDTEntry",
            encoded(|buf| {
                encode_bdt_entry(
                    buf,
                    &BACnetBDTEntry {
                        bbmd_address: host_n_port(),
                        broadcast_mask: Some([255, 255, 255, 0]),
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetFDTEntry",
            encoded(|buf| {
                encode_fdt_entry(
                    buf,
                    &BACnetFDTEntry {
                        address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)), 47809),
                        time_to_live: 600,
                        remaining_time_to_live: 630,
                    },
                )
            }),
        ),
        (
            "BACnetActionCommand",
            encoded(|buf| encode_action_command(buf, &action_command()).unwrap()),
        ),
        (
            "BACnetActionList",
            encoded(|buf| {
                encode_action_list(
                    buf,
                    &BACnetActionList {
                        commands: vec![action_command(), action_command()],
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetAssignedAccessRights",
            encoded(|buf| {
                encode_assigned_access_rights(
                    buf,
                    &BACnetAssignedAccessRights {
                        assigned_access_rights: device_object_reference(),
                        enable: true,
                    },
                )
            }),
        ),
        (
            "BACnetAuthenticationFactor",
            encoded(|buf| encode_authentication_factor(buf, &authentication_factor())),
        ),
        (
            "BACnetCredentialAuthenticationFactor",
            encoded(|buf| {
                encode_credential_authentication_factor(
                    buf,
                    &BACnetCredentialAuthenticationFactor {
                        disable: AccessAuthenticationFactorDisable::from_raw(0),
                        authentication_factor: authentication_factor(),
                    },
                )
            }),
        ),
        (
            "BACnetLogMultipleRecord",
            encoded(|buf| {
                encode_log_multiple_record(
                    &BACnetLogMultipleRecord {
                        date: date(),
                        time: time(),
                        log_data: LogData::LogStatus(LogStatus::BUFFER_PURGED),
                    },
                    buf,
                )
                .unwrap()
            }),
        ),
        (
            "BACnetStageLimitValue",
            encoded(|buf| {
                encode_stage_limit_value(
                    buf,
                    &BACnetStageLimitValue {
                        limit: 50.0,
                        values: vec![true, false, true],
                        deadband: 2.0,
                    },
                )
            }),
        ),
        (
            "BACnetDeviceObjectReference",
            encoded(|buf| encode_device_object_reference(buf, &device_object_reference())),
        ),
        (
            "BACnetDeviceObjectPropertyReference",
            encoded(|buf| {
                encode_device_object_property_reference(buf, &device_property_reference())
            }),
        ),
        (
            "BACnetTimeValue",
            encoded(|buf| encode_time_value(buf, &time_value()).unwrap()),
        ),
        (
            "BACnetSpecialEvent period",
            encoded(|buf| {
                encode_special_event_period(
                    buf,
                    &SpecialEventPeriod::CalendarReference(object(ObjectType::CALENDAR, 1)),
                )
            }),
        ),
        (
            "BACnetSpecialEvent",
            encoded(|buf| encode_special_event(buf, &special_event()).unwrap()),
        ),
        (
            "BACnetDailySchedule",
            encoded(|buf| encode_daily_schedule(buf, &[time_value(), time_value()]).unwrap()),
        ),
        (
            "Exception_Schedule",
            encoded(|buf| encode_exception_schedule(buf, &[special_event()]).unwrap()),
        ),
        (
            "BACnetLogRecord",
            encoded(|buf| {
                encode_log_record(
                    &BACnetLogRecord {
                        date: date(),
                        time: time(),
                        log_datum: LogDatum::RealValue(19.25),
                        status_flags: Some(status_flags()),
                    },
                    buf,
                )
                .unwrap()
            }),
        ),
        (
            "BACnetEventNotificationSubscription",
            encoded(|buf| {
                encode_event_notification_subscription(
                    buf,
                    &BACnetEventNotificationSubscription {
                        recipient: device_recipient(),
                        process_identifier: 4,
                        issue_confirmed_notifications: true,
                        time_remaining: 120,
                    },
                )
                .unwrap()
            }),
        ),
        (
            "BACnetObjectPropertyReference",
            encoded(|buf| encode_object_property_reference(buf, &property_reference())),
        ),
        (
            "BACnetEventLogRecord",
            encoded(|buf| {
                encode_event_log_record(
                    &BACnetEventLogRecord {
                        date: date(),
                        time: time(),
                        log_datum: EventLogDatum::Notification(event_notification()),
                    },
                    buf,
                )
                .unwrap()
            }),
        ),
        (
            "ReadAccessSpecification",
            encoded(|buf| {
                encode_read_access_specification(
                    buf,
                    &ReadAccessSpecification {
                        object_identifier: object(ObjectType::DEVICE, 1001),
                        list_of_property_references: vec![
                            PropertyReference {
                                property_identifier: PropertyIdentifier::OBJECT_NAME,
                                property_array_index: None,
                            },
                            PropertyReference {
                                property_identifier: PropertyIdentifier::OBJECT_LIST,
                                property_array_index: Some(0),
                            },
                        ],
                    },
                )
            }),
        ),
        (
            "BACnetPropertyValue",
            encoded(|buf| {
                encode_bacnet_property_value(
                    &BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: Some(2),
                        value: encoded(|value| {
                            encode_property_value(value, &PropertyValue::Unsigned(300)).unwrap()
                        }),
                        priority: Some(16),
                    },
                    buf,
                )
                .unwrap()
            }),
        ),
        (
            "BACnetNotificationParameters",
            encoded(|buf| encode_notification_parameters(&out_of_range(), buf).unwrap()),
        ),
        (
            "event notification, tolerant",
            encoded(|buf| encode_event_notification(&event_notification(), buf).unwrap()),
        ),
    ];
    entries
        .into_iter()
        .map(|(label, body)| {
            let octets = selected(CODECS, label, &body);
            Seed::new(
                "constructed_decode",
                format!("{:02}-{}", octets[0], slug(label)),
                octets,
            )
        })
        .collect()
}

/// A label as a file name: lowercase, with runs of anything else as one
/// hyphen.
pub(crate) fn slug(label: &str) -> String {
    let mut name = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    name.trim_matches('-').to_owned()
}
