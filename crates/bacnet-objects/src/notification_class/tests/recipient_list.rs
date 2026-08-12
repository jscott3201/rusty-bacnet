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
fn recipient_list_indexed_write_rejected_list_unchanged() {
    // Regression (review blocker): an indexed Recipient_List write silently
    // replaced the whole list with the written destination after the framing
    // migration. Dev @ 6b9ac4f rejects such writes with
    // PROPERTY/INVALID_DATA_TYPE; Recipient_List is a BACnetLIST, not an
    // array (Clause 15.5) — restore the rejection.
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(make_dest_device(10));
    nc.add_destination(make_dest_device(20));
    nc.add_destination(make_dest_device(30));

    // Framed single destination at an index.
    let mut framed = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut framed, &[make_dest_device(99)]);
    let result = nc.write_property(
        PropertyIdentifier::RECIPIENT_LIST,
        Some(2),
        PropertyValue::ApplicationData(framed.to_vec()),
        None,
    );
    match result.unwrap_err() {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
            );
            assert_eq!(
                code,
                bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE.to_raw() as u32
            );
        }
        other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
    }

    // Legacy flat single-entry shape (what an indexed network write decodes
    // to) — rejected too.
    let flat_single = PropertyValue::List(vec![
        PropertyValue::BitString {
            unused_bits: 1,
            data: vec![0xFE],
        },
        PropertyValue::Time(make_time(0, 0)),
        PropertyValue::Time(make_time(23, 59)),
        PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
        PropertyValue::Unsigned(1),
        PropertyValue::Boolean(false),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        },
    ]);
    assert!(nc
        .write_property(
            PropertyIdentifier::RECIPIENT_LIST,
            Some(1),
            flat_single,
            None
        )
        .is_err());

    // The whole list is untouched.
    assert_eq!(nc.recipient_list.len(), 3);
    let instances: Vec<u32> = nc
        .recipient_list
        .iter()
        .map(|d| match &d.recipient {
            BACnetRecipient::Device(oid) => oid.instance_number(),
            other => panic!("expected Device recipient, got {other:?}"),
        })
        .collect();
    assert_eq!(instances, vec![10, 20, 30]);
}

#[test]
fn recipient_list_malformed_tail_fails_whole_decode_no_prefix_delivery() {
    // Review blocker: previously decode stopped at the first malformed
    // destination and silently kept the valid PREFIX — routing would then
    // notify only part of the configured list. Strict now: the whole decode
    // fails and the filter yields NOTHING.
    let good = make_dest_device(10);
    let mut bytes = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut bytes, &[good]);
    // Trailing garbage: an opening tag [5] with no closing.
    let mut framed = bytes.to_vec();
    framed.push(0x5E);
    let val = PropertyValue::ApplicationData(framed);
    assert!(decode_destination_list_pv(&val).is_err());
    let hits = filter_recipient_list(&val, EventTransition::ToOffnormal, 0x01, &make_time(12, 0));
    assert!(
        hits.is_empty(),
        "malformed list must NOT deliver to the valid prefix"
    );
}

#[test]
fn routing_skips_delivery_when_stored_recipient_list_is_malformed() {
    // End-to-end fail-closed: a NotificationClass whose stored Recipient_List
    // is undecodable yields None from the strict lookup (the router skips
    // the notification) rather than delivering to a decodable prefix.
    use std::borrow::Cow;

    struct MalformedListObject {
        oid: ObjectIdentifier,
    }

    impl BACnetObject for MalformedListObject {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }
        fn object_name(&self) -> &str {
            "malformed-nc"
        }
        fn read_property(
            &self,
            property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            if property == PropertyIdentifier::NOTIFICATION_CLASS {
                Ok(PropertyValue::Unsigned(1))
            } else if property == PropertyIdentifier::RECIPIENT_LIST {
                // One fully-valid destination followed by a truncated one.
                let mut framed = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_destination_list(
                    &mut framed,
                    &[make_dest_device(10)],
                );
                let mut bytes = framed.to_vec();
                bytes.push(0x5E); // opening [5], never closed
                Ok(PropertyValue::ApplicationData(bytes))
            } else {
                Err(Error::Protocol {
                    class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                    code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                })
            }
        }
        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Protocol {
                class: bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                code: bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            })
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[PropertyIdentifier::NOTIFICATION_CLASS])
        }
    }

    let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(MalformedListObject { oid })).unwrap();

    let None = get_notification_recipients_strict(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    ) else {
        panic!("undecodable Recipient_List must fail closed with None");
    };

    // A missing class is the legacy empty case (Some([])), NOT None.
    let empty = get_notification_recipients_strict(
        &db,
        99,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    );
    assert_eq!(empty, Some(Vec::new()));
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
