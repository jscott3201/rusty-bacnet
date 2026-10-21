//! Member helpers the elevator codecs (landing calls, landing doors,
//! assigned landing calls and car call lists) and the lighting and colour
//! command codecs share.
//!
//! Each decodes primitive Unsigned members and keeps two kinds of failure
//! apart, so a property writer can answer with the matching Clause 15.9.1.3
//! error: a malformed member fails at once, while a well-formed member whose
//! value doesn't fit its type is only recorded, and the codec reports it as
//! [`Error::Range`] once the rest of the value has been checked. The
//! codecs read each member's contents with [`super::tagged`].

use bacnet_types::error::{DecodeError, DecodingKind, Error};

use super::tagged::decode_ctx_primitive;

/// The value of an Unsigned member's content octets, or `None` when the
/// encoding is well formed but the value needs more than 64 bits. Empty
/// content is malformed.
pub(super) fn unsigned_member(content: &[u8], at: usize) -> Result<Option<u64>, Error> {
    if content.is_empty() {
        return Err(Error::decoding(at, "Unsigned member has no content octets"));
    }
    let first = content.iter().position(|&octet| octet != 0);
    let significant = first.map_or(&[][..], |first| &content[first..]);
    if significant.len() > 8 {
        return Ok(None);
    }
    Ok(Some(
        significant
            .iter()
            .fold(0, |value, &octet| (value << 8) | u64::from(octet)),
    ))
}

/// Narrow a decoded Unsigned to its member type. When it doesn't fit, record
/// `what` as the first oversized member (if none is recorded yet) and yield a
/// placeholder, so decoding can still check the rest of the structure.
pub(super) fn narrow<T: TryFrom<u64> + Default>(
    value: Option<u64>,
    what: &'static str,
    oversized: &mut Option<&'static str>,
) -> T {
    match value.map(T::try_from) {
        Some(Ok(value)) => value,
        _ => {
            oversized.get_or_insert(what);
            T::default()
        }
    }
}

/// The most contents octets an Unsigned or ENUMERATED field of a command
/// may have when its first octet is zero. Every such field fits in 32 bits,
/// so a longer one that opens with a zero octet isn't in its shortest form
/// (Clause 20.2.4).
const MAX_PADDED_OCTETS: usize = 4;

/// Read the Unsigned or ENUMERATED under primitive context tag `tag` at
/// `offset` and narrow it to `T`, for the command codecs (`what` names the
/// production in errors). A value too wide for `T` is recorded as
/// `oversized` and read as zero, so the rest of the structure still gets
/// checked; more than four contents octets that open with zero are
/// malformed.
pub(super) fn unsigned_field<T: TryFrom<u64> + Default>(
    data: &[u8],
    offset: usize,
    tag: u8,
    what: &'static str,
    too_wide: &'static str,
    oversized: &mut Option<&'static str>,
) -> Result<(T, usize), Error> {
    let (content, end) = decode_ctx_primitive(data, offset, tag, what)?;
    if content.len() > MAX_PADDED_OCTETS && content[0] == 0 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "[{}] has {} contents octets and a leading zero",
        )
        .within(what)
        .with_values(tag.into(), content.len() as u64)
        .into());
    }
    let value = unsigned_member(content, end - content.len())?;
    Ok((narrow(value, too_wide, oversized), end))
}
