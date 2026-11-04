//! Generated `service_decode` seeds: one value through each codec that no
//! harvested seed reaches, and requests with every optional field present.

use bacnet_encoding::constructed::encode_event_notification;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::alarm_event::{EventSummary, GetEventInformationAck};
use bacnet_services::audit::AuditLogQueryRequest;
use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue, COVReference,
    COVSubscriptionSpecification, SubscribeCOVPropertyMultipleError,
    SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicWriteFileAck, FileReadAckMethod, FileWriteAckMethod,
};
use bacnet_services::list_manipulation::{ChangeListError, ListElementRequest};
use bacnet_services::object_mgmt::CreateObjectError;
use bacnet_services::private_transfer::PrivateTransferError;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::virtual_terminal::VTCloseError;
use bacnet_services::wpm::WritePropertyMultipleError;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetAuditLogQueryParameters, BACnetObjectPropertyReference, PropertyReference,
};
use bacnet_types::enums::{
    BACnetSuccessFilter, ErrorClass, ErrorCode, EventState, NotifyType, ObjectType,
    PropertyIdentifier,
};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use super::constructed::{event_notification, slug};
use super::{Seed, date, encoded, object, selected, time};
use crate::service::CODECS;

fn value(value: PropertyValue) -> Vec<u8> {
    encoded(|buf| encode_property_value(buf, &value).unwrap())
}

fn reference() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference {
        object_identifier: object(ObjectType::ANALOG_OUTPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: Some(3),
    }
}

/// The seeds, by codec label.
pub(super) fn seeds() -> Vec<Seed> {
    let notification =
        encoded(|buf| encode_event_notification(&event_notification(), buf).unwrap());
    let read_property_ack = ReadPropertyACK {
        object_identifier: object(ObjectType::MULTI_STATE_VALUE, 2),
        property_identifier: PropertyIdentifier::STATE_TEXT,
        property_array_index: Some(1),
        property_value: value(PropertyValue::CharacterString("Occupied".into())),
    };
    let summary = EventSummary {
        object_identifier: object(ObjectType::ANALOG_INPUT, 1),
        event_state: EventState::HIGH_LIMIT,
        acknowledged_transitions: EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL,
        event_timestamps: [
            BACnetTimeStamp::Time(time()),
            BACnetTimeStamp::SequenceNumber(5),
            BACnetTimeStamp::DateTime {
                date: date(),
                time: time(),
            },
        ],
        notify_type: NotifyType::ALARM,
        event_enable: EventTransitionBits::all(),
        event_priorities: [10, 20, 30],
    };
    let entries: Vec<(&str, Vec<u8>)> = vec![
        ("ConfirmedEventNotification-Request", notification.clone()),
        (
            "RemoveListElement-Request",
            encoded(|buf| {
                ListElementRequest {
                    object_identifier: object(ObjectType::NOTIFICATION_CLASS, 1),
                    property_identifier: PropertyIdentifier::RECIPIENT_LIST,
                    property_array_index: None,
                    list_of_elements: value(PropertyValue::Unsigned(7)),
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        (
            "SubscribeCOVPropertyMultiple-Request",
            encoded(|buf| {
                SubscribeCOVPropertyMultipleRequest {
                    subscriber_process_identifier: 18,
                    issue_confirmed_notifications: true,
                    lifetime: Some(3600),
                    max_notification_delay: Some(10),
                    list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                        monitored_object_identifier: object(ObjectType::ANALOG_INPUT, 1),
                        list_of_cov_references: vec![COVReference {
                            monitored_property: PropertyReference {
                                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                                property_array_index: None,
                            },
                            cov_increment: Some(0.25),
                            timestamped: true,
                        }],
                    }],
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        (
            "ConfirmedCOVNotificationMultiple-Request",
            encoded(|buf| {
                COVNotificationMultipleRequest {
                    subscriber_process_identifier: 18,
                    initiating_device_identifier: object(ObjectType::DEVICE, 1001),
                    time_remaining: 120,
                    timestamp: Some((date(), time())),
                    list_of_cov_notifications: vec![COVNotificationItem {
                        monitored_object_identifier: object(ObjectType::ANALOG_INPUT, 1),
                        list_of_values: vec![COVNotificationValue {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            value: value(PropertyValue::Real(22.0)),
                            time_of_change: Some(time()),
                        }],
                    }],
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        (
            "AuditLogQuery-Request",
            encoded(|buf| {
                AuditLogQueryRequest {
                    audit_log: object(ObjectType::AUDIT_LOG, 1),
                    query_parameters: BACnetAuditLogQueryParameters::BySource {
                        source_device_identifier: object(ObjectType::DEVICE, 1001),
                        source_device_address: None,
                        source_object_identifier: Some(object(ObjectType::ANALOG_VALUE, 1)),
                        operations: None,
                        successful_actions_only: BACnetSuccessFilter::SUCCESSES_ONLY,
                    },
                    start_at_sequence_number: Some(100),
                    requested_count: 10,
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        (
            "AtomicReadFile-ACK",
            encoded(|buf| {
                AtomicReadFileAck {
                    end_of_file: false,
                    access: FileReadAckMethod::Record {
                        file_start_record: 4,
                        returned_record_count: 2,
                        file_record_data: vec![b"line one".to_vec(), b"line two".to_vec()],
                    },
                }
                .encode(buf)
            }),
        ),
        (
            "AtomicWriteFile-ACK",
            encoded(|buf| {
                AtomicWriteFileAck {
                    access: FileWriteAckMethod::Stream {
                        file_start_position: 1024,
                    },
                }
                .encode(buf)
            }),
        ),
        (
            "GetEventInformation-ACK",
            encoded(|buf| {
                GetEventInformationAck {
                    list_of_event_summaries: vec![summary],
                    more_events: true,
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        (
            "ChangeList-Error",
            encoded(|buf| {
                ChangeListError {
                    error_class: ErrorClass::SERVICES,
                    error_code: ErrorCode::INVALID_DATA_TYPE,
                    first_failed_element_number: 2,
                }
                .encode(buf)
            }),
        ),
        (
            "CreateObject-Error",
            encoded(|buf| {
                CreateObjectError {
                    error_class: ErrorClass::OBJECT,
                    error_code: ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
                    first_failed_element_number: 0,
                }
                .encode(buf)
            }),
        ),
        (
            "ConfirmedPrivateTransfer-Error",
            encoded(|buf| {
                PrivateTransferError {
                    error_class: ErrorClass::SERVICES,
                    error_code: ErrorCode::OTHER,
                    vendor_id: 555,
                    service_number: 3,
                    error_parameters: Some(value(PropertyValue::Unsigned(9))),
                }
                .encode(buf)
            }),
        ),
        (
            "SubscribeCOVPropertyMultiple-Error",
            encoded(|buf| {
                SubscribeCOVPropertyMultipleError {
                    error_class: ErrorClass::OBJECT,
                    error_code: ErrorCode::UNKNOWN_OBJECT,
                    first_failed_subscription: Some(reference()),
                }
                .encode(buf)
            }),
        ),
        (
            "VT-Close-Error",
            encoded(|buf| {
                VTCloseError {
                    error_class: ErrorClass::VT,
                    error_code: ErrorCode::UNKNOWN_VT_SESSION,
                    list_of_vt_session_identifiers: Some(vec![1, 4]),
                }
                .encode(buf)
            }),
        ),
        (
            "WritePropertyMultiple-Error",
            encoded(|buf| {
                WritePropertyMultipleError {
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::WRITE_ACCESS_DENIED,
                    first_failed_write_attempt: reference(),
                }
                .encode(buf)
            }),
        ),
        ("ForwardedEventNotification", notification),
        (
            "ReadProperty-ACK, borrowed",
            encoded(|buf| read_property_ack.encode(buf).unwrap()),
        ),
        (
            "WriteProperty-Request",
            encoded(|buf| {
                WritePropertyRequest {
                    object_identifier: object(ObjectType::ANALOG_VALUE, 1),
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: value(PropertyValue::Real(0.0)),
                    priority: Some(16),
                }
                .encode(buf)
                .unwrap()
            }),
        ),
        // Written out by hand: ReadProperty with an array index,
        // ReadPropertyMultiple, SubscribeCOV and WritePropertyMultiple.
        (
            "ReadProperty-Request",
            vec![0x0C, 0x02, 0x00, 0x00, 0x05, 0x19, 0x57, 0x29, 0x03],
        ),
        (
            "ReadPropertyMultiple-Request",
            vec![
                0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x09, 0x55, 0x09, 0x6F, 0x1F,
            ],
        ),
        (
            "SubscribeCOV-Request",
            vec![
                0x09, 0x12, 0x1C, 0x00, 0x00, 0x00, 0x01, 0x29, 0x01, 0x39, 0x3C,
            ],
        ),
        (
            "WritePropertyMultiple-Request",
            vec![
                0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x09, 0x55, 0x2E, 0x44, 0x42, 0x91, 0x00, 0x00,
                0x2F, 0x39, 0x08, 0x1F,
            ],
        ),
    ];
    entries
        .into_iter()
        .map(|(label, body)| {
            let octets = selected(CODECS, label, &body);
            Seed::new(
                "service_decode",
                format!("{:02}-{}", octets[0], slug(label)),
                octets,
            )
        })
        .collect()
}
