//! Bounded immutable values retained by COV subscription snapshots.
use bacnet_objects::cov_change::{reports_change, same_value};
use bacnet_types::{
    enums::{ErrorClass, ErrorCode},
    error::Error,
    primitives::PropertyValue,
};
use std::sync::Arc;

/// A validated typed COV baseline. Clones share immutable, normalized storage.
/// Local retention limits: 32 nested Lists, 1,024 nodes and 65,536 payload bytes.
#[derive(Debug, Clone)]
pub struct CovSample(Arc<PropertyValue>);
impl CovSample {
    /// Validate before copying or encoding. Caller-owned spare capacity is not retained.
    pub fn new(value: &PropertyValue) -> Result<Self, Error> {
        let (mut nodes, mut bytes) = (0usize, 0usize);
        validate(value, 0, &mut nodes, &mut bytes)?;
        Ok(Self(Arc::new(normalize(value))))
    }
    /// Immutable typed content; mutation and unchecked baseline construction are unavailable.
    pub fn value(&self) -> &PropertyValue {
        &self.0
    }

    /// Whether this sample is a change from `previous` worth reporting,
    /// by the shared Clause 13.1 test ([`reports_change`]); with no
    /// previous sample it always is.
    pub(crate) fn reports(
        &self,
        previous: Option<&Self>,
        increment: Option<f64>,
        numeric: bool,
    ) -> bool {
        previous.is_none_or(|previous| {
            reports_change(self.value(), previous.value(), increment, numeric)
        })
    }
}
impl PartialEq for CovSample {
    fn eq(&self, other: &Self) -> bool {
        same_value(self.value(), other.value())
    }
}
impl Eq for CovSample {}
fn capacity_error() -> Error {
    Error::Protocol {
        class: ErrorClass::RESOURCES,
        code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    }
}
fn validate(
    v: &PropertyValue,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), Error> {
    *nodes = nodes.checked_add(1).ok_or_else(capacity_error)?;
    if *nodes > 1024 {
        return Err(capacity_error());
    }
    use PropertyValue::*;
    let size = match v {
        List(values) => {
            if depth == 32 {
                return Err(capacity_error());
            }
            for value in values {
                validate(value, depth + 1, nodes, bytes)?;
            }
            0
        }
        OctetString(v) | ApplicationData(v) => v.len(),
        CharacterString(v) => v.len(),
        BitString { data, .. } => data.len(),
        Null => 0,
        Boolean(_) => 1,
        Real(_) | Signed(_) | Enumerated(_) | Date(_) | Time(_) | ObjectIdentifier(_) => 4,
        Unsigned(_) | Double(_) => 8,
    };
    *bytes = bytes.checked_add(size).ok_or_else(capacity_error)?;
    if *bytes > 65536 {
        return Err(capacity_error());
    }
    Ok(())
}
fn normalize(v: &PropertyValue) -> PropertyValue {
    use PropertyValue::*;
    fn bytes(v: &[u8]) -> Vec<u8> {
        v.to_vec().into_boxed_slice().into_vec()
    }
    match v {
        List(v) => List(
            v.iter()
                .map(normalize)
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .into_vec(),
        ),
        OctetString(v) => OctetString(bytes(v)),
        ApplicationData(v) => ApplicationData(bytes(v)),
        CharacterString(v) => CharacterString(v.as_str().into()),
        BitString { unused_bits, data } => BitString {
            unused_bits: *unused_bits,
            data: bytes(data),
        },
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests;
