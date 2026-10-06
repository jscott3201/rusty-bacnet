//! Clause 21 codecs for the Network Port object's host and broadcast-table
//! values: `BACnetHostAddress`, `BACnetHostNPort`, `BACnetBDTEntry` and
//! `BACnetFDTEntry` (Clause 12.56).
//!
//! The shapes, as the Clause 21 productions lay them out:
//!
//! - `BACnetHostAddress` is a CHOICE of primitive alternatives: `[0]` NULL
//!   for no host, `[1]` OCTET STRING for an IP address (4 or 16 octets) and
//!   `[2]` CharacterString for a host name.
//! - `BACnetHostNPort` is a SEQUENCE of the host, under `[0]`, and the port
//!   as an Unsigned16 under `[1]`. Since the host is a CHOICE, its `[0]` is
//!   an opening and closing pair around the chosen alternative.
//! - `BACnetBDTEntry` is a SEQUENCE of the BBMD's `BACnetHostNPort` under an
//!   opening and closing `[0]`, then the optional broadcast mask, a 4-octet
//!   OCTET STRING under `[1]`.
//! - `BACnetFDTEntry` is a SEQUENCE of three primitives: the registrant's
//!   address under `[0]` (6 octets for B/IP, 18 for B/IPv6), then the time
//!   to live and the time remaining, each an Unsigned16, under `[1]` and
//!   `[2]`.
//!
//! A BACnetLIST of entries is the plain concatenation of its elements. None
//! of this is the 10-octet BVLL layout Read-Broadcast-Distribution-Table-Ack
//! and Read-Foreign-Device-Table-Ack carry (Annex J.2), which the transport
//! crate owns.
//!
//! Decoding is strict: a missing, misplaced, truncated or wrong-sized member,
//! or a port or time past an Unsigned16, is an error.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::tagged::{
    contents, decode_ctx_character_string, decode_ctx_fixed, decode_ctx_unsigned,
    decode_optional_ctx, expect_closing, expect_opening, misplaced_tag,
};
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, TagClass};

/// Encode one `BACnetHostAddress` CHOICE alternative.
///
/// Fails, leaving `buf` untouched, only when a host name is too long to
/// encode.
pub fn encode_host_address(buf: &mut BytesMut, host: &BACnetHostAddress) -> Result<(), Error> {
    match host {
        BACnetHostAddress::None => tags::encode_tag(buf, 0, TagClass::Context, 0),
        BACnetHostAddress::Ip(IpAddr::V4(ip)) => {
            primitives::encode_ctx_octet_string(buf, 1, &ip.octets())
        }
        BACnetHostAddress::Ip(IpAddr::V6(ip)) => {
            primitives::encode_ctx_octet_string(buf, 1, &ip.octets())
        }
        BACnetHostAddress::Name(name) => {
            let mut encoded = BytesMut::new();
            primitives::encode_ctx_character_string(&mut encoded, 2, name)?;
            buf.extend_from_slice(&encoded);
        }
    }
    Ok(())
}

/// Decode one `BACnetHostAddress` CHOICE alternative at `offset`; returns it
/// and the offset past it.
///
/// A NULL with contents, an IP address of other than 4 or 16 octets, an
/// undecodable name, or any tag other than primitive `[0]`, `[1]` or `[2]` is
/// a decode error.
pub fn decode_host_address(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetHostAddress, usize), Error> {
    let what = "BACnetHostAddress";
    let (tag, start) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) {
        if tag.length != 0 {
            return Err(Error::decoding(
                offset,
                format!("{what} none [0]: NULL has {} contents octets", tag.length),
            ));
        }
        return Ok((BACnetHostAddress::None, start));
    }
    if tag.is_context(1) {
        let (octets, end) = contents(data, start, tag.length)?;
        let ip = match *octets {
            [a, b, c, d] => IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
            _ => match <[u8; 16]>::try_from(octets) {
                Ok(v6) => IpAddr::V6(Ipv6Addr::from(v6)),
                Err(_) => {
                    return Err(Error::decoding(
                        offset,
                        format!(
                            "{what} ip-address [1]: {} octets, expected 4 or 16",
                            octets.len()
                        ),
                    ))
                }
            },
        };
        return Ok((BACnetHostAddress::Ip(ip), end));
    }
    if tag.is_context(2) {
        let (name, end) = decode_ctx_character_string(data, offset, 2, what)?;
        return Ok((BACnetHostAddress::Name(name), end));
    }
    Err(misplaced_tag(
        data,
        &tag,
        None,
        offset,
        format!("{what}: expected none [0], ip-address [1] or name [2]"),
    ))
}

/// Encode one bare `BACnetHostNPort` SEQUENCE.
///
/// Fails, leaving `buf` untouched, only when a host name is too long to
/// encode.
pub fn encode_host_n_port(buf: &mut BytesMut, value: &BACnetHostNPort) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    tags::encode_opening_tag(&mut encoded, 0);
    encode_host_address(&mut encoded, &value.host)?;
    tags::encode_closing_tag(&mut encoded, 0);
    primitives::encode_ctx_unsigned(&mut encoded, 1, value.port.into());
    buf.extend_from_slice(&encoded);
    Ok(())
}

/// Decode one bare `BACnetHostNPort` SEQUENCE at `offset`; returns it and the
/// offset past its port.
pub fn decode_host_n_port(data: &[u8], offset: usize) -> Result<(BACnetHostNPort, usize), Error> {
    let what = "BACnetHostNPort";
    let pos = expect_opening(data, offset, 0, what)?;
    let (host, pos) = decode_host_address(data, pos)?;
    let pos = expect_closing(data, pos, 0, what)?;
    let (port, pos) = decode_ctx_unsigned::<u16>(data, pos, 1, what)?;
    Ok((BACnetHostNPort { host, port }, pos))
}

/// Encode one bare `BACnetBDTEntry` SEQUENCE.
///
/// Fails, leaving `buf` untouched, only when a host name is too long to
/// encode.
pub fn encode_bdt_entry(buf: &mut BytesMut, entry: &BACnetBDTEntry) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    tags::encode_opening_tag(&mut encoded, 0);
    encode_host_n_port(&mut encoded, &entry.bbmd_address)?;
    tags::encode_closing_tag(&mut encoded, 0);
    if let Some(mask) = &entry.broadcast_mask {
        primitives::encode_ctx_octet_string(&mut encoded, 1, mask);
    }
    buf.extend_from_slice(&encoded);
    Ok(())
}

/// Decode one bare `BACnetBDTEntry` SEQUENCE at `offset`; returns it and the
/// offset past it. A broadcast mask, when present, must be 4 octets.
pub fn decode_bdt_entry(data: &[u8], offset: usize) -> Result<(BACnetBDTEntry, usize), Error> {
    let what = "BACnetBDTEntry";
    let pos = expect_opening(data, offset, 0, what)?;
    let (bbmd_address, pos) = decode_host_n_port(data, pos)?;
    let pos = expect_closing(data, pos, 0, what)?;
    let (mask, pos) = decode_optional_ctx(data, pos, 1, what, |data, at, tag, what| {
        decode_ctx_fixed(data, at, tag, 4, "broadcast mask", what)
    })?;
    let broadcast_mask = mask.map(|octets| [octets[0], octets[1], octets[2], octets[3]]);
    Ok((
        BACnetBDTEntry {
            bbmd_address,
            broadcast_mask,
        },
        pos,
    ))
}

/// Encode one bare `BACnetFDTEntry` SEQUENCE.
pub fn encode_fdt_entry(buf: &mut BytesMut, entry: &BACnetFDTEntry) {
    let mut address = [0u8; 18];
    let len = match entry.address {
        SocketAddr::V4(v4) => {
            address[..4].copy_from_slice(&v4.ip().octets());
            address[4..6].copy_from_slice(&v4.port().to_be_bytes());
            6
        }
        SocketAddr::V6(v6) => {
            address[..16].copy_from_slice(&v6.ip().octets());
            address[16..].copy_from_slice(&v6.port().to_be_bytes());
            18
        }
    };
    primitives::encode_ctx_octet_string(buf, 0, &address[..len]);
    primitives::encode_ctx_unsigned(buf, 1, entry.time_to_live.into());
    primitives::encode_ctx_unsigned(buf, 2, entry.remaining_time_to_live.into());
}

/// Decode one bare `BACnetFDTEntry` SEQUENCE at `offset`; returns it and the
/// offset past it. The address must be 6 or 18 octets.
pub fn decode_fdt_entry(data: &[u8], offset: usize) -> Result<(BACnetFDTEntry, usize), Error> {
    let what = "BACnetFDTEntry";
    let (tag, start) = tags::decode_tag(data, offset)?;
    if !tag.is_context(0) {
        return Err(misplaced_tag(
            data,
            &tag,
            Some(0),
            offset,
            format!("{what}: expected bacnetip-address [0]"),
        ));
    }
    let (octets, pos) = contents(data, start, tag.length)?;
    let address = match octets.len() {
        6 => SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
            u16::from_be_bytes([octets[4], octets[5]]),
        )),
        18 => {
            let mut ip = [0u8; 16];
            ip.copy_from_slice(&octets[..16]);
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(ip),
                u16::from_be_bytes([octets[16], octets[17]]),
                0,
                0,
            ))
        }
        other => {
            return Err(Error::decoding(
                offset,
                format!("{what} bacnetip-address [0]: {other} octets, expected 6 or 18"),
            ))
        }
    };
    let (time_to_live, pos) = decode_ctx_unsigned::<u16>(data, pos, 1, what)?;
    let (remaining_time_to_live, pos) = decode_ctx_unsigned::<u16>(data, pos, 2, what)?;
    Ok((
        BACnetFDTEntry {
            address,
            time_to_live,
            remaining_time_to_live,
        },
        pos,
    ))
}

/// Encode a BACnetLIST of `BACnetBDTEntry` as concatenated elements.
///
/// Fails, leaving `buf` untouched, when any element fails to encode.
pub fn encode_bdt_entry_list(buf: &mut BytesMut, entries: &[BACnetBDTEntry]) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    for entry in entries {
        encode_bdt_entry(&mut encoded, entry)?;
    }
    buf.extend_from_slice(&encoded);
    Ok(())
}

/// Encode a BACnetLIST of `BACnetFDTEntry` as concatenated elements.
pub fn encode_fdt_entry_list(buf: &mut BytesMut, entries: &[BACnetFDTEntry]) {
    for entry in entries {
        encode_fdt_entry(buf, entry);
    }
}

/// Decode a complete BACnetLIST of `BACnetBDTEntry`. Every byte must belong
/// to an element; an empty input is an empty list.
pub fn decode_bdt_entry_list(data: &[u8]) -> Result<Vec<BACnetBDTEntry>, Error> {
    decode_list(data, "BACnetBDTEntry list", decode_bdt_entry)
}

/// Decode a complete BACnetLIST of `BACnetFDTEntry`. Every byte must belong
/// to an element; an empty input is an empty list.
pub fn decode_fdt_entry_list(data: &[u8]) -> Result<Vec<BACnetFDTEntry>, Error> {
    decode_list(data, "BACnetFDTEntry list", decode_fdt_entry)
}

fn decode_list<T>(
    data: &[u8],
    what: &str,
    decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> Result<Vec<T>, Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        if values.len() == MAX_FRAMED_ITEMS {
            return Err(Error::decoding(
                offset,
                format!("{what}: more than {MAX_FRAMED_ITEMS} entries"),
            ));
        }
        let (value, end) = decode(data, offset)?;
        values.push(value);
        offset = end;
    }
    Ok(values)
}
