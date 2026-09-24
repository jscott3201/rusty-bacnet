//! Bounded immutable values retained by COV subscription snapshots.
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

    pub(crate) fn reports(
        &self,
        previous: Option<&Self>,
        increment: Option<f32>,
        numeric: bool,
    ) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        if !numeric {
            return self != previous;
        }
        use PropertyValue::*;
        match (self.value(), previous.value()) {
            (Unsigned(a), Unsigned(b)) => integer_delta(a.abs_diff(*b), increment),
            (Signed(a), Signed(b)) => integer_delta(u64::from(a.abs_diff(*b)), increment),
            (Real(a), Real(b)) => {
                if !a.is_finite() || !b.is_finite() {
                    a.to_bits() != b.to_bits()
                } else {
                    float_delta((f64::from(*a) - f64::from(*b)).abs(), increment)
                }
            }
            (Double(a), Double(b)) => {
                if !a.is_finite() || !b.is_finite() {
                    a.to_bits() != b.to_bits()
                } else {
                    float_delta((a - b).abs(), increment)
                }
            }
            _ => self != previous,
        }
    }
}
impl PartialEq for CovSample {
    fn eq(&self, other: &Self) -> bool {
        equal(self.value(), other.value())
    }
}
impl Eq for CovSample {}
fn integer_delta(delta: u64, increment: Option<f32>) -> bool {
    match increment {
        None => delta != 0,
        Some(i) if i <= 0.0 => true,
        Some(i) if i.is_nan() || i >= 18_446_744_073_709_551_616.0 => false,
        Some(i) => delta >= f64::from(i).ceil() as u64,
    }
}
fn float_delta(delta: f64, increment: Option<f32>) -> bool {
    match increment {
        None => delta != 0.0,
        Some(i) if i <= 0.0 => true,
        Some(i) if !i.is_finite() => false,
        Some(i) => delta >= f64::from(i),
    }
}
fn capacity_error() -> Error {
    Error::Protocol {
        class: ErrorClass::RESOURCES.to_raw() as u32,
        code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
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
fn equal(a: &PropertyValue, b: &PropertyValue) -> bool {
    use PropertyValue::*;
    match (a, b) {
        (Real(a), Real(b)) => a.to_bits() == b.to_bits(),
        (Double(a), Double(b)) => a.to_bits() == b.to_bits(),
        (List(a), List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b)),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests;
