//! The generic view of tagged octets the formatter has no model for: each
//! field by its context tag, `[0] 30, [1] (…)`, an application value as
//! the formatter writes it.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_types::enums::PropertyIdentifier;

use super::primitive::text;
use super::{Floats, raw_text};

/// Deepest nesting of opening and closing tags shown before the rest is
/// left as hex.
const MAX_DEPTH: usize = 8;

/// `data` as its tagged fields: a context primitive as `[n] [raw …]`
/// (its datatype isn't known, so its octets aren't read as a number), a
/// constructed field as `[n] (…)`, and an application value as its text. Octets that
/// don't parse end the view as `[raw …]`.
pub(super) fn fields_text(data: &[u8], floats: Floats) -> String {
    let mut parts = Vec::new();
    let (end, _) = fields(data, 0, None, 0, floats, &mut parts);
    if end < data.len() {
        parts.push(raw_text(&data[end..]));
    }
    parts.join(", ")
}

/// The fields from `offset` up to the closing tag `closing` (or the end),
/// pushed to `parts`. Returns the offset reached, past the closing tag, and
/// whether that tag was found; on octets that don't parse, the offset where
/// they start.
fn fields(
    data: &[u8],
    mut offset: usize,
    closing: Option<u8>,
    depth: usize,
    floats: Floats,
    parts: &mut Vec<String>,
) -> (usize, bool) {
    // Any property: no enumeration, named bits or state.
    const ANY: PropertyIdentifier = PropertyIdentifier::LOG_BUFFER;
    while offset < data.len() {
        let Ok((tag, contents)) = decode_tag(data, offset) else {
            return (offset, false);
        };
        if tag.is_closing {
            return if Some(tag.number) == closing {
                (contents, true)
            } else {
                (offset, false)
            };
        }
        if tag.class == TagClass::Application {
            let Ok((value, next)) = decode_application_value(data, offset) else {
                return (offset, false);
            };
            parts.push(text(ANY, &value, floats));
            offset = next;
            continue;
        }
        if tag.is_opening {
            if depth >= MAX_DEPTH {
                return (offset, false);
            }
            let mut inner = Vec::new();
            let (next, closed) = fields(
                data,
                contents,
                Some(tag.number),
                depth + 1,
                floats,
                &mut inner,
            );
            if !closed {
                return (offset, false);
            }
            parts.push(format!("[{}] ({})", tag.number, inner.join(", ")));
            offset = next;
            continue;
        }
        let Some(octets) = contents
            .checked_add(tag.length as usize)
            .and_then(|end| data.get(contents..end))
        else {
            return (offset, false);
        };
        if octets.is_empty() {
            parts.push(format!("[{}]", tag.number));
            offset = contents;
            continue;
        }
        parts.push(format!("[{}] {}", tag.number, raw_text(octets)));
        offset = contents + octets.len();
    }
    (offset, closing.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_show_context_tags_values_and_nesting() {
        // [0] 30, [1] ( REAL 1.5 ), [2] 0x0102030405
        let data = [
            0x09, 30, 0x1E, 0x44, 0x3F, 0xC0, 0x00, 0x00, 0x1F, 0x2D, 5, 1, 2, 3, 4, 5,
        ];
        assert_eq!(
            fields_text(&data, Floats::Exact),
            "[0] [raw 1e], [1] (1.5), [2] [raw 01 02 03 04 05]"
        );
        // A field cut short ends the view as raw octets.
        assert_eq!(
            fields_text(&[0x09, 30, 0x1A, 1], Floats::Exact),
            "[0] [raw 1e], [raw 1a 01]"
        );
    }
}
