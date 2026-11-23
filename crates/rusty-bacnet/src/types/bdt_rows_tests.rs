//! `PropertyValue.bdt(rows)` (#2710): rows a read gives build its octets
//! again, and a row the entry can't hold is refused, naming the row.

use std::ffi::CStr;

use bacnet_encoding::constructed::encode_bdt_entry_list;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bytes::BytesMut;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::types::PyBytes;

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

#[test]
fn rows_a_read_gives_build_its_octets_again() {
    let name = "n".repeat(MAX_HOST_NAME_OCTETS);
    let tables = [
        vec![],
        vec![entry(
            BACnetHostAddress::Ip([192, 0, 2, 2].into()),
            47808,
            Some([255, 255, 255, 0]),
        )],
        vec![
            entry(
                BACnetHostAddress::Ip("2001:db8::2".parse().unwrap()),
                0,
                Some([255, 255, 255, 255]),
            ),
            entry(BACnetHostAddress::Name("hub.example".into()), 65535, None),
            // A name may hold a colon, and the last one parts it from the
            // port; one that isn't an IPv6 address is a name.
            entry(BACnetHostAddress::Name("a:b".into()), 47809, None),
            entry(BACnetHostAddress::Name("é.example".into()), 1, None),
            entry(BACnetHostAddress::Name(name), 47808, Some([0, 0, 0, 0])),
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

#[test]
fn a_row_the_entry_cant_hold_is_refused_by_its_index() {
    let long = format!(
        "[{{'address': '192.0.2.1:1'}}, {{'address': '{}:47808'}}]",
        "n".repeat(MAX_HOST_NAME_OCTETS + 1)
    );
    let long = std::ffi::CString::new(long).unwrap();
    let cases: [(&CStr, bool, &str); 19] = [
        (
            c"[{'address': None}]",
            true,
            "rows[0]: a BDT entry names its BBMD",
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
            c"[{'address': '192.0.2.1'}]",
            true,
            "rows[0]: address \"192.0.2.1\" has no port",
        ),
        (c"[{'address': 'hub'}]", true, "has no port"),
        (
            c"[{'address': '192.0.2.1:65536'}]",
            true,
            "not a number from 0 to 65535",
        ),
        (
            c"[{'address': 'hub:-1'}]",
            true,
            "not a number from 0 to 65535",
        ),
        (
            c"[{'address': 'hub:+1'}]",
            true,
            "not a number from 0 to 65535",
        ),
        (
            c"[{'address': 'hub:'}]",
            true,
            "not a number from 0 to 65535",
        ),
        (
            c"[{'address': ':47808'}]",
            true,
            "has no host before the port",
        ),
        (
            c"[{'address': '2001:db8::2:47808'}]",
            true,
            "write an IPv6 address in brackets",
        ),
        (
            c"[{'address': '[fe80::1%2]:47808'}]",
            true,
            "has an IPv6 zone",
        ),
        (
            c"[{'address': '[fe80::1%eth0]:47808'}]",
            true,
            "\"[v6]:port\"",
        ),
        (c"[{'address': '[2001:db8::2]'}]", true, "\"[v6]:port\""),
        (
            long.as_c_str(),
            true,
            "rows[1]: host name of 254 octets is longer than 253",
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
