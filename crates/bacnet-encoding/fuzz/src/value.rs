//! `value_decode`: tags, application values and context-tagged elements.
//!
//! - `decode_tag` walks the input tag by tag, always moving forward and
//!   never past the end.
//! - `decode_application_value` reads values one after another; each round
//!   trips through `encode_property_value` as [`round_trip`] states.
//! - The owned decoder, the borrowed view (`decode_application_value_ref`)
//!   and `validate_application_value` agree on an application-tagged value:
//!   where the owned decoder accepts, the view accepts the same value ending
//!   at the same offset, and so does the validator unless the value is a
//!   Signed (which the validator also holds to its shortest form). Where the
//!   view accepts and the owned decoder refuses, the value is a
//!   CharacterString in a character set the owned decoder doesn't convert.
//! - An opening tag's element, walked by `extract_context_value` and
//!   `extract_raw_context`, ends within the input.
//! - A `BACnetTimeStamp` round trips through its CHOICE codec.
//!
//! [`round_trip`]: crate::round_trip

use bacnet_encoding::primitives::{
    ApplicationValueRef, decode_application_value, decode_application_value_ref,
    decode_timestamp_choice, encode_property_value, encode_timestamp_choice,
    validate_application_value,
};
use bacnet_encoding::tags::{
    TagClass, app_tag, decode_tag, extract_context_value, extract_raw_context,
};
use bacnet_types::primitives::PropertyValue;

use crate::{at, round_trip};

/// Runs every check on `input`.
pub fn check(input: &[u8]) {
    walk_tags(input);
    application_values(input);
    context_element(input);
    round_trip(
        "BACnetTimeStamp",
        input,
        at(decode_timestamp_choice),
        |value, buf| encode_timestamp_choice(buf, value),
    );
}

/// Every tag header from the start, each element's contents skipped.
fn walk_tags(input: &[u8]) {
    let mut offset = 0;
    while offset < input.len() {
        let Ok((tag, next)) = decode_tag(input, offset) else {
            return;
        };
        assert!(
            offset < next && next <= input.len(),
            "a tag at {offset} ends at {next}, in {} octets",
            input.len()
        );
        let boolean = tag.class == TagClass::Application && tag.number == app_tag::BOOLEAN;
        offset = if tag.is_opening || tag.is_closing || boolean {
            next
        } else {
            match next.checked_add(tag.length as usize) {
                Some(end) => end,
                None => return,
            }
        };
    }
}

/// Application values one after another, each checked against the view
/// and the validator.
fn application_values(input: &[u8]) {
    let mut offset = 0;
    while offset < input.len() {
        let application =
            decode_tag(input, offset).is_ok_and(|(tag, _)| tag.class == TagClass::Application);
        let owned = decode_application_value(input, offset);
        if application {
            agree(input, offset, owned.as_ref().ok());
        }
        let Ok((_, end)) = owned else {
            return;
        };
        assert!(
            offset < end && end <= input.len(),
            "a value at {offset} ends at {end}, in {} octets",
            input.len()
        );
        round_trip(
            "application value",
            &input[offset..],
            at(decode_application_value),
            |value, buf| encode_property_value(buf, value),
        );
        offset = end;
    }
}

/// The view and the validator against the owned decoder's result for the
/// application-tagged value at `offset`.
fn agree(input: &[u8], offset: usize, owned: Option<&(PropertyValue, usize)>) {
    let view = decode_application_value_ref(input, offset);
    let validated = validate_application_value(input, offset);
    match (owned, &view) {
        (Some((value, end)), Ok((viewed, view_end))) => {
            assert_eq!(end, view_end, "the view ends elsewhere");
            assert!(same(value, viewed), "owned {value:?}, viewed {viewed:?}");
            if !matches!(value, PropertyValue::Signed(_)) {
                assert_eq!(
                    validated.as_ref().ok(),
                    Some(end),
                    "the validator disagrees"
                );
            }
        }
        (Some(value), Err(error)) => panic!("the view refused {value:?}: {error:?}"),
        (None, Ok((viewed, _))) => assert!(
            matches!(viewed, ApplicationValueRef::CharacterString(_)),
            "the owned decoder refused {viewed:?}"
        ),
        (None, Err(_)) => {}
    }
}

/// Whether the owned value and the viewed one are the same value; floats by
/// their bits, so a NaN matches itself.
fn same(owned: &PropertyValue, viewed: &ApplicationValueRef<'_>) -> bool {
    use ApplicationValueRef as V;
    use PropertyValue as P;
    match (owned, viewed) {
        (P::Null, V::Null) => true,
        (P::Boolean(a), V::Boolean(b)) => a == b,
        (P::Unsigned(a), V::Unsigned(b)) => a == b,
        (P::Signed(a), V::Signed(b)) => a == b,
        (P::Real(a), V::Real(b)) => a.to_bits() == b.to_bits(),
        (P::Double(a), V::Double(b)) => a.to_bits() == b.to_bits(),
        (P::OctetString(a), V::OctetString(b)) => a == b,
        (P::CharacterString(a), V::CharacterString(b)) => {
            // The owned decoder converts every character set it takes to
            // UTF-8; the view keeps the octets, and reads them as text only
            // when they are UTF-8 already.
            b.as_str().is_none_or(|text| text == a)
        }
        (
            P::BitString { unused_bits, data },
            V::BitString {
                unused_bits: viewed_bits,
                data: viewed_data,
            },
        ) => unused_bits == viewed_bits && data == viewed_data,
        (P::Enumerated(a), V::Enumerated(b)) => a == b,
        (P::Date(a), V::Date(b)) => a == b,
        (P::Time(a), V::Time(b)) => a == b,
        (P::ObjectIdentifier(a), V::ObjectIdentifier(b)) => a == b,
        _ => false,
    }
}

/// The element an opening tag at the start begins, walked both ways.
fn context_element(input: &[u8]) {
    let Ok((tag, next)) = decode_tag(input, 0) else {
        return;
    };
    if !tag.is_opening {
        return;
    }
    if let Ok((inner, end)) = extract_context_value(input, next, tag.number) {
        assert!(end <= input.len() && inner.len() <= end - next);
    }
    if let Ok((inner, end)) = extract_raw_context(input, next, tag.number) {
        assert!(end <= input.len() && inner.len() <= end - next);
    }
}
