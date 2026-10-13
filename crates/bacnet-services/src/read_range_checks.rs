//! The rules a ReadRange acknowledgement keeps against its request, and the
//! choice of refusing one that breaks them or keeping it with the list of
//! what it broke (#1531).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::error::{Error, ReadRangeViolation};

use super::{RangeSpec, ReadRangeAck, ReadRangeRequest};

/// How a requester treats an acknowledgement that breaks a rule it checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadRangeValidation {
    /// Refuse it with [`Error::ReadRangeViolation`], naming the first rule
    /// broken.
    #[default]
    Strict,
    /// Keep it, listing every rule it broke in
    /// [`ReadRangeReply::violations`].
    Lenient,
}

/// A decoded acknowledgement and the rules it broke, empty unless the read
/// was [`ReadRangeValidation::Lenient`].
#[derive(Debug, Clone)]
pub struct ReadRangeReply {
    /// The acknowledgement as the device sent it.
    pub ack: ReadRangeAck,
    /// The rules the acknowledgement broke, in the order they are checked.
    pub violations: Vec<ReadRangeViolation>,
}

impl ReadRangeReply {
    /// Check `ack` against the `request` it answers: refuse it on the first
    /// rule it breaks under `Strict`, keep it with every rule it breaks under
    /// `Lenient`.
    pub fn check(
        request: &ReadRangeRequest,
        ack: ReadRangeAck,
        validation: ReadRangeValidation,
    ) -> Result<Self, Error> {
        let violations = ack.violations(request);
        match (validation, violations.first()) {
            (ReadRangeValidation::Strict, Some(&violation)) => {
                Err(Error::ReadRangeViolation(violation))
            }
            _ => Ok(Self { ack, violations }),
        }
    }
}

impl ReadRangeAck {
    /// Every rule this acknowledgement breaks against the `request` it
    /// answers, in a fixed order: the echoed object, property and array
    /// index, then the first sequence number, the result flags and the item
    /// count.
    pub fn violations(&self, request: &ReadRangeRequest) -> Vec<ReadRangeViolation> {
        let mut found = Vec::new();
        if self.object_identifier != request.object_identifier {
            found.push(ReadRangeViolation::ObjectMismatch);
        }
        if self.property_identifier != request.property_identifier {
            found.push(ReadRangeViolation::PropertyMismatch);
        }
        if self.property_array_index != request.property_array_index {
            found.push(ReadRangeViolation::ArrayIndexMismatch);
        }

        let (sequenced, count) = match &request.range {
            None => (false, None),
            Some(RangeSpec::ByPosition { count, .. }) => (false, Some(*count)),
            Some(RangeSpec::BySequenceNumber { count, .. } | RangeSpec::ByTime { count, .. }) => {
                (true, Some(*count))
            }
        };
        match (sequenced, self.item_count, self.first_sequence_number) {
            (true, 1.., None) => found.push(ReadRangeViolation::MissingFirstSequenceNumber),
            (true, 1.., Some(0)) => found.push(ReadRangeViolation::ZeroFirstSequenceNumber),
            (true, 1.., Some(_)) | (_, _, None) => {}
            // Only a nonempty sequenced answer numbers its first item.
            (_, _, Some(_)) => found.push(ReadRangeViolation::UnexpectedFirstSequenceNumber),
        }

        let (first_item, last_item, more_items) = self.result_flags;
        // A ranged read's reference fixes which end a cut page gives up. A
        // read with no range has none, and Clause 15.8.2 doesn't say which
        // items it keeps, so there only a page holding both ends yet still
        // claiming more is a contradiction.
        let past_end = match count {
            Some(count) if count < 0 => first_item,
            Some(_) => last_item,
            None => first_item && last_item,
        };
        if more_items && past_end {
            found.push(ReadRangeViolation::MoreItemsPastEnd);
        }
        if count.is_some_and(|count| u64::from(self.item_count) > u64::from(count.unsigned_abs())) {
            found.push(ReadRangeViolation::ItemCountExceedsRequest);
        }
        found
    }
}

#[cfg(test)]
#[path = "read_range_checks_tests.rs"]
mod tests;
