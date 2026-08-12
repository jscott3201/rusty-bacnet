//! `Recipient_List` address-recipient round-trip tests for #126.
//!
//! These cover preserving the `BACnetAddress` network number through the
//! read/write round-trip (previously dropped on encode, reconstructed as 0 on
//! decode), across local/remote/broadcast forms and malformed-input rejection.
//! Split out of `tests.rs` to keep both files under the 700-LOC cap.

use super::super::*;
use super::{make_dest_device, make_time};
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
    // Framed form: the network number is the application-Unsigned16 member
    // inside the constructed address [1] recipient.
    let PropertyValue::ApplicationData(bytes) = &val else {
        panic!("Expected ApplicationData");
    };
    let decoded = bacnet_encoding::constructed::decode_destination_list(bytes).unwrap();
    match &decoded[0].recipient {
        BACnetRecipient::Address(addr) => {
            assert_eq!(addr.network_number, 0xBAC0);
            assert_eq!(addr.mac_address, mac);
        }
        other => panic!("expected Address recipient, got {other:?}"),
    }
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
fn recipient_list_framed_eight_entry_write_round_trip() {
    // Annex K.2.25 (AE-CRL-B) requires at least 8 writable Recipient_List
    // entries: write a framed 8-entry BACnetLIST, read it back.
    let mut nc = NotificationClass::new(9, "NC-9").unwrap();
    let entries: Vec<BACnetDestination> = (0..8u32)
        .map(|i| {
            let mut d = make_dest_device(100 + i);
            d.process_identifier = i;
            if i % 2 == 1 {
                d.recipient = BACnetRecipient::Address(BACnetAddress {
                    network_number: (1000 + i) as u16,
                    mac_address: MacAddr::from_slice(&[10, 0, i as u8, 1, 0xBA, 0xC0]),
                });
            }
            d
        })
        .collect();
    let mut framed = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut framed, &entries);
    nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::ApplicationData(framed.to_vec()),
        None,
    )
    .unwrap();
    assert_eq!(nc.recipient_list, entries);
    // The read arm re-emits the identical framed bytes.
    let val = nc
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(framed.to_vec()));
    // And the filter decodes them for routing.
    let mut db = ObjectDatabase::new();
    db.add(Box::new(nc)).unwrap();
    let noon = make_time(12, 0);
    let hits = get_notification_recipients(&db, 9, EventTransition::ToOffnormal, 0x01, &noon);
    assert_eq!(hits.len(), 8);
}

#[test]
fn recipient_list_framed_bad_recipient_tag_rejected() {
    // recipient under context tag [2] — not a BACnetRecipient choice.
    let mut framed = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_bit_string(&mut framed, 1, &[0xFE]);
    bacnet_encoding::primitives::encode_app_time(&mut framed, &make_time(0, 0));
    bacnet_encoding::primitives::encode_app_time(&mut framed, &make_time(23, 59));
    bacnet_encoding::primitives::encode_ctx_unsigned(&mut framed, 2, 1);
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::ApplicationData(framed.to_vec()),
        None,
    );
    assert!(result.is_err(), "recipient tag [2] must be rejected");
}

#[test]
fn recipient_list_framed_opening_without_closing_rejected() {
    // address [1] opened but never closed, mid-list.
    let mut framed = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_bit_string(&mut framed, 1, &[0xFE]);
    bacnet_encoding::primitives::encode_app_time(&mut framed, &make_time(0, 0));
    bacnet_encoding::primitives::encode_app_time(&mut framed, &make_time(23, 59));
    bacnet_encoding::tags::encode_opening_tag(&mut framed, 1);
    bacnet_encoding::primitives::encode_app_unsigned(&mut framed, 0xBAC0);
    bacnet_encoding::primitives::encode_app_octet_string(&mut framed, &[1, 2, 3]);
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let result = nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::ApplicationData(framed.to_vec()),
        None,
    );
    assert!(result.is_err(), "unbalanced address must be rejected");
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

#[test]
fn written_valid_days_decodes_msb_first() {
    // Decode-side witness for #203: valid_days arrives as wire bytes and the
    // filter observes the decoded internal mask, so a Monday-only wire byte
    // (monday(0) = 0x80 per Clause 20.2.10) must match today_bit 0x01 (Monday)
    // and not 0x40 (Sunday). Pure round trips cannot catch an inverted decode;
    // this asymmetric byte can.
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    let entry = PropertyValue::List(vec![
        PropertyValue::BitString {
            unused_bits: 1,
            data: vec![0b1000_0000], // Monday only
        },
        PropertyValue::Time(make_time(0, 0)),
        PropertyValue::Time(make_time(23, 59)),
        PropertyValue::ObjectIdentifier(dev_oid),
        PropertyValue::Unsigned(7),
        PropertyValue::Boolean(false),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000], // TO_OFFNORMAL only
        },
    ]);
    nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        PropertyValue::List(vec![entry]),
        None,
    )
    .unwrap();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(nc)).unwrap();
    let noon = make_time(12, 0);

    let monday = get_notification_recipients(&db, 1, EventTransition::ToOffnormal, 0x01, &noon);
    assert_eq!(monday.len(), 1, "Monday-only entry must match Monday");

    let sunday = get_notification_recipients(&db, 1, EventTransition::ToOffnormal, 0x40, &noon);
    assert!(sunday.is_empty(), "Monday-only entry must not match Sunday");
}
