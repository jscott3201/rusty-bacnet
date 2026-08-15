use super::*;
use bacnet_encoding::tags::{self, TagClass};

const ONE: &[u8] = &[1];
const TRANSITIONS: &[u8] = &[5, 0xa0];
const U32_MAX_PADDED: &[u8] = &[0, 0xff, 0xff, 0xff, 0xff];
const U32_OVERFLOW: &[u8] = &[1, 0, 0, 0, 0];
const U64_MAX: &[u8] = &[0xff; 8];

#[derive(Clone, Copy)]
struct AckTags {
    list_open: u8,
    list_close: u8,
    object_identifier: u8,
    event_state: u8,
    acknowledged_transitions: u8,
    timestamps_open: u8,
    timestamps_close: u8,
    notify_type: u8,
    event_enable: u8,
    priorities_open: u8,
    priorities_close: u8,
    more_events: u8,
}

impl Default for AckTags {
    fn default() -> Self {
        Self {
            list_open: 0,
            list_close: 0,
            object_identifier: 0,
            event_state: 1,
            acknowledged_transitions: 2,
            timestamps_open: 3,
            timestamps_close: 3,
            notify_type: 4,
            event_enable: 5,
            priorities_open: 6,
            priorities_close: 6,
            more_events: 1,
        }
    }
}

struct SummaryWire<'a> {
    tags: AckTags,
    event_state: &'a [u8],
    acknowledged_transitions: &'a [u8],
    timestamp_count: usize,
    notify_type: &'a [u8],
    event_enable: &'a [u8],
    priorities: Vec<&'a [u8]>,
    priority_class: TagClass,
    priority_tag: u8,
}

fn valid_wire() -> SummaryWire<'static> {
    SummaryWire {
        tags: AckTags::default(),
        event_state: ONE,
        acknowledged_transitions: TRANSITIONS,
        timestamp_count: 3,
        notify_type: ONE,
        event_enable: TRANSITIONS,
        priorities: vec![ONE, ONE, ONE],
        priority_class: TagClass::Application,
        priority_tag: tags::app_tag::UNSIGNED,
    }
}

fn encode_value(buf: &mut BytesMut, class: TagClass, number: u8, content: &[u8]) {
    tags::encode_tag(buf, number, class, content.len() as u32);
    buf.extend_from_slice(content);
}

fn encode_summary(buf: &mut BytesMut, wire: &SummaryWire<'_>) {
    let object_identifier = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    primitives::encode_ctx_object_id(buf, wire.tags.object_identifier, &object_identifier);
    encode_value(
        buf,
        TagClass::Context,
        wire.tags.event_state,
        wire.event_state,
    );
    encode_value(
        buf,
        TagClass::Context,
        wire.tags.acknowledged_transitions,
        wire.acknowledged_transitions,
    );
    tags::encode_opening_tag(buf, wire.tags.timestamps_open);
    for value in 0..wire.timestamp_count {
        primitives::encode_timestamp_choice(buf, &BACnetTimeStamp::SequenceNumber(value as u64))
            .unwrap();
    }
    tags::encode_closing_tag(buf, wire.tags.timestamps_close);
    encode_value(
        buf,
        TagClass::Context,
        wire.tags.notify_type,
        wire.notify_type,
    );
    encode_value(
        buf,
        TagClass::Context,
        wire.tags.event_enable,
        wire.event_enable,
    );
    tags::encode_opening_tag(buf, wire.tags.priorities_open);
    for priority in &wire.priorities {
        encode_value(buf, wire.priority_class, wire.priority_tag, priority);
    }
    tags::encode_closing_tag(buf, wire.tags.priorities_close);
}

fn encode_ack(
    wire: &SummaryWire<'_>,
    summary_count: usize,
    more_events: &[u8],
    trailing: &[u8],
) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, wire.tags.list_open);
    for _ in 0..summary_count {
        encode_summary(&mut buf, wire);
    }
    tags::encode_closing_tag(&mut buf, wire.tags.list_close);
    encode_value(
        &mut buf,
        TagClass::Context,
        wire.tags.more_events,
        more_events,
    );
    buf.extend_from_slice(trailing);
    buf
}

#[test]
fn request_rejects_wrong_tag_and_trailing_data() {
    let object_identifier = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut wrong_tag = BytesMut::new();
    primitives::encode_ctx_object_id(&mut wrong_tag, 1, &object_identifier);
    assert!(GetEventInformationRequest::decode(&wrong_tag).is_err());

    let mut trailing = BytesMut::new();
    primitives::encode_ctx_object_id(&mut trailing, 0, &object_identifier);
    trailing.extend_from_slice(&[0]);
    assert!(GetEventInformationRequest::decode(&trailing).is_err());
}

#[test]
fn ack_accepts_u32_max_with_leading_zero_octet() {
    let mut wire = valid_wire();
    wire.event_state = U32_MAX_PADDED;
    wire.notify_type = U32_MAX_PADDED;
    wire.priorities = vec![U32_MAX_PADDED; 3];

    let decoded = GetEventInformationAck::decode(&encode_ack(&wire, 1, &[1], &[])).unwrap();
    let summary = &decoded.list_of_event_summaries[0];
    assert_eq!(summary.event_state, u32::MAX);
    assert_eq!(summary.notify_type, u32::MAX);
    assert_eq!(summary.event_priorities, [u32::MAX; 3]);
}

#[derive(Clone, Copy, Debug)]
enum NumericField {
    EventState,
    NotifyType,
    Priority(usize),
}

#[test]
fn ack_rejects_all_unsigned_values_above_u32() {
    let fields = [
        NumericField::EventState,
        NumericField::NotifyType,
        NumericField::Priority(0),
        NumericField::Priority(1),
        NumericField::Priority(2),
    ];
    for value in [U32_OVERFLOW, U64_MAX] {
        for field in fields {
            let mut wire = valid_wire();
            match field {
                NumericField::EventState => wire.event_state = value,
                NumericField::NotifyType => wire.notify_type = value,
                NumericField::Priority(index) => wire.priorities[index] = value,
            }
            assert!(
                GetEventInformationAck::decode(&encode_ack(&wire, 1, &[0], &[])).is_err(),
                "{field:?} accepted {value:?}"
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TagField {
    ListOpen,
    ListClose,
    ObjectIdentifier,
    EventState,
    AcknowledgedTransitions,
    TimestampsOpen,
    TimestampsClose,
    NotifyType,
    EventEnable,
    PrioritiesOpen,
    PrioritiesClose,
    MoreEvents,
}

#[test]
fn ack_rejects_wrong_context_tags() {
    let fields = [
        TagField::ListOpen,
        TagField::ListClose,
        TagField::ObjectIdentifier,
        TagField::EventState,
        TagField::AcknowledgedTransitions,
        TagField::TimestampsOpen,
        TagField::TimestampsClose,
        TagField::NotifyType,
        TagField::EventEnable,
        TagField::PrioritiesOpen,
        TagField::PrioritiesClose,
        TagField::MoreEvents,
    ];
    for field in fields {
        let mut wire = valid_wire();
        match field {
            TagField::ListOpen => wire.tags.list_open = 7,
            TagField::ListClose => wire.tags.list_close = 7,
            TagField::ObjectIdentifier => wire.tags.object_identifier = 7,
            TagField::EventState => wire.tags.event_state = 7,
            TagField::AcknowledgedTransitions => wire.tags.acknowledged_transitions = 7,
            TagField::TimestampsOpen => wire.tags.timestamps_open = 7,
            TagField::TimestampsClose => wire.tags.timestamps_close = 7,
            TagField::NotifyType => wire.tags.notify_type = 7,
            TagField::EventEnable => wire.tags.event_enable = 7,
            TagField::PrioritiesOpen => wire.tags.priorities_open = 7,
            TagField::PrioritiesClose => wire.tags.priorities_close = 7,
            TagField::MoreEvents => wire.tags.more_events = 7,
        }
        assert!(
            GetEventInformationAck::decode(&encode_ack(&wire, 1, &[0], &[])).is_err(),
            "{field:?} accepted the wrong tag"
        );
    }
}

#[test]
fn ack_requires_exact_timestamp_and_priority_counts() {
    for count in [2, 4] {
        let mut wire = valid_wire();
        wire.timestamp_count = count;
        assert!(GetEventInformationAck::decode(&encode_ack(&wire, 1, &[0], &[])).is_err());
    }
    for count in [2, 4] {
        let mut wire = valid_wire();
        wire.priorities = vec![ONE; count];
        assert!(GetEventInformationAck::decode(&encode_ack(&wire, 1, &[0], &[])).is_err());
    }
}

#[test]
fn ack_rejects_wrong_priority_datatype() {
    let mut wrong_application_tag = valid_wire();
    wrong_application_tag.priority_tag = tags::app_tag::ENUMERATED;
    assert!(
        GetEventInformationAck::decode(&encode_ack(&wrong_application_tag, 1, &[0], &[])).is_err()
    );

    let mut context_tag = valid_wire();
    context_tag.priority_class = TagClass::Context;
    assert!(GetEventInformationAck::decode(&encode_ack(&context_tag, 1, &[0], &[])).is_err());
}

#[test]
fn ack_requires_three_bit_transition_fields_with_zero_padding() {
    for malformed in [&[][..], &[5][..], &[4, 0][..], &[5, 1][..], &[5, 0, 0][..]] {
        let mut acknowledged = valid_wire();
        acknowledged.acknowledged_transitions = malformed;
        assert!(GetEventInformationAck::decode(&encode_ack(&acknowledged, 1, &[0], &[])).is_err());

        let mut enabled = valid_wire();
        enabled.event_enable = malformed;
        assert!(GetEventInformationAck::decode(&encode_ack(&enabled, 1, &[0], &[])).is_err());
    }
}

#[test]
fn ack_rejects_malformed_boolean_and_trailing_data() {
    let wire = valid_wire();
    for malformed in [&[][..], &[2][..], &[0, 0][..]] {
        assert!(GetEventInformationAck::decode(&encode_ack(&wire, 1, malformed, &[])).is_err());
    }
    assert!(GetEventInformationAck::decode(&encode_ack(&wire, 1, &[0], &[0])).is_err());
}

#[test]
fn ack_enforces_event_summary_limit() {
    let wire = valid_wire();
    let maximum = encode_ack(&wire, MAX_DECODED_ITEMS, &[0], &[]);
    assert_eq!(
        GetEventInformationAck::decode(&maximum)
            .unwrap()
            .list_of_event_summaries
            .len(),
        MAX_DECODED_ITEMS
    );

    let overflow = encode_ack(&wire, MAX_DECODED_ITEMS + 1, &[0], &[]);
    assert!(GetEventInformationAck::decode(&overflow).is_err());
}
