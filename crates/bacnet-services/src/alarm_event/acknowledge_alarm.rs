#[cfg(not(feature = "std"))]
use alloc::string::String;

use super::*;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_object_id, decode_ctx_primitive, decode_ctx_unsigned, expect_end,
};

fn decode_acknowledgment_source(content: &[u8]) -> Result<String, Error> {
    if content.is_empty() {
        return primitives::decode_character_string(content);
    }
    Ok(primitives::decode_character_string(content).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// AcknowledgeAlarm
// ---------------------------------------------------------------------------

/// AcknowledgeAlarm-Request service parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AcknowledgeAlarmRequest {
    /// Identifies the process on the requesting device that performs the acknowledgment; how it
    /// is assigned is a local matter (Clause 13.5.1).
    pub acknowledging_process_identifier: u32,
    /// Object whose event transition is being acknowledged.
    pub event_object_identifier: ObjectIdentifier,
    /// Event state of the transition being acknowledged.
    pub event_state_acknowledged: EventState,
    /// Timestamp of the event transition being acknowledged, as given in the original notification.
    pub timestamp: BACnetTimeStamp,
    /// Free-form identification of the operator or system acknowledging the alarm.
    pub acknowledgment_source: String,
    /// Time of acknowledgment.
    pub time_of_acknowledgment: BACnetTimeStamp,
}

impl AcknowledgeAlarmRequest {
    /// Append the ASN.1 encoding to `buf`; fails if a string or timestamp is unencodable.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] acknowledgingProcessIdentifier
        primitives::encode_ctx_unsigned(buf, 0, self.acknowledging_process_identifier as u64);
        // [1] eventObjectIdentifier
        primitives::encode_ctx_object_id(buf, 1, &self.event_object_identifier);
        // [2] eventStateAcknowledged
        primitives::encode_ctx_enumerated(buf, 2, self.event_state_acknowledged.to_raw());
        // [3] timestamp
        primitives::encode_timestamp(buf, 3, &self.timestamp)?;
        // [4] acknowledgmentSource
        primitives::encode_ctx_character_string(buf, 4, &self.acknowledgment_source)?;
        // [5] timeOfAcknowledgment
        primitives::encode_timestamp(buf, 5, &self.time_of_acknowledgment)?;
        Ok(())
    }

    /// Decode the request from `data`; errors on missing, malformed or truncated fields.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0]
        let (acknowledging_process_identifier, end) =
            decode_ctx_unsigned::<u32>(data, offset, 0, "AcknowledgeAlarm process-id")?;
        offset = end;

        // [1]
        let (event_object_identifier, end) =
            decode_ctx_object_id(data, offset, 1, "AcknowledgeAlarm object-id")?;
        offset = end;

        // [2]
        let (event_state, end) =
            decode_ctx_unsigned::<u32>(data, offset, 2, "AcknowledgeAlarm event-state")?;
        let event_state_acknowledged = EventState::from_raw(event_state);
        offset = end;

        // [3] timestamp
        let (timestamp, new_offset) = primitives::decode_timestamp(data, offset, 3)?;
        offset = new_offset;

        // [4] acknowledgmentSource
        let (content, end) =
            decode_ctx_primitive(data, offset, 4, "AcknowledgeAlarm acknowledgment-source")?;
        let acknowledgment_source = decode_acknowledgment_source(content)?;
        offset = end;

        // [5] timeOfAcknowledgment
        let (time_of_acknowledgment, new_offset) = primitives::decode_timestamp(data, offset, 5)?;
        expect_end(data, new_offset, new_offset, "AcknowledgeAlarm")?;

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
