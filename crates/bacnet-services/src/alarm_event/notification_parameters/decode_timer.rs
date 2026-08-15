use super::decode_helpers::decode_context_status_flags;
use super::*;
use crate::common::decode_context_u32;

fn decode_date_time(
    data: &[u8],
    offset: usize,
    context_tag: u8,
    field: &str,
) -> Result<((Date, Time), usize), Error> {
    let (opening, mut pos) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(context_tag) {
        return Err(Error::decoding(
            offset,
            format!("{field} expected opening tag {context_tag}"),
        ));
    }

    let (date_tag, content_start) = tags::decode_tag(data, pos)?;
    if date_tag.class != tags::TagClass::Application || date_tag.number != tags::app_tag::DATE {
        return Err(Error::decoding(pos, format!("{field} expected Date")));
    }
    let content_end = content_start
        .checked_add(date_tag.length as usize)
        .ok_or_else(|| Error::decoding(content_start, format!("{field} Date length overflow")))?;
    let date = Date::decode(
        data.get(content_start..content_end)
            .ok_or_else(|| Error::decoding(content_start, format!("{field} truncated Date")))?,
    )?;
    pos = content_end;

    let (time_tag, content_start) = tags::decode_tag(data, pos)?;
    if time_tag.class != tags::TagClass::Application || time_tag.number != tags::app_tag::TIME {
        return Err(Error::decoding(pos, format!("{field} expected Time")));
    }
    let content_end = content_start
        .checked_add(time_tag.length as usize)
        .ok_or_else(|| Error::decoding(content_start, format!("{field} Time length overflow")))?;
    let time = Time::decode(
        data.get(content_start..content_end)
            .ok_or_else(|| Error::decoding(content_start, format!("{field} truncated Time")))?,
    )?;
    pos = content_end;

    let (closing, next) = tags::decode_tag(data, pos)?;
    if !closing.is_closing_tag(context_tag) {
        return Err(Error::decoding(
            pos,
            format!("{field} expected closing tag {context_tag}"),
        ));
    }
    Ok(((date, time), next))
}

pub(super) fn decode_change_of_timer(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] new-state
    let (new_state, pos) = decode_context_u32(data, inner_start, 0, "ChangeOfTimer new-state")?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfTimer status-flags")?;
    // [2] update-time: BACnetDateTime — opening/closing [2]
    let (update_time, pos) = decode_date_time(data, pos, 2, "ChangeOfTimer update-time")?;
    // [3] last-state-change
    let (last_state_change, pos) =
        decode_context_u32(data, pos, 3, "ChangeOfTimer last-state-change")?;
    // [4] initial-timeout
    let (initial_timeout, pos) = decode_context_u32(data, pos, 4, "ChangeOfTimer initial-timeout")?;
    // [5] expiration-time: BACnetDateTime — opening/closing [5]
    let (expiration_time, pos) = decode_date_time(data, pos, 5, "ChangeOfTimer expiration-time")?;
    if pos != variant_body_end {
        return Err(Error::decoding(
            pos,
            "ChangeOfTimer unexpected fields before closing tag 22",
        ));
    }
    Ok(NotificationParameters::ChangeOfTimer {
        new_state,
        status_flags,
        update_time,
        last_state_change,
        initial_timeout,
        expiration_time,
    })
}

pub(super) fn decode_change_of_discrete_value(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] new-value — opening/closing, raw
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 0 {
        return Err(Error::decoding(
            pos,
            "ChangeOfDiscreteValue: expected opening [0]",
        ));
    }
    let (new_value, after) = extract_raw_context(data, p, 0)?;
    pos = after;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfDiscreteValue status-flags")?;
    if pos != variant_body_end {
        return Err(Error::decoding(
            pos,
            "ChangeOfDiscreteValue unexpected fields before closing tag 21",
        ));
    }
    Ok(NotificationParameters::ChangeOfDiscreteValue {
        new_value,
        status_flags,
    })
}
