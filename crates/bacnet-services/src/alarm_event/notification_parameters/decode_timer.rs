use super::*;

pub(super) fn decode_change_of_timer(
    data: &[u8],
    inner_start: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] new-state
    let (t, p) = tags::decode_tag(data, pos)?;
    let end = p + t.length as usize;
    if end > data.len() {
        return Err(Error::decoding(p, "ChangeOfTimer: truncated new_state"));
    }
    let new_state = primitives::decode_unsigned(&data[p..end])? as u32;
    pos = end;
    // [1] status-flags
    let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
    let sf_end = sf_pos + sf_tag.length as usize;
    if sf_end > data.len() {
        return Err(Error::decoding(sf_pos, "ChangeOfTimer: truncated flags"));
    }
    let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
    pos = sf_end;
    // [2] update-time: BACnetDateTime — opening/closing [2]
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 2 {
        return Err(Error::decoding(
            pos,
            "ChangeOfTimer: expected opening [2] for update-time",
        ));
    }
    pos = p;
    // Application-tagged Date
    let (d_tag, d_pos) = tags::decode_tag(data, pos)?;
    let d_end = d_pos + d_tag.length as usize;
    if d_end > data.len() {
        return Err(Error::decoding(
            d_pos,
            "ChangeOfTimer: truncated update date",
        ));
    }
    let update_date = Date::decode(&data[d_pos..d_end])?;
    pos = d_end;
    // Application-tagged Time
    let (t_tag, t_pos) = tags::decode_tag(data, pos)?;
    let t_end = t_pos + t_tag.length as usize;
    if t_end > data.len() {
        return Err(Error::decoding(
            t_pos,
            "ChangeOfTimer: truncated update time",
        ));
    }
    let update_time_val = Time::decode(&data[t_pos..t_end])?;
    pos = t_end;
    // Closing tag [2]
    let (ct, cp) = tags::decode_tag(data, pos)?;
    if !ct.is_closing || ct.number != 2 {
        return Err(Error::decoding(pos, "ChangeOfTimer: expected closing [2]"));
    }
    pos = cp;
    let update_time = (update_date, update_time_val);
    // [3] last-state-change
    let (t, p) = tags::decode_tag(data, pos)?;
    let end = p + t.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            p,
            "ChangeOfTimer: truncated last_state_change",
        ));
    }
    let last_state_change = primitives::decode_unsigned(&data[p..end])? as u32;
    pos = end;
    // [4] initial-timeout
    let (t, p) = tags::decode_tag(data, pos)?;
    let end = p + t.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            p,
            "ChangeOfTimer: truncated initial_timeout",
        ));
    }
    let initial_timeout = primitives::decode_unsigned(&data[p..end])? as u32;
    pos = end;
    // [5] expiration-time: BACnetDateTime — opening/closing [5]
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 5 {
        return Err(Error::decoding(
            pos,
            "ChangeOfTimer: expected opening [5] for expiration-time",
        ));
    }
    pos = p;
    let (d_tag, d_pos) = tags::decode_tag(data, pos)?;
    let d_end = d_pos + d_tag.length as usize;
    if d_end > data.len() {
        return Err(Error::decoding(
            d_pos,
            "ChangeOfTimer: truncated expiration date",
        ));
    }
    let exp_date = Date::decode(&data[d_pos..d_end])?;
    pos = d_end;
    let (t_tag, t_pos) = tags::decode_tag(data, pos)?;
    let t_end = t_pos + t_tag.length as usize;
    if t_end > data.len() {
        return Err(Error::decoding(
            t_pos,
            "ChangeOfTimer: truncated expiration time",
        ));
    }
    let exp_time = Time::decode(&data[t_pos..t_end])?;
    let _ = (pos, t_end);
    let expiration_time = (exp_date, exp_time);
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
    let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
    let sf_end = sf_pos + sf_tag.length as usize;
    if sf_end > data.len() {
        return Err(Error::decoding(
            sf_pos,
            "ChangeOfDiscreteValue: truncated flags",
        ));
    }
    let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
    Ok(NotificationParameters::ChangeOfDiscreteValue {
        new_value,
        status_flags,
    })
}
