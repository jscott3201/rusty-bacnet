//! A Network Port's BBMD_Broadcast_Distribution_Table built from rows
//! (#2710): `PropertyValue.bdt(rows)`, the way back from a whole read's
//! `value`.
//!
//! A row is the mapping a read gives (`BdtEntry`): `address`, the BBMD as
//! `"ip:port"`, `"[v6]:port"` or `"name:port"`, and `broadcast_mask`, dotted
//! IPv4 text or `None` (the key may be left out). Each row is encoded with
//! the stack's BDT entry codec, so the value holds exactly the octets the
//! stack would write, and rows read from such a value build those octets
//! again. The value is tagged `bdt`, as a read is, and `.value` gives the
//! rows back.
//!
//! Each row is checked as it is read, and a refusal names it by its index
//! (`rows[2]`): ValueError for a value the entry can't hold, TypeError for
//! one of the wrong type.
//! - `address` missing or `None`: a BDT entry names its BBMD, and Clause
//!   12.56.34 keeps the no-host choice out of the list.
//! - A port that isn't a decimal number from 0 to 65535.
//! - An IPv6 address written without brackets, or with a zone: the entry
//!   holds the address's 16 octets and nothing else.
//! - A host name that is empty or longer than 253 octets, the longest name
//!   DNS resolves.
//! - A mask that isn't four dotted decimal octets.
//! - A key other than `address` and `broadcast_mask`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use bacnet_encoding::constructed::encode_bdt_entry;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;

use super::PyPropertyValue;
use super::constructed_read::Element;
use super::mapping::{mapping, optional_item, required_item, validate_keys};

/// The longest host name a row takes, in octets: the longest DNS resolves
/// (RFC 1035), as the stack's own BBMD name lookups take (#1852).
const MAX_HOST_NAME_OCTETS: usize = 253;

/// The `bdt` value `rows` describe, each row checked and encoded.
pub(super) fn bdt(rows: &Bound<'_, PyAny>) -> PyResult<PyPropertyValue> {
    let mut entries = Vec::new();
    for (index, row) in rows.try_iter()?.enumerate() {
        let name = format!("rows[{index}]");
        let entry = entry(&row?, &name)?;
        let mut octets = BytesMut::new();
        encode_bdt_entry(&mut octets, &entry)
            .map_err(|error| PyValueError::new_err(format!("{name}: {error}")))?;
        entries.push(PropertyValue::ApplicationData(octets.to_vec()));
    }
    Ok(PyPropertyValue::constructed(
        PropertyValue::List(entries),
        Element::BdtEntry,
    ))
}

/// One row, as the entry it describes.
fn entry(row: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetBDTEntry> {
    let row = mapping(row, name)?;
    validate_keys(row, name, &["address"], &["broadcast_mask"])?;
    let address = required_item(row, name, "address")?;
    if address.is_none() {
        return Err(PyValueError::new_err(format!(
            "{name}: a BDT entry names its BBMD, so address can't be None"
        )));
    }
    let bbmd_address = host_n_port(&text(&address, name, "address")?)
        .map_err(|reason| PyValueError::new_err(format!("{name}: {reason}")))?;
    let broadcast_mask = match optional_item(row, "broadcast_mask")? {
        None => None,
        Some(mask) => Some(
            broadcast_mask(&text(&mask, name, "broadcast_mask")?)
                .map_err(|reason| PyValueError::new_err(format!("{name}: {reason}")))?,
        ),
    };
    Ok(BACnetBDTEntry {
        bbmd_address,
        broadcast_mask,
    })
}

/// A row's `key`, which must be a `str`.
fn text(value: &Bound<'_, PyAny>, name: &str, key: &str) -> PyResult<String> {
    if value.cast::<PyString>().is_err() {
        return Err(PyTypeError::new_err(format!("{name}.{key} must be a str")));
    }
    value.extract()
}

/// The host and port `text` names, in the form a read shows them:
/// `"ip:port"`, `"[v6]:port"` or `"name:port"`. A name is everything before
/// the last colon, so a name holding a colon reads back as it was shown.
pub(super) fn host_n_port(text: &str) -> Result<BACnetHostNPort, String> {
    if let Ok(socket) = text.parse::<SocketAddr>() {
        return match socket {
            SocketAddr::V4(v4) => Ok(BACnetHostNPort {
                host: BACnetHostAddress::Ip(IpAddr::V4(*v4.ip())),
                port: v4.port(),
            }),
            SocketAddr::V6(v6) if v6.scope_id() == 0 && v6.flowinfo() == 0 => Ok(BACnetHostNPort {
                host: BACnetHostAddress::Ip(IpAddr::V6(*v6.ip())),
                port: v6.port(),
            }),
            SocketAddr::V6(_) => Err(format!(
                "address {text:?} has an IPv6 zone, which an entry can't hold"
            )),
        };
    }
    if text.starts_with('[') {
        return Err(format!(
            "address {text:?} isn't an IPv6 address and port in the form \"[v6]:port\""
        ));
    }
    let Some((host, port)) = text.rsplit_once(':') else {
        return Err(format!(
            "address {text:?} has no port: write \"ip:port\", \"[v6]:port\" or \"name:port\""
        ));
    };
    let port = parse_port(port).ok_or_else(|| {
        format!("address {text:?} has port {port:?}, not a number from 0 to 65535")
    })?;
    if host.is_empty() {
        return Err(format!("address {text:?} has no host before the port"));
    }
    if host.parse::<Ipv6Addr>().is_ok() {
        return Err(format!(
            "address {text:?}: write an IPv6 address in brackets, \"[{host}]:{port}\""
        ));
    }
    if host.len() > MAX_HOST_NAME_OCTETS {
        return Err(format!(
            "host name of {} octets is longer than {MAX_HOST_NAME_OCTETS}",
            host.len()
        ));
    }
    Ok(BACnetHostNPort {
        host: BACnetHostAddress::Name(host.to_owned()),
        port,
    })
}

/// A port written as one to five decimal digits, at most 65535.
fn parse_port(port: &str) -> Option<u16> {
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    port.parse().ok()
}

/// A broadcast mask: four dotted decimal octets.
fn broadcast_mask(text: &str) -> Result<[u8; 4], String> {
    text.parse::<Ipv4Addr>()
        .map(|mask| mask.octets())
        .map_err(|_| format!("broadcast_mask {text:?} isn't four dotted decimal octets"))
}

#[cfg(test)]
#[path = "bdt_rows_tests.rs"]
mod tests;
