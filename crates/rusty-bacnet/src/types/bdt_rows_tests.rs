//! `PropertyValue.bdt(rows)` (#2710): rows a read gives build its octets
//! again, and a row the entry can't hold is refused, naming the row.

use std::ffi::CStr;

use bacnet_encoding::constructed::encode_bdt_entry_list;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bytes::BytesMut;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::types::{PyBytes, PyDict, PyList};

use super::*;
use crate::types::read_value::decode_read_value;

const BDT: PropertyIdentifier = PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE;

fn entry(host: BACnetHostAddress, port: u16, mask: Option<[u8; 4]>) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort { host, port },
        broadcast_mask: mask,
    }
}

fn octets(entries: &[BACnetBDTEntry]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_bdt_entry_list(&mut buf, entries).unwrap();
    buf.to_vec()
}

/// The octets the value built from `rows` (Python source) holds.
fn built(py: Python<'_>, rows: &CStr) -> PyResult<Vec<u8>> {
    let rows = py.eval(rows, None, None)?;
    let value = Bound::new(py, bdt(&rows)?)?;
    assert_eq!(value.getattr("tag")?.extract::<String>()?, "bdt");
    Ok(value
        .getattr("octets")?
        .cast::<PyBytes>()?
        .as_bytes()
        .to_vec())
}

/// The longest host name the rule takes: dotted labels, 253 octets.
fn longest_name() -> String {
    let labels: Vec<String> = std::iter::repeat_n("a".repeat(63), 4).collect();
    labels.join(".")[..253].to_owned()
}

#[test]
fn rows_a_read_gives_build_its_octets_again() {
    let tables = [
        vec![],
        vec![entry(
            BACnetHostAddress::Ip([192, 0, 2, 2].into()),
            47808,
            Some([255, 255, 255, 0]),
        )],
        vec![
            // Rows a B/IP BBMD wouldn't take still build as given: an IPv6
            // address with a mask, a name, an IPv4 address without a mask.
            entry(
                BACnetHostAddress::Ip("2001:db8::2".parse().unwrap()),
                0,
                Some([255, 255, 255, 255]),
            ),
            entry(BACnetHostAddress::Name("hub.example".into()), 65535, None),
            entry(BACnetHostAddress::Name("10.example".into()), 1, None),
            entry(
                BACnetHostAddress::Name(longest_name()),
                47808,
                Some([0, 0, 0, 0]),
            ),
            // The same row twice is a list a read can give.
            entry(BACnetHostAddress::Ip([10, 0, 0, 1].into()), 47808, None),
            entry(BACnetHostAddress::Ip([10, 0, 0, 1].into()), 47808, None),
        ],
    ];
    Python::initialize();
    for table in tables {
        let octets = octets(&table);
        Python::attach(|py| {
            let read = decode_read_value(ObjectType::NETWORK_PORT, BDT, None, &octets).unwrap();
            let read = Bound::new(py, read).unwrap();
            let rows = read.getattr("value").unwrap();
            let value = Bound::new(py, bdt(&rows).unwrap()).unwrap();
            assert!(value.eq(&read).unwrap(), "{octets:02x?}");
            assert!(value.getattr("value").unwrap().eq(&rows).unwrap());
            let rebuilt = value.getattr("octets").unwrap();
            assert_eq!(rebuilt.cast::<PyBytes>().unwrap().as_bytes(), octets);
        });
    }
}

/// A device's table can name a BBMD in a way the host-name rule doesn't
/// take (a colon, a name past ASCII, digits and dots alone): the read keeps
/// its octets, to be written back as they are, but its rows don't build.
#[test]
fn a_read_row_the_name_rule_refuses_does_not_build() {
    Python::initialize();
    for name in ["a:b", "é.example", "192.0.2.300"] {
        let octets = octets(&[
            entry(BACnetHostAddress::Ip([10, 0, 0, 1].into()), 47808, None),
            entry(BACnetHostAddress::Name(name.into()), 47809, None),
        ]);
        Python::attach(|py| {
            let read = decode_read_value(ObjectType::NETWORK_PORT, BDT, None, &octets).unwrap();
            let read = Bound::new(py, read).unwrap();
            let rows = read.getattr("value").unwrap();
            let error = bdt(&rows).map(|_| ()).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py), "{name}: {error}");
            assert!(error.to_string().contains("rows[1]: "), "{name}: {error}");
            let kept = read.getattr("octets").unwrap();
            assert_eq!(kept.cast::<PyBytes>().unwrap().as_bytes(), octets);
        });
    }
}

#[test]
fn a_row_may_leave_its_mask_out() {
    Python::initialize();
    Python::attach(|py| {
        let expected = octets(&[entry(
            BACnetHostAddress::Ip([192, 0, 2, 9].into()),
            47808,
            None,
        )]);
        for rows in [
            c"[{'address': '192.0.2.9:47808'}]",
            c"({'address': '192.0.2.9:47808', 'broadcast_mask': None},)",
            c"iter([{'broadcast_mask': None, 'address': '192.0.2.9:47808'}])",
        ] {
            assert_eq!(built(py, rows).unwrap(), expected, "{rows:?}");
        }
    });
}

/// The ValueError text one row `{'address': address}` gives.
fn address_refusal(py: Python<'_>, address: &str) -> String {
    let row = PyDict::new(py);
    row.set_item("address", address).unwrap();
    let rows = PyList::new(py, [row]).unwrap();
    let error = bdt(rows.as_any()).map(|_| ()).unwrap_err();
    assert!(
        error.is_instance_of::<PyValueError>(py),
        "{address:?}: {error}"
    );
    error.to_string()
}

#[test]
fn an_address_the_entry_cant_hold_is_refused_by_its_index() {
    let too_long = format!("{}a:47808", longest_name());
    let cases = [
        ("192.0.2.1", "has no port"),
        ("hub", "has no port"),
        ("fe80::1", "write an IPv6 address as \"[fe80::1]:port\""),
        ("2001:db8::2:47808", "write an IPv6 address in brackets"),
        ("[fe80::1%2]:47808", "has an IPv6 zone"),
        ("[fe80::1%eth0]:47808", "has an IPv6 zone"),
        ("[2001:db8::2]", "\"[v6]:port\""),
        ("[2001:db8::zz]:47808", "isn't an IPv6 address"),
        (":47808", "has no host before the port"),
        // Ports: plain decimal only.
        ("192.0.2.1:65536", "not plain decimal from 0 to 65535"),
        ("hub:-1", "not plain decimal"),
        ("hub:+1", "not plain decimal"),
        ("hub: 1", "not plain decimal"),
        ("hub:1 ", "not plain decimal"),
        ("hub:\u{0663}", "not plain decimal"),
        ("hub:047808", "not plain decimal"),
        ("hub:0080", "not plain decimal"),
        ("hub:", "not plain decimal"),
        // Hosts: an IPv4 address or a host name, never something that only
        // looks like one.
        (
            "192.168.1.300:47808",
            "is neither an IPv4 address nor a host name",
        ),
        ("01.2.3.4:47808", "is neither"),
        (" 10.0.0.1:47808", "is neither"),
        ("10.0.0.1 :47808", "is neither"),
        ("a:b:47808", "is neither"),
        ("bbmd.exämple:47808", "is neither"),
        (too_long.as_str(), "is neither"),
    ];
    Python::initialize();
    Python::attach(|py| {
        for (address, expected) in cases {
            let message = address_refusal(py, address);
            assert!(message.contains("rows[0]: "), "{address:?}: {message}");
            assert!(message.contains(expected), "{address:?}: {message}");
        }
    });
}

#[test]
fn a_row_the_entry_cant_hold_is_refused_by_its_index() {
    let cases: [(&CStr, bool, &str); 7] = [
        (
            c"[{'address': '192.0.2.1:1'}, {'address': None}]",
            true,
            "rows[1]: a BDT entry names its BBMD",
        ),
        (
            c"[{'broadcast_mask': None}]",
            true,
            "rows[0] is missing required key 'address'",
        ),
        (
            c"[{'address': '192.0.2.1:1', 'mask': None}]",
            true,
            "rows[0] contains unknown key 'mask'",
        ),
        (
            c"[{'address': 'hub:1', 'broadcast_mask': '255.255.0'}]",
            true,
            "isn't four dotted",
        ),
        (
            c"[{'address': 'hub:1', 'broadcast_mask': 'ffff:ffff::'}]",
            true,
            "isn't four dotted",
        ),
        (c"[{'address': 5}]", false, "rows[0].address must be a str"),
        (c"['192.0.2.1:47808']", false, "rows[0] must be a mapping"),
    ];
    Python::initialize();
    Python::attach(|py| {
        for (rows, value_error, expected) in cases {
            let error = built(py, rows).unwrap_err();
            let message = error.to_string();
            if value_error {
                assert!(
                    error.is_instance_of::<PyValueError>(py),
                    "{rows:?}: {message}"
                );
            } else {
                assert!(
                    error.is_instance_of::<PyTypeError>(py),
                    "{rows:?}: {message}"
                );
            }
            assert!(message.contains(expected), "{rows:?}: {message}");
        }
        let error = built(py, c"[{'address': 'hub:1', 'broadcast_mask': 4}]").unwrap_err();
        assert!(error.is_instance_of::<PyTypeError>(py), "{error}");
        assert!(
            error
                .to_string()
                .contains("rows[0].broadcast_mask must be a str")
        );
        // Not iterable at all.
        assert!(
            built(py, c"5")
                .unwrap_err()
                .is_instance_of::<PyTypeError>(py)
        );
    });
}
