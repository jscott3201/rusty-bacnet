//! The generic view of tagged octets the formatter has no model for: each
//! field by its context tag, `[0] 30, [1] (…)`, an application value as
//! the formatter writes it.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_types::enums::PropertyIdentifier;

use super::primitive::text;
use super::{Datum, Fields, Floats, raw_text};
use crate::octets::hex;

/// The element kind the data names an unknown choice by.
const UNKNOWN_CHOICE: &str = "unknown-choice";

/// An alternative of a CHOICE under a context tag the standard doesn't
/// assign, kept as received (#2474): a log record's datum, a Trend Log
/// Multiple member's value. As text, the tagged view of the alternative
/// itself: `[11] [raw 01 02]` for a primitive one, `[30] ([0] [raw 1e])`
/// for a constructed one. As data, an `unknown-choice` element whose fields
/// are its `tag`, whether it is `constructed`, and its octets (a
/// primitive's contents, a constructed one's body) as hex under `data`.
///
/// A constructed body may hold an application CharacterString, so the
/// element isn't plain: front ends treat its text as device text.
pub fn unknown_choice(tag: u8, constructed: bool, data: &[u8], floats: Floats) -> (String, Datum) {
    let text = if constructed {
        format!("[{tag}] ({})", fields_text(data, floats))
    } else if data.is_empty() {
        format!("[{tag}]")
    } else {
        format!("[{tag}] {}", raw_text(data))
    };
    let fields = Fields::new(vec![
        ("tag".into(), Datum::Unsigned(u64::from(tag))),
        ("constructed".into(), Datum::Boolean(constructed)),
        ("data".into(), Datum::Raw { raw: hex(data) }),
    ]);
    let datum = Datum::Constructed {
        element: UNKNOWN_CHOICE.into(),
        text: text.clone(),
        fields: Some(fields),
    };
    (text, datum)
}

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
    fn unknown_choices_show_their_tag_and_octets() {
        // A primitive [11] of two octets, one empty, and a constructed [30]
        // around [0] 30.
        let (text, datum) = unknown_choice(11, false, &[1, 2], Floats::Exact);
        assert_eq!(text, "[11] [raw 01 02]");
        assert_eq!(
            serde_json::to_value(&datum).unwrap(),
            serde_json::json!({
                "element": "unknown-choice",
                "text": "[11] [raw 01 02]",
                "fields": {"tag": 11, "constructed": false, "data": {"raw": "0102"}},
            })
        );
        assert_eq!(unknown_choice(12, false, &[], Floats::Exact).0, "[12]");
        let (text, datum) = unknown_choice(30, true, &[0x09, 30], Floats::Exact);
        assert_eq!(text, "[30] ([0] [raw 1e])");
        let Datum::Constructed { fields, .. } = &datum else {
            panic!("expected an element, got {datum:?}");
        };
        let fields = fields.as_ref().unwrap();
        assert_eq!(fields.get("constructed"), Some(&Datum::Boolean(true)));
        assert_eq!(fields.get("data"), Some(&Datum::Raw { raw: "091e".into() }));
        // Its body may quote a device's string.
        assert!(datum.may_hold_device_text());
    }

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

    /// Pinned before the walk moved to the shared tag-tree decoder (#2618).
    #[test]
    fn fields_keep_their_text() {
        // Every application datatype inside [3] ( ... ).
        let mut data = vec![0x3E, 0x00, 0x11, 0x21, 72, 0x31, 0xB8];
        data.extend_from_slice(&[0x44, 0x42, 0x91, 0, 0]);
        data.extend_from_slice(&[0x55, 0x08, 0x40, 0x52, 0x20, 0, 0, 0, 0, 0]);
        data.extend_from_slice(&[0x62, 0x12, 0x34, 0x73, 0x00, b'h', b'i']);
        data.extend_from_slice(&[0x82, 0x05, 0xE0, 0x91, 0x03]);
        data.extend_from_slice(&[0xA4, 126, 10, 3, 6, 0xB4, 12, 30, 0, 0]);
        data.extend_from_slice(&[0xC4, 0x00, 0x00, 0x00, 0x07, 0x3F]);
        assert_eq!(
            fields_text(&data, Floats::Exact),
            "[3] (null, true, 72, -72, 72.5, 72.5, 0x1234, \"hi\", bits(111), 3, \
             2026-10-03 Sat, 12:30, analog-input 7)"
        );
        // Extended tag numbers (Clause 20.2.1.2): [20] primitive, [200]
        // constructed, an empty context primitive.
        assert_eq!(
            fields_text(&[0xF9, 20, 0x05, 0xFE, 200, 0x28, 0xFF, 200], Floats::Exact),
            "[20] [raw 05], [200] ([2])"
        );
        // A closing tag with no opening tag, and a closing tag of another
        // number: the rest is raw.
        assert_eq!(
            fields_text(&[0x21, 0x01, 0x1F, 0x21, 0x02], Floats::Exact),
            "1, [raw 1f 21 02]"
        );
        assert_eq!(
            fields_text(&[0x09, 0x01, 0x1E, 0x21, 0x02, 0x2F], Floats::Exact),
            "[0] [raw 01], [raw 1e 21 02 2f]"
        );
        // An opening tag that never closes.
        assert_eq!(
            fields_text(&[0x09, 0x01, 0x1E, 0x21, 0x02], Floats::Exact),
            "[0] [raw 01], [raw 1e 21 02]"
        );
        // Nothing at all.
        assert_eq!(fields_text(&[], Floats::Exact), "");
    }
}
