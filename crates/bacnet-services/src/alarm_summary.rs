//! GetAlarmSummary service per ASHRAE 135-2020 Clause 13.10 (deprecated).

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_encoding::tags::{app_tag, TagClass};
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
            if tag.class != TagClass::Application
                || tag.number != app_tag::OBJECT_IDENTIFIER
                || data[offset] & 0x07 > 5
            {
                return Err(Error::decoding(
                    offset,
                    "AlarmSummaryAck expected object-id application tag",
                ));
            }
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
            if tag.class != TagClass::Application
                || tag.number != app_tag::ENUMERATED
                || data[offset] & 0x07 > 5
            {
                return Err(Error::decoding(
                    offset,
                    "AlarmSummaryAck expected enumerated application tag",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck truncated at alarmState",
                ));
            }
            let alarm_state = primitives::decode_unsigned(&data[pos..end])?;
            let alarm_state = u32::try_from(alarm_state)
                .map(EventState::from_raw)
                .map_err(|_| Error::decoding(pos, "AlarmSummaryAck alarmState exceeds u32"))?;
            offset = end;

            // acknowledgedTransitions (app bitstring)
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if tag.class != TagClass::Application
                || tag.number != app_tag::BIT_STRING
                || data[offset] & 0x07 > 5
            {
                return Err(Error::decoding(
                    offset,
                    "AlarmSummaryAck expected bit-string application tag",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck truncated at acknowledgedTransitions",
                ));
            }
            let acknowledged_transitions = primitives::decode_bit_string(&data[pos..end])?;
            if acknowledged_transitions.0 != 5
                || acknowledged_transitions.1.len() != 1
                || acknowledged_transitions.1[0] & 0x1F != 0
            {
                return Err(Error::decoding(
                    pos,
                    "AlarmSummaryAck acknowledgedTransitions must contain three bits with zero padding",
                ));
            }
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

    fn ack_with_fields(state: &[u8], unused_bits: u8, transitions: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        let object_identifier = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        primitives::encode_app_object_id(&mut buf, &object_identifier);
        tags::encode_tag(
            &mut buf,
            app_tag::ENUMERATED,
            TagClass::Application,
            state.len() as u32,
        );
        buf.extend_from_slice(state);
        primitives::encode_app_bit_string(&mut buf, unused_bits, transitions);
        buf
    }

    fn ack_with_alarm_state(state: &[u8]) -> BytesMut {
        ack_with_fields(state, 5, &[0b10100000])
    }

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

    #[test]
    fn alarm_state_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];
        let decoded =
            GetAlarmSummaryAck::decode(&ack_with_alarm_state(&max_with_leading_zero)).unwrap();
        assert_eq!(decoded.entries[0].alarm_state.to_raw(), u32::MAX);

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            assert!(
                GetAlarmSummaryAck::decode(&ack_with_alarm_state(&overflow.to_be_bytes())).is_err()
            );
        }
    }

    #[test]
    fn ack_requires_application_field_tags() {
        let encoded = ack_with_alarm_state(&[0]);
        let (object_tag, object_pos) = tags::decode_tag(&encoded, 0).unwrap();
        let state_offset = object_pos + object_tag.length as usize;
        let (state_tag, state_pos) = tags::decode_tag(&encoded, state_offset).unwrap();
        let transitions_offset = state_pos + state_tag.length as usize;

        let mut wrong_object = encoded.clone();
        wrong_object[0] = (app_tag::OCTET_STRING << 4) | (wrong_object[0] & 0x0F);
        assert!(GetAlarmSummaryAck::decode(&wrong_object).is_err());

        let mut wrong_state = encoded.clone();
        wrong_state[state_offset] = (app_tag::UNSIGNED << 4) | (wrong_state[state_offset] & 0x0F);
        assert!(GetAlarmSummaryAck::decode(&wrong_state).is_err());

        let mut wrong_transitions = encoded.clone();
        wrong_transitions[transitions_offset] =
            (app_tag::OCTET_STRING << 4) | (wrong_transitions[transitions_offset] & 0x0F);
        assert!(GetAlarmSummaryAck::decode(&wrong_transitions).is_err());

        let mut trailing = encoded;
        primitives::encode_app_null(&mut trailing);
        assert!(GetAlarmSummaryAck::decode(&trailing).is_err());
    }

    #[test]
    fn ack_rejects_reserved_application_lvt_forms() {
        let encoded = ack_with_alarm_state(&[0]);
        let (object_tag, object_pos) = tags::decode_tag(&encoded, 0).unwrap();
        let state_offset = object_pos + object_tag.length as usize;
        let (state_tag, state_pos) = tags::decode_tag(&encoded, state_offset).unwrap();
        let transitions_offset = state_pos + state_tag.length as usize;

        for lvt in [6, 7] {
            for (offset, declared_length) in [(0, 4), (state_offset, 1), (transitions_offset, 2)] {
                let mut reserved = encoded.to_vec();
                reserved[offset] = (reserved[offset] & 0xF8) | lvt;
                reserved.insert(offset + 1, declared_length);
                assert!(GetAlarmSummaryAck::decode(&reserved).is_err());
            }
        }
    }

    #[test]
    fn acknowledged_transitions_must_contain_three_bits() {
        for (unused_bits, transitions) in [
            (5, &[][..]),
            (4, &[0][..]),
            (0, &[0][..]),
            (5, &[0, 0][..]),
            (5, &[0b00011111][..]),
        ] {
            assert!(
                GetAlarmSummaryAck::decode(&ack_with_fields(&[0], unused_bits, transitions,))
                    .is_err()
            );
        }
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
