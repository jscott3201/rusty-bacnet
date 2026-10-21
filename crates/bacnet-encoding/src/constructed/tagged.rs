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
//!   `primitives::decode_application_value` reads.
//! - Every other refusal is [`Error::Decoding`], including a tag header cut
//!   short, which [`tags::decode_tag`] refuses, and a frame whose closing tag
//!   never comes.
//!
//! Every label is a `&'static str`, and each refusal is a [`DecodeError`]
//! that builds its text only when displayed (#1646). The readers that return
//! only borrowed octets and numbers need no allocator; those returning a
//! copy, a `String` or a `PropertyValue` need `alloc`.
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

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{string::String, vec::Vec};

use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::ObjectIdentifier;
#[cfg(feature = "alloc")]
use bacnet_types::primitives::PropertyValue;

#[cfg(feature = "alloc")]
use super::MAX_FRAMED_ITEMS;
use crate::primitives;
use crate::tags::{self, Tag, TagClass, MAX_CONTEXT_NESTING_DEPTH};

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
#[cfg(feature = "alloc")]
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
/// doesn't land on `at` (contents that aren't tags) says no, and so does one
/// that finds more than [`MAX_CONTEXT_NESTING_DEPTH`] frames open at once.
/// The depth counts from the start of `data`, the frames around the member's
/// production included, so well-formed input nested less than that deep in
/// its own production can still reach past it. Saying no is the safe
/// answer: the tag is then [`DecodingKind::InvalidTag`] rather than
/// [`DecodingKind::Missing`], and the input is refused either way.
fn closes_open_frame(data: &[u8], at: usize, number: u8) -> bool {
    let mut open = [0u8; MAX_CONTEXT_NESTING_DEPTH];
    let mut depth = 0;
    let mut pos = 0;
    while pos < at {
        let Ok((tag, next)) = tags::decode_tag(data, pos) else {
            return false;
        };
        pos = if tag.is_opening {
            let Some(slot) = open.get_mut(depth) else {
                return false;
            };
            *slot = tag.number;
            depth += 1;
            next
        } else if tag.is_closing {
            if depth == 0 || open[depth - 1] != tag.number {
                return false;
            }
            depth -= 1;
            next
        } else if tag.class == TagClass::Application && tag.number == tags::app_tag::BOOLEAN {
            next
        } else {
            next.saturating_add(tag.length as usize)
        };
    }
    pos == at && depth > 0 && open[depth - 1] == number
}

/// The error for the tag `found`, at `offset` in `data`, where a required
/// member is due, of the kind [`misplaced_kind`] gives, described by
/// `label`. [`misplaced`] gives the same fault as a [`DecodeError`], to
/// name a context or fill the label's placeholders.
pub fn misplaced_tag(
    data: &[u8],
    found: &Tag,
    expected: Option<u8>,
    offset: usize,
    label: &'static str,
) -> Error {
    misplaced(data, found, expected, offset, label).into()
}

/// [`misplaced_tag`]'s fault as a [`DecodeError`], for a caller that adds a
/// context or values before returning it.
pub fn misplaced(
    data: &[u8],
    found: &Tag,
    expected: Option<u8>,
    offset: usize,
    label: &'static str,
) -> DecodeError {
    DecodeError::new(misplaced_kind(data, offset, found, expected), offset, label)
}

/// The refusal of the tag `found` at `offset` where `what`'s member is due:
/// context tag `expected`, or for `None` an application-tagged one. `label`
/// names the member through its one placeholder, which `named` fills: the
/// context tag's number, or the application tag's.
#[cold]
fn misplaced_member(
    data: &[u8],
    found: &Tag,
    expected: Option<u8>,
    offset: usize,
    what: &'static str,
    (label, named): (&'static str, u8),
) -> Error {
    DecodeError::new(misplaced_kind(data, offset, found, expected), offset, label)
        .within(what)
        .with_value(named.into())
        .into()
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
pub fn expect_opening(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_opening_tag(tag) {
        return Err(misplaced_member(
            data,
            &t,
            Some(tag),
            offset,
            what,
            ("expected opening tag [{}]", tag),
        ));
    }
    Ok(pos)
}

/// Require a closing context tag `tag` at `offset`; return the offset past it.
/// Any other tag there is of the kind [`unclosed_kind`] gives.
pub fn expect_closing(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<usize, Error> {
    let (t, pos) = tags::decode_tag(data, offset)?;
    if !t.is_closing_tag(tag) {
        return Err(
            DecodeError::new(unclosed_kind(&t), offset, "expected closing tag [{}]")
                .within(what)
                .with_value(tag.into())
                .into(),
        );
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
    what: &'static str,
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
/// the value re-encodes to the same octets. Only with `alloc`.
#[cfg(feature = "alloc")]
pub(crate) fn decode_framed_value(
    data: &[u8],
    content: usize,
    tag: u8,
    what: &'static str,
) -> Result<(PropertyValue, usize), Error> {
    let (inner, end) = tags::extract_context_value(data, content, tag)?;
    let body = &data[..content + inner.len()];
    let mut values = Vec::new();
    let mut offset = content;
    while offset < body.len() {
        if values.len() >= MAX_FRAMED_ITEMS {
            return Err(DecodeError::new(
                DecodingKind::Overflow,
                offset,
                "value exceeds item limit",
            )
            .within(what)
            .into());
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
pub fn expect_end(data: &[u8], end: usize, at: usize, what: &'static str) -> Result<(), Error> {
    if end == data.len() {
        return Ok(());
    }
    let kind = match tags::decode_tag(data, end) {
        Ok((tag, _)) if !tag.is_closing => DecodingKind::Trailing,
        _ => DecodingKind::InvalidTag,
    };
    Err(DecodeError::new(kind, at, "{} trailing byte(s)")
        .within(what)
        .with_value(data.len().saturating_sub(end) as u64)
        .into())
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
#[inline]
pub(crate) fn contents(data: &[u8], start: usize, length: u32) -> Result<(&[u8], usize), Error> {
    let end = usize::try_from(length).map_or(usize::MAX, |length| start.saturating_add(length));
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((&data[start..end], end))
}

/// Require a primitive context tag `tag` at `offset`; return its header and
/// the offset of its contents. `misplaced` is the refusal's label, its `{}`
/// the tag number.
#[inline]
fn ctx_header(
    data: &[u8],
    offset: usize,
    tag: u8,
    misplaced: &'static str,
    what: &'static str,
) -> Result<(Tag, usize), Error> {
    let (t, start) = tags::decode_tag(data, offset)?;
    if !t.is_context(tag) {
        return Err(misplaced_member(
            data,
            &t,
            Some(tag),
            offset,
            what,
            (misplaced, tag),
        ));
    }
    Ok((t, start))
}

/// Require a primitive context tag `tag` at `offset`; return its contents and
/// the offset past them.
#[inline]
pub fn decode_ctx_primitive<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, "expected context tag [{}]", what)?;
    contents(data, start, t.length)
}

/// A fixed-size primitive type [`decode_ctx_fixed`] reads: how many contents
/// octets it has, and the labels of its two refusals.
///
/// The consts cover the types the shared readers take. A codec with another
/// one builds its own, its labels in the same form:
///
/// ```
/// use bacnet_encoding::constructed::tagged::{decode_ctx_fixed, Fixed};
///
/// const MASK: Fixed = Fixed {
///     octets: 4,
///     misplaced: "expected context tag [{}] broadcast mask",
///     wrong_length: "[{}] broadcast mask has {} contents octets, expected 4",
/// };
/// let refused = decode_ctx_fixed(&[0x1B, 255, 255, 0], 0, 1, MASK, "BDT entry").unwrap_err();
/// assert_eq!(
///     refused.to_string(),
///     "decoding error at offset 0: BDT entry: [1] broadcast mask has 3 contents octets, expected 4"
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fixed {
    /// The contents octets every value of the type has.
    pub octets: u32,
    /// The label when another tag stands where the member's is due; its `{}`
    /// is the member's tag number.
    pub misplaced: &'static str,
    /// The label when the member's header announces another length; its two
    /// `{}` are the tag number and the length announced.
    pub wrong_length: &'static str,
}

/// [`Fixed`] for the type `$name` of `$octets` octets.
macro_rules! fixed {
    ($name:literal, $octets:literal) => {
        Fixed {
            octets: $octets,
            misplaced: concat!("expected context tag [{}] ", $name),
            wrong_length: concat!(
                "[{}] ",
                $name,
                " has {} contents octets, expected ",
                $octets
            ),
        }
    };
}

impl Fixed {
    /// A REAL: four octets.
    pub const REAL: Self = fixed!("REAL", 4);
    /// A Double: eight octets.
    pub const DOUBLE: Self = fixed!("Double", 8);
    /// A BOOLEAN: one octet.
    pub const BOOLEAN: Self = fixed!("BOOLEAN", 1);
    /// A Date: four octets.
    pub const DATE: Self = fixed!("Date", 4);
    /// A Time: four octets.
    pub const TIME: Self = fixed!("Time", 4);
    /// An object identifier: four octets.
    pub const OBJECT_IDENTIFIER: Self = fixed!("object identifier", 4);
}

/// Require a primitive context tag `tag` at `offset` holding exactly
/// `fixed.octets` contents octets of that type; return those contents and the
/// offset past them. A header announcing any other length is
/// [`Error::Decoding`] even when the data also stops early.
#[inline]
pub fn decode_ctx_fixed<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    fixed: Fixed,
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = ctx_header(data, offset, tag, fixed.misplaced, what)?;
    if t.length != fixed.octets {
        return Err(ctx_length_error(offset, tag, fixed, what, t.length));
    }
    contents(data, start, fixed.octets)
}

/// Report a wrong fixed length before attempting to read its contents.
#[cold]
fn ctx_length_error(
    offset: usize,
    tag: u8,
    fixed: Fixed,
    what: &'static str,
    length: u32,
) -> Error {
    DecodeError::new(DecodingKind::InvalidEncoding, offset, fixed.wrong_length)
        .within(what)
        .with_values(tag.into(), length.into())
        .into()
}

/// Require a primitive context tag `tag` at `offset` holding a REAL.
pub fn decode_ctx_real(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(f32, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, Fixed::REAL, what)?;
    Ok((primitives::decode_real(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding a BOOLEAN: one
/// contents octet, 0 or 1 (Clause 20.2.3).
pub fn decode_ctx_boolean(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(bool, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, Fixed::BOOLEAN, what)?;
    match octets[0] {
        0 => Ok((false, end)),
        1 => Ok((true, end)),
        _ => Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            end - 1,
            "[{}] BOOLEAN contents must be 0 or 1",
        )
        .within(what)
        .with_value(tag.into())
        .into()),
    }
}

/// Require a primitive context tag `tag` at `offset` holding an object
/// identifier, which is always four octets.
#[inline]
pub fn decode_ctx_object_id(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (octets, end) = decode_ctx_fixed(data, offset, tag, Fixed::OBJECT_IDENTIFIER, what)?;
    Ok((ObjectIdentifier::decode(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding a BIT STRING;
/// returns its unused-bit count and a copy of its data octets. Only with
/// `alloc`; [`decode_ctx_bit_string_ref`] borrows the octets.
#[cfg(feature = "alloc")]
pub fn decode_ctx_bit_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let ((unused, bits), end) = decode_ctx_bit_string_ref(data, offset, tag, what)?;
    Ok(((unused, bits.to_vec()), end))
}

/// Require a primitive context tag `tag` at `offset` holding a BIT STRING;
/// returns its unused-bit count and data octets, borrowed from `data`.
pub fn decode_ctx_bit_string_ref<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<((u8, &'a [u8]), usize), Error> {
    let (t, start) = ctx_header(
        data,
        offset,
        tag,
        "expected context tag [{}] BIT STRING",
        what,
    )?;
    let (octets, end) = contents(data, start, t.length)?;
    Ok((primitives::decode_bit_string_ref(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding an OCTET STRING;
/// returns a copy of its octets. Only with `alloc`;
/// [`decode_ctx_octet_string_ref`] borrows them.
#[cfg(feature = "alloc")]
pub fn decode_ctx_octet_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(Vec<u8>, usize), Error> {
    let (octets, end) = decode_ctx_octet_string_ref(data, offset, tag, what)?;
    Ok((octets.to_vec(), end))
}

/// Require a primitive context tag `tag` at `offset` holding an OCTET STRING;
/// returns its octets, borrowed from `data`.
pub fn decode_ctx_octet_string_ref<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = ctx_header(
        data,
        offset,
        tag,
        "expected context tag [{}] OCTET STRING",
        what,
    )?;
    contents(data, start, t.length)
}

/// Require a primitive context tag `tag` at `offset` holding a
/// CharacterString. Only with `alloc`; [`decode_ctx_character_string_ref`]
/// reads it without converting it.
#[cfg(feature = "alloc")]
pub fn decode_ctx_character_string(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(String, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    Ok((primitives::decode_character_string(octets)?, end))
}

/// Require a primitive context tag `tag` at `offset` holding a
/// CharacterString; returns it borrowed from `data`, its character set
/// checked (see [`primitives::CharacterStringRef::decode`]).
pub fn decode_ctx_character_string_ref<'a>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<(primitives::CharacterStringRef<'a>, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    Ok((primitives::CharacterStringRef::decode(octets)?, end))
}

/// Read the optional member under primitive context tag `tag` with `decode`
/// when that tag comes next; otherwise `None` with the offset unchanged.
/// What `decode` returns may borrow from `data`, so [`decode_ctx_primitive`]
/// yields the member's contents.
pub fn decode_optional_ctx<'a, T>(
    data: &'a [u8],
    offset: usize,
    tag: u8,
    what: &'static str,
    decode: impl FnOnce(&'a [u8], usize, u8, &'static str) -> Result<(T, usize), Error>,
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
    /// The label when a context-tagged value doesn't fit: its two `{}` are
    /// the tag number and the value.
    const CONTEXT_EXCEEDS: &'static str;
    /// The label when an application-tagged Unsigned doesn't fit.
    const UNSIGNED_EXCEEDS: &'static str;
    /// The label when an application-tagged ENUMERATED doesn't fit.
    const ENUMERATED_EXCEEDS: &'static str;
}

/// [`UnsignedWidth`] for `$ty`.
macro_rules! width {
    ($ty:ident) => {
        impl UnsignedWidth for $ty {
            const NAME: &'static str = stringify!($ty);
            const CONTEXT_EXCEEDS: &'static str =
                concat!("[{}] value {} exceeds ", stringify!($ty));
            const UNSIGNED_EXCEEDS: &'static str = concat!("Unsigned exceeds ", stringify!($ty));
            const ENUMERATED_EXCEEDS: &'static str =
                concat!("ENUMERATED exceeds ", stringify!($ty));
        }
    };
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

width!(u8);
width!(u16);
width!(u32);
width!(u64);

/// Narrow `value`, read from context tag `tag` at `offset`, to `T`.
fn narrow<T: UnsignedWidth>(
    value: u64,
    offset: usize,
    tag: u8,
    what: &'static str,
) -> Result<T, Error> {
    T::try_from(value).map_err(|_| {
        DecodeError::new(DecodingKind::OutOfRange, offset, T::CONTEXT_EXCEEDS)
            .within(what)
            .with_values(tag.into(), value)
            .into()
    })
}

/// Require a primitive context tag `tag` at `offset` holding an Unsigned
/// that fits `T`. An ENUMERATED's contents are encoded the same way, so this
/// reads those too. Leading zero octets are accepted, as
/// [`primitives::decode_unsigned`] accepts them.
#[inline]
pub fn decode_ctx_unsigned<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
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
    what: &'static str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_ctx_primitive(data, offset, tag, what)?;
    let value = decode_canonical_unsigned(octets, offset, what)?;
    Ok((narrow(value, offset, tag, what)?, end))
}

/// The value of Unsigned or ENUMERATED contents in their shortest encoding:
/// one to eight octets, and no leading zero octet unless it is the only one.
/// Errors report `offset`, the field's tag.
pub fn decode_canonical_unsigned(
    octets: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<u64, Error> {
    if octets.is_empty() || octets.len() > 8 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "has {} contents octets, expected one to eight",
        )
        .within(what)
        .with_value(octets.len() as u64)
        .into());
    }
    if octets.len() > 1 && octets[0] == 0 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "must use the shortest Unsigned/Enumerated encoding",
        )
        .within(what)
        .into());
    }
    primitives::decode_unsigned(octets)
}

// ---------------------------------------------------------------------------
// Application-tagged items
// ---------------------------------------------------------------------------

/// Require an application tag `number` (one of [`tags::app_tag`]) at
/// `offset`; return its contents and the offset past them.
///
/// A BOOLEAN is refused with [`Error::Decoding`] whatever the data holds: its
/// value is the tag's length field, so it has no contents to return (see
/// [`Tag::is_boolean_true`]).
///
/// The label is static, so a read that succeeds builds none (#1645), and a
/// refusal builds its text only when displayed (#1646).
///
/// ```
/// use bacnet_encoding::constructed::tagged::decode_app_primitive;
/// use bacnet_encoding::tags::app_tag;
///
/// // A record that is an Unsigned rather than an OCTET STRING.
/// let error = decode_app_primitive(&[0x21, 0x01], 0, app_tag::OCTET_STRING, "record").unwrap_err();
/// assert_eq!(
///     error.to_string(),
///     "decoding error at offset 0: record: expected application-tagged OCTET STRING",
/// );
/// ```
pub fn decode_app_primitive<'a>(
    data: &'a [u8],
    offset: usize,
    number: u8,
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
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
    what: &'static str,
) -> Result<(Tag, usize), Error> {
    if number == tags::app_tag::BOOLEAN {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "an application-tagged BOOLEAN has no contents to read",
        )
        .within(what)
        .into());
    }
    let (t, start) = tags::decode_tag(data, offset)?;
    if t.class != TagClass::Application || t.number != number {
        return Err(misplaced_member(
            data,
            &t,
            None,
            offset,
            what,
            ("expected application-tagged {t}", number),
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
    what: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (t, start) = app_header(data, offset, number, what)?;
    if t.length != octets {
        let refused = match primitives::fixed_length_label(number) {
            Some((label, _)) => DecodeError::new(DecodingKind::InvalidEncoding, offset, label)
                .with_value(t.length.into()),
            None => DecodeError::new(
                DecodingKind::InvalidEncoding,
                offset,
                "application value has {} contents octets, expected {}",
            )
            .with_values(t.length.into(), octets.into()),
        };
        return Err(refused.within(what).into());
    }
    contents(data, start, octets)
}

/// Require an application-tagged object identifier, four contents octets,
/// at `offset`.
pub fn decode_app_object_id(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(ObjectIdentifier, usize), Error> {
    let (octets, end) = decode_app_fixed(data, offset, tags::app_tag::OBJECT_IDENTIFIER, 4, what)?;
    Ok((ObjectIdentifier::decode(octets)?, end))
}

/// Decode one application-tagged Unsigned that fits `T`. Leading zero octets
/// are accepted.
pub fn decode_app_unsigned<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::UNSIGNED, what)?;
    let value = primitives::decode_unsigned(octets)?;
    Ok((
        narrow_app(value, end - octets.len(), T::UNSIGNED_EXCEEDS, what)?,
        end,
    ))
}

/// Decode one application-tagged BIT STRING (`SEQUENCE OF BIT STRING` item).
/// Only with `alloc`: it copies the data octets.
#[cfg(feature = "alloc")]
pub fn decode_app_bit_string(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<((u8, Vec<u8>), usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::BIT_STRING, what)?;
    Ok((primitives::decode_bit_string(octets)?, end))
}

/// Decode one application-tagged ENUMERATED (a `SEQUENCE OF` item, or an
/// error class or code) that fits `T`. Leading zero octets are accepted.
pub fn decode_app_enumerated<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::ENUMERATED, what)?;
    let value = primitives::decode_unsigned(octets)?;
    Ok((
        narrow_app(value, end - octets.len(), T::ENUMERATED_EXCEEDS, what)?,
        end,
    ))
}

/// [`decode_app_enumerated`] for a codec that must re-encode what it read
/// octet for octet: the contents must also be the shortest encoding (see
/// [`decode_canonical_unsigned`]).
#[cfg(feature = "alloc")]
pub(crate) fn decode_app_canonical_enumerated<T: UnsignedWidth>(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(T, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::ENUMERATED, what)?;
    let value = decode_canonical_unsigned(octets, offset, what)?;
    Ok((
        narrow_app(value, end - octets.len(), T::ENUMERATED_EXCEEDS, what)?,
        end,
    ))
}

/// Narrow an application-tagged value whose contents start at `contents` to
/// `T`, `label` naming the refusal.
fn narrow_app<T: UnsignedWidth>(
    value: u64,
    contents: usize,
    label: &'static str,
    what: &'static str,
) -> Result<T, Error> {
    T::try_from(value).map_err(|_| {
        DecodeError::new(DecodingKind::OutOfRange, contents, label)
            .within(what)
            .into()
    })
}

/// Decode one application-tagged CharacterString. Only with `alloc`;
/// [`decode_app_character_string_ref`] reads it without converting it.
#[cfg(feature = "alloc")]
pub fn decode_app_character_string(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<(String, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::CHARACTER_STRING, what)?;
    Ok((primitives::decode_character_string(octets)?, end))
}

/// Decode one application-tagged CharacterString, borrowed from `data`, its
/// character set checked (see [`primitives::CharacterStringRef::decode`]).
pub fn decode_app_character_string_ref<'a>(
    data: &'a [u8],
    offset: usize,
    what: &'static str,
) -> Result<(primitives::CharacterStringRef<'a>, usize), Error> {
    let (octets, end) = decode_app_primitive(data, offset, tags::app_tag::CHARACTER_STRING, what)?;
    Ok((primitives::CharacterStringRef::decode(octets)?, end))
}

#[cfg(all(test, feature = "alloc"))]
#[path = "tagged_frame_tests.rs"]
mod frame_tests;
