//! Properties of the walk over generated tags (#2618): tags that frame
//! read whole; on any octets, the fields read before a fault are the fields
//! of the elements before it, and every element's members cover its
//! contents in order, the octets the walk didn't read in a fault's leaf.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::tags::tree::TagReader;
use bacnet_encoding::tags::{TagClass, encode_closing_tag, encode_opening_tag, encode_tag};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;
use proptest::collection::vec;
use proptest::prelude::*;

use super::super::super::{Field, decode_apdu_tree};
use super::unnamed;

/// The walk's fields for `params`, as an unnamed service's tree holds them.
fn walk_fields(params: &[u8]) -> Vec<Field> {
    let tree = decode_apdu_tree(&unnamed(params));
    match tree.children.last() {
        Some(node) if node.label.ends_with(" parameters") => node.children.clone(),
        _ => Vec::new(),
    }
}

/// Whether `field` or a part of it marks where the walk stopped.
fn stops(field: &Field) -> bool {
    field.label.starts_with("Tag walk stopped") || field.children.iter().any(stops)
}

/// `fields` cover `start..end` in order, each where the one before it ends,
/// and so does each constructed element's members cover its contents: from
/// just past its opening tag to its closing tag, or to its end where the
/// walk stopped inside it.
fn covers(fields: &[Field], start: usize, end: usize) -> Result<(), String> {
    let mut at = start;
    for field in fields {
        if field.range.start != at {
            return Err(format!("{} @{:?} isn't at {at}", field.label, field.range));
        }
        at = field.range.end;
        let Some(number) = field
            .label
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix("] opening"))
        else {
            continue;
        };
        // A tag number from 15 up takes a second header octet.
        let tag = if number.parse::<u8>().map_err(|e| e.to_string())? < 15 {
            1
        } else {
            2
        };
        let members_end = if field.children.last().is_some_and(stops) {
            field.range.end
        } else {
            field.range.end.saturating_sub(tag)
        };
        covers(&field.children, field.range.start + tag, members_end)?;
    }
    if at == end {
        Ok(())
    } else {
        Err(format!("the fields end at {at}, not {end}"))
    }
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

/// Tagged octets that frame.
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

    /// Tags that frame read whole: no fault anywhere, and every element's
    /// members cover its contents.
    #[test]
    fn framed_tags_read_whole(params in framed()) {
        prop_assert!(TagReader::new(&params).all(|item| item.is_ok()));
        let fields = walk_fields(&params);
        prop_assert!(!fields.iter().any(stops));
        prop_assert_eq!(covers(&fields, 2, params.len() + 2), Ok(()));
    }

    /// On any octets: the fields before a fault are the fields of the
    /// whole elements before it, and the fields cover every octet in
    /// order, inside every element too.
    #[test]
    fn a_fault_keeps_what_was_read_before_it(
        params in prop_oneof![broken(), vec(any::<u8>(), 0..64)],
    ) {
        let fields = walk_fields(&params);
        prop_assert_eq!(covers(&fields, 2, params.len() + 2), Ok(()));
        if let Some(Err(error)) = TagReader::new(&params).find(Result::is_err) {
            let before = walk_fields(&params[..error.valid_up_to]);
            prop_assert!(fields.len() > before.len());
            prop_assert_eq!(&fields[..before.len()], &before[..]);
            prop_assert!(fields[before.len()..].iter().any(stops));
        }
    }
}
