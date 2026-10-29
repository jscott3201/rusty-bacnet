//! A watched point's first read (from the TUI's watch list, #2115): its
//! properties with, when asked, the object's name and state texts, which
//! name its later values.

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ErrorClass, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use crate::engine::{Halt, Reader, Wanted};
use crate::format::{RawRow, StateTexts};

/// Read `wanted` of `object` once; the first of `wanted` is the watched
/// property.
///
/// With `name`, Object_Name comes free with the read. A device may answer
/// it with an error of its own, which only leaves the point nameless; one
/// that fails the whole read in a way the name could explain is asked
/// again without it (see [`name_may_explain`]). With `texts`, a binary or
/// multi-state object's state texts follow in a request of their own once
/// the read returned the watched property; they are only a nicety, so any
/// failure leaves them out. A refusal of RPM is left in `reader.rpm`.
pub async fn first_read<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    object: ObjectIdentifier,
    wanted: &[PropertyIdentifier],
    name: bool,
    texts: bool,
) -> Result<Vec<RawRow>, Halt> {
    let named = name && !wanted.contains(&PropertyIdentifier::OBJECT_NAME);
    let with_name = [wanted, &[PropertyIdentifier::OBJECT_NAME]].concat();
    let first_ask = if named { &with_name[..] } else { wanted };
    let mut read = reader.properties(object, Wanted::List(first_ask)).await;
    if let (true, Err(Halt::Device(error))) = (named, &read)
        && name_may_explain(error)
    {
        tracing::debug!(
            "{object}: first read failed ({error}); reading it again without Object_Name"
        );
        read = reader.properties(object, Wanted::List(wanted)).await;
    }
    let (mut rows, _) = read?;
    let watched = wanted.first();
    if texts && rows.iter().any(|(p, _)| Some(p) == watched) {
        let extra = state_texts(reader, object, wanted).await;
        rows.extend(extra);
    }
    Ok(rows)
}

/// True for a failed first read that asking for Object_Name may have
/// caused: a whole-request error of the PROPERTY class, or an answer that
/// didn't decode. Anything else (a timeout, a reject, an OBJECT-class
/// error) would fail the same way without the name.
pub fn name_may_explain(error: &Error) -> bool {
    match error {
        Error::Remote(RemoteError::Error { class, .. }) => *class == ErrorClass::PROPERTY,
        Error::Decoding(_) => true,
        _ => false,
    }
}

/// A binary or multi-state object's state texts not among `wanted`, read
/// through the same gate; none when the read fails in any way (a refusal,
/// an unknown property, an answer too large).
async fn state_texts<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    object: ObjectIdentifier,
    wanted: &[PropertyIdentifier],
) -> Vec<RawRow> {
    let texts: Vec<PropertyIdentifier> = StateTexts::properties(object.object_type())
        .iter()
        .copied()
        .filter(|property| !wanted.contains(property))
        .collect();
    if texts.is_empty() {
        return Vec::new();
    }
    match reader.properties(object, Wanted::List(&texts)).await {
        Ok((rows, _)) => rows,
        Err(error) => {
            tracing::debug!("{object}: state texts not read ({error:?}); values show without them");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bacnet_types::enums::{AbortReason, ErrorCode, RejectReason};

    use super::*;

    #[test]
    fn only_a_property_error_or_a_bad_answer_retries_without_the_name() {
        let error = |class| {
            Error::Remote(RemoteError::Error {
                class,
                code: ErrorCode::UNKNOWN_PROPERTY,
            })
        };
        assert!(name_may_explain(&error(ErrorClass::PROPERTY)));
        assert!(name_may_explain(&Error::decoding(3, "read access result")));
        assert!(!name_may_explain(&error(ErrorClass::OBJECT)));
        assert!(!name_may_explain(&Error::reject(
            RejectReason::UNRECOGNIZED_SERVICE
        )));
        assert!(!name_may_explain(&Error::abort(AbortReason::TSM_TIMEOUT)));
        assert!(!name_may_explain(&Error::Timeout(Duration::from_secs(6))));
    }
}
