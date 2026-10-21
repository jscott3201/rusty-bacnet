//! `Recipient_List` full ASN.1 framing (ASHRAE 135-2020 Clauses 12.21, 21).
//!
//! The property is a BACnetLIST of `BACnetDestination`, so on the wire it is
//! the destinations back to back with no list wrapper. A destination has
//! seven members and none carries a context tag, so they travel in this order
//! as application-tagged elements (the recipient CHOICE brings its own tags):
//!
//! | Member | Wire form |
//! |---|---|
//! | `valid-days` | Bit String (`BACnetDaysOfWeek`) |
//! | `from-time` | Time |
//! | `to-time` | Time |
//! | `recipient` | `BACnetRecipient` CHOICE, see below |
//! | `process-identifier` | Unsigned, 32-bit range |
//! | `issue-confirmed-notifications` | Boolean |
//! | `transitions` | Bit String (`BACnetEventTransitionBits`) |
//!
//! `BACnetRecipient` picks either `device [0]`, an object identifier, or
//! `address [1]`, a `BACnetAddress`. The device form tags a primitive
//! ObjectIdentifier — a context-specific PRIMITIVE tag 0, length 4.
//! `BACnetAddress` is constructed (a 16-bit network number, then the MAC
//! address as an OCTET STRING, both application-tagged), so `address [1]` is
//! an opening tag 1 / application-tagged Unsigned16 + OCTET STRING / closing
//! tag 1.
//!
//! A `BACnetAddress` MAC is at most [`BACnetAddress::MAX_MAC_LEN`] octets in
//! both directions: [`decode_recipient`] refuses a longer one before copying
//! it (#1124 for configured recipients, #1156 everywhere else) and
//! [`encode_recipient`] refuses to write one. Every other codec that carries a
//! `BACnetAddress` (ValueSource, the AuditLogQuery filters) shares
//! [`check_decoded_mac_len`] and [`check_encoded_mac_len`], so the bound has
//! one definition. The source addresses the stack learns off the network fit
//! it too: the NPDU codec bounds SADR the same way (#1141), and the network
//! layer drops a frame from a longer link-layer source MAC (#1198).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::error::{DecodeError, DecodingKind, EncodeError, Error};
use bacnet_types::primitives::Time;
use bacnet_types::MacAddr;
use bytes::BytesMut;

use crate::primitives;
use crate::tags::{self, TagClass};

use super::tagged::{
    contents, decode_app_bit_string, decode_app_unsigned, decode_ctx_object_id, expect_closing,
    misplaced,
};
use super::MAX_FRAMED_ITEMS;

/// Whether a `BACnetAddress` MAC of `length` octets is longer than
/// [`BACnetAddress::MAX_MAC_LEN`]. The one place the bound is compared.
fn mac_len_excess(length: usize) -> bool {
    length > BACnetAddress::MAX_MAC_LEN
}

/// Refuse, as a decode error at `offset`, a `BACnetAddress` mac-address whose
/// OCTET STRING holds more than [`BACnetAddress::MAX_MAC_LEN`] octets (#1156),
/// or any other OCTET STRING that names a MAC on a data link, such as the
/// You-Are device MAC (#1200). Decoders call it on the tag's length, before
/// copying any octet.
pub fn check_decoded_mac_len(length: usize, offset: usize, what: &'static str) -> Result<(), Error> {
    if !mac_len_excess(length) {
        return Ok(());
    }
    Err(DecodeError::new(
        DecodingKind::Overflow,
        offset,
        "mac-address of {} octets exceeds the {}-octet limit",
    )
    .within(what)
    .with_values(length as u64, BACnetAddress::MAX_MAC_LEN as u64)
    .into())
}

/// Refuse to encode a MAC longer than [`BACnetAddress::MAX_MAC_LEN`] octets,
/// which [`check_decoded_mac_len`] would refuse on the way back in: a
/// `BACnetAddress` mac-address (#1156) or the You-Are device MAC (#1200).
pub fn check_encoded_mac_len(mac: &[u8], what: &'static str) -> Result<(), Error> {
    if !mac_len_excess(mac.len()) {
        return Ok(());
    }
    Err(
        EncodeError::new("mac-address of {} octets exceeds the {}-octet limit")
            .within(what)
            .with_values(mac.len() as u32, BACnetAddress::MAX_MAC_LEN as u32)
            .into(),
    )
}

/// [`check_encoded_mac_len`] for the address form of a recipient; the device
/// form always encodes.
pub(super) fn check_encoded_recipient(recipient: &BACnetRecipient) -> Result<(), Error> {
    match recipient {
        BACnetRecipient::Device(_) => Ok(()),
        BACnetRecipient::Address(address) => {
            check_encoded_mac_len(&address.mac_address, "BACnetRecipient")
        }
    }
}

/// Encode one [`BACnetDestination`].
///
/// The two bit-string members travel MSB-first per Clause 20.2.10:
/// `BACnetDaysOfWeek` is 7 bits (monday(0) at 0x80), so the fill octet has 1
/// unused bit; `BACnetEventTransitionBits` is 3 bits, so 5 unused.
///
/// A recipient MAC past [`BACnetAddress::MAX_MAC_LEN`] octets is an error,
/// returned before `buf` changes.
pub fn encode_destination(buf: &mut BytesMut, dest: &BACnetDestination) -> Result<(), Error> {
    check_encoded_recipient(&dest.recipient)?;
    write_destination(buf, dest);
    Ok(())
}

/// Write a destination whose recipient has passed [`check_encoded_recipient`].
fn write_destination(buf: &mut BytesMut, dest: &BACnetDestination) {
    primitives::encode_app_bit_string(buf, 1, &[dest.valid_days.to_bacnet()]);
    primitives::encode_app_time(buf, &dest.from_time);
    primitives::encode_app_time(buf, &dest.to_time);
    write_recipient(buf, &dest.recipient);
    primitives::encode_app_unsigned(buf, dest.process_identifier as u64);
    primitives::encode_app_boolean(buf, dest.issue_confirmed_notifications);
    primitives::encode_app_bit_string(buf, 5, &[dest.transitions.to_bacnet()]);
}

/// Encode a `BACnetLIST of BACnetDestination`: plain concatenation, no
/// wrapper (Clause 12.21 `Recipient_List`). Every destination is checked
/// before `buf` changes, so a refused list writes nothing.
pub fn encode_destination_list(
    buf: &mut BytesMut,
    destinations: &[BACnetDestination],
) -> Result<(), Error> {
    for dest in destinations {
        check_encoded_recipient(&dest.recipient)?;
    }
    for dest in destinations {
        write_destination(buf, dest);
    }
    Ok(())
}

/// Encode a [`BACnetRecipient`]. An address MAC past
/// [`BACnetAddress::MAX_MAC_LEN`] octets is an error, returned before `buf`
/// changes.
pub fn encode_recipient(buf: &mut BytesMut, recipient: &BACnetRecipient) -> Result<(), Error> {
    check_encoded_recipient(recipient)?;
    write_recipient(buf, recipient);
    Ok(())
}

/// Write a recipient that has passed [`check_encoded_recipient`].
pub(super) fn write_recipient(buf: &mut BytesMut, recipient: &BACnetRecipient) {
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
///
/// An address whose MAC is longer than [`BACnetAddress::MAX_MAC_LEN`] octets
/// names no node this stack reaches, so it is a decode error, raised before
/// the MAC is copied. That holds for a configured recipient (a Recipient_List
/// destination or the Audit_Notification_Recipient, #1124) and for one a
/// service carries or the stack reports (COV subscription lists, audit
/// records, enrollment filters, #1156).
pub fn decode_recipient(data: &[u8], offset: usize) -> Result<(BACnetRecipient, usize), Error> {
    let what = "BACnetRecipient";
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) {
        // device [0] tags the primitive ObjectIdentifier.
        let (device, end) = decode_ctx_object_id(data, offset, 0, "BACnetRecipient device")?;
        return Ok((BACnetRecipient::Device(device), end));
    }
    if tag.is_opening_tag(1) {
        // network-number Unsigned16
        let (network_number, pos) = decode_app_unsigned::<u64>(data, pos, what)?;
        let network_number = u16::try_from(network_number).map_err(|_| {
            DecodeError::new(
                DecodingKind::OutOfRange,
                pos,
                "network-number exceeds Unsigned16",
            )
            .within(what)
        })?;
        // mac-address OCTET STRING (a zero-length string is a broadcast)
        let (mac_address, pos) = decode_app_mac_address(data, pos, what)?;
        let end = expect_closing(data, pos, 1, "BACnetRecipient address")?;
        return Ok((
            BACnetRecipient::Address(BACnetAddress {
                network_number,
                mac_address,
            }),
            end,
        ));
    }
    let label = if tag.is_opening {
        "expected [0] (device) or [1] (address), got opening tag [{}]"
    } else if tag.is_closing {
        "expected [0] (device) or [1] (address), got closing tag [{}]"
    } else {
        match tag.class {
            TagClass::Context => "expected [0] (device) or [1] (address), got primitive context tag [{}]",
            TagClass::Application => "expected [0] (device) or [1] (address), got application tag {}",
        }
    };
    Err(misplaced(data, &tag, None, offset, label)
        .within(what)
        .with_value(tag.number.into())
        .into())
}

/// Decode one [`BACnetDestination`] at `offset`; returns it and the offset
/// past the last member. Its recipient decodes through [`decode_recipient`],
/// so an address MAC past [`BACnetAddress::MAX_MAC_LEN`] octets is refused
/// (#1124).
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
    let valid_days = DaysOfWeek::from_bacnet(&days_data);
    // from-time / to-time.
    let (from_time, pos) = decode_app_time(data, pos, what)?;
    let (to_time, pos) = decode_app_time(data, pos, what)?;
    // recipient CHOICE, its address MAC bounded.
    let (recipient, pos) = decode_recipient(data, pos)?;
    // process-identifier Unsigned32.
    let (process_identifier, pos) = decode_app_unsigned::<u64>(data, pos, what)?;
    let process_identifier = u32::try_from(process_identifier).map_err(|_| {
        DecodeError::new(
            DecodingKind::OutOfRange,
            pos,
            "process-identifier exceeds Unsigned32",
        )
        .within(what)
    })?;
    // issue-confirmed-notifications BOOLEAN (application-tagged: the value
    // rides in the tag's L/V/T bits with no contents octets).
    let (tag, pos) = tags::decode_tag(data, pos)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::BOOLEAN {
        return Err(
            misplaced(data, &tag, None, pos, "expected application-tagged BOOLEAN")
                .within(what)
                .into(),
        );
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
    let transitions = EventTransitionBits::from_bacnet(&transitions_data);

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
            return Err(Error::overflow(
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

/// Decode the application-tagged OCTET STRING of a `BACnetAddress` MAC,
/// refusing one longer than [`BACnetAddress::MAX_MAC_LEN`] octets before
/// copying it.
pub(super) fn decode_app_mac_address(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(MacAddr, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::OCTET_STRING {
        return Err(misplaced(
            data,
            &tag,
            None,
            offset,
            "expected application-tagged OCTET STRING",
        )
        .within(what)
        .into());
    }
    check_decoded_mac_len(tag.length as usize, offset, what)?;
    let (mac, end) = contents(data, pos, tag.length)?;
    Ok((MacAddr::from_slice(mac), end))
}

/// Decode an application-tagged Time member (exactly 4 contents octets).
fn decode_app_time(data: &[u8], offset: usize, what: &'static str) -> Result<(Time, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.number != tags::app_tag::TIME {
        return Err(
            misplaced(data, &tag, None, offset, "expected application-tagged Time")
                .within(what)
                .into(),
        );
    }
    if tag.length != 4 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "Time has {} contents octets, expected 4",
        )
        .within(what)
        .with_value(tag.length.into())
        .into());
    }
    let (octets, end) = contents(data, pos, 4)?;
    Ok((Time::decode(octets)?, end))
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
    name: &'static str,
) -> Result<(), Error> {
    if data.len() != 1 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            0,
            "expected 1 content octet, got {}",
        )
        .within(name)
        .with_value(data.len() as u64)
        .into());
    }
    if unused_bits != expected_unused {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            0,
            "expected unused_bits {}, got {}",
        )
        .within(name)
        .with_values(expected_unused.into(), unused_bits.into())
        .into());
    }
    Ok(())
}
