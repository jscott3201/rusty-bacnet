//! GetEventInformation-ACK `eventTimeStamps` conformance tests (#259).
//!
//! The ACK's `eventTimeStamps [3] SEQUENCE OF BACnetTimeStamp` items are
//! bare CHOICE elements and must use the single shared primitives codec —
//! `time [0]` as a primitive context tag 0 (length 4, raw Time octets),
//! `sequence-number [1]` constrained to `0..=65535`, `datetime [2]` as an
//! opening/closing tag 2 pair around the application-tagged Date and Time
//! (ASHRAE 135-2020 Clauses 13.12, 20.2.1.5, 21).

use super::*;

fn ts_time() -> BACnetTimeStamp {
    BACnetTimeStamp::Time(Time {
        hour: 14,
        minute: 30,
        second: 45,
        hundredths: 50,
    })
}

fn ts_datetime() -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: 126, // 2026
            month: 2,
            day: 28,
            day_of_week: 6,
        },
        time: Time {
            hour: 10,
            minute: 15,
            second: 0,
            hundredths: 0,
        },
    }
}

fn ack_with_timestamps(event_timestamps: [BACnetTimeStamp; 3]) -> GetEventInformationAck {
    GetEventInformationAck {
        list_of_event_summaries: vec![EventSummary {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state: 3,
            acknowledged_transitions: 0b101,
            event_timestamps,
            notify_type: 0,
            event_enable: 0b111,
            event_priorities: [3, 3, 3],
            notification_class: 0,
        }],
        more_events: true,
    }
}

#[test]
fn ack_timestamps_all_three_alternatives_golden() {
    let ack = ack_with_timestamps([
        ts_time(),
        BACnetTimeStamp::SequenceNumber(42),
        ts_datetime(),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x0E, // [0] listOfEventSummaries opening
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] objectIdentifier ctx tag 0
            0x19, 0x03, // [1] eventState ctx tag 1
            0x2A, 0x05, 0xA0, // [2] acknowledgedTransitions ctx tag 2 (0b101 MSB-first)
            0x3E, // [3] eventTimeStamps opening
            0x0C, 0x0E, 0x1E, 0x2D, 0x32, // time [0]: PRIMITIVE ctx tag 0, raw Time octets
            0x19, 0x2A, // sequence-number [1] = 42
            0x2E, // datetime [2] opening
            0xA4, 0x7E, 0x02, 0x1C, 0x06, // application Date 2026-02-28 (dow 6)
            0xB4, 0x0A, 0x0F, 0x00, 0x00, // application Time 10:15:00.00
            0x2F, // datetime [2] closing
            0x3F, // [3] eventTimeStamps closing
            0x49, 0x00, // [4] notifyType ctx tag 4
            0x5A, 0x05, 0xE0, // [5] eventEnable ctx tag 5 (0b111 MSB-first)
            0x6E, // [6] eventPriorities opening
            0x21, 0x03, 0x21, 0x03, 0x21, 0x03, // three application Unsigned values
            0x6F, // [6] eventPriorities closing
            0x0F, // [0] listOfEventSummaries closing
            0x19, 0x01, // [1] moreEvents TRUE
        ]
    );
    let decoded = GetEventInformationAck::decode(&buf).unwrap();
    let s = &decoded.list_of_event_summaries[0];
    assert_eq!(s.event_timestamps[0], ts_time());
    assert_eq!(s.event_timestamps[1], BACnetTimeStamp::SequenceNumber(42));
    assert_eq!(s.event_timestamps[2], ts_datetime());
    assert!(decoded.more_events);
}

#[test]
fn ack_timestamps_round_trip_each_alternative() {
    // One summary per alternative, each round-trips through the ACK codec.
    for ts in [ts_time(), BACnetTimeStamp::SequenceNumber(0), ts_datetime()] {
        let ack = ack_with_timestamps([ts.clone(), ts.clone(), ts]);
        let mut buf = BytesMut::new();
        ack.encode(&mut buf).unwrap();
        let decoded = GetEventInformationAck::decode(&buf).unwrap();
        assert_eq!(
            decoded.list_of_event_summaries[0].event_timestamps,
            ack.list_of_event_summaries[0].event_timestamps
        );
    }
}

#[test]
fn ack_timestamps_cross_codec_matrix_primitives_into_ack() {
    // Bytes produced by the shared primitives codec for a SEQUENCE OF
    // BACnetTimeStamp must decode through the GetEventInformationAck path:
    // splice open-3 + encode_timestamp_choice items + close-3 into a live ACK.
    let stamps = [ts_time(), BACnetTimeStamp::SequenceNumber(7), ts_datetime()];
    let mut stamp_bytes = BytesMut::new();
    bacnet_encoding::tags::encode_opening_tag(&mut stamp_bytes, 3);
    for ts in &stamps {
        bacnet_encoding::primitives::encode_timestamp_choice(&mut stamp_bytes, ts).unwrap();
    }
    bacnet_encoding::tags::encode_closing_tag(&mut stamp_bytes, 3);

    // Locate the ACK's [3] section and graft the primitives-produced bytes in.
    let ack = ack_with_timestamps([
        BACnetTimeStamp::SequenceNumber(1),
        BACnetTimeStamp::SequenceNumber(2),
        BACnetTimeStamp::SequenceNumber(3),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    let start = buf
        .windows(1)
        .position(|w| w == [0x3E])
        .expect("ACK contains [3] opening");
    let end = buf
        .windows(1)
        .rposition(|w| w == [0x3F])
        .expect("ACK contains [3] closing");
    let mut spliced = Vec::from(&buf[..start]);
    spliced.extend_from_slice(&stamp_bytes);
    spliced.extend_from_slice(&buf[end + 1..]);

    let decoded = GetEventInformationAck::decode(&spliced).unwrap();
    assert_eq!(decoded.list_of_event_summaries[0].event_timestamps, stamps);
}

#[test]
fn ack_timestamps_cross_codec_matrix_ack_into_primitives() {
    // Bytes produced by the ACK path must decode through the bare primitives
    // codec at the same offsets, element by element.
    let stamps = [
        ts_time(),
        BACnetTimeStamp::SequenceNumber(42),
        ts_datetime(),
    ];
    let ack = ack_with_timestamps(stamps.clone());
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();

    let mut offset = buf
        .windows(1)
        .position(|w| w == [0x3E])
        .expect("ACK contains [3] opening")
        + 1;
    for expected in &stamps {
        let (decoded, new_offset) =
            bacnet_encoding::primitives::decode_timestamp_choice(&buf, offset).unwrap();
        assert_eq!(&decoded, expected);
        offset = new_offset;
    }
    assert_eq!(buf[offset], 0x3F, "primitive scans land on [3] closing");
}

#[test]
fn ack_timestamps_non_conformant_time_form_rejected() {
    // The legacy ACK-only encoding of time [0] — opening tag 0 / application
    // Time / closing tag 0 — is not the conformant primitive ctx tag 0 and
    // must now be rejected by the shared codec.
    let ack = ack_with_timestamps([
        BACnetTimeStamp::SequenceNumber(1),
        BACnetTimeStamp::SequenceNumber(2),
        BACnetTimeStamp::SequenceNumber(3),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    let start = buf.windows(1).position(|w| w == [0x3E]).unwrap() + 1;
    let end = buf.windows(1).rposition(|w| w == [0x3F]).unwrap();
    let mut spliced = Vec::from(&buf[..start]);
    spliced.extend_from_slice(&[0x0E, 0xB4, 14, 30, 45, 50, 0x0F]); // old form
    spliced.extend_from_slice(&buf[start..end]); // remaining two seq-number stamps
    spliced.extend_from_slice(&buf[end..]);
    assert!(
        GetEventInformationAck::decode(&spliced).is_err(),
        "opening-tag-0 time form must be rejected"
    );
}

#[test]
fn ack_timestamps_wrong_section_tag_rejected() {
    // Craft an ACK whose timestamps field is filed under opening tag [4].
    let ack = ack_with_timestamps([
        BACnetTimeStamp::SequenceNumber(1),
        BACnetTimeStamp::SequenceNumber(2),
        BACnetTimeStamp::SequenceNumber(3),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    let pos = buf.windows(1).position(|w| w == [0x3E]).unwrap();
    let mut wrong = buf.to_vec();
    wrong[pos] = 0x4E; // opening tag 4 instead of 3
    assert!(GetEventInformationAck::decode(&wrong).is_err());
}

#[test]
fn ack_decode_truncated_in_timestamps_rejected() {
    let ack = ack_with_timestamps([
        BACnetTimeStamp::SequenceNumber(42),
        BACnetTimeStamp::SequenceNumber(43),
        BACnetTimeStamp::SequenceNumber(44),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    for cut in 1..buf.len() {
        assert!(
            GetEventInformationAck::decode(&buf[..cut]).is_err(),
            "truncated at {cut} bytes must fail"
        );
    }
}

#[test]
fn ack_timestamps_sequence_number_over_65535_rejected() {
    // A peer emitting sequence-number 65536 (3 content octets) violates
    // Unsigned (0..65535) — decode must refuse it.
    let ack = ack_with_timestamps([
        BACnetTimeStamp::SequenceNumber(1),
        BACnetTimeStamp::SequenceNumber(2),
        BACnetTimeStamp::SequenceNumber(3),
    ]);
    let mut buf = BytesMut::new();
    ack.encode(&mut buf).unwrap();
    let start = buf.windows(1).position(|w| w == [0x3E]).unwrap() + 1;
    let end = buf.windows(1).rposition(|w| w == [0x3F]).unwrap();
    let mut spliced = Vec::from(&buf[..start]);
    spliced.extend_from_slice(&[0x1B, 0x01, 0x00, 0x00]); // seq-num 65536
    spliced.extend_from_slice(&buf[start + 2..end]); // drop the first (1-octet) stamp
    spliced.extend_from_slice(&buf[end..]);
    assert!(GetEventInformationAck::decode(&spliced).is_err());
}
