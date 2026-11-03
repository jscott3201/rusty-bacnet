//! The header the one-file formats share: an eight-octet magic tag naming
//! the format and its version, then the four-octet identifier of the object
//! the file belongs to. The format's own body follows.

use crate::prelude::*;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{SnapshotError, SnapshotErrorKind};

/// The magic tag and the object identifier.
pub(crate) const HEADER_LEN: usize = 8 + 4;

/// The body of a file's `octets` saved for `object`, after the header, and
/// whether its tag is `previous` (a version of the format before `magic`,
/// which still loads) rather than `magic`. Refuses a file past `max_len`
/// before looking at any of it, one without either tag, and one saved for
/// another object.
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

/// The file octets for `body` saved for `object` under `magic`.
pub(crate) fn seal(magic: &[u8; 8], object: ObjectIdentifier, body: &[u8]) -> Vec<u8> {
    let mut octets = Vec::with_capacity(HEADER_LEN + body.len());
    octets.extend_from_slice(magic);
    octets.extend_from_slice(&object.encode());
    octets.extend_from_slice(body);
    octets
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
