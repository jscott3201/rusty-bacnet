//! `Recipient_List` framing tests (#152): golden Clause-20.2 vectors for the
//! device-form and address-form destinations (nonzero network, zero-length
//! MAC broadcast), an 8-entry list round-trip, and negatives.

use super::*;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;

fn t(h: u8, m: u8, s: u8, cs: u8) -> Time {
    Time {
        hour: h,
        minute: m,
        second: s,
        hundredths: cs,
    }
}

fn device_destination() -> BACnetDestination {
    // valid_days: all seven days -> MSB-first fill octet 0xFE.
    // transitions: all three -> 0xE0.
    BACnetDestination {
        valid_days: 0b0111_1111,
        from_time: t(0, 0, 0, 0),
        to_time: t(23, 59, 59, 99),
        recipient: BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
        process_identifier: 1,
        issue_confirmed_notifications: true,
        transitions: 0b0000_0111,
    }
}

#[test]
fn destination_device_form_golden() {
    let dest = device_destination();
    let mut buf = BytesMut::new();
    encode_destination(&mut buf, &dest);
    assert_eq!(
        buf.as_ref(),
        &[
            0x82, 0x01, 0xFE, // valid-days: Bit String, 1 unused, 0b1111_1110
            0xB4, 0x00, 0x00, 0x00, 0x00, // from-time: Time 00:00:00.00
            0xB4, 0x17, 0x3B, 0x3B, 0x63, // to-time: Time 23:59:59.99
            0x0C, 0x02, 0x00, 0x00, 0x63, // recipient device [0]: (8<<22)|99
            0x21, 0x01, // process-identifier: Unsigned 1
            0x11, // issue-confirmed-notifications: TRUE (L/V/T=1)
            0x82, 0x05, 0xE0, // transitions: Bit String, 5 unused, 0b1110_0000
        ]
    );
    let (decoded, end) = decode_destination(&buf, 0).unwrap();
    assert_eq!(decoded, dest);
    assert_eq!(end, buf.len());
}

#[test]
fn destination_address_form_golden() {
    // Address recipient on a nonzero network with a 6-octet MAC.
    let dest = BACnetDestination {
        valid_days: 0b0011_1110, // Tue..Sat -> MSB-first 0b0111_1100
        from_time: t(8, 0, 0, 0),
        to_time: t(17, 0, 0, 0),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0xBAC0,
            mac_address: bacnet_types::MacAddr::from_slice(&[192, 168, 1, 100, 0xBA, 0xC0]),
        }),
        process_identifier: 42,
        issue_confirmed_notifications: false,
        transitions: 0b0000_0001, // TO_OFFNORMAL only -> 0x80
    };
    let mut buf = BytesMut::new();
    encode_destination(&mut buf, &dest);
    assert_eq!(
        buf.as_ref(),
        &[
            0x82, 0x01, 0x7C, // valid-days Tue..Sat
            0xB4, 0x08, 0x00, 0x00, 0x00, // from-time 08:00:00.00
            0xB4, 0x11, 0x00, 0x00, 0x00, // to-time 17:00:00.00
            0x1E, // recipient address [1] opening
            0x22, 0xBA, 0xC0, // network-number Unsigned16 0xBAC0
            // mac-address OCTET STRING, 6 octets: extended-length tag form
            // (6 octets > 4, so tag 0x65 then one length octet).
            0x65, 0x06, 0xC0, 0xA8, 0x01, 0x64, 0xBA, 0xC0, 0x1F, // closing [1]
            0x21, 0x2A, // process-identifier 42
            0x10, // issue-confirmed-notifications FALSE
            0x82, 0x05, 0x80, // transitions: TO_OFFNORMAL only
        ]
    );
    let (decoded, end) = decode_destination(&buf, 0).unwrap();
    assert_eq!(decoded, dest);
    assert_eq!(end, buf.len());
}

#[test]
fn destination_broadcast_address_golden() {
    // Broadcast address: nonzero network number AND zero-length MAC.
    let dest = BACnetDestination {
        valid_days: 0b0111_1111,
        from_time: t(0, 0, 0, 0),
        to_time: t(23, 59, 59, 99),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0xFFFF,
            mac_address: bacnet_types::MacAddr::new(),
        }),
        process_identifier: 0,
        issue_confirmed_notifications: false,
        transitions: 0b0000_0111,
    };
    let mut buf = BytesMut::new();
    encode_destination(&mut buf, &dest);
    assert_eq!(
        buf.as_ref(),
        &[
            0x82, 0x01, 0xFE, 0xB4, 0x00, 0x00, 0x00, 0x00, 0xB4, 0x17, 0x3B, 0x3B, 0x63,
            0x1E, // address [1] opening
            0x22, 0xFF, 0xFF, // network-number 0xFFFF
            0x60, // mac-address OCTET STRING length 0 (broadcast)
            0x1F, 0x21, 0x00, // process-identifier 0
            0x10, // FALSE
            0x82, 0x05, 0xE0,
        ]
    );
    let (decoded, end) = decode_destination(&buf, 0).unwrap();
    assert_eq!(decoded, dest);
    assert_eq!(end, buf.len());
}

#[test]
fn destination_list_eight_entries_round_trip() {
    // Annex K.2.25 (AE-CRL-B) requires at least 8 writable Recipient_List
    // entries; encode/decode a full 8-entry list as concatenation.
    let entries: Vec<BACnetDestination> = (0..8u32)
        .map(|i| {
            let mut d = device_destination();
            d.process_identifier = i;
            d.recipient = if i % 2 == 0 {
                BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 100 + i).unwrap())
            } else {
                BACnetRecipient::Address(BACnetAddress {
                    network_number: (1000 + i) as u16,
                    mac_address: bacnet_types::MacAddr::from_slice(&[
                        10, 0, i as u8, 1, 0xBA, 0xC0,
                    ]),
                })
            };
            d
        })
        .collect();
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, &entries);
    // 4 device-form entries (24 bytes each) + 4 address-form entries
    // (32 bytes each: 13-byte recipient incl. 6-octet MAC in extended-length
    // octet string).
    assert_eq!(buf.len(), 4 * 24 + 4 * 32);
    let decoded = decode_destination_list(&buf).unwrap();
    assert_eq!(decoded, entries);
}

#[test]
fn destination_list_empty_encodes_to_nothing() {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, &[]);
    assert!(buf.is_empty());
    assert!(decode_destination_list(&buf).unwrap().is_empty());
}

// --- Negatives -----------------------------------------------------------------

#[test]
fn destination_recipient_tag_2_rejected() {
    // A recipient under context tag [2] is not a BACnetRecipient.
    let base = device_destination();
    let mut buf = BytesMut::new();
    primitives::encode_app_bit_string(&mut buf, 1, &[0xFE]);
    primitives::encode_app_time(&mut buf, &base.from_time);
    primitives::encode_app_time(&mut buf, &base.to_time);
    // recipient: opening tag 2 / app unsigned / app octet / closing 2
    tags::encode_opening_tag(&mut buf, 2);
    primitives::encode_app_unsigned(&mut buf, 0);
    primitives::encode_app_octet_string(&mut buf, &[1, 2, 3]);
    tags::encode_closing_tag(&mut buf, 2);
    assert!(decode_destination(&buf, 0).is_err());
    // Primitive context tag 2 as well.
    let mut buf = BytesMut::new();
    primitives::encode_app_bit_string(&mut buf, 1, &[0xFE]);
    primitives::encode_app_time(&mut buf, &base.from_time);
    primitives::encode_app_time(&mut buf, &base.to_time);
    primitives::encode_ctx_unsigned(&mut buf, 2, 1);
    assert!(decode_destination(&buf, 0).is_err());
}

#[test]
fn destination_address_opening_without_closing_rejected() {
    let base = device_destination();
    let mut buf = BytesMut::new();
    primitives::encode_app_bit_string(&mut buf, 1, &[0xFE]);
    primitives::encode_app_time(&mut buf, &base.from_time);
    primitives::encode_app_time(&mut buf, &base.to_time);
    tags::encode_opening_tag(&mut buf, 1);
    primitives::encode_app_unsigned(&mut buf, 0xBAC0);
    primitives::encode_app_octet_string(&mut buf, &[192, 168, 1, 100, 0xBA, 0xC0]);
    // no closing tag — remainder of a destination follows
    primitives::encode_app_unsigned(&mut buf, 1);
    assert!(decode_destination(&buf, 0).is_err());
}

#[test]
fn destination_truncated_members_rejected() {
    let dest = device_destination();
    let mut buf = BytesMut::new();
    encode_destination(&mut buf, &dest);
    for cut in 1..buf.len() {
        assert!(
            decode_destination(&buf[..cut], 0).is_err(),
            "truncated at {cut} bytes must fail"
        );
    }
}

#[test]
fn destination_wrong_member_type_rejected() {
    // from-time replaced with an Unsigned — the Time check must fire.
    let mut buf = BytesMut::new();
    primitives::encode_app_bit_string(&mut buf, 1, &[0xFE]);
    primitives::encode_app_unsigned(&mut buf, 3600);
    assert!(decode_destination(&buf, 0).is_err());
}

#[test]
fn destination_wrong_width_valid_days_rejected() {
    // BACnetDaysOfWeek is exactly 1 content octet with unused_bits == 1
    // (Clause 20.2.10 + Clause 21); empty, overlong, or mismatched encodings
    // must fail rather than unpack to an all-zero dormant destination.
    let base = device_destination();
    let tail = {
        let mut tail = BytesMut::new();
        // from/to time + recipient + process id + confirmed + transitions
        primitives::encode_app_time(&mut tail, &base.from_time);
        primitives::encode_app_time(&mut tail, &base.to_time);
        encode_recipient(&mut tail, &base.recipient);
        primitives::encode_app_unsigned(&mut tail, base.process_identifier as u64);
        primitives::encode_app_boolean(&mut tail, base.issue_confirmed_notifications);
        primitives::encode_app_bit_string(&mut tail, 5, &[0xE0]);
        tail.to_vec()
    };
    for bad_days in [
        // zero content octets
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 0, &[]);
            b.to_vec()
        },
        // two content octets (overlong)
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 6, &[0xFE, 0x00]);
            b.to_vec()
        },
        // one octet but wrong unused-bits count (claims all 8 bits)
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 0, &[0xFE]);
            b.to_vec()
        },
        // one octet, correct count for `transitions` (5) but NOT valid-days (1)
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 5, &[0xFE]);
            b.to_vec()
        },
    ] {
        let mut buf = bad_days.clone();
        buf.extend_from_slice(&tail);
        assert!(
            decode_destination(&buf, 0).is_err(),
            "valid-days {bad_days:?} must be rejected"
        );
    }
}

#[test]
fn destination_wrong_width_transitions_rejected() {
    let base = device_destination();
    let mut head = BytesMut::new();
    primitives::encode_app_bit_string(&mut head, 1, &[0xFE]);
    primitives::encode_app_time(&mut head, &base.from_time);
    primitives::encode_app_time(&mut head, &base.to_time);
    encode_recipient(&mut head, &base.recipient);
    primitives::encode_app_unsigned(&mut head, base.process_identifier as u64);
    primitives::encode_app_boolean(&mut head, base.issue_confirmed_notifications);

    for bad_transitions in [
        // empty
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 0, &[]);
            b.to_vec()
        },
        // two content octets
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, -5i8 as u8 & 0xFF, &[0xE0, 0x00]);
            b.to_vec()
        },
        // wrong unused-bits count (1 — the valid-days count)
        {
            let mut b = BytesMut::new();
            primitives::encode_app_bit_string(&mut b, 1, &[0xE0]);
            b.to_vec()
        },
    ] {
        let mut buf = head.clone();
        buf.extend_from_slice(&bad_transitions);
        assert!(
            decode_destination(&buf, 0).is_err(),
            "transitions {bad_transitions:?} must be rejected"
        );
    }
}

#[test]
fn destination_network_number_over_unsigned16_rejected() {
    let base = device_destination();
    let mut buf = BytesMut::new();
    primitives::encode_app_bit_string(&mut buf, 1, &[0xFE]);
    primitives::encode_app_time(&mut buf, &base.from_time);
    primitives::encode_app_time(&mut buf, &base.to_time);
    tags::encode_opening_tag(&mut buf, 1);
    primitives::encode_app_unsigned(&mut buf, 65536); // Unsigned16 overflow
    primitives::encode_app_octet_string(&mut buf, &[1, 2, 3]);
    tags::encode_closing_tag(&mut buf, 1);
    assert!(decode_destination(&buf, 0).is_err());
}
