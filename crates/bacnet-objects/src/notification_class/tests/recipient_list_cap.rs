//! The Recipient_List cap (#1098): writes past
//! [`MAX_RECIPIENT_LIST_DESTINATIONS`] are refused, naming the first
//! destination that doesn't fit, and a full list stays small on the wire. An
//! address MAC past [`BACnetAddress::MAX_MAC_LEN`] is refused too (#1124), which
//! bounds the size of each destination.

use super::super::*;
use super::make_dest_device;
use crate::common::assert_list_element_refused;
use bacnet_encoding::constructed::{encode_destination, encode_destination_list};
use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;

const CAP: usize = MAX_RECIPIENT_LIST_DESTINATIONS;

/// `count` distinct device destinations: process identifiers 1 to `count`.
fn destinations(count: usize) -> Vec<BACnetDestination> {
    (1..=count)
        .map(|process_identifier| BACnetDestination {
            process_identifier: process_identifier as u32,
            ..make_dest_device(10)
        })
        .collect()
}

fn framed(list: &[BACnetDestination]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, list).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

/// A device destination readdressed to network 1000 at a `len`-octet MAC.
fn address_destination(len: usize) -> BACnetDestination {
    BACnetDestination {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 1000,
            mac_address: bacnet_types::MacAddr::from_slice(&vec![0xA5; len]),
        }),
        ..make_dest_device(10)
    }
}

fn write(nc: &mut NotificationClass, value: PropertyValue) -> Result<(), Error> {
    nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, value, None)
}

/// Assert a plain class / code refusal, naming no element.
fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: actual_class,
            code: actual_code,
        })) => assert_eq!((actual_class, actual_code), (class, code)),
        other => panic!("expected {class:?}/{code:?}, got {other:?}"),
    }
}

#[test]
fn recipient_list_write_past_the_cap_names_the_first_destination_that_does_not_fit() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    write(&mut nc, framed(&destinations(CAP))).unwrap();
    assert_eq!(nc.recipient_list(), destinations(CAP));
    let mut garbage_past_the_cap = framed(&destinations(CAP));
    if let PropertyValue::ApplicationData(bytes) = &mut garbage_past_the_cap {
        bytes.push(0xFF);
    }
    for (what, value) in [
        ("framed, one over", framed(&destinations(CAP + 1))),
        ("framed, many over", framed(&destinations(3 * CAP))),
        // Destinations are checked in order: the one past the cap is refused
        // before it is decoded.
        ("framed, garbage past the cap", garbage_past_the_cap),
    ] {
        assert_list_element_refused(
            write(&mut nc, value),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            CAP as u32 + 1,
            what,
        );
        assert_eq!(nc.recipient_list(), destinations(CAP), "{what}");
    }
    // A malformed destination within the cap is still a datatype error.
    let mut malformed = framed(&destinations(2));
    if let PropertyValue::ApplicationData(bytes) = &mut malformed {
        bytes.push(0xFF);
    }
    assert_refused(
        write(&mut nc, malformed),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
}

#[test]
fn add_destination_refuses_past_the_cap() {
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    for destination in destinations(CAP) {
        nc.add_destination(destination).unwrap();
    }
    assert_refused(
        nc.add_destination(make_dest_device(99)),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert_eq!(nc.recipient_list(), destinations(CAP));
}

#[test]
fn recipient_list_mac_past_the_bound_is_refused() {
    // #1124: an address MAC longer than B/IPv6's 18 octets is refused by a
    // write as an undecodable destination, and by `add_destination` with
    // the same code, leaving the list as it was.
    let longest = address_destination(BACnetAddress::MAX_MAC_LEN);
    let too_long = address_destination(BACnetAddress::MAX_MAC_LEN + 1);
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.add_destination(longest.clone()).unwrap();
    assert_refused(
        nc.add_destination(too_long.clone()),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    // The encoder refuses the long MAC too (#1156), so its octets are built
    // from the primitives.
    assert!(encode_destination(&mut BytesMut::new(), &too_long).is_err());
    let PropertyValue::ApplicationData(mut list) = framed(&[make_dest_device(10)]) else {
        unreachable!("framed list")
    };
    let mut raw = BytesMut::new();
    primitives::encode_app_bit_string(&mut raw, 1, &[too_long.valid_days.to_bacnet()]);
    primitives::encode_app_time(&mut raw, &too_long.from_time);
    primitives::encode_app_time(&mut raw, &too_long.to_time);
    tags::encode_opening_tag(&mut raw, 1);
    primitives::encode_app_unsigned(&mut raw, 1000);
    primitives::encode_app_octet_string(&mut raw, &[0xA5; BACnetAddress::MAX_MAC_LEN + 1]);
    tags::encode_closing_tag(&mut raw, 1);
    primitives::encode_app_unsigned(&mut raw, too_long.process_identifier.into());
    primitives::encode_app_boolean(&mut raw, too_long.issue_confirmed_notifications);
    primitives::encode_app_bit_string(&mut raw, 5, &[too_long.transitions.to_bacnet()]);
    list.extend_from_slice(&raw);
    assert_refused(
        write(&mut nc, PropertyValue::ApplicationData(list)),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(nc.recipient_list(), std::slice::from_ref(&longest));
    write(&mut nc, framed(&[make_dest_device(10), longest.clone()])).unwrap();
    assert_eq!(nc.recipient_list(), [make_dest_device(10), longest]);
}

#[test]
fn recipient_list_at_the_cap_reads_in_one_unsegmented_apdu() {
    // The largest destination of each form the cap's justification counts: a
    // device, and an address with a 6-octet MAC on the largest network
    // number, each with the largest process identifier.
    let device = BACnetDestination {
        process_identifier: u32::MAX,
        ..make_dest_device(10)
    };
    let address = BACnetDestination {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: u16::MAX,
            mac_address: bacnet_types::MacAddr::from_slice(&[0xFF; 6]),
        }),
        ..device.clone()
    };
    let encoded_len = |destination: &BACnetDestination| {
        let mut buf = BytesMut::new();
        encode_destination(&mut buf, destination).unwrap();
        buf.len()
    };
    assert_eq!(encoded_len(&device), 27);
    assert_eq!(encoded_len(&address), 35);

    // A ReadProperty-ACK spends 12 octets around the list: the PDU type,
    // invoke ID and service choice, the object identifier [0], the property
    // identifier [1], and the opening and closing tags of the value [3].
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    for _ in 0..CAP {
        nc.add_destination(address.clone()).unwrap();
    }
    let Ok(PropertyValue::ApplicationData(list)) =
        nc.read_property(PropertyIdentifier::RECIPIENT_LIST, None)
    else {
        panic!("Recipient_List reads as framed data");
    };
    assert_eq!(list.len(), 1_120);
    assert!(12 + list.len() <= 1476);

    // The MAC bound (#1124) caps every destination: the longest MAC takes 12
    // octets more than a 6-octet one, so a full list of them is 1,504 octets.
    let longest = BACnetDestination {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: u16::MAX,
            mac_address: bacnet_types::MacAddr::from_slice(&[0xFF; BACnetAddress::MAX_MAC_LEN]),
        }),
        ..device
    };
    assert_eq!(encoded_len(&longest), 47);
    let mut nc = NotificationClass::new(2, "NC-2").unwrap();
    for _ in 0..CAP {
        nc.add_destination(longest.clone()).unwrap();
    }
    let Ok(PropertyValue::ApplicationData(list)) =
        nc.read_property(PropertyIdentifier::RECIPIENT_LIST, None)
    else {
        panic!("Recipient_List reads as framed data");
    };
    assert_eq!(list.len(), 1_504);
}
