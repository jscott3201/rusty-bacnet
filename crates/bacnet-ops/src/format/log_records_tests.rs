//! Event and audit notifications from Event Log and Audit Log records, as
//! typed data (#2103).

use bacnet_types::constructed::{AuditPropertyReference, BACnetAddress, NotificationParameters};
use bacnet_types::enums::{
    AuditOperation, ErrorClass, ErrorCode, EventState, EventType, NotifyType,
};
use bacnet_types::primitives::{BACnetTimeStamp, StatusFlags, Time};
use serde_json::json;

use super::*;

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn alarm() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 7,
        initiating_device_identifier: device(1201),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        notification_class: 3,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("Zone hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: Some(NotificationParameters::OutOfRange {
            exceeding_value: 101.5,
            status_flags: StatusFlags::IN_ALARM,
            deadband: 1.0,
            exceeded_limit: 100.0,
        }),
    }
}

/// Every field named or typed; the message stays text, and the event
/// values come from the shared renderer.
#[test]
fn an_event_notification_names_its_fields() {
    let data = EventNotificationData::new(&alarm());
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["process_identifier"], json!(7));
    assert_eq!(json["initiating_device"]["instance"], json!(1201));
    assert_eq!(json["event_object"]["type"], json!("analog-input"));
    assert_eq!(json["event_type"]["name"], json!("out-of-range"));
    assert_eq!(json["notify_type"]["name"], json!("alarm"));
    assert_eq!(json["from_state"]["name"], json!("normal"));
    assert_eq!(json["to_state"]["name"], json!("high-limit"));
    assert_eq!(json["ack_required"], json!(true));
    assert_eq!(json["message_text"], json!("Zone hot"));
    assert_eq!(json["timestamp"]["element"], json!("time-stamp"));
    let values = EventValues::new(alarm().event_values.as_ref().unwrap());
    assert_eq!(data.event_values, values.data);
    assert_eq!(
        json["event_values"]["fields"]["exceeding_value"],
        json!(101.5)
    );
    assert!(data.message_text.may_hold_device_text());

    let bare = EventNotificationData::new(&EventNotificationRequest {
        message_text: None,
        event_values: None,
        ..alarm()
    });
    assert_eq!(
        (bare.message_text, bare.event_values),
        (Datum::Null, Datum::Null)
    );

    // An acknowledgment carries no from-state or ack-required.
    let ack = EventNotificationData::new(&EventNotificationRequest {
        notify_type: NotifyType::ACK_NOTIFICATION,
        ..alarm()
    });
    assert_eq!(
        (ack.from_state, ack.ack_required),
        (Datum::Null, Datum::Null)
    );
    assert_eq!(ack.to_state, data.to_state);
}

fn write() -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: Some(BACnetTimeStamp::Time(Time {
            hour: 8,
            minute: 0,
            second: 0,
            hundredths: 0,
        })),
        target_timestamp: None,
        source_device: BACnetRecipient::Device(device(1)),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: Some("operator".into()),
        target_comment: None,
        invoke_id: Some(9),
        source_user_id: Some(12),
        source_user_role: None,
        target_device: BACnetRecipient::Address(BACnetAddress {
            network_number: 5,
            mac_address: [0x0A, 0x01].as_slice().into(),
        }),
        target_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }),
        target_priority: Some(8),
        // REAL 21.5 and 20.0, application tagged.
        target_value: Some(vec![0x44, 0x41, 0xAC, 0x00, 0x00]),
        current_value: Some(vec![0x44, 0x41, 0xA0, 0x00, 0x00]),
        result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    }
}

/// The operation and property by name, the values decoded as the target
/// property's, an address recipient as a plain element, the comments as
/// text, and the error named.
#[test]
fn an_audit_notification_names_and_decodes_its_fields() {
    let data = AuditNotificationData::new(&write());
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["operation"], json!({ "value": 1, "name": "write" }));
    assert_eq!(json["source_device"]["instance"], json!(1));
    assert_eq!(
        json["target_device"],
        json!({ "element": "recipient", "text": "network 5 mac 0a:01" })
    );
    assert!(!data.target_device.may_hold_device_text());
    assert_eq!(json["target_object"]["type"], json!("analog-value"));
    assert_eq!(json["target_property"]["name"], json!("present-value"));
    assert_eq!(json["target_priority"], json!(8));
    assert_eq!(json["target_value"], json!(21.5));
    assert_eq!(json["current_value"], json!(20.0));
    assert_eq!(json["source_comment"], json!("operator"));
    assert_eq!(json["target_comment"], json!(null));
    assert_eq!(json["source_user_id"], json!(12));
    assert_eq!(json["invoke_id"], json!(9));
    assert_eq!(json["result"]["error_class"]["name"], json!("property"));
    assert_eq!(
        json["result"]["error_code"]["name"],
        json!("write-access-denied")
    );

    // Unknown operations keep their number; values with no property to
    // decode them by stay octets.
    let odd = AuditNotificationData::new(&BACnetAuditNotification {
        operation: AuditOperation::from_raw(99),
        target_property: None,
        result: None,
        ..write()
    });
    assert_eq!(odd.operation, by_name(99, None));
    assert_eq!(
        odd.target_value,
        Datum::Octets {
            octets: "4441ac0000".into()
        }
    );
    assert_eq!(odd.result, None);
}
