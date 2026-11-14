//! Folding element entries into a whole-property write (#1801).
//!
//! A whole write's pre-image is the array as it stands, element writes
//! included. The array before any of them is that pre-image with each
//! element entry's own pre-image put back at its index.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_ops::format::{Floats, StateTexts, ValueFormat, decode_all};
use bacnet_types::enums::ObjectType;
use bytes::BytesMut;

use super::{Entry, Stored};

/// The original array: `whole`, the whole write's pre-image, with each of
/// `elements`' pre-images put back. `None` (unknown) when any of them is
/// unknown, the array doesn't decode, or an index is outside it (a resize
/// was among them).
pub(super) fn original(whole: Option<&Stored>, elements: &[Entry]) -> Option<Stored> {
    let first = elements.first()?;
    let mut values = decode_all(&whole?.bytes)?;
    // One entry per index: repeated writes to it keep its first pre-image.
    for entry in elements {
        let index = usize::try_from(entry.index?).ok()?;
        let old = decode_all(&entry.before.as_ref()?.bytes)?;
        let [old] = <[_; 1]>::try_from(old).ok()?;
        *values.get_mut(index.checked_sub(1)?)? = old;
    }
    let mut buf = BytesMut::new();
    for value in &values {
        encode_property_value(&mut buf, value).ok()?;
    }
    let texts = StateTexts::default();
    let format = ValueFormat::new(
        ObjectType::from_raw(first.object_type),
        &texts,
        Floats::Exact,
    );
    Some(Stored {
        text: format.value(first.property(), &buf),
        bytes: buf.to_vec(),
    })
}
