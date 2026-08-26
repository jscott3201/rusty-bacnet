//! Shared storage for intrinsic-reporting event history properties.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventHistory {
    /// Transition slots ordered `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`.
    pub(crate) time_stamps: [BACnetTimeStamp; 3],
    /// Transition slots ordered `[TO_OFFNORMAL, TO_FAULT, TO_NORMAL]`.
    pub(crate) message_texts: [String; 3],
}

impl Default for EventHistory {
    fn default() -> Self {
        Self {
            time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            message_texts: [String::new(), String::new(), String::new()],
        }
    }
}

impl EventHistory {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Reads the flattened representation used by the existing object API.
    ///
    /// Array-index behavior is implemented here; #171 owns the remaining array
    /// integration. Timestamp projection is deliberately lossy: alternatives
    /// other than `SequenceNumber` flatten to zero pending #171, while #259 owns
    /// the timestamp choice wire representation.
    pub(crate) fn read(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        match property {
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Some(match array_index {
                None => Ok(PropertyValue::List(
                    self.time_stamps.iter().map(timestamp_value).collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(index @ 1..=3) => Ok(timestamp_value(&self.time_stamps[index as usize - 1])),
                Some(_) => Err(crate::common::invalid_array_index_error()),
            }),
            p if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS => Some(match array_index {
                None => Ok(PropertyValue::List(
                    self.message_texts
                        .iter()
                        .cloned()
                        .map(PropertyValue::CharacterString)
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(index @ 1..=3) => Ok(PropertyValue::CharacterString(
                    self.message_texts[index as usize - 1].clone(),
                )),
                Some(_) => Err(crate::common::invalid_array_index_error()),
            }),
            _ => None,
        }
    }
}

fn timestamp_value(stamp: &BACnetTimeStamp) -> PropertyValue {
    PropertyValue::Unsigned(match stamp {
        BACnetTimeStamp::SequenceNumber(n) => u64::from(*n),
        _ => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_restores_mutated_history_to_default() {
        let mut history = EventHistory::default();
        history.time_stamps[1] = BACnetTimeStamp::SequenceNumber(42);
        history.message_texts[2] = "transition".into();

        history.reset();

        assert_eq!(history, EventHistory::default());
    }
}
