//! `Recipient_List` address-recipient round-trip tests for #126.
//!
//! These cover preserving the `BACnetAddress` network number through the
//! read/write round-trip (previously dropped on encode, reconstructed as 0 on
//! decode), across local/remote/broadcast forms and malformed-input rejection.
//! Split out of `tests.rs` to keep both files under the 700-LOC cap.

use super::super::*;
use super::make_time;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::MacAddr;

/// Build an address destination with the given network number and MAC.
fn make_dest_address(network_number: u16, mac: &[u8]) -> BACnetDestination {
    BACnetDestination {
        valid_days: 0b0111_1111,
        from_time: make_time(0, 0),
        to_time: make_time(23, 59),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number,
            mac_address: MacAddr::from_slice(mac),
        }),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: 0b0000_0111,
    }
}

#[test]
fn recipient_address_preserves_network_number_all_forms() {
    // #126: an address recipient must carry its network number through the
    // read/write round-trip. Previously the network number was dropped on
    // encode and reconstructed as 0 on decode, so a recipient on a nonzero
    // BACnet network could not round-trip. Covers the core nonzero case plus
    // broadcast (net 65535, empty MAC), local (net 0), and remote (net 1000).
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let mac = MacAddr::from_slice(&[192u8, 168, 1, 100, 0xBA, 0xC0]);
    nc.add_destination(make_dest_address(0xBAC0, &mac.to_vec()));

    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let PropertyValue::List(outer) = &val else {
        panic!("Expected outer List")
    };
    let PropertyValue::List(fields) = &outer[0] else {
        panic!("Expected entry List")
    };
    assert_eq!(
        fields[3],
        PropertyValue::List(vec![
            PropertyValue::Unsigned(0xBAC0),
            PropertyValue::OctetString(mac.to_vec()),
        ])
    );
    nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, val, None)
        .unwrap();
    match &nc.recipient_list[0].recipient {
        BACnetRecipient::Address(addr) => {
            assert_eq!(addr.network_number, 0xBAC0, "network number must survive");
            assert_eq!(addr.mac_address, mac);
        }
        other => panic!("expected Address recipient, got {other:?}"),
    }

    // Broadcast / local / remote all survive a read-then-write.
    let mut nc2 = NotificationClass::new(2, "NC-2").unwrap();
    nc2.add_destination(make_dest_address(0xFFFF, &[]));
    nc2.add_destination(make_dest_address(0, &[0x0A, 0x00, 0x01, 0x01, 0xBA, 0xC0]));
    nc2.add_destination(make_dest_address(
        1000,
        &[0x0A, 0x00, 0x02, 0x01, 0xBA, 0xC0],
    ));
    let val2 = nc2
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    nc2.write_property(PropertyIdentifier::RECIPIENT_LIST, None, val2, None)
        .unwrap();
    let stored: Vec<u16> = nc2
        .recipient_list
        .iter()
        .map(|d| match &d.recipient {
            BACnetRecipient::Address(a) => a.network_number,
            _ => 0xFFFF,
        })
        .collect();
    assert_eq!(
        stored,
        vec![0xFFFF, 0, 1000],
        "all three network forms survive"
    );
}

#[test]
fn write_recipient_list_rejects_malformed_address() {
    // A malformed address recipient (wrong element count or field type) is
    // rejected rather than silently dropping the network number.
    let mk_entry = |recipient: PropertyValue| {
        PropertyValue::List(vec![
            PropertyValue::BitString {
                unused_bits: 1,
                data: vec![0b0111_1110],
            },
            PropertyValue::Time(make_time(0, 0)),
            PropertyValue::Time(make_time(23, 59)),
            recipient,
            PropertyValue::Unsigned(1),
            PropertyValue::Boolean(false),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0b1110_0000],
            },
        ])
    };
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let cases = [
        mk_entry(PropertyValue::List(vec![PropertyValue::Unsigned(100)])),
        mk_entry(PropertyValue::List(vec![
            PropertyValue::Boolean(true),
            PropertyValue::OctetString(vec![0x01]),
        ])),
    ];
    for bad in cases {
        let result = nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, bad, None);
        assert!(result.is_err(), "malformed address must be rejected");
    }
}
