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
    if let PropertyValue::List(items) = val {
        assert!(items.is_empty());
    } else {
        panic!("Expected List");
    }
}

#[test]
fn add_destination_device_and_read_back() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(make_dest_device(99));

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let PropertyValue::List(outer) = val else {
        panic!("Expected outer List");
    };
    assert_eq!(outer.len(), 1);

    let PropertyValue::List(fields) = &outer[0] else {
        panic!("Expected inner List");
    };
    // 7 fields: valid_days, from_time, to_time, recipient, process_id, confirmed, transitions
    assert_eq!(fields.len(), 7);

    // valid_days bitstring: all days = 0b0111_1111 << 1 = 0b1111_1110 = 0xFE
    assert_eq!(
        fields[0],
        PropertyValue::BitString {
            unused_bits: 1,
            data: vec![0b1111_1110],
        }
    );

    // from_time
    assert_eq!(fields[1], PropertyValue::Time(make_time(0, 0)));

    // to_time
    assert_eq!(fields[2], PropertyValue::Time(make_time(23, 59)));

    // recipient = Device OID for instance 99
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    assert_eq!(fields[3], PropertyValue::ObjectIdentifier(dev_oid));

    // process_identifier
    assert_eq!(fields[4], PropertyValue::Unsigned(1));

    // issue_confirmed_notifications
    assert_eq!(fields[5], PropertyValue::Boolean(true));

    // transitions: all = 0b0000_0111 << 5 = 0b1110_0000 = 0xE0
    assert_eq!(
        fields[6],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
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
    nc.add_destination(dest);

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let PropertyValue::List(outer) = val else {
        panic!("Expected outer List");
    };
    assert_eq!(outer.len(), 1);

    let PropertyValue::List(fields) = &outer[0] else {
        panic!("Expected inner List");
    };

    // recipient = Address as List[Unsigned(network_number), OctetString(mac)]
    assert_eq!(
        fields[3],
        PropertyValue::List(vec![
            PropertyValue::Unsigned(0),
            PropertyValue::OctetString(mac.to_vec()),
        ])
    );

    // process_identifier = 42
    assert_eq!(fields[4], PropertyValue::Unsigned(42));

    // issue_confirmed = false
    assert_eq!(fields[5], PropertyValue::Boolean(false));

    // transitions: bit 0 only = 0b0000_0001 << 5 = 0b0010_0000 = 0x20
    assert_eq!(
        fields[6],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b0010_0000],
        }
    );
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
    let PropertyValue::List(outer) = val else {
        panic!("Expected List");
    };
    assert_eq!(outer.len(), 3);
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
