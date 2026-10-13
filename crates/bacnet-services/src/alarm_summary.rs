//! GetAlarmSummary service per ASHRAE 135-2020 Clause 13.10 (deprecated).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_app_bit_string, decode_app_enumerated, decode_app_object_id,
};
use bacnet_encoding::primitives;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::EventState;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// GetAlarmSummaryAck
// ---------------------------------------------------------------------------

/// One entry in the GetAlarmSummary-ACK sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmSummaryEntry {
    /// Object that is in an alarm condition.
    pub object_identifier: ObjectIdentifier,
    /// Event state the object currently holds.
    pub alarm_state: EventState,
    /// The object's `Acked_Transitions`.
    pub acknowledged_transitions: EventTransitionBits,
}

/// GetAlarmSummary-ACK: a sequence of alarm summary entries.
///
/// GetAlarmSummary-Request has no parameters so no struct is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetAlarmSummaryAck {
    /// Summary entries, one per alarming object.
    pub entries: Vec<AlarmSummaryEntry>,
}

impl GetAlarmSummaryAck {
    /// Append the ASN.1 encoding of the ACK to `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        for entry in &self.entries {
            primitives::encode_app_object_id(buf, &entry.object_identifier);
            primitives::encode_app_enumerated(buf, entry.alarm_state.to_raw());
            primitives::encode_app_bit_string(
                buf,
                5,
                &[entry.acknowledged_transitions.to_bacnet()],
            );
        }
    }

    /// Decode the ACK from `data`; errors on malformed input or too many entries.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset < data.len() {
            if entries.len() >= MAX_DECODED_ITEMS {
                return Err(Error::overflow(offset, "AlarmSummaryAck too many entries"));
            }

            // objectIdentifier, alarmState, acknowledgedTransitions (all
            // application-tagged)
            let (object_identifier, end) =
                decode_app_object_id(data, offset, "AlarmSummaryAck object-id")?;
            let (alarm_state, end) =
                decode_app_enumerated::<u32>(data, end, "AlarmSummaryAck alarmState")?;
            let alarm_state = EventState::from_raw(alarm_state);
            let transitions_at = end;
            let ((unused_bits, transitions), end) =
                decode_app_bit_string(data, end, "AlarmSummaryAck acknowledgedTransitions")?;
            if unused_bits != 5 || transitions.len() != 1 || transitions[0] & 0x1F != 0 {
                return Err(Error::decoding(
                    transitions_at,
                    "AlarmSummaryAck acknowledgedTransitions must contain three bits with zero padding",
                ));
            }
            let acknowledged_transitions = EventTransitionBits::from_bacnet(&transitions);
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
    use bacnet_encoding::tags::{self, app_tag, TagClass};
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
                    acknowledged_transitions: EventTransitionBits::TO_OFFNORMAL
                        | EventTransitionBits::TO_NORMAL,
                },
                AlarmSummaryEntry {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 10).unwrap(),
                    alarm_state: EventState::OFFNORMAL,
                    acknowledged_transitions: EventTransitionBits::all(),
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
                acknowledged_transitions: EventTransitionBits::TO_FAULT,
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
    fn acknowledged_transitions_keep_wire_bit_order() {
        // TO_OFFNORMAL rides the most significant bit of the content octet, then
        // TO_FAULT. The vector is asymmetric, so reversing the order fails it.
        let encoded = ack_with_fields(&[0], 5, &[0b1100_0000]);
        let decoded = GetAlarmSummaryAck::decode(&encoded).unwrap();
        assert_eq!(
            decoded.entries[0].acknowledged_transitions,
            EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_FAULT
        );
        let mut reencoded = BytesMut::new();
        decoded.encode(&mut reencoded);
        assert_eq!(reencoded, encoded);
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
                acknowledged_transitions: EventTransitionBits::TO_OFFNORMAL
                    | EventTransitionBits::TO_NORMAL,
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
                acknowledged_transitions: EventTransitionBits::TO_OFFNORMAL
                    | EventTransitionBits::TO_NORMAL,
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
