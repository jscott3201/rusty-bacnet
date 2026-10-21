use super::decode_helpers::decode_context_status_flags;
use super::*;

fn decode_date_time(
    data: &[u8],
    offset: usize,
    context_tag: u8,
    field: &'static str,
) -> Result<((Date, Time), usize), Error> {
    let (opening, mut pos) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(context_tag) {
        return Err(misplaced(
            data,
            &opening,
            Some(context_tag),
            offset,
            "expected opening tag {}",
        )
        .within(field)
        .with_value(context_tag.into())
        .into());
    }

    let (date_tag, content_start) = tags::decode_tag(data, pos)?;
    if date_tag.class != tags::TagClass::Application || date_tag.number != tags::app_tag::DATE {
        return Err(misplaced(data, &date_tag, None, pos, "expected Date")
            .within(field)
            .into());
    }
    if date_tag.length != 4 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            pos,
            "Date must be four octets",
        )
        .within(field)
        .into());
    }
    let content_end = content_start
        .checked_add(date_tag.length as usize)
        .ok_or_else(|| {
            DecodeError::new(
                DecodingKind::InvalidEncoding,
                content_start,
                "Date length overflow",
            )
            .within(field)
        })?;
    let date = Date::decode(
        data.get(content_start..content_end)
            .ok_or_else(|| Error::buffer_too_short(content_end, data.len()))?,
    )?;
    pos = content_end;

    let (time_tag, content_start) = tags::decode_tag(data, pos)?;
    if time_tag.class != tags::TagClass::Application || time_tag.number != tags::app_tag::TIME {
        return Err(misplaced(data, &time_tag, None, pos, "expected Time")
            .within(field)
            .into());
    }
    if time_tag.length != 4 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            pos,
            "Time must be four octets",
        )
        .within(field)
        .into());
    }
    let content_end = content_start
        .checked_add(time_tag.length as usize)
        .ok_or_else(|| {
            DecodeError::new(
                DecodingKind::InvalidEncoding,
                content_start,
                "Time length overflow",
            )
            .within(field)
        })?;
    let time = Time::decode(
        data.get(content_start..content_end)
            .ok_or_else(|| Error::buffer_too_short(content_end, data.len()))?,
    )?;
    pos = content_end;

    let (closing, next) = tags::decode_tag(data, pos)?;
    if !closing.is_closing_tag(context_tag) {
        return Err(
            DecodeError::new(unclosed_kind(&closing), pos, "expected closing tag {}")
                .within(field)
                .with_value(context_tag.into())
                .into(),
        );
    }
    Ok(((date, time), next))
}

pub(super) fn decode_change_of_timer(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] new-state
    let (new_state, pos) = decode_context_enum(
        data,
        inner_start,
        0,
        "ChangeOfTimer new-state",
        TimerState::from_raw,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfTimer status-flags")?;
    // [2] update-time: BACnetDateTime — opening/closing [2]
    let (update_time, pos) = decode_date_time(data, pos, 2, "ChangeOfTimer update-time")?;
    let mut pos = pos;
    let last_state_change =
        if pos < variant_body_end && tags::decode_tag(data, pos)?.0.is_context(3) {
            let (value, next) = decode_context_enum(
                data,
                pos,
                3,
                "ChangeOfTimer last-state-change",
                TimerTransition::from_raw,
            )?;
            pos = next;
            Some(value)
        } else {
            None
        };
    let initial_timeout = if pos < variant_body_end && tags::decode_tag(data, pos)?.0.is_context(4)
    {
        let (value, next) = decode_context_u32(data, pos, 4, "ChangeOfTimer initial-timeout")?;
        pos = next;
        Some(value)
    } else {
        None
    };
    let expiration_time =
        if pos < variant_body_end && tags::decode_tag(data, pos)?.0.is_opening_tag(5) {
            let (value, next) = decode_date_time(data, pos, 5, "ChangeOfTimer expiration-time")?;
            pos = next;
            Some(value)
        } else {
            None
        };
    if pos != variant_body_end {
        return Err(Error::trailing(
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
        return Err(misplaced_tag(
            data,
            &t,
            Some(0),
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
        return Err(Error::trailing(
            pos,
            "ChangeOfDiscreteValue unexpected fields before closing tag 21",
        ));
    }
    Ok(NotificationParameters::ChangeOfDiscreteValue {
        new_value,
        status_flags,
    })
}
