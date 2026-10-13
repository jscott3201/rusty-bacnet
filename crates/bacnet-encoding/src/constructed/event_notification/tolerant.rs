//! A notification decoder that drops a message text it can't read.
//!
//! A peer may send its message text in a character set this stack does not
//! decode (DBCS, JIS X 0208, UCS-4 and so on). That shouldn't lose the rest of
//! the notification, so the receiving client and the Event Log record decoder
//! read such a request with no message text. Only the text's characters are
//! forgiven: it must still be one well-framed primitive context 7 in its place
//! between the event type and the notify type, and every other field keeps
//! the strict decoder's checks.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use core::ops::Range;

use super::*;

/// [`decode_event_notification`], except that a message text whose characters
/// don't decode is dropped, so the request reads with `message_text: None`.
///
/// The text must still be a complete primitive context 7 directly after the
/// event type and directly before the notify type; a malformed frame, a
/// missing charset octet or any other bad field fails as it does in
/// [`decode_event_notification`].
pub fn decode_event_notification_tolerant(data: &[u8]) -> Result<EventNotificationRequest, Error> {
    let strict_error = match decode_event_notification(data) {
        Ok(notification) => return Ok(notification),
        Err(error) => error,
    };
    let Some(text) = unreadable_message_text(data) else {
        return Err(strict_error);
    };
    // One retry over a copy without the text, strictly shorter than the input.
    let mut without_text = Vec::with_capacity(data.len() - text.len());
    without_text.extend_from_slice(&data[..text.start]);
    without_text.extend_from_slice(&data[text.end..]);
    decode_event_notification(&without_text)
}

/// The byte range of the message text field when it is framed correctly but
/// its characters don't decode.
fn unreadable_message_text(data: &[u8]) -> Option<Range<usize>> {
    let mut offset = 0;
    // Step over the fixed fields [0] to [6] only, never searching inside a
    // value for something that looks like a tag.
    for number in 0..7 {
        let (tag, content) = tags::decode_tag(data, offset).ok()?;
        if number == 3 {
            if !tag.is_opening_tag(number) {
                return None;
            }
            offset = tags::extract_context_value(data, content, number).ok()?.1;
        } else {
            if !tag.is_context(number) {
                return None;
            }
            offset = content.checked_add(tag.length as usize)?;
            data.get(content..offset)?;
        }
    }
    let (tag, content) = tags::decode_tag(data, offset).ok()?;
    if !tag.is_context(7) || tag.length == 0 {
        return None;
    }
    let end = content.checked_add(tag.length as usize)?;
    let text = data.get(content..end)?;
    // The notify type must follow, so dropping this field can't promote a
    // second text field into the optional slot. Data that ends with the text
    // is read without it too, so the request reports its missing notify type
    // rather than the text, as any request cut short does.
    if end < data.len() && !tags::decode_tag(data, end).ok()?.0.is_context(8) {
        return None;
    }
    primitives::decode_character_string(text)
        .is_err()
        .then_some(offset..end)
}
