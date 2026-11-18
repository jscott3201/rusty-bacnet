//! The walk against the hand-written walker it replaced (#2618), kept here
//! as a test oracle only: on tags that frame, the two give the same fields;
//! on any octets, the fields read before a fault are the fields of the
//! octets before it.

use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_encoding::tags::tree::TagReader;
use bacnet_encoding::tags::{
    MAX_CONTEXT_NESTING_DEPTH, TagClass, decode_tag, encode_closing_tag, encode_opening_tag,
    encode_tag,
};
use bacnet_ops::octets::hex_upper;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;
use proptest::collection::vec;
use proptest::prelude::*;

use super::super::super::{APPLICATION_TAGS, Field, decode_apdu_tree, value_text};
use super::{octets, unnamed};

/// The walker before #2618, as it was, for comparison only. It took a
/// closing tag of any number as the end of its level, ran an unclosed
/// element and a primitive's contents to the end of the octets, and cut
/// nesting one level short of the reader's bound.
fn old_tags(data: &[u8], base: usize, start: usize, end: usize, depth: usize) -> Vec<Field> {
    let mut fields = old_walk(data, base, start, end, depth).0;
    let covered = fields
        .last()
        .map_or(start, |field| field.range.end.saturating_sub(base));
    if covered < end {
        fields.push(Field::leaf(
            format!("Trailing octets: {}", end - covered),
            base + covered..base + end,
        ));
    }
    fields
}

fn old_walk(
    data: &[u8],
    base: usize,
    start: usize,
    end: usize,
    depth: usize,
) -> (Vec<Field>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    while pos < end {
        let (tag, content) = match decode_tag(&data[..end], pos) {
            Ok(read) if depth < MAX_CONTEXT_NESTING_DEPTH => read,
            Ok(_) => {
                fields.push(Field::leaf("Nested too deeply", base + pos..base + end));
                return (fields, end);
            }
            Err(error) => {
                fields.push(Field::leaf(
                    format!("Malformed tag: {error}"),
                    base + pos..base + end,
                ));
                return (fields, end);
            }
        };
        if tag.is_closing {
            return (fields, content);
        }
        if tag.is_opening {
            let (inner, after) = old_walk(data, base, content, end, depth + 1);
            fields.push(Field::node(
                format!("[{}] opening", tag.number),
                base + pos..base + after,
                inner,
            ));
            pos = after;
            continue;
        }
        let decoded = (tag.class == TagClass::Application)
            .then(|| decode_application_value(&data[..end], pos));
        let after = match &decoded {
            Some(Ok((_, after))) => *after,
            _ => content.saturating_add(tag.length as usize),
        }
        .min(end)
        .max(content);
        let device_text = matches!(&decoded, Some(Ok((PropertyValue::CharacterString(_), _))));
        let label = match decoded {
            Some(Ok((value, _))) => format!(
                "{}: {}",
                APPLICATION_TAGS
                    .get(usize::from(tag.number))
                    .unwrap_or(&"Application"),
                value_text(&value)
            ),
            Some(Err(error)) => format!("Application tag {}: {error}", tag.number),
            None => {
                let octets = &data[content..after];
                let number = (octets.len() <= 4)
                    .then(|| octets.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)));
                match number {
                    Some(n) if !octets.is_empty() => {
                        format!("[{}] {} ({n})", tag.number, hex_upper(octets))
                    }
                    _ => format!("[{}] {}", tag.number, hex_upper(octets)),
                }
            }
        };
        let mut field = Field::leaf(label, base + pos..base + after);
        field.device_text = device_text;
        fields.push(field);
        pos = after;
    }
    (fields, end)
}

/// The walk's fields for `params`, as an unnamed service's tree holds them.
fn new_fields(params: &[u8]) -> Vec<Field> {
    let tree = decode_apdu_tree(&unnamed(params));
    match tree.children.last() {
        Some(node) if node.label.ends_with(" parameters") => node.children.clone(),
        _ => Vec::new(),
    }
}

/// The old walker's fields for the same parameters, at the same offsets.
fn old_fields(params: &[u8]) -> Vec<Field> {
    let apdu = unnamed(params);
    old_tags(&apdu, 0, 2, apdu.len(), 0)
}

/// One generated tagged element.
#[derive(Debug, Clone)]
enum Tagged {
    /// An application value, typed and well formed.
    Value(PropertyValue),
    /// An application tag of any number but BOOLEAN's, with any contents:
    /// framed, but often not its datatype's encoding.
    Application(u8, Vec<u8>),
    /// An application BOOLEAN.
    Boolean(bool),
    /// A context-tagged primitive.
    Context(u8, Vec<u8>),
    /// A constructed element.
    Constructed(u8, Vec<Tagged>),
}

fn encode(elements: &[Tagged], buf: &mut BytesMut) {
    for element in elements {
        match element {
            Tagged::Value(value) => encode_property_value(buf, value).unwrap(),
            Tagged::Application(number, contents) => {
                encode_tag(buf, *number, TagClass::Application, contents.len() as u32);
                buf.extend_from_slice(contents);
            }
            Tagged::Boolean(value) => {
                encode_tag(buf, 1, TagClass::Application, u32::from(*value));
            }
            Tagged::Context(number, contents) => {
                encode_tag(buf, *number, TagClass::Context, contents.len() as u32);
                buf.extend_from_slice(contents);
            }
            Tagged::Constructed(number, members) => {
                encode_opening_tag(buf, *number);
                encode(members, buf);
                encode_closing_tag(buf, *number);
            }
        }
    }
}

fn contents() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        8 => vec(any::<u8>(), 0..10),
        1 => vec(any::<u8>(), 250..300),
    ]
}

fn tag_number() -> impl Strategy<Value = u8> {
    prop_oneof![4 => 0u8..15, 1 => 15u8..=254]
}

fn value() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        Just(PropertyValue::Null),
        any::<u64>().prop_map(PropertyValue::Unsigned),
        any::<i32>().prop_map(PropertyValue::Signed),
        any::<f32>().prop_map(PropertyValue::Real),
        any::<f64>().prop_map(PropertyValue::Double),
        any::<u32>().prop_map(PropertyValue::Enumerated),
        "[ -~]{0,12}".prop_map(PropertyValue::CharacterString),
        vec(any::<u8>(), 0..6).prop_map(PropertyValue::OctetString),
    ]
}

fn tagged() -> impl Strategy<Value = Tagged> {
    let leaf = prop_oneof![
        3 => value().prop_map(Tagged::Value),
        2 => (prop_oneof![0u8..1, 2u8..=20], vec(any::<u8>(), 0..10))
            .prop_map(|(number, contents)| Tagged::Application(number, contents)),
        1 => any::<bool>().prop_map(Tagged::Boolean),
        3 => (tag_number(), contents()).prop_map(|(number, contents)| Tagged::Context(number, contents)),
    ];
    leaf.prop_recursive(8, 96, 6, |inner| {
        (tag_number(), vec(inner, 0..6))
            .prop_map(|(number, members)| Tagged::Constructed(number, members))
    })
}

/// Tagged octets that frame, one level of opening tags short of the old
/// walker's cut.
fn framed() -> impl Strategy<Value = Vec<u8>> {
    vec(tagged(), 0..8).prop_map(|elements| {
        let mut buf = BytesMut::new();
        encode(&elements, &mut buf);
        buf.to_vec()
    })
}

/// `params`, broken one way: cut short, an octet changed, or an opening
/// or closing tag put in or taken out.
fn broken() -> impl Strategy<Value = Vec<u8>> {
    (framed(), any::<prop::sample::Index>(), 0u8..5, any::<u8>()).prop_map(
        |(mut params, at, how, octet)| {
            let at = at.index(params.len() + 1);
            match how {
                0 => params.truncate(at),
                1 if at < params.len() => params[at] = octet,
                2 => params.insert(at, 0x0F | (octet & 0xE0)),
                3 => params.insert(at, 0x0E | (octet & 0xE0)),
                _ => {
                    if let Some(closing) = params.iter().rposition(|o| o & 0x0F == 0x0F) {
                        params.remove(closing);
                    }
                }
            }
            params
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// On tags that frame, the walk's fields are the old walker's, label,
    /// range and device-text mark alike.
    #[test]
    fn framed_tags_walk_as_before(params in framed()) {
        prop_assert!(TagReader::new(&params).all(|item| item.is_ok()));
        prop_assert_eq!(new_fields(&params), old_fields(&params));
    }

    /// On any octets: the fields before a fault are the fields of the
    /// whole elements before it, as the old walker gave them too, and the
    /// parameters' top-level fields cover every octet in order.
    #[test]
    fn a_fault_keeps_what_was_read_before_it(
        params in prop_oneof![broken(), vec(any::<u8>(), 0..64)],
    ) {
        let fields = new_fields(&params);
        let mut at = 2;
        for field in &fields {
            prop_assert_eq!(field.range.start, at);
            at = field.range.end;
        }
        prop_assert_eq!(at, params.len() + 2);
        if let Some(Err(error)) = TagReader::new(&params).find(Result::is_err) {
            let whole = &params[..error.valid_up_to];
            let before = new_fields(whole);
            prop_assert!(fields.len() > before.len());
            prop_assert_eq!(&fields[..before.len()], &before[..]);
            if !odd_boolean(whole) {
                prop_assert_eq!(&before, &old_fields(whole));
            }
        } else if !odd_boolean(&params) {
            prop_assert_eq!(fields, old_fields(&params));
        }
    }
}

/// Whether an application BOOLEAN in `params` has a value other than 0 or
/// 1 (Clause 20.2.3): the reader reads its header alone, as the value
/// decoder does for a valid one, where the old walker skipped as many
/// octets as the value.
fn odd_boolean(params: &[u8]) -> bool {
    TagReader::new(params).flatten().any(|item| {
        item.tag.class == TagClass::Application && item.tag.number == 1 && item.tag.length > 1
    })
}

/// Fixed cases the two walkers agree on: the deepest nesting the old one
/// read, extended tag numbers and lengths, and values that frame but
/// don't decode as their datatype.
#[test]
fn edge_cases_walk_as_before() {
    let deepest = {
        let depth = MAX_CONTEXT_NESTING_DEPTH - 1;
        let mut params = vec![0x1E; depth];
        params.extend_from_slice(&[0x09, 0x07]);
        params.extend(std::iter::repeat_n(0x1F, depth));
        params
    };
    // [3] of 5 octets, then of 256 (a two-octet extended length).
    let mut long = octets("3D 05 0102030405 3D FE 0100");
    long.extend([0xA5; 256]);
    let cases = [
        deepest,
        // Tag numbers from 15 up, opening and closing tags too.
        octets("FE 15 09 01 FF 15 19 02"),
        long,
        // A REAL of 3 octets, an empty string, the reserved tag 13, both
        // BOOLEANs, an UNSIGNED.
        octets("43 010203 71 00 D1 AA 11 10 21 00"),
        // "Fan", then a string in a character set the decoder doesn't read.
        octets("74 00 46616E 7508 03 666F6F62617221"),
    ];
    for params in &cases {
        assert!(
            TagReader::new(params).all(|item| item.is_ok()),
            "{params:02X?}"
        );
        assert_eq!(new_fields(params), old_fields(params), "{params:02X?}");
    }
}
