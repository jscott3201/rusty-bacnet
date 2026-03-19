//! GetAlarmSummary service per ASHRAE 135-2020 Clause 13.7 (deprecated).

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::EventState;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// GetAlarmSummaryAck
// ---------------------------------------------------------------------------

/// One entry in the GetAlarmSummary-ACK sequence.
///
/// `acknowledged_transitions` is a 3-bit bitstring encoded as
/// `(unused_bits, data)`. Bits represent: to-offnormal, to-fault, to-normal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmSummaryEntry {
    pub object_identifier: ObjectIdentifier,
    pub alarm_state: EventState,
    /// Raw bitstring: (unused_bits, data bytes).
    pub acknowledged_transitions: (u8, Vec<u8>),
}

/// GetAlarmSummary-ACK: a sequence of alarm summary entries.
///
/// GetAlarmSummary-Request has no parameters so no struct is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetAlarmSummaryAck {
    pub entries: Vec<AlarmSummaryEntry>,
}

impl GetAlarmSummaryAck {
    pub fn encode(&self, buf: &mut BytesMut) {
        for entry in &self.entries {
            primitives::encode_app_object_id(buf, &entry.object_identifier);
            primitives::encode_app_enumerated(buf, entry.alarm_state.to_raw());
            primitives::encode_app_bit_string(
                buf,
                entry.acknowledged_transitions.0,
                &entry.acknowledged_transitions.1,
            );
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            if entries.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "AlarmSummaryAck too many entries"));
            }

            // objectIdentifier (app)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck truncated at object-id",
                ));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // alarmState (app enumerated)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck truncated at alarmState",
                ));
            }
            let alarm_state =
                EventState::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
            offset = end;

            // acknowledgedTransitions (app bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck truncated at acknowledgedTransitions",
                ));
            }
            let acknowledged_transitions = primitives::decode_bit_string(&data[pos..end])?;
            offset = end;

            entries.push(AlarmSummaryEntry {
                object_identifier,
                alarm_state,
                acknowledged_transitions,
            });
        }

        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    #[test]
    fn ack_round_trip() {
        let ack = GetAlarmSummaryAck {
            entries: vec![
                AlarmSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                    alarm_state: EventState::HIGH_LIMIT,
                    // 3 bits used (5 unused): to-offnormal=1, to-fault=0, to-normal=1
                    acknowledged_transitions: (5, vec![0b10100000]),
                },
                AlarmSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 10).unwrap(),
                    alarm_state: EventState::OFFNORMAL,
                    acknowledged_transitions: (5, vec![0b11100000]),
                },
            ],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetAlarmSummaryAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_empty_round_trip() {
        let ack = GetAlarmSummaryAck { entries: vec![] };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetAlarmSummaryAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_single_entry_round_trip() {
        let ack = GetAlarmSummaryAck {
            entries: vec![AlarmSummaryEntry {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 42).unwrap(),
                alarm_state: EventState::FAULT,
                acknowledged_transitions: (5, vec![0b01000000]),
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = GetAlarmSummaryAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_ack_truncated_1_byte() {
        let ack = GetAlarmSummaryAck {
            entries: vec![AlarmSummaryEntry {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                alarm_state: EventState::HIGH_LIMIT,
                acknowledged_transitions: (5, vec![0b10100000]),
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(GetAlarmSummaryAck::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_ack_truncated_half() {
        let ack = GetAlarmSummaryAck {
            entries: vec![AlarmSummaryEntry {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                alarm_state: EventState::HIGH_LIMIT,
                acknowledged_transitions: (5, vec![0b10100000]),
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(GetAlarmSummaryAck::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_ack_invalid_tag() {
        assert!(GetAlarmSummaryAck::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
