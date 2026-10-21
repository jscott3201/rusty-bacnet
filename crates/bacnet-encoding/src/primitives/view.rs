//! Borrowed views of CharacterString contents and application-tagged values,
//! which read the octets in place instead of copying them (#1614).

use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::{
    charset, decode_bit_string_ref, decode_double, decode_real, decode_signed, decode_unsigned,
    fixed_length_label, invalid_utf8, unknown_charset,
};
use crate::tags::{self, app_tag, Tag, TagClass};

/// A CharacterString's contents, borrowed from the octets they were decoded
/// from: the character-set octet, then the string in that character set
/// (Clause 20.2.9).
///
/// [`decode`](Self::decode) checks the contents' structure as every decoder
/// of a CharacterString does (#1778): a character set the clause defines,
/// valid UTF-8, a DBCS string's two-octet code page, and whole UCS-2 and
/// UCS-4 characters that are Unicode scalar values. So
/// [`as_str`](Self::as_str) borrows UTF-8 contents as a `&str`; contents in
/// another character set are carried as they are, for the caller to convert.
///
/// ```
/// use bacnet_encoding::primitives::{charset, CharacterStringRef};
///
/// let text = CharacterStringRef::decode(&[charset::UTF8, b'A', b'V'])?;
/// assert_eq!((text.charset(), text.as_str()), (charset::UTF8, Some("AV")));
///
/// let latin1 = CharacterStringRef::decode(&[charset::ISO_8859_1, 0xE9])?;
/// assert_eq!((latin1.as_str(), latin1.bytes()), (None, &[0xE9][..]));
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharacterStringRef<'a> {
    /// The character-set octet and the string after it; never empty once
    /// decoded.
    contents: &'a [u8],
}

impl<'a> CharacterStringRef<'a> {
    /// Read CharacterString contents, refusing the malformed structures the
    /// type's docs list, each [`DecodingKind::InvalidEncoding`] at an offset
    /// into `contents`: no or a reserved character-set octet, invalid UTF-8,
    /// a DBCS string without its code page, and broken UCS-2 or UCS-4.
    pub fn decode(contents: &'a [u8]) -> Result<Self, Error> {
        check_character_string(contents)?;
        Ok(Self { contents })
    }

    /// Contents taken as they are, for a decoder that checks them itself.
    pub(super) fn unchecked(contents: &'a [u8]) -> Self {
        Self { contents }
    }

    /// The character set, one of [`charset`].
    pub fn charset(&self) -> u8 {
        self.contents.first().copied().unwrap_or(charset::UTF8)
    }

    /// The string's octets in its character set, after the character-set
    /// octet.
    pub fn bytes(&self) -> &'a [u8] {
        self.contents.get(1..).unwrap_or_default()
    }

    /// The complete contents: the character-set octet and the string.
    pub fn contents(&self) -> &'a [u8] {
        self.contents
    }

    /// The string, when it is UTF-8; `None` for any other character set.
    pub fn as_str(&self) -> Option<&'a str> {
        if self.charset() != charset::UTF8 {
            return None;
        }
        core::str::from_utf8(self.bytes()).ok()
    }
}

/// Check CharacterString `contents`, the character-set octet and the string
/// after it (Clause 20.2.9). The one check the borrowed view and the owned
/// validator share, so they can't drift apart (#1778). It refuses, each as
/// [`DecodingKind::InvalidEncoding`] at an offset into `contents`:
///
/// - no character-set octet, or one the clause doesn't define;
/// - UTF-8 contents that aren't valid UTF-8;
/// - an IBM/Microsoft DBCS string without its two-octet code page;
/// - UCS-2 or UCS-4 contents that aren't whole two- or four-octet
///   characters, or hold one that isn't a Unicode scalar value.
///
/// JIS X 0208 and ISO 8859-1 contents are taken as they are.
pub(super) fn check_character_string(contents: &[u8]) -> Result<(), Error> {
    let Some((&charset_id, payload)) = contents.split_first() else {
        return Err(Error::decoding(
            0,
            "CharacterString requires at least 1 byte for charset",
        ));
    };
    match charset_id {
        charset::UTF8 => {
            core::str::from_utf8(payload).map_err(invalid_utf8)?;
        }
        charset::IBM_MICROSOFT_DBCS if payload.len() < 2 => {
            return Err(Error::decoding(
                1,
                "DBCS CharacterString requires a two-octet code page",
            ));
        }
        charset::UCS2 => {
            let (characters, rest) = payload.as_chunks::<2>();
            if !rest.is_empty() {
                return Err(Error::decoding(1, "UCS-2 data must have even length"));
            }
            let not_scalar = |c: &[u8; 2]| char::from_u32(u16::from_be_bytes(*c).into()).is_none();
            if let Some(index) = characters.iter().position(not_scalar) {
                return Err(Error::decoding(1 + index * 2, "invalid UCS-2 code point"));
            }
        }
        charset::UCS4 => {
            let (characters, rest) = payload.as_chunks::<4>();
            if !rest.is_empty() {
                return Err(Error::decoding(
                    1,
                    "UCS-4 CharacterString length must be a multiple of four",
                ));
            }
            let not_scalar = |c: &[u8; 4]| char::from_u32(u32::from_be_bytes(*c)).is_none();
            if let Some(index) = characters.iter().position(not_scalar) {
                return Err(Error::decoding(1 + index * 4, "invalid UCS-4 code point"));
            }
        }
        charset::IBM_MICROSOFT_DBCS | charset::JIS_X_0208 | charset::ISO_8859_1 => {}
        other => return Err(unknown_charset(other)),
    }
    Ok(())
}

/// One application-tagged value of a primitive datatype (Clause 20.2.2 to
/// 20.2.14), borrowed from the octets it was decoded from.
///
/// [`decode_application_value_ref`] reads one; strings, octet strings and bit
/// strings stay in place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApplicationValueRef<'a> {
    /// Null.
    Null,
    /// Boolean.
    Boolean(bool),
    /// Unsigned integer, up to 64 bits.
    Unsigned(u64),
    /// Signed integer.
    Signed(i32),
    /// IEEE 754 single-precision float.
    Real(f32),
    /// IEEE 754 double-precision float.
    Double(f64),
    /// Octet string.
    OctetString(&'a [u8]),
    /// Character string.
    CharacterString(CharacterStringRef<'a>),
    /// Bit string.
    BitString {
        /// How many trailing bits of the final data octet carry nothing.
        unused_bits: u8,
        /// The bit data octets.
        data: &'a [u8],
    },
    /// Enumerated value.
    Enumerated(u32),
    /// Date.
    Date(Date),
    /// Time.
    Time(Time),
    /// Object identifier.
    ObjectIdentifier(ObjectIdentifier),
}

/// Decode one application-tagged primitive value from `data` at `offset`,
/// without copying it; returns the value and the offset past it.
///
/// It refuses what the owned decoder, `decode_application_value`, refuses
/// in an application-tagged value: a fixed-size value whose header gives
/// another length is [`Error::Decoding`], contents that run past the end of
/// `data` are [`Error::BufferTooShort`]. Every offset it reports indexes
/// `data`, a fault inside the contents included, where the owned decoder
/// reports one of those from the contents' start. It also refuses a
/// context-tagged element, which only the owned decoder keeps (as its raw
/// octets), with [`DecodingKind::InvalidTag`]. A CharacterString is checked
/// as [`CharacterStringRef::decode`] checks it, so one in a character set the
/// owned decoder can't convert still reads here.
///
/// ```
/// use bacnet_encoding::primitives::{decode_application_value_ref, ApplicationValueRef};
///
/// // A REAL 72.5, then a CharacterString "AV".
/// let data = [0x44, 0x42, 0x91, 0x00, 0x00, 0x73, 0x00, b'A', b'V'];
/// let (real, next) = decode_application_value_ref(&data, 0)?;
/// assert_eq!(real, ApplicationValueRef::Real(72.5));
/// let (text, end) = decode_application_value_ref(&data, next)?;
/// let ApplicationValueRef::CharacterString(text) = text else { unreachable!() };
/// assert_eq!((text.as_str(), end), (Some("AV"), data.len()));
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn decode_application_value_ref(
    data: &[u8],
    offset: usize,
) -> Result<(ApplicationValueRef<'_>, usize), Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if tag.class == TagClass::Context {
        return Err(Error::invalid_tag(
            offset,
            "expected an application-tagged value, found a context tag",
        ));
    }
    let (value, end) = decode_application_primitive::<true>(data, offset, tag, content_start)?;
    if let ApplicationValueRef::CharacterString(text) = value {
        let checked =
            CharacterStringRef::decode(text.contents()).map_err(rebased(content_start))?;
        return Ok((ApplicationValueRef::CharacterString(checked), end));
    }
    Ok((value, end))
}

/// A refusal of contents that start at `base` in the data, its offset moved
/// from the contents' start to the data's.
pub(super) fn rebased(base: usize) -> impl Fn(Error) -> Error {
    move |error| match error {
        Error::Decoding(mut refused) => {
            refused.offset = refused.offset.saturating_add(base);
            refused.into()
        }
        other => other,
    }
}

/// Decode the application-tagged value whose header `tag`, at `offset`, has
/// its contents at `content_start`. A CharacterString comes back unchecked:
/// the owned decoder converts it, and the view checks it. A fault inside the
/// contents is reported at its offset in `data` when `IN_DATA`, and from the
/// contents' start otherwise, as the owned decoder always has. A const
/// parameter, so the owned decoder's code carries no offset arithmetic.
pub(super) fn decode_application_primitive<const IN_DATA: bool>(
    data: &[u8],
    offset: usize,
    tag: Tag,
    content_start: usize,
) -> Result<(ApplicationValueRef<'_>, usize), Error> {
    if tag.is_opening || tag.is_closing {
        return Err(Error::invalid_tag(offset, "unexpected opening/closing tag"));
    }
    if tag.number == app_tag::NULL && tag.length != 0 {
        return Err(Error::decoding(
            offset,
            "application NULL must have no contents",
        ));
    }
    if tag.number == app_tag::BOOLEAN && tag.length > 1 {
        return Err(Error::decoding(
            offset,
            "application BOOLEAN L/V/T must be 0 or 1",
        ));
    }
    let content_end = content_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(content_start, "length overflow"))?;
    if tag.number == app_tag::BOOLEAN {
        return Ok((ApplicationValueRef::Boolean(tag.length != 0), content_start));
    }
    // A fixed-size value's length is judged before its contents are read, so
    // a wrong length is reported as such even when the data also stops early.
    if let Some((label, octets)) = fixed_length_label(tag.number) {
        if tag.length != octets {
            return Err(
                DecodeError::new(DecodingKind::InvalidEncoding, offset, label)
                    .with_value(tag.length.into())
                    .into(),
            );
        }
    }
    if data.len() < content_end {
        return Err(Error::buffer_too_short(content_end, data.len()));
    }
    let content = &data[content_start..content_end];
    let at = |error| {
        if IN_DATA {
            rebased(content_start)(error)
        } else {
            error
        }
    };
    let value = match tag.number {
        app_tag::NULL => ApplicationValueRef::Null,
        app_tag::UNSIGNED => ApplicationValueRef::Unsigned(decode_unsigned(content).map_err(at)?),
        app_tag::SIGNED => ApplicationValueRef::Signed(decode_signed(content).map_err(at)?),
        app_tag::REAL => ApplicationValueRef::Real(decode_real(content).map_err(at)?),
        app_tag::DOUBLE => ApplicationValueRef::Double(decode_double(content).map_err(at)?),
        app_tag::OCTET_STRING => ApplicationValueRef::OctetString(content),
        app_tag::CHARACTER_STRING => {
            ApplicationValueRef::CharacterString(CharacterStringRef::unchecked(content))
        }
        app_tag::BIT_STRING => {
            let (unused_bits, data) = decode_bit_string_ref(content).map_err(at)?;
            ApplicationValueRef::BitString { unused_bits, data }
        }
        app_tag::ENUMERATED => {
            let value = u32::try_from(decode_unsigned(content).map_err(at)?)
                .map_err(|_| Error::out_of_range(content_start, "ENUMERATED exceeds u32"))?;
            ApplicationValueRef::Enumerated(value)
        }
        app_tag::DATE => ApplicationValueRef::Date(Date::decode(content).map_err(at)?),
        app_tag::TIME => ApplicationValueRef::Time(Time::decode(content).map_err(at)?),
        app_tag::OBJECT_IDENTIFIER => {
            ApplicationValueRef::ObjectIdentifier(ObjectIdentifier::decode(content).map_err(at)?)
        }
        other => {
            return Err(DecodeError::new(
                DecodingKind::InvalidTag,
                offset,
                "unknown application tag number {}",
            )
            .with_value(other.into())
            .into());
        }
    };
    Ok((value, content_end))
}

#[cfg(test)]
mod tests;
