//! A Network Port's BDT and FDT (#2690) read through the shared element
//! table: `bdt` and `fdt` lists of entry mappings in `read_bdt` and
//! `read_fdt` form, each entry keeping its octets.

use super::*;
use std::ffi::CStr;

use bacnet_encoding::constructed::{
    encode_bdt_entry_list, encode_fdt_entry_list, encode_host_n_port,
};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bytes::BytesMut;
use pyo3::types::PyBytes;

use crate::types::read_value::decode_read_value;

const PORT: ObjectType = ObjectType::NETWORK_PORT;
const BDT: PropertyIdentifier = PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE;
const FDT: PropertyIdentifier = PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE;

fn bdt_entry(host: BACnetHostAddress, port: u16, mask: Option<[u8; 4]>) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort { host, port },
        broadcast_mask: mask,
    }
}

fn bdt_octets(entries: &[BACnetBDTEntry]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_bdt_entry_list(&mut buf, entries).unwrap();
    buf.to_vec()
}

fn fdt_octets(entries: &[BACnetFDTEntry]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_fdt_entry_list(&mut buf, entries);
    buf.to_vec()
}

/// Read `octets` from `property` of `object_type` and check its tag, that
/// `.value` equals `expected` (Python source) and that `.octets` gives the
/// octets back unchanged.
fn assert_read(
    (object_type, property, index): (ObjectType, PropertyIdentifier, Option<u32>),
    octets: &[u8],
    tag: &str,
    expected: &CStr,
) {
    Python::initialize();
    Python::attach(|py| {
        let value = decode_read_value(object_type, property, index, octets).unwrap();
        assert_eq!(value.tag_name(), tag);
        let value = Bound::new(py, value).unwrap();
        let read = value.getattr("value").unwrap();
        let expected = py.eval(expected, None, None).unwrap();
        assert!(read.eq(&expected).unwrap(), "{read} != {expected}");
        let encoded = value.getattr("octets").unwrap();
        assert_eq!(encoded.cast::<PyBytes>().unwrap().as_bytes(), octets);
    });
}

#[test]
fn a_bdt_reads_as_its_entries_in_every_host_form() {
    let entries = [
        bdt_entry(
            BACnetHostAddress::Ip([192, 0, 2, 2].into()),
            47808,
            Some([255, 255, 255, 0]),
        ),
        bdt_entry(
            BACnetHostAddress::Ip("2001:db8::2".parse().unwrap()),
            47809,
            None,
        ),
        bdt_entry(BACnetHostAddress::Name("hub.example".into()), 123, None),
        bdt_entry(BACnetHostAddress::None, 0, Some([255, 255, 255, 255])),
    ];
    let octets = bdt_octets(&entries);
    assert_read(
        (PORT, BDT, None),
        &octets,
        "bdt",
        cr"[
            {'address': '192.0.2.2:47808', 'broadcast_mask': '255.255.255.0'},
            {'address': '[2001:db8::2]:47809', 'broadcast_mask': None},
            {'address': 'hub.example:123', 'broadcast_mask': None},
            {'address': None, 'broadcast_mask': '255.255.255.255'},
        ]",
    );
    // The issue's vector, from a reference server.
    assert_read(
        (PORT, BDT, None),
        &[
            0x0e, 0x0e, 0x1c, 192, 0, 2, 2, 0x0f, 0x1a, 0xba, 0xc0, 0x0f, 0x1c, 255, 255, 255, 0,
        ],
        "bdt",
        cr"[{'address': '192.0.2.2:47808', 'broadcast_mask': '255.255.255.0'}]",
    );
}

#[test]
fn an_fdt_reads_as_the_entries_read_fdt_gives() {
    let entries = [
        BACnetFDTEntry {
            address: "192.0.2.3:47808".parse().unwrap(),
            time_to_live: 60,
            remaining_time_to_live: 50,
        },
        BACnetFDTEntry {
            address: "[2001:db8::2]:47809".parse().unwrap(),
            time_to_live: 4,
            remaining_time_to_live: 3,
        },
    ];
    assert_read(
        (PORT, FDT, None),
        &fdt_octets(&entries),
        "fdt",
        cr"[
            {'address': '192.0.2.3:47808', 'ttl': 60, 'seconds_remaining': 50},
            {'address': '[2001:db8::2]:47809', 'ttl': 4, 'seconds_remaining': 3},
        ]",
    );
}

/// An empty table keeps its tag, where an empty Recipient_List stays the
/// generic `list`.
#[test]
fn an_empty_table_keeps_its_tag() {
    assert_read((PORT, BDT, None), &[], "bdt", c"[]");
    assert_read((PORT, FDT, None), &[], "fdt", c"[]");
    let recipients = decode_read_value(
        ObjectType::NOTIFICATION_CLASS,
        PropertyIdentifier::RECIPIENT_LIST,
        None,
        &[],
    )
    .unwrap();
    assert_eq!(recipients.tag_name(), "list");
    assert_eq!(recipients.element(), None);
}

/// An indexed read is one entry, as for every other collection.
#[test]
fn an_indexed_read_is_one_entry() {
    let octets = fdt_octets(&[BACnetFDTEntry {
        address: "192.0.2.3:47808".parse().unwrap(),
        time_to_live: 60,
        remaining_time_to_live: 50,
    }]);
    assert_read(
        (PORT, FDT, Some(1)),
        &octets,
        "fdt_entry",
        cr"{'address': '192.0.2.3:47808', 'ttl': 60, 'seconds_remaining': 50}",
    );
}

/// Octets that aren't the entries, to the last octet, fall back to the
/// generic decoder, as any typed collection's do; so do the same entries
/// on another object type.
#[test]
fn values_that_are_not_entries_read_generically() {
    let entry = bdt_octets(&[bdt_entry(
        BACnetHostAddress::Ip([192, 0, 2, 2].into()),
        47808,
        Some([255, 255, 255, 0]),
    )]);
    // A three-octet mask, and a port past an Unsigned16.
    let short_mask = [&entry[..entry.len() - 5], &[0x1b, 255, 255, 255]].concat();
    let big_port = [0x0e, 0x0e, 0x1c, 192, 0, 2, 2, 0x0f, 0x1b, 1, 0, 0, 0x0f];
    for octets in [&short_mask[..], &big_port[..]] {
        let value = decode_read_value(PORT, BDT, None, octets).unwrap();
        assert_eq!(value.tag_name(), "tagged", "{octets:02x?}");
        assert_eq!(value.element(), None);
    }
    // Application-tagged octets are no table either, and stay as received
    // rather than a re-encoded list (#2690).
    for property in [BDT, FDT] {
        let value = decode_read_value(PORT, property, None, &[0x22, 0x00, 0x02]).unwrap();
        assert_eq!(value.element(), None);
        assert_eq!(value.tag_name(), "application_data");
        assert_eq!(
            value.inner,
            PropertyValue::ApplicationData(vec![0x22, 0x00, 0x02])
        );
    }
    // An indexed read of one entry is not a whole read: it keeps the
    // generic rules.
    let value = decode_read_value(PORT, FDT, Some(1), &[0x22, 0x00, 0x02]).unwrap();
    assert_eq!(value.tag_name(), "unsigned");
    let value = decode_read_value(ObjectType::ANALOG_INPUT, BDT, None, &entry).unwrap();
    assert_eq!(value.tag_name(), "tagged");
    // Broken framing still raises.
    assert!(decode_read_value(PORT, BDT, None, &[0x0e, 0x09, 0x01]).is_err());
}

/// More entries than the decoders' bound read as their octets, not as a
/// typed table.
#[test]
fn a_table_past_the_row_bound_reads_as_its_octets() {
    let row = fdt_octets(&[BACnetFDTEntry {
        address: "192.0.2.3:47808".parse().unwrap(),
        time_to_live: 60,
        remaining_time_to_live: 50,
    }]);
    let most = row.repeat(10_000);
    let value = decode_read_value(PORT, FDT, None, &most).unwrap();
    assert_eq!(value.tag_name(), "fdt");
    let past = row.repeat(10_001);
    let value = decode_read_value(PORT, FDT, None, &past).unwrap();
    assert_eq!(value.tag_name(), "application_data");
    assert_eq!(value.inner, PropertyValue::ApplicationData(past));
}

/// FD_BBMD_Address and BACnet_IP_Global_Address (#2710) read as one
/// `host_n_port`: the address text a BDT row gives its BBMD, the octets
/// exact. A value that isn't exactly one host and port reads generically,
/// keeping its octets, and so does the same value on another object type.
#[test]
fn a_host_and_port_reads_as_a_bdt_rows_address() {
    let encoded = |host, port| {
        let mut buf = BytesMut::new();
        encode_host_n_port(&mut buf, &BACnetHostNPort { host, port }).unwrap();
        buf.to_vec()
    };
    for property in [
        PropertyIdentifier::FD_BBMD_ADDRESS,
        PropertyIdentifier::BACNET_IP_GLOBAL_ADDRESS,
    ] {
        for (host, port, expected) in [
            (
                BACnetHostAddress::Ip([192, 0, 2, 2].into()),
                47808,
                c"'192.0.2.2:47808'",
            ),
            (
                BACnetHostAddress::Ip("2001:db8::2".parse().unwrap()),
                47809,
                c"'[2001:db8::2]:47809'",
            ),
            (
                BACnetHostAddress::Name("bbmd.example".into()),
                47808,
                c"'bbmd.example:47808'",
            ),
            // Not determined yet (Clause 12.56.33): no host and port 0.
            (BACnetHostAddress::None, 0, c"None"),
        ] {
            let octets = encoded(host, port);
            assert_read((PORT, property, None), &octets, "host_n_port", expected);
        }
        let one = encoded(BACnetHostAddress::Ip([192, 0, 2, 2].into()), 47808);
        // Two values, a trailing octet, a port past an Unsigned16.
        let two = [&one[..], &one[..]].concat();
        let trailing = [&one[..], &[0x21, 0x01]].concat();
        let big_port = [0x0e, 0x1c, 192, 0, 2, 2, 0x0f, 0x1b, 1, 0, 0];
        for octets in [&two[..], &trailing[..], &big_port[..]] {
            let value = decode_read_value(PORT, property, None, octets).unwrap();
            assert_eq!(value.element(), None, "{octets:02x?}");
            assert_eq!(value.tag_name(), "tagged", "{octets:02x?}");
            assert_eq!(value.inner, PropertyValue::ApplicationData(octets.to_vec()));
        }
        let other = decode_read_value(ObjectType::ANALOG_INPUT, property, None, &one).unwrap();
        assert_eq!(other.tag_name(), "tagged");
    }
}
