//! Decoders of the `BACnetEventParameter` alternatives added for #2090:
//! command-failure, change-of-life-safety, buffer-ready, unsigned-range,
//! access-event, the three typed out-of-range forms, change-of-characterstring,
//! change-of-status-flags, change-of-discrete-value and change-of-timer.
//!
//! Split out of `mod.rs` to keep every file under the 700-LOC cap. Each
//! decoder starts after the alternative's opening tag and returns the offset
//! past its closing tag.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{AccessEvent, LifeSafetyState, TimerState};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::StatusFlags;

use crate::primitives;

use super::super::tagged::{
    Fixed, decode_app_character_string, decode_app_enumerated, decode_ctx_bit_string,
    decode_ctx_fixed, decode_ctx_primitive, decode_ctx_unsigned, expect_closing, expect_opening,
    next_is_closing,
};
use super::super::{MAX_FRAMED_ITEMS, decode_dopr_body};

type Decoded = (BACnetEventParameter, usize);

/// Decode the body of alternative `tag` at `pos`, just past its opening tag.
pub(super) fn decode(data: &[u8], pos: usize, tag: u8) -> Result<Decoded, Error> {
    use BACnetEventParameter as EP;
    let (value, pos) = match tag {
        3 => {
            let what = "BACnetEventParameter command-failure";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (feedback_property_reference, pos) = reference(data, pos, 1, what)?;
            let value = EP::CommandFailure {
                time_delay,
                feedback_property_reference,
            };
            (value, pos)
        }
        8 => {
            let what = "BACnetEventParameter change-of-life-safety";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (life_safety, pos) = enumerations(data, pos, 1, what)?;
            let (alarm, pos) = enumerations(data, pos, 2, what)?;
            let (mode_property_reference, pos) = reference(data, pos, 3, what)?;
            let value = EP::ChangeOfLifeSafety {
                time_delay,
                list_of_life_safety_alarm_values: life_safety
                    .into_iter()
                    .map(LifeSafetyState::from_raw)
                    .collect(),
                list_of_alarm_values: alarm.into_iter().map(LifeSafetyState::from_raw).collect(),
                mode_property_reference,
            };
            (value, pos)
        }
        10 => {
            let what = "BACnetEventParameter buffer-ready";
            let (notification_threshold, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (previous_notification_count, pos) =
                decode_ctx_unsigned::<u32>(data, pos, 1, what)?;
            let value = EP::BufferReady {
                notification_threshold,
                previous_notification_count,
            };
            (value, pos)
        }
        11 => {
            let what = "BACnetEventParameter unsigned-range";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (low_limit, pos) = decode_ctx_unsigned::<u64>(data, pos, 1, what)?;
            let (high_limit, pos) = decode_ctx_unsigned::<u64>(data, pos, 2, what)?;
            let value = EP::UnsignedRange {
                time_delay,
                low_limit,
                high_limit,
            };
            (value, pos)
        }
        13 => {
            let what = "BACnetEventParameter access-event";
            let (events, pos) = enumerations(data, pos, 0, what)?;
            let (access_event_time_reference, pos) = reference(data, pos, 1, what)?;
            let value = EP::AccessEvent {
                list_of_access_events: events.into_iter().map(AccessEvent::from_raw).collect(),
                access_event_time_reference,
            };
            (value, pos)
        }
        14 => {
            let what = "BACnetEventParameter double-out-of-range";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (low_limit, pos) = double(data, pos, 1, what)?;
            let (high_limit, pos) = double(data, pos, 2, what)?;
            let (deadband, pos) = double(data, pos, 3, what)?;
            let value = EP::DoubleOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            };
            (value, pos)
        }
        15 => {
            let what = "BACnetEventParameter signed-out-of-range";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (low_limit, pos) = signed(data, pos, 1, what)?;
            let (high_limit, pos) = signed(data, pos, 2, what)?;
            let (deadband, pos) = decode_ctx_unsigned::<u64>(data, pos, 3, what)?;
            let value = EP::SignedOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            };
            (value, pos)
        }
        16 => {
            let what = "BACnetEventParameter unsigned-out-of-range";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (low_limit, pos) = decode_ctx_unsigned::<u64>(data, pos, 1, what)?;
            let (high_limit, pos) = decode_ctx_unsigned::<u64>(data, pos, 2, what)?;
            let (deadband, pos) = decode_ctx_unsigned::<u64>(data, pos, 3, what)?;
            let value = EP::UnsignedOutOfRange {
                time_delay,
                low_limit,
                high_limit,
                deadband,
            };
            (value, pos)
        }
        17 => {
            let what = "BACnetEventParameter change-of-characterstring";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (list_of_alarm_values, pos) =
                list(data, pos, 1, what, |data, pos| {
                    decode_app_character_string(data, pos, what)
                })?;
            let value = EP::ChangeOfCharacterstring {
                time_delay,
                list_of_alarm_values,
            };
            (value, pos)
        }
        18 => {
            let what = "BACnetEventParameter change-of-status-flags";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (selected_flags, pos) = status_flags(data, pos, 1, what)?;
            let value = EP::ChangeOfStatusFlags {
                time_delay,
                selected_flags,
            };
            (value, pos)
        }
        21 => {
            let what = "BACnetEventParameter change-of-discrete-value";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            (EP::ChangeOfDiscreteValue { time_delay }, pos)
        }
        22 => {
            let what = "BACnetEventParameter change-of-timer";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (states, pos) = enumerations(data, pos, 1, what)?;
            let (update_time_reference, pos) = reference(data, pos, 2, what)?;
            let value = EP::ChangeOfTimer {
                time_delay,
                alarm_values: states.into_iter().map(TimerState::from_raw).collect(),
                update_time_reference,
            };
            (value, pos)
        }
        _ => {
            return Err(Error::decoding(
                pos,
                "BACnetEventParameter: no decoder for this alternative",
            ));
        }
    };
    let what = "BACnetEventParameter";
    let pos = expect_closing(data, pos, tag, what)?;
    Ok((value, pos))
}

/// A reference member `[tag]`: the reference's members between its opening
/// and closing tags.
fn reference(
    data: &[u8],
    pos: usize,
    tag: u8,
    what: &'static str,
) -> Result<(BACnetDeviceObjectPropertyReference, usize), Error> {
    let pos = expect_opening(data, pos, tag, what)?;
    let (reference, pos) = decode_dopr_body(data, pos, what)?;
    let pos = expect_closing(data, pos, tag, what)?;
    Ok((reference, pos))
}

/// A list member `[tag]`: items read by `item` up to its closing tag, at
/// most [`MAX_FRAMED_ITEMS`] of them.
fn list<T>(
    data: &[u8],
    pos: usize,
    tag: u8,
    what: &'static str,
    item: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> Result<(Vec<T>, usize), Error> {
    let mut pos = expect_opening(data, pos, tag, what)?;
    let mut items = Vec::new();
    while !next_is_closing(data, pos, tag)? {
        if items.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(pos, "BACnetEventParameter: list exceeds limit"));
        }
        let (value, next) = item(data, pos)?;
        items.push(value);
        pos = next;
    }
    let pos = expect_closing(data, pos, tag, what)?;
    Ok((items, pos))
}

/// A list of application-tagged ENUMERATED values under `[tag]`.
fn enumerations(
    data: &[u8],
    pos: usize,
    tag: u8,
    what: &'static str,
) -> Result<(Vec<u32>, usize), Error> {
    list(data, pos, tag, what, |data, pos| {
        decode_app_enumerated::<u32>(data, pos, what)
    })
}

/// A Double member `[tag]`.
fn double(data: &[u8], pos: usize, tag: u8, what: &'static str) -> Result<(f64, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, pos, tag, Fixed::DOUBLE, what)?;
    Ok((primitives::decode_double(octets)?, end))
}

/// An INTEGER member `[tag]` that fits 32 bits.
fn signed(data: &[u8], pos: usize, tag: u8, what: &'static str) -> Result<(i32, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, pos, tag, what)?;
    Ok((primitives::decode_signed(octets)?, end))
}

/// A BACnetStatusFlags member `[tag]`: exactly four bits, the padding zero.
fn status_flags(
    data: &[u8],
    pos: usize,
    tag: u8,
    what: &'static str,
) -> Result<(StatusFlags, usize), Error> {
    let ((unused_bits, octets), end) = decode_ctx_bit_string(data, pos, tag, what)?;
    match (unused_bits, octets.as_slice()) {
        (4, [bits]) if bits & 0x0f == 0 => Ok((StatusFlags::from_bits_retain(bits >> 4), end)),
        _ => Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            pos,
            "[{}] status flags must be four bits with zero padding",
        )
        .within(what)
        .with_value(tag.into())
        .into()),
    }
}
