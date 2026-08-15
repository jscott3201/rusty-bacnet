use super::*;

use crate::common::{decode_context, decode_context_u32};

// ---------------------------------------------------------------------------
// AcknowledgeAlarm
// ---------------------------------------------------------------------------

/// AcknowledgeAlarm-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AcknowledgeAlarmRequest {
    pub acknowledging_process_identifier: u32,
    pub event_object_identifier: ObjectIdentifier,
    pub event_state_acknowledged: u32,
    pub timestamp: BACnetTimeStamp,
    pub acknowledgment_source: String,
    /// Time of acknowledgment.
    pub time_of_acknowledgment: BACnetTimeStamp,
}

impl AcknowledgeAlarmRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] acknowledgingProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.acknowledging_process_identifier as u64);
        // [1] eventObjectIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.event_object_identifier);
        // [2] eventStateAcknowledged
        primitives::encode_ctx_enumerated(buf, 2, self.event_state_acknowledged);
        // [3] timestamp
        primitives::encode_timestamp(buf, 3, &self.timestamp)?;
        // [4] acknowledgmentSource
        primitives::encode_ctx_character_string(buf, 4, &self.acknowledgment_source)?;
        // [5] timeOfAcknowledgment
        primitives::encode_timestamp(buf, 5, &self.time_of_acknowledgment)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0]
        let (acknowledging_process_identifier, end) =
            decode_context_u32(data, offset, 0, "AcknowledgeAlarm process-id")?;
        offset = end;

        // [1]
        let (content, end) = decode_context(data, offset, 1, "AcknowledgeAlarm object-id")?;
        let event_object_identifier = ObjectIdentifier::decode(content)?;
        offset = end;

        // [2]
        let (event_state_acknowledged, end) =
            decode_context_u32(data, offset, 2, "AcknowledgeAlarm event-state")?;
        offset = end;

        // [3] timestamp
        let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_offset;

        // [4] acknowledgmentSource
        let (content, end) =
            decode_context(data, offset, 4, "AcknowledgeAlarm acknowledgment-source")?;
        let acknowledgment_source = primitives::decode_character_string(content)?;
        offset = end;

        // [5] timeOfAcknowledgment
        let (time_of_acknowledgment, new_offset) = primitives::decode_timestamp(data, offset, 5)?;
        if new_offset != data.len() {
            return Err(Error::decoding(
                new_offset,
                "AcknowledgeAlarm trailing data after request",
            ));
        }

        Ok(Self {
            acknowledging_process_identifier,
            event_object_identifier,
            event_state_acknowledged,
            timestamp,
            acknowledgment_source,
            time_of_acknowledgment,
        })
    }
}
