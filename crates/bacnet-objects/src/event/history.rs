//! Shared storage for intrinsic-reporting event history properties.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventHistory {
    pub(crate) time_stamps: [BACnetTimeStamp; 3],
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
    /// The array index is reserved for #171 and is intentionally ignored here.
    /// The timestamp choice wire representation remains owned by #171/#259.
    pub(crate) fn read(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        match property {
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => Some(Ok(PropertyValue::List(
                self.time_stamps
                    .iter()
                    .map(|stamp| {
                        PropertyValue::Unsigned(match stamp {
                            BACnetTimeStamp::SequenceNumber(n) => *n as u64,
                            _ => 0,
                        })
                    })
                    .collect(),
            ))),
            p if p == PropertyIdentifier::EVENT_MESSAGE_TEXTS => Some(Ok(PropertyValue::List(
                self.message_texts
                    .iter()
                    .cloned()
                    .map(PropertyValue::CharacterString)
                    .collect(),
            ))),
            _ => None,
        }
    }
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
