//! Decode helpers for tagged fields, shared by the constructed codecs, the
//! formal Error bodies in `apdu/` and the service parameters bacnet-services
//! decodes.
//!
//! Each helper reads one field at an offset and returns what it read with the
//! offset just past it. Each takes the `what` label its codec passes (the
//! production, sometimes narrowed to one alternative or member), so every
//! codec words a refusal the same way:
//!
//! - a tag of the wrong number, class or form: `{what}: expected context tag
//!   [n]`, `{what}: expected opening tag [n]`, `{what}: expected closing tag
//!   [n]` or `{what}: expected application-tagged Unsigned`, reported at the
//!   offending tag;
//! - a fixed-size primitive of the wrong length: `{what}: [n] REAL has 5
//!   contents octets, expected 4`;
//! - an Unsigned or ENUMERATED too large for its field: `{what}: [n] value
//!   70000 exceeds u16`, or `{what}: Unsigned exceeds u16` for an
//!   application-tagged one;
//! - bytes left after a value that must fill its input: `{what}: 2 trailing
//!   byte(s)`.
//!
//! One error-kind rule holds across the crate's decoders (#1333):
//!
//! - Contents that run past the end of the data fail with
//!   [`Error::BufferTooShort`], wherever the member stands: at the top level,
//!   or inside a constructed frame, where [`tags::extract_context_value`]
//!   finds it while [`decode_ctx_constructed`] or `decode_framed_value`
//!   extracts the frame.
//! - A fixed-size member has its length checked against its header before
//!   its contents are read, so a wrong length is [`Error::Decoding`] even
//!   when the data also stops early. That covers the context-tagged ones (an
//!   object identifier, REAL, BOOLEAN, or any type read with
//!   [`decode_ctx_fixed`]), the application-tagged ones read with
//!   [`decode_app_fixed`] or [`decode_app_object_id`], and the REAL, Double,
//!   Date, Time and object identifier that
//!   [`primitives::decode_application_value`] reads.
//! - Every other refusal is [`Error::Decoding`], including a tag header cut
//!   short, which [`tags::decode_tag`] refuses, and a frame whose closing tag
//!   never comes.
//!
//! An [`Error::Decoding`] carries the [`DecodingKind`] of the fault, which a
//! responder turns into a Reject reason (#1446):
//!
//! - [`DecodingKind::Missing`]: the data ends where a required member, a
//!   tag header's remaining octets or a frame's closing tag is due, or the
//!   tag found there shows the member was left out (see [`misplaced_kind`]);
//! - [`DecodingKind::InvalidTag`]: any other tag that doesn't fit where it
//!   stands, a malformed tag header, or nesting deeper than the walk takes;
//! - [`DecodingKind::Trailing`]: octets after the last member of a value
//!   that must fill its input ([`expect_end`]) or close its frame
//!   ([`expect_closing`]), unless they open with a closing tag or a header
//!   that doesn't decode, which is [`DecodingKind::InvalidTag`];
//! - [`DecodingKind::OutOfRange`]: an Unsigned or ENUMERATED too large for
//!   its field;
//! - [`DecodingKind::Overflow`]: more items than a framed value takes, or a
//!   tag length past its sanity bound;
//! - [`DecodingKind::InvalidEncoding`]: contents whose encoding isn't valid
//!   for their datatype, such as a fixed-size member of the wrong length,
//!   the kind [`Error::decoding`] makes.
//!
//! The helpers another crate needs are public: the peeks for an optional
//! member, a frame's opening and closing tags and its body, the context
//! readers for contents, fixed-size contents, Unsigned or ENUMERATED values,
//! REAL, BOOLEAN, BIT STRING, OCTET STRING, CharacterString and object
//! identifiers, the optional-member wrapper, the application readers, the
//! trailing-data check, and the kinds a misplaced tag is ([`misplaced_kind`],
//! [`misplaced_tag`], [`unclosed_kind`]). The rest stay private to this
//! crate.
//!
//! ```
//! use bacnet_encoding::constructed::tagged::{decode_ctx_unsigned, expect_end, next_is_context};
//! use bacnet_types::error::Error;
//!
//! // A required `[0]` Unsigned 7 and no optional `[1]` after it.
//! let data = [0x09, 0x07];
//! let (value, end) = decode_ctx_unsigned::<u32>(&data, 0, 0, "Example")?;
//! assert_eq!((value, end), (7, 2));
//! assert!(!next_is_context(&data, end, 1)?);
//! expect_end(&data, end, end, "Example")?;
//!
//! // The same member announcing two contents octets but holding one.
//! let cut = decode_ctx_unsigned::<u32>(&[0x0A, 0x07], 0, 0, "Example");
//! assert!(matches!(cut, Err(Error::BufferTooShort { need: 3, have: 2 })));
//! # Ok::<(), Error>(())
//! ```

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};
use core::fmt;

use bacnet_types::error::{DecodingKind, Error};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, Tag, TagClass};

// ---------------------------------------------------------------------------
// Peeking at the next tag
// ---------------------------------------------------------------------------

/// Whether the tag at `offset` passes `test`; `false` at the end of the data.
fn next_tag_is(data: &[u8], offset: usize, test: impl FnOnce(&Tag) -> bool) -> Result<bool, Error> {
    if offset >= data.len() {
        return Ok(false);
    }
    Ok(test(&tags::decode_tag(data, offset)?.0))
}

/// Whether a primitive context tag `tag` starts at `offset`; `false` at the
/// end of the data, so an optional member may be the last one. A malformed
/// tag there is an error.
pub fn next_is_context(data: &[u8], offset: usize, tag: u8) -> Result<bool, Error> {
    next_tag_is(data, offset, |t| t.is_context(tag))
}

/// Whether an opening context tag `tag` starts at `offset`; `false` at the
/// end of the data. A malformed tag there is an error.
pub fn next_is_opening(data: &[u8], offset: usize, tag: u8) -> Result<bool, Error> {
    next_tag_is(data, offset, |t| t.is_opening_tag(tag))
}

/// Whether an application tag `number` (one of [`tags::app_tag`]) starts at
/// `offset`; `false` at the end of the data, so an optional
/// application-tagged member may be the last one. A malformed tag there is an
/// error.
pub fn next_is_application(data: &[u8], offset: usize, number: u8) -> Result<bool, Error> {
    next_tag_is(data, offset, |t| {
        t.class == TagClass::Application && t.number == number
    })
}

/// Whether a closing context tag `tag` starts at `offset`, for walking the
/// items inside a frame. Unlike [`next_is_context`], running out of data is
/// an error: the frame never closed.
pub(crate) fn next_is_closing(data: &[u8], offset: usize, tag: u8) -> Result<bool, Error> {
    Ok(tags::decode_tag(data, offset)?.0.is_closing_tag(tag))
}

// ---------------------------------------------------------------------------
// Frames and whole inputs
// ---------------------------------------------------------------------------

/// The kind of fault the tag `found`, at `at` in `data`, is where a required
/// member is due.
///
/// `expected` is the member's context tag number, or `None` for an
/// application-tagged member or a CHOICE none of whose alternatives came.
/// The member was left out, [`DecodingKind::Missing`], when the tag closes
/// the frame open around it, so the frame ended first, or is a context tag
/// numbered above `expected`, since members follow in tag order. Any other
/// tag doesn't fit there, [`DecodingKind::InvalidTag`]: a closing tag that
/// closes no open frame (one at the top level, or of another number) among
/// them.
pub fn misplaced_kind(data: &[u8], at: usize, found: &Tag, expected: Option<u8>) -> DecodingKind {
    let later = expected.is_some_and(|number| {
        found.class == TagClass::Context && !found.is_closing && found.number > number
    });
    if later || (found.is_closing && closes_open_frame(data, at, found.number)) {
        DecodingKind::Missing
    } else {
        DecodingKind::InvalidTag
    }
}

/// Whether a closing tag `number` at `at` closes the innermost frame open
/// there, found by walking the tags of `data` from its start. A walk that
/// doesn't land on `at` (contents that aren't tags) says no.
fn closes_open_frame(data: &[u8], at: usize, number: u8) -> bool {
    let mut open = Vec::new();
    let mut pos = 0;
    while pos < at {
        let Ok((tag, next)) = tags::decode_tag(data, pos) else {
            return false;
        };
        pos = if tag.is_opening {
            open.push(tag.number);
            next
        } else if tag.is_closing {
            if open.pop() != Some(tag.number) {
                return false;
            }
            next
        } else if tag.class == TagClass::Application && tag.number == tags::app_tag::BOOLEAN {
            next
        } else {
            next.saturating_add(tag.length as usize)
        };
    }
    pos == at && open.last() == Some(&number)
}

/// The error for the tag `found`, at `offset` in `data`, where a required
/// member is due, of the kind [`misplaced_kind`] gives.
pub fn misplaced_tag(
    data: &[u8],
    found: &Tag,
    expected: Option<u8>,
    offset: usize,
    message: impl Into<String>,
) -> Error {
    Error::decoding_kind(
        misplaced_kind(data, offset, found, expected),
        offset,
        message,
    )
}

/// The kind of fault the tag `found` is where a frame's closing tag is due:
/// another closing tag is [`DecodingKind::InvalidTag`]; any other tag is a
/// member the frame holds beyond its last one, [`DecodingKind::Trailing`].
pub fn unclosed_kind(found: &Tag) -> DecodingKind {
    if found.is_closing {
        DecodingKind::InvalidTag
    } else {
        DecodingKind::Trailing
    }
}

/// Require an opening context tag `tag` at `offset`; return the offset of its
/// content.
pub fn expect_opening(data: &[u8], offset: usize, tag: u8, what: &str) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_opening_tag(tag) {
        return Err(misplaced_tag(
            data,
            &t,
            Some(tag),
            offset,
            format!("{what}: expected opening tag [{tag}]"),
        ));
    }
    Ok(pos)
}

/// Require a closing context tag `tag` at `offset`; return the offset past it.
/// Any other tag there is of the kind [`unclosed_kind`] gives.
pub fn expect_closing(data: &[u8], offset: usize, tag: u8, what: &str) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_closing_tag(tag) {
        return Err(Error::decoding_kind(
            unclosed_kind(&t),
            offset,
            format!("{what}: expected closing tag [{tag}]"),
        ));
    }
    Ok(pos)
}

/// Require constructed context tag `tag` at `offset`; return its body (the
/// octets between its opening and closing tags, with nested frames balanced)
/// and the offset past its closing tag.
///
/// A member inside the frame whose contents run past the end of the data is
/// found while the frame is extracted, and is [`Error::BufferTooShort`], as
/// it would be outside the frame. Once the body is extracted, every member
/// in it fits.
pub fn decode_ctx_constructed<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    let content = expect_opening(data, offset, tag, what)?;
    tags::extract_context_value(data, content, tag)
}

/// Decode an `ABSTRACT-SYNTAX.&Type` value framed in an opening/closing
/// context tag `tag` pair whose content starts at `content`; returns it and
/// the offset past the closing tag.
///
/// One application element decodes as itself and any other count, none
/// included, as a [`PropertyValue::List`] (an array written or read whole). A
/// context-tagged element decodes to [`PropertyValue::ApplicationData`], so
/// the value re-encodes to the same octets.
pub(crate) fn decode_framed_value(
    data: &[u8],
    content: usize,
    tag: u8,
    what: &str,
) -> Result<(PropertyValue, usize), Error> {
    let (inner, end) = tags::extract_context_value(data, content, tag)?;
    let body = &data[..content + inner.len()];
    let mut values = Vec::new();
    let mut offset = content;
    while offset < body.len() {
        if values.len() >= MAX_FRAMED_ITEMS {
            return Err(Error::overflow(
                offset,
                format!("{what}: value exceeds item limit"),
            ));
        }
        let (value, next) = primitives::decode_application_value(body, offset)?;
        values.push(value);
        offset = next;
    }
    let value = match <[PropertyValue; 1]>::try_from(values) {
        Ok([value]) => value,
        Err(values) => PropertyValue::List(values),
    };
    Ok((value, end))
}

/// Require a value that stopped at `end` to have used all of `data`.
///
/// Octets left over are [`DecodingKind::Trailing`], arguments past the
/// value's last member, unless they open with a closing tag, which closes
/// nothing, or with a tag header that doesn't decode: those are
/// [`DecodingKind::InvalidTag`].
///
/// `at` is the offset the error reports: `end` itself when `data` is the
/// whole input, or the frame's own offset in the larger input when `data` is
/// a frame body sliced out of it (offsets into the slice would mislead).
pub fn expect_end(data: &[u8], end: usize, at: usize, what: &str) -> Result<(), Error> {
    if end == data.len() {
        return Ok(());
    }
    let kind = match tags::decode_tag(data, end) {
        Ok((tag, _)) if !tag.is_closing => DecodingKind::Trailing,
        _ => DecodingKind::InvalidTag,
    };
    Err(Error::decoding_kind(
        kind,
        at,
        format!(
            "{what}: {} trailing byte(s)",
            data.len().saturating_sub(end)
        ),
    ))
}

// ---------------------------------------------------------------------------
// Primitive contents
// ---------------------------------------------------------------------------

/// The `length` contents octets of a primitive that start at `start`, and the
/// offset past them; contents that run past the end of `data` are
/// [`Error::BufferTooShort`].
///
/// [`tags::decode_tag`] caps a length at 1 MiB and puts `start` inside
/// `data`, so the end can't overflow; saturating keeps that true for any
/// caller.
pub(crate) fn contents(data: &[u8], start: usize, length: u32) -> Result<(&[u8], usize), Error> {
    let end = usize::try_from(length).map_or(usize::MAX, |length| start.saturating_add(length));
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((&data[start..end], end))
}

/// Require a primitive context tag `tag` at `offset`; return its header and
/// the offset of its contents. `kind`, unless empty, names the expected type
/// in the error.
fn ctx_header(
    data: &[u8],
    offset: usize,
    tag: u8,
    kind: &str,
    what: &str,
) -> Result<(Tag, usize), Error> {
    let (t, start) = tags::decode_tag(data, offset)?;
    if !t.is_context(tag) {
        let space = if kind.is_empty() { "" } else { " " };
        return Err(misplaced_tag(
            data,
            &t,
            Some(tag),
            offset,
            format!("{what}: expected context tag [{tag}]{space}{kind}"),
        ));
    }
    Ok((t, start))
}

/// Require a primitive context tag `tag` at `offset`; return its contents and
/// the offset past them.
pub fn decode_ctx_primitive<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, "", what)?;
    contents(data, start, t.length)
}

/// Require a primitive context tag `tag` at `offset` holding exactly `octets`
/// contents octets of the type `kind` names (`"Time"`, say, for the error);
/// return those contents and the offset past them. A header announcing any
/// other length is [`Error::Decoding`] even when the data also stops early.
pub fn decode_ctx_fixed<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    octets: u32,
    kind: &str,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, kind, what)?;
    if t.length != octets {
        return Err(Error::decoding(
            offset,
            format!(
                "{what}: [{tag}] {kind} has {} contents octets, expected {octets}",
                t.length
            ),
        ));
    }
    contents(data, start, octets)
}

/// Require a primitive context tag `tag` at `offset` holding a REAL.
pub fn decode_ctx_real(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(f32, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, 4, "REAL", what)?;
    Ok((primitives::decode_real(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding a BOOLEAN: one
/// contents octet, 0 or 1 (Clause 20.2.3).
pub fn decode_ctx_boolean(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(bool, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, 1, "BOOLEAN", what)?;
    match octets[0] {
        0 => Ok((false, end)),
        1 => Ok((true, end)),
        _ => Err(Error::decoding(
            end - 1,
            format!("{what}: [{tag}] BOOLEAN contents must be 0 or 1"),
        )),
    }
}

/// Require a primitive context tag `tag` at `offset` holding an object
/// identifier, which is always four octets.
pub fn decode_ctx_object_id(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, 4, "object identifier", what)?;
    Ok((ObjectIdentifier::decode(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding a BIT STRING;
/// returns its unused-bit count and data octets.
pub fn decode_ctx_bit_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, "BIT STRING", what)?;
    let (octets, end) = contents(data, start, t.length)?;
    Ok((primitives::decode_bit_string(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding an OCTET STRING;
/// returns a copy of its octets.
pub fn decode_ctx_octet_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(Vec<u8>, usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, "OCTET STRING", what)?;
    let (octets, end) = contents(data, start, t.length)?;
    Ok((octets.to_vec(), end))
}

/// Require a primitive context tag `tag` at `offset` holding a
/// CharacterString.
pub fn decode_ctx_character_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(String, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    Ok((primitives::decode_character_string(octets)?, end))
}

/// Read the optional member under primitive context tag `tag` with `decode`
/// when that tag comes next; otherwise `None` with the offset unchanged.
/// What `decode` returns may borrow from `data`, so [`decode_ctx_primitive`]
/// yields the member's contents.
pub fn decode_optional_ctx<'a, T>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &str,
    decode: impl FnOnce(&'a [u8], usize, u8, &str) -> Result<(T, usize), Error>,
) -> Result<(Option<T>, usize), Error> {
    if !next_is_context(data, offset, tag)? {
        return Ok((None, offset));
    }
    let (value, end) = decode(data, offset, tag, what)?;
    Ok((Some(value), end))
}

// ---------------------------------------------------------------------------
// Unsigned and ENUMERATED
// ---------------------------------------------------------------------------

/// An unsigned integer type a decoded Unsigned or ENUMERATED is narrowed to:
/// `u8`, `u16`, `u32` or `u64`. Sealed, so no other type implements it.
pub trait UnsignedWidth: sealed::Sealed + TryFrom<u64> {
    /// The type's name, for the error when a value doesn't fit it.
    const NAME: &'static str;
}

mod sealed {
    /// Keeps [`UnsignedWidth`](super::UnsignedWidth) to the widths this
    /// module implements it for.
    pub trait Sealed {}

    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
}

impl UnsignedWidth for u8 {
    const NAME: &'static str = "u8";
}

impl UnsignedWidth for u16 {
    const NAME: &'static str = "u16";
}

impl UnsignedWidth for u32 {
    const NAME: &'static str = "u32";
}

impl UnsignedWidth for u64 {
    const NAME: &'static str = "u64";
}

/// Narrow `value`, read from context tag `tag` at `offset`, to `T`.
fn narrow<T: UnsignedWidth>(value: u64, offset: usize, tag: u8, what: &str) -> Result<T, Error> {
    T::try_from(value).map_err(|_| {
        Error::out_of_range(
            offset,
            format!("{what}: [{tag}] value {value} exceeds {}", T::NAME),
        )
    })
}

/// Require a primitive context tag `tag` at `offset` holding an Unsigned
/// that fits `T`. An ENUMERATED's contents are encoded the same way, so this
/// reads those too. Leading zero octets are accepted, as
/// [`primitives::decode_unsigned`] accepts them.
pub fn decode_ctx_unsigned<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    let value = primitives::decode_unsigned(octets)?;
    Ok((narrow(value, offset, tag, what)?, end))
}

/// [`decode_ctx_unsigned`] for a codec that must re-encode what it read octet
/// for octet: the contents must also be the shortest encoding (see
/// [`decode_canonical_unsigned`]).
pub fn decode_ctx_canonical_unsigned<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    let value = decode_canonical_unsigned(octets, offset, what)?;
    Ok((narrow(value, offset, tag, what)?, end))
}

/// The value of Unsigned or ENUMERATED contents in their shortest encoding:
/// one to eight octets, and no leading zero octet unless it is the only one.
/// Errors report `offset`, the field's tag.
pub fn decode_canonical_unsigned(octets: &[u8], offset: usize, what: &str) -> Result<u64, Error> {
    if octets.is_empty() || octets.len() > 8 {
        return Err(Error::decoding(
            offset,
            format!(
                "{what}: has {} contents octets, expected one to eight",
                octets.len()
            ),
        ));
    }
    if octets.len() > 1 && octets[0] == 0 {
        return Err(Error::decoding(
            offset,
            format!("{what}: must use the shortest Unsigned/Enumerated encoding"),
        ));
    }
    primitives::decode_unsigned(octets)
}

// ---------------------------------------------------------------------------
// Application-tagged items
// ---------------------------------------------------------------------------

/// The name an error gives the type of application tag `number`.
fn app_kind(number: u8) -> &'static str {
    match number {
        tags::app_tag::NULL => "NULL",
        tags::app_tag::BOOLEAN => "BOOLEAN",
        tags::app_tag::UNSIGNED => "Unsigned",
        tags::app_tag::SIGNED => "INTEGER",
        tags::app_tag::REAL => "REAL",
        tags::app_tag::DOUBLE => "Double",
        tags::app_tag::OCTET_STRING => "OCTET STRING",
        tags::app_tag::CHARACTER_STRING => "CharacterString",
        tags::app_tag::BIT_STRING => "BIT STRING",
        tags::app_tag::ENUMERATED => "ENUMERATED",
        tags::app_tag::DATE => "Date",
        tags::app_tag::TIME => "Time",
        tags::app_tag::OBJECT_IDENTIFIER => "BACnetObjectIdentifier",
        _ => "value of a reserved type",
    }
}

/// Require an application tag `number` (one of [`tags::app_tag`]) at
/// `offset`; return its contents and the offset past them.
///
/// A BOOLEAN is refused with [`Error::Decoding`] whatever the data holds: its
/// value is the tag's length field, so it has no contents to return (see
/// [`Tag::is_boolean_true`]).
///
/// `what` may be any [`Display`](fmt::Display), not only a `&str`, for a
/// label that carries a value, such as a list element's index: pass
/// [`format_args!`], and the label is formatted only if an error names it,
/// so a read that succeeds builds none (#1645).
///
/// ```
/// use bacnet_encoding::constructed::tagged::decode_app_primitive;
/// use bacnet_encoding::tags::app_tag;
///
/// // Two records, the second an Unsigned rather than an OCTET STRING.
/// let data = [0x61, 0xAB, 0x21, 0x01];
/// let mut offset = 0;
/// let mut error = None;
/// for i in 0..2 {
///     match decode_app_primitive(&data, offset, app_tag::OCTET_STRING, format_args!("record {i}")) {
///         Ok((_, end)) => offset = end,
///         Err(e) => error = Some(e.to_string()),
///     }
/// }
/// assert_eq!(
///     error.as_deref(),
///     Some("decoding error at offset 2: record 1: expected application-tagged OCTET STRING"),
/// );
/// ```
pub fn decode_app_primitive(
    data: &[u8],
    offset: usize,
    number: u8,
    what: impl fmt::Display,
) -> Result<(&[u8], usize), Error> {
    let (t, start) = app_header(data, offset, number, what)?;
    contents(data, start, t.length)
}

/// Require an application tag `number` at `offset`; return its header and
/// the offset of its contents. A BOOLEAN has none (see
/// [`decode_app_primitive`]).
fn app_header(
    data: &[u8],
    offset: usize,
    number: u8,
    what: impl fmt::Display,
) -> Result<(Tag, usize), Error> {
    if number == tags::app_tag::BOOLEAN {
        return Err(Error::decoding(
            offset,
            format!("{what}: an application-tagged BOOLEAN has no contents to read"),
        ));
    }
    let (t, start) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.number != number {
        return Err(misplaced_tag(
            data,
            &t,
            None,
            offset,
            format!("{what}: expected application-tagged {}", app_kind(number)),
        ));
    }
    Ok((t, start))
}

/// Require an application tag `number` at `offset` holding exactly `octets`
/// contents octets; return them and the offset past them. As with
/// [`decode_ctx_fixed`], a header announcing any other length is
/// [`Error::Decoding`] even when the data also stops early.
pub fn decode_app_fixed<'a>(
    data: &'a [u8],
    offset: usize,
    number: u8,
    octets: u32,
    what: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = app_header(data, offset, number, what)?;
    if t.length != octets {
        return Err(Error::decoding(
            offset,
            format!(
                "{what}: {} has {} contents octets, expected {octets}",
                app_kind(number),
                t.length
            ),
        ));
    }
    contents(data, start, octets)
}

/// Require an application-tagged object identifier, four contents octets,
/// at `offset`.
pub fn decode_app_object_id(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (octets, end) = decode_app_fixed(data, offset, tags::app_tag::OBJECT_IDENTIFIER, 4, what)?;
    Ok((ObjectIdentifier::decode(octets)?, end))
}

/// Decode one application-tagged Unsigned that fits `T`. Leading zero octets
/// are accepted.
pub fn decode_app_unsigned<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::UNSIGNED, what)?;
    let value = primitives::decode_unsigned(octets)?;
    Ok((
        narrow_app(value, end - octets.len(), "Unsigned", what)?,
        end,
    ))
}

/// Decode one application-tagged BIT STRING (`SEQUENCE OF BIT STRING` item).
pub fn decode_app_bit_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::BIT_STRING, what)?;
    Ok((primitives::decode_bit_string(octets)?, end))
}

/// Decode one application-tagged ENUMERATED (a `SEQUENCE OF` item, or an
/// error class or code) that fits `T`. Leading zero octets are accepted.
pub fn decode_app_enumerated<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::ENUMERATED, what)?;
    let value = primitives::decode_unsigned(octets)?;
    Ok((
        narrow_app(value, end - octets.len(), "ENUMERATED", what)?,
        end,
    ))
}

/// [`decode_app_enumerated`] for a codec that must re-encode what it read
/// octet for octet: the contents must also be the shortest encoding (see
/// [`decode_canonical_unsigned`]).
pub(crate) fn decode_app_canonical_enumerated<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::ENUMERATED, what)?;
    let value = decode_canonical_unsigned(octets, offset, what)?;
    Ok((
        narrow_app(value, end - octets.len(), "ENUMERATED", what)?,
        end,
    ))
}

/// Narrow an application-tagged `kind` value whose contents start at
/// `contents` to `T`.
fn narrow_app<T: UnsignedWidth>(
    value: u64,
    contents: usize,
    kind: &str,
    what: &str,
) -> Result<T, Error> {
    T::try_from(value)
        .map_err(|_| Error::out_of_range(contents, format!("{what}: {kind} exceeds {}", T::NAME)))
}

/// Decode one application-tagged CharacterString.
pub fn decode_app_character_string(
    data: &[u8],
    offset: usize,
    what: &str,
) -> Result<(String, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::CHARACTER_STRING, what)?;
    Ok((primitives::decode_character_string(octets)?, end))
}

#[cfg(test)]
#[path = "tagged_frame_tests.rs"]
mod frame_tests;
