use super::decode_changes::{
    decode_buffer_ready, decode_change_of_bitstring, decode_change_of_characterstring,
    decode_change_of_life_safety, decode_change_of_reliability, decode_change_of_state,
    decode_change_of_status_flags, decode_change_of_value, decode_command_failure, decode_extended,
};
use super::decode_helpers::{closing_tag_start, finish_variant as check_variant_end};
use super::decode_ranges::{
    decode_double_out_of_range, decode_floating_limit, decode_out_of_range,
    decode_signed_out_of_range, decode_unsigned_out_of_range, decode_unsigned_range,
};
use super::decode_timer::{decode_change_of_discrete_value, decode_change_of_timer};
use super::structured::{decode_access_event, decode_complex_event_type};
use super::*;
use crate::constructed::log_fields::decode_opaque;
use crate::constructed::tagged::contents;

/// Decode one notification-parameter choice, with an optional enclosing `[12]` close.
///
/// That close is the octet after the choice's own closing tag, so the last
/// octet of an unassigned choice's closing tag that reads as a closing
/// `[12]` (`[207]`'s is `FF CF`) stays part of the choice.
pub fn decode_notification_parameters(
    data: &[u8],
    offset: usize,
) -> Result<NotificationParameters, Error> {
    let enclosing =
        || closing_tag_start(data, data.len(), 12, "NotificationParameters event-values");
    // Where the choice itself ends: past its closing tag, or past a
    // primitive's contents (#2483), whose last octet may read as a close.
    let frame_end = tags::decode_tag(data, offset)
        .ok()
        .and_then(|(tag, start)| {
            if tag.is_opening {
                tags::extract_context_value(data, start, tag.number)
                    .ok()
                    .map(|(_, end)| end)
            } else if tag.is_closing {
                None
            } else {
                contents(data, start, tag.length).ok().map(|(_, end)| end)
            }
        });
    let end = match frame_end {
        Some(end) if end + 1 == data.len() && enclosing().is_ok() => end,
        Some(_) => data.len(),
        // Not a whole frame: fail as before, short of any closing [12].
        None => enclosing().unwrap_or(data.len()),
    };
    decode_bounded(data, offset, end)
}

/// Decode one notification-parameter choice that ends exactly at `end`.
pub(super) fn decode_bounded(
    data: &[u8],
    offset: usize,
    end: usize,
) -> Result<NotificationParameters, Error> {
    let data = data
        .get(..end)
        .ok_or_else(|| Error::decoding(end, "NotificationParameters boundary exceeds input"))?;
    let framed = data
        .get(offset..)
        .ok_or_else(|| Error::decoding(offset, "NotificationParameters offset exceeds input"))?;
    // Peek the inner opening tag to determine the variant
    if offset >= data.len() {
        return Err(Error::missing(
            offset,
            "NotificationParameters: empty payload",
        ));
    }
    let (inner_tag, inner_start) = tags::decode_tag(data, offset)?;
    if !inner_tag.is_opening {
        // Every alternative Clause 21 assigns is constructed; one under a
        // tag it doesn't may be primitive (#2483), kept with its form.
        if let Some((opaque, end)) = decode_opaque(data, &inner_tag, inner_start, is_opaque_tag)? {
            if end != data.len() {
                return Err(Error::trailing(
                    end,
                    "NotificationParameters has octets after the choice",
                ));
            }
            return Ok(NotificationParameters::Opaque(opaque));
        }
        return Err(misplaced_tag(
            data,
            &inner_tag,
            None,
            offset,
            "NotificationParameters: expected opening tag for variant",
        ));
    }
    let variant_tag = inner_tag.number;
    if variant_tag == 6 {
        let (_, next) = tags::extract_context_value(data, inner_start, 6)?;
        if next != data.len() {
            return Err(Error::trailing(
                next,
                "ComplexEventType has trailing encoded fields",
            ));
        }
    } else {
        validate_tlv_sequence(framed, "NotificationParameters")?;
    }
    let variant_body_end = closing_tag_start(
        data,
        data.len(),
        variant_tag,
        "NotificationParameters variant",
    )?;
    let finish_variant =
        |value, consumed| check_variant_end(value, consumed, variant_body_end, variant_tag);

    match variant_tag {
        // [1] Change of state
        1 => decode_change_of_state(data, inner_start, variant_body_end),
        // [2] Change of value
        2 => decode_change_of_value(data, inner_start, variant_body_end),
        // [5] Out of range
        5 => decode_out_of_range(data, inner_start, variant_body_end),
        // [6] Complex event type
        6 => decode_complex_event_type(data, inner_start, variant_body_end),
        // [10] Buffer ready
        10 => decode_buffer_ready(data, inner_start, variant_body_end),
        // [11] Unsigned range
        11 => decode_unsigned_range(data, inner_start, variant_body_end),
        // [0] Change of bitstring
        0 => decode_change_of_bitstring(data, inner_start, variant_body_end),
        // [3] Command failure
        3 => decode_command_failure(data, inner_start, variant_body_end),
        // [4] Floating limit
        4 => decode_floating_limit(data, inner_start, variant_body_end),
        // [8] Change of life safety
        8 => decode_change_of_life_safety(data, inner_start, variant_body_end),
        // [9] Extended
        9 => decode_extended(data, inner_start, variant_body_end),
        // [13] Access event
        13 => decode_access_event(data, inner_start, variant_body_end),
        // [14] Double out of range
        14 => decode_double_out_of_range(data, inner_start, variant_body_end),
        // [15] Signed out of range
        15 => decode_signed_out_of_range(data, inner_start, variant_body_end),
        // [16] Unsigned out of range
        16 => decode_unsigned_out_of_range(data, inner_start, variant_body_end),
        // [17] Change of characterstring
        17 => decode_change_of_characterstring(data, inner_start, variant_body_end),
        // [18] Change of status flags
        18 => decode_change_of_status_flags(data, inner_start, variant_body_end),
        // [19] Change of reliability
        19 => decode_change_of_reliability(data, inner_start, variant_body_end),
        // [21] Change of discrete value
        21 => decode_change_of_discrete_value(data, inner_start, variant_body_end),
        // [22] Change of timer
        22 => decode_change_of_timer(data, inner_start, variant_body_end),
        // A tag the 2020 production gives no alternative (a later
        // revision's, or one it deprecates or reserves): keep its body as
        // received. Its nested tags were checked above, so only a
        // well-formed alternative gets here.
        tag if is_opaque_tag(tag) => {
            let (body, _) = tags::extract_context_value(data, inner_start, tag)?;
            finish_variant(
                NotificationParameters::Opaque(OpaqueChoice {
                    tag,
                    constructed: true,
                    data: body.to_vec(),
                }),
                inner_start + body.len(),
            )
        }
        // 20, which Clause 21 leaves out to match the event type that
        // reports no values.
        other => Err(DecodeError::new(
            DecodingKind::InvalidTag,
            offset,
            "NotificationParameters context tag [{}] carries no values in Clause 21",
        )
        .with_value(other.into())
        .into()),
    }
}
