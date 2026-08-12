//! NotificationClass property read/write tests.
//!
//! Split out of `tests/mod.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::{make_dest_device, make_time};
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::MacAddr;

#[test]
fn object_type_is_notification_class() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    assert_eq!(
        nc.object_identifier().object_type(),
        ObjectType::NOTIFICATION_CLASS
    );
    assert_eq!(nc.object_identifier().instance_number(), 1);
}

#[test]
fn read_notification_class_number() {
    let nc = NotificationClass::new(42, "NC-42").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    if let PropertyValue::Unsigned(n) = val {
        assert_eq!(n, 42);
    } else {
        panic!("Expected Unsigned");
    }
}

#[test]
fn read_priority_array_index() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    // Index 0 = array length
    let len = nc
        .read_property(PropertyIdentifier::PRIORITY, Some(0))
        .unwrap();
    if let PropertyValue::Unsigned(n) = len {
        assert_eq!(n, 3);
    } else {
        panic!("Expected Unsigned");
    }

    // Index 1 = TO_OFFNORMAL priority (default 255)
    let p1 = nc
        .read_property(PropertyIdentifier::PRIORITY, Some(1))
        .unwrap();
    if let PropertyValue::Unsigned(n) = p1 {
        assert_eq!(n, 255);
    } else {
        panic!("Expected Unsigned");
    }
}

#[test]
fn read_priority_all() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::PRIORITY, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], PropertyValue::Unsigned(255));
        assert_eq!(items[1], PropertyValue::Unsigned(255));
        assert_eq!(items[2], PropertyValue::Unsigned(255));
    } else {
        panic!("Expected List");
    }
}

#[test]
fn read_priority_invalid_index() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.read_property(PropertyIdentifier::PRIORITY, Some(4));
    assert!(result.is_err());
}

#[test]
fn read_object_name() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::OBJECT_NAME, None)
        .unwrap();
    if let PropertyValue::CharacterString(s) = val {
        assert_eq!(s, "NC-1");
    } else {
        panic!("Expected CharacterString");
    }
}

#[test]
fn write_notification_class_number() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(99),
        None,
    )
    .unwrap();
    assert_eq!(nc.notification_class, 99);
}

#[test]
fn write_notification_class_wrong_type() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn property_list_contains_recipient_list() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let props = nc.property_list();
    assert!(props.contains(&PropertyIdentifier::NOTIFICATION_CLASS));
    assert!(props.contains(&PropertyIdentifier::PRIORITY));
    assert!(props.contains(&PropertyIdentifier::ACK_REQUIRED));
    assert!(props.contains(&PropertyIdentifier::RECIPIENT_LIST));
}

#[test]
fn read_ack_required_default() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::ACK_REQUIRED, None)
        .unwrap();
    if let PropertyValue::BitString { unused_bits, data } = val {
        assert_eq!(unused_bits, 5);
        assert_eq!(data, vec![0]); // all false
    } else {
        panic!("Expected BitString");
    }
}

#[test]
fn read_recipient_list_empty() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    // An empty BACnetLIST encodes to zero bytes.
    assert_eq!(val, PropertyValue::ApplicationData(Vec::new()));
}

#[test]
fn add_destination_device_and_read_back() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(make_dest_device(99));

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    // Full ASN.1 framing: a BACnetDestination SEQUENCE with untagged
    // (application-tagged, in-order) members and the recipient under
    // primitive context tag [0].
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![
            0x82, 0x01, 0xFE, // valid-days: all seven days MSB-first
            0xB4, 0x00, 0x00, 0x00, 0x00, // from_time 00:00:00.00
            0xB4, 0x17, 0x3B, 0x00, 0x00, // to_time 23:59:00.00
            0x0C, 0x02, 0x00, 0x00, 0x63, // recipient device [0]: (8<<22)|99
            0x21, 0x01, // process_identifier 1
            0x11, // issue_confirmed_notifications TRUE
            0x82, 0x05, 0xE0, // transitions: all three MSB-first
        ])
    );

    // …and it decodes back to the exact destination.
    let PropertyValue::ApplicationData(bytes) = &val else {
        unreachable!();
    };
    let decoded = bacnet_encoding::constructed::decode_destination_list(bytes).unwrap();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].valid_days, 0b0111_1111);
    assert_eq!(decoded[0].from_time, make_time(0, 0));
    assert_eq!(decoded[0].to_time, make_time(23, 59));
    assert_eq!(decoded[0].recipient, BACnetRecipient::Device(dev_oid));
    assert_eq!(decoded[0].process_identifier, 1);
    assert!(decoded[0].issue_confirmed_notifications);
    assert_eq!(decoded[0].transitions, 0b0000_0111);
}

#[test]
fn add_destination_address_variant() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let mac = MacAddr::from_slice(&[192u8, 168, 1, 100, 0xBA, 0xC0]);
    let dest = BACnetDestination {
        valid_days: 0b0011_1110, // Tue–Sat (bits 1..5)
        from_time: make_time(8, 0),
        to_time: make_time(17, 0),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: mac.clone(),
        }),
        process_identifier: 42,
        issue_confirmed_notifications: false,
        transitions: 0b0000_0001, // TO_OFFNORMAL only
    };
    nc.add_destination(dest.clone());

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    // address [1] is constructed BACnetAddress: opening tag 1 /
    // application-Unsigned16 network number / application-OctetString MAC /
    // closing tag 1. valid_days Tue–Sat is asymmetric under bit reversal,
    // so the 0x7C byte witnesses the MSB-first packing (Clause 20.2.10).
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![
            0x82, 0x01, 0x7C, // valid-days Tue–Sat
            0xB4, 0x08, 0x00, 0x00, 0x00, // from_time 08:00:00.00
            0xB4, 0x11, 0x00, 0x00, 0x00, // to_time 17:00:00.00
            0x1E, 0x21, 0x00, 0x65, 0x06, 0xC0, 0xA8, 0x01, 0x64, 0xBA, 0xC0,
            0x1F, // recipient address [1] (network 0 + MAC)
            0x21, 0x2A, // process_identifier 42
            0x10, // issue_confirmed_notifications FALSE
            0x82, 0x05, 0x80, // transitions: TO_OFFNORMAL only
        ])
    );
    let PropertyValue::ApplicationData(bytes) = &val else {
        unreachable!();
    };
    let decoded = bacnet_encoding::constructed::decode_destination_list(bytes).unwrap();
    assert_eq!(decoded, vec![dest]);
}

#[test]
fn add_multiple_destinations() {
    let mut nc = NotificationClass::new(5, "NC-5").unwrap();
    nc.add_destination(make_dest_device(100));
    nc.add_destination(make_dest_device(200));
    nc.add_destination(make_dest_device(300));

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let PropertyValue::ApplicationData(bytes) = &val else {
        panic!("Expected ApplicationData");
    };
    let decoded = bacnet_encoding::constructed::decode_destination_list(bytes).unwrap();
    assert_eq!(decoded.len(), 3);
    let instances: Vec<u32> = decoded
        .iter()
        .map(|d| match &d.recipient {
            BACnetRecipient::Device(oid) => oid.instance_number(),
            other => panic!("expected Device recipient, got {other:?}"),
        })
        .collect();
    assert_eq!(instances, vec![100, 200, 300]);
}

#[test]
fn write_recipient_list_clears_existing() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(make_dest_device(10));
    nc.add_destination(make_dest_device(20));
    assert_eq!(nc.recipient_list.len(), 2);

    // Write an empty list — should clear
    nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::List(vec![]),
        None,
    )
    .unwrap();
    assert!(nc.recipient_list.is_empty());
}

#[test]
fn write_recipient_list_wrong_type_denied() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::Unsigned(0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn write_recipient_list_round_trip() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(make_dest_device(10));
    // Read the encoded list, then write it back
    let encoded = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, encoded, None)
        .unwrap();
    assert_eq!(nc.recipient_list.len(), 1);
    assert_eq!(nc.recipient_list[0].process_identifier, 1);
}

#[test]
fn read_event_state_default() {
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let val = nc
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn write_out_of_service() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let val = nc
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn write_unknown_property_denied() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
