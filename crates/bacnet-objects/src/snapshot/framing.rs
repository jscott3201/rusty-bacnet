//! The header the one-file formats share: an eight-octet magic tag naming
//! the format and its version, then the four-octet identifier of the object
//! the file belongs to. The format's own body follows.

use crate::prelude::*;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{SnapshotError, SnapshotErrorKind};
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The magic tag and the object identifier.
pub(crate) const HEADER_LEN: usize = 8 + 4;

/// The body of a file's `octets` saved for `object`, after the header, and
/// whether its tag is `previous` (a version of the format before `magic`,
/// which still loads) rather than `magic`. Refuses a file past `max_len`
/// before looking at any of it, one saved for another object, and one
/// without either tag: a tag of the same family (its first six octets) with
/// another two-digit version is a version this build doesn't read, so a
/// backend keeping two copies doesn't fall back past it; any other tag is
/// another format.
pub(crate) fn open<'a>(
    octets: &'a [u8],
    magic: &[u8; 8],
    previous: Option<&[u8; 8]>,
    max_len: usize,
    object: ObjectIdentifier,
) -> Result<(bool, &'a [u8]), SnapshotError> {
    if octets.len() > max_len {
        return Err(SnapshotError::new(
            SnapshotErrorKind::TooLarge,
            "is too large",
        ));
    }
    let no_header = SnapshotError::new(SnapshotErrorKind::NotThisFormat, "has no valid header");
    let is_previous = match octets.get(..magic.len()) {
        Some(tag) if tag == magic => false,
        Some(tag) if previous.is_some_and(|previous| tag == previous) => true,
        Some(tag) if tag[..FAMILY_LEN] == magic[..FAMILY_LEN] => {
            return Err(version(&tag[FAMILY_LEN..]).map_or(no_header, |version| {
                SnapshotError::new(SnapshotErrorKind::UnknownVersion(version), "")
            }));
        }
        _ => return Err(no_header),
    };
    if octets.len() < HEADER_LEN {
        return Err(no_header);
    }
    if octets[magic.len()..HEADER_LEN] != object.encode() {
        return Err(SnapshotError::new(
            SnapshotErrorKind::OtherObject,
            "belongs to another object",
        ));
    }
    Ok((is_previous, &octets[HEADER_LEN..]))
}

/// The octets of a tag that name its format: the rest, two ASCII digits,
/// name the version.
const FAMILY_LEN: usize = 6;

/// The version two ASCII digits name, or `None` for anything else.
fn version(digits: &[u8]) -> Option<u16> {
    digits.iter().try_fold(0u16, |version, digit| {
        digit
            .is_ascii_digit()
            .then(|| version * 10 + u16::from(digit - b'0'))
    })
}

/// The file octets for `body` saved for `object` under `magic`.
pub(crate) fn seal(magic: &[u8; 8], object: ObjectIdentifier, body: &[u8]) -> Vec<u8> {
    let mut octets = Vec::with_capacity(HEADER_LEN + body.len());
    octets.extend_from_slice(magic);
    octets.extend_from_slice(&object.encode());
    octets.extend_from_slice(body);
    octets
}

/// The file octets for `body` saved for `object` under `magic`, or, with a
/// written `description`, under `described`, the Description going before
/// `body` (#2364): so a file without one keeps `magic`'s octets, and a
/// build that predates `described` refuses one with it as an unknown
/// version. Refuses a Description past 16 KiB.
pub(crate) fn seal_described(
    (magic, described): (&[u8; 8], &[u8; 8]),
    object: ObjectIdentifier,
    description: Option<&str>,
    body: &[u8],
) -> Result<Vec<u8>, SnapshotError> {
    let Some(text) = description else {
        return Ok(seal(magic, object, body));
    };
    let mut described_body = Vec::with_capacity(4 + text.len() + body.len());
    put_description(&mut described_body, text)?;
    described_body.extend_from_slice(body);
    Ok(seal(described, object, &described_body))
}

/// [`open`] for a format whose `described` version holds a Description
/// before the body of `magic`'s ([`seal_described`]): the Description, if
/// the file has one, and the body after it.
pub(crate) fn open_described<'a>(
    octets: &'a [u8],
    (magic, described): (&[u8; 8], &[u8; 8]),
    max_len: usize,
    object: ObjectIdentifier,
) -> Result<(Option<String>, &'a [u8]), SnapshotError> {
    let (plain, body) = open(octets, described, Some(magic), max_len, object)?;
    if plain {
        return Ok((None, body));
    }
    let (description, body) = take_description(body)?;
    Ok((Some(description), body))
}

/// Append a kept Description: its length in four big-endian octets, then
/// its UTF-8, at most [`MAX_DESCRIPTION_OCTETS`].
pub(crate) fn put_description(body: &mut Vec<u8>, text: &str) -> Result<(), SnapshotError> {
    if text.len() > MAX_DESCRIPTION_OCTETS {
        return Err(SnapshotError::out_of_range(
            "holds a Description past its cap",
        ));
    }
    body.extend_from_slice(&(text.len() as u32).to_be_bytes());
    body.extend_from_slice(text.as_bytes());
    Ok(())
}

/// The Description [`put_description`] wrote at the start of `body`, and
/// what follows it.
pub(crate) fn take_description(body: &[u8]) -> Result<(String, &[u8]), SnapshotError> {
    let Some((length, rest)) = body.split_first_chunk::<4>() else {
        return Err(SnapshotError::truncated("ends inside its Description"));
    };
    let length = usize::try_from(u32::from_be_bytes(*length))
        .ok()
        .filter(|length| *length <= MAX_DESCRIPTION_OCTETS)
        .ok_or(SnapshotError::damaged("holds a Description past its cap"))?;
    if rest.len() < length {
        return Err(SnapshotError::truncated("ends inside its Description"));
    }
    let (text, rest) = rest.split_at(length);
    let text = String::from_utf8(text.to_vec())
        .map_err(|_| SnapshotError::damaged("holds a Description that isn't UTF-8"))?;
    Ok((text, rest))
}

/// Decode `bytes` as one element after another with `decode`, refusing
/// more than `cap` of them or one that does not decode.
pub(crate) fn decode_capped<T>(
    bytes: &[u8],
    cap: usize,
    decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> Result<Vec<T>, SnapshotError> {
    let mut list = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if list.len() == cap {
            return Err(SnapshotError::damaged("holds more entries than the cap"));
        }
        let (element, next) = decode(bytes, offset)
            .map_err(|_| SnapshotError::damaged("holds an entry that does not decode"))?;
        list.push(element);
        offset = next;
    }
    Ok(list)
}
