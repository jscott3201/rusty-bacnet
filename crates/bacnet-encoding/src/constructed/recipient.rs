//! `Recipient_List` full ASN.1 framing (ASHRAE 135-2020 Clauses 12.21, 21).
//!
//! ```text
//! Recipient_List ::= BACnetLIST OF BACnetDestination    -- concatenation, no wrapper
//!
//! BACnetDestination ::= SEQUENCE {
//!     valid-days                    BACnetDaysOfWeek,          -- untagged
//!     from-time                     Time,                      -- untagged
//!     to-time                       Time,                      -- untagged
//!     recipient                     BACnetRecipient,           -- CHOICE
//!     process-identifier            Unsigned32,                -- untagged
//!     issue-confirmed-notifications BOOLEAN,                   -- untagged
//!     transitions                   BACnetEventTransitionBits  -- untagged
//! }
//!
//! BACnetRecipient ::= CHOICE { device [0] BACnetObjectIdentifier,
//!                              address [1] BACnetAddress }
//!
//! BACnetAddress ::= SEQUENCE { network-number Unsigned16,      -- untagged
//!                              mac-address OCTET STRING }      -- untagged
//! ```
//!
//! Tag forms: the destination members are UNTAGGED, so they travel as seven
//! application-tagged elements in order (Bit String, Time, Time, recipient,
//! Unsigned, Boolean, Bit String). `device [0]` tags a primitive
//! ObjectIdentifier — a context-specific PRIMITIVE tag 0, length 4.
//! `BACnetAddress` is constructed, so `address [1]` is an opening tag 1 /
//! application-tagged Unsigned16 + OCTET STRING / closing tag 1.

use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, Time};
use bacnet_types::MacAddr;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};

use super::MAX_FRAMED_ITEMS;

/// The two bit-string members travel MSB-first per Clause 20.2.10:
/// `BACnetDaysOfWeek` is 7 bits (monday(0) at 0x80), so the fill octet has 1
/// unused bit; `BACnetEventTransitionBits` is 3 bits, so 5 unused.
use bacnet_types::bitstring::{pack_octet, unpack_octet};

/// Encode one [`BACnetDestination`].
pub fn encode_destination(buf: &mut BytesMut, dest: &BACnetDestination) {
    primitives::encode_app_bit_string(buf, 1, &[pack_octet(dest.valid_days)]);
    primitives::encode_app_time(buf, &dest.from_time);
    primitives::encode_app_time(buf, &dest.to_time);
    encode_recipient(buf, &dest.recipient);
    primitives::encode_app_unsigned(buf, dest.process_identifier as u64);
    primitives::encode_app_boolean(buf, dest.issue_confirmed_notifications);
    primitives::encode_app_bit_string(buf, 5, &[pack_octet(dest.transitions)]);
}

/// Encode a `BACnetLIST of BACnetDestination`: plain concatenation, no
/// wrapper (Clause 12.21 `Recipient_List`).
pub fn encode_destination_list(buf: &mut BytesMut, destinations: &[BACnetDestination]) {
    for dest in destinations {
        encode_destination(buf, dest);
    }
}

/// Encode a [`BACnetRecipient`].
pub fn encode_recipient(buf: &mut BytesMut, recipient: &BACnetRecipient) {
    match recipient {
        BACnetRecipient::Device(oid) => {
            // device [0] tags the primitive ObjectIdentifier.
            tags::encode_tag(buf, 0, TagClass::Context, 4);
            buf.extend_from_slice(&oid.encode());
        }
        BACnetRecipient::Address(address) => {
            // address [1] tags the constructed BACnetAddress.
            tags::encode_opening_tag(buf, 1);
            primitives::encode_app_unsigned(buf, address.network_number as u64);
            primitives::encode_app_octet_string(buf, &address.mac_address);
            tags::encode_closing_tag(buf, 1);
        }
    }
}

/// Decode one [`BACnetRecipient`] at `offset`.
pub fn decode_recipient(data: &[u8], offset: usize) -> Result<(BACnetRecipient, usize), Error> {
    let what = "BACnetRecipient";
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) {
        // device [0] — primitive, exactly 4 contents octets.
        if tag.length != 4 {
            return Err(Error::decoding(
                offset,
                format!(
                    "{what}: device [0] expects 4 contents octets, got {}",
                    tag.length
                ),
            ));
        }
        let end = pos + 4;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        return Ok((
            BACnetRecipient::Device(ObjectIdentifier::decode(&data[pos..end])?),
            end,
        ));
    }
    if tag.is_opening_tag(1) {
        // network-number Unsigned16
        let (network_number, pos) = decode_app_unsigned(data, pos, what)?;
        let network_number = u16::try_from(network_number).map_err(|_| {
            Error::decoding(pos, format!("{what}: network-number exceeds Unsigned16"))
        })?;
        // mac-address OCTET STRING (a zero-length string is a broadcast)
        let (mac_address, pos) = decode_app_octet_string(data, pos, what)?;
        let (close, close_pos) = tags::decode_tag(data, pos)?;
        if !close.is_closing_tag(1) {
            return Err(Error::decoding(
                pos,
                format!("{what}: missing closing tag [1] for BACnetAddress"),
            ));
        }
        return Ok((
            BACnetRecipient::Address(BACnetAddress {
                network_number,
                mac_address,
            }),
            close_pos,
        ));
    }
    Err(Error::decoding(
        offset,
        format!(
            "{what}: expected [0] (device) or [1] (address), got {}",
            if tag.is_opening {
                format!("opening tag [{}]", tag.number)
            } else if tag.is_closing {
                format!("closing tag [{}]", tag.number)
            } else {
                match tag.class {
                    TagClass::Context => format!("primitive context tag [{}]", tag.number),
                    TagClass::Application => format!("application tag {}", tag.number),
                }
            }
        ),
    ))
}

/// Decode one [`BACnetDestination`] at `offset`; returns it and the offset
/// past the last member.
pub fn decode_destination(data: &[u8], offset: usize) -> Result<(BACnetDestination, usize), Error> {
    let what = "BACnetDestination";
    // valid-days: BACnetDaysOfWeek (bit 0 = Monday, MSB-first on the wire).
    let ((days_unused, days_data), pos) = decode_app_bit_string(data, offset, what)?;
    check_fixed_bit_string(
        days_unused,
        &days_data,
        1,
        "valid-days (BACnetDaysOfWeek: 7 bits)",
    )?;
    let valid_days = unpack_octet(&days_data, 7);
    // from-time / to-time.
    let (from_time, pos) = decode_app_time(data, pos, what)?;
    let (to_time, pos) = decode_app_time(data, pos, what)?;
    // recipient CHOICE.
    let (recipient, pos) = decode_recipient(data, pos)?;
    // process-identifier Unsigned32.
    let (process_identifier, pos) = decode_app_unsigned(data, pos, what)?;
    let process_identifier = u32::try_from(process_identifier).map_err(|_| {
        Error::decoding(
            pos,
            format!("{what}: process-identifier exceeds Unsigned32"),
        )
    })?;
    // issue-confirmed-notifications BOOLEAN (application-tagged: the value
    // rides in the tag's L/V/T bits with no contents octets).
    let (tag, pos) = tags::decode_tag(data, pos)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::BOOLEAN {
        return Err(Error::decoding(
            pos,
            format!("{what}: expected application-tagged BOOLEAN"),
        ));
    }
    let issue_confirmed_notifications = tag.length != 0;
    // transitions: BACnetEventTransitionBits (3 bits).
    let ((transitions_unused, transitions_data), pos) = decode_app_bit_string(data, pos, what)?;
    check_fixed_bit_string(
        transitions_unused,
        &transitions_data,
        5,
        "transitions (BACnetEventTransitionBits: 3 bits)",
    )?;
    let transitions = unpack_octet(&transitions_data, 3);

    Ok((
        BACnetDestination {
            valid_days,
            from_time,
            to_time,
            recipient,
            process_identifier,
            issue_confirmed_notifications,
            transitions,
        },
        pos,
    ))
}

/// Decode a whole `Recipient_List` (`BACnetLIST of BACnetDestination` —
/// concatenated entries). Every entry must parse; trailing garbage errors.
pub fn decode_destination_list(data: &[u8]) -> Result<Vec<BACnetDestination>, Error> {
    let mut destinations = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if destinations.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::decoding(
                pos,
                "Recipient_List: destination count exceeds limit",
            ));
        }
        let (dest, new_pos) = decode_destination(data, pos)?;
        destinations.push(dest);
        pos = new_pos;
    }
    Ok(destinations)
}

// ---------------------------------------------------------------------------
// Small application-tagged member helpers
// ---------------------------------------------------------------------------

/// Decode an application-tagged Unsigned member, returning raw value + offset.
fn decode_app_unsigned(data: &[u8], offset: usize, what: &str) -> Result<(u64, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::UNSIGNED {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged Unsigned"),
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((primitives::decode_unsigned(&data[pos..end])?, end))
}

/// Decode an application-tagged OCTET STRING member.
fn decode_app_octet_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(MacAddr, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::OCTET_STRING {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged OCTET STRING"),
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((MacAddr::from_slice(&data[pos..end]), end))
}

/// Decode an application-tagged Time member (exactly 4 contents octets).
fn decode_app_time(data: &[u8], offset: usize, what: &str) -> Result<(Time, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::TIME || tag.length != 4 {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged Time (4 octets)"),
        ));
    }
    let end = pos + 4;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((Time::decode(&data[pos..end])?, end))
}

/// Fixed-width bit-string member validation. Per Clause 20.2.10 (initial
/// octet = unused-bit count; defined bits MSB-first) and the Clause 21
/// productions, the named-bit sets here fit in exactly one subsequent
/// octet: `BACnetDaysOfWeek` has 7 defined bits (unused_bits 1) and
/// `BACnetEventTransitionBits` has 3 (unused_bits 5). A zero-length,
/// overlong, or mismatched-unused-bits encoding is not a destination a
/// notifier may act on — reject it rather than unpacking an all-zero
/// (dormant) one.
fn check_fixed_bit_string(
    unused_bits: u8,
    data: &[u8],
    expected_unused: u8,
    name: &str,
) -> Result<(), Error> {
    if data.len() != 1 || unused_bits != expected_unused {
        return Err(Error::decoding(
            0,
            format!(
                "{name}: expected 1 content octet with unused_bits {expected_unused} \
                 (got {} content octet(s), unused_bits {unused_bits})",
                data.len()
            ),
        ));
    }
    Ok(())
}

/// Decode an application-tagged BIT STRING member, returning
/// `(unused_bits, data)` and the offset past it.
fn decode_app_bit_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::BIT_STRING {
        return Err(Error::decoding(
            offset,
            format!("{what}: expected application-tagged BIT STRING"),
        ));
    }
    let end = pos
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(pos, format!("{what}: length overflow")))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    let (unused_bits, bits) = primitives::decode_bit_string(&data[pos..end])?;
    Ok(((unused_bits, bits), end))
}
