use std::cell::RefCell;

use bacnet_types::enums::{EventState, EventType, NotifyType};
use bacnet_types::primitives::{Date, Time};
use serde_json::json;

use super::*;

/// The table cells name values as the shared formatter does (bits only
/// with commas for its bars), so the alarm table and `read` agree.
#[test]
fn cells_agree_with_the_formatter() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::DEVICE, &texts, Floats::Exact);
    let bits = |bits: EventTransitionBits| PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bits.to_bacnet()],
    };
    let cases = [
        (
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(ai(7)),
        ),
        (
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw()),
        ),
        (
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(600),
        ),
        (
            PropertyIdentifier::EVENT_TYPE,
            PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw()),
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(NotifyType::ACK_NOTIFICATION.to_raw()),
        ),
        (PropertyIdentifier::PRIORITY, PropertyValue::Unsigned(200)),
        (
            PropertyIdentifier::ACKED_TRANSITIONS,
            bits(EventTransitionBits::all()),
        ),
        (
            PropertyIdentifier::ACKED_TRANSITIONS,
            bits(EventTransitionBits::TO_FAULT),
        ),
        (
            PropertyIdentifier::EVENT_ENABLE,
            bits(EventTransitionBits::empty()),
        ),
    ];
    for (property, value) in cases {
        assert_eq!(
            cell(&format.element_data(property, &value)),
            format.element(property, &value).replace(" | ", ", "),
            "{property} {value:?}"
        );
    }
    let stamp = BACnetTimeStamp::SequenceNumber(9);
    assert_eq!(
        cell(&timestamp(&stamp)),
        constructed::timestamp_text(&stamp)
    );
    assert_eq!(cell(&Datum::Null), "-");
}

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn summary(object: ObjectIdentifier) -> EventSummary {
    EventSummary {
        object_identifier: object,
        event_state: EventState::HIGH_LIMIT,
        acknowledged_transitions: EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL,
        event_timestamps: [
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 10,
                    day: 25,
                    day_of_week: 7,
                },
                time: Time {
                    hour: 14,
                    minute: 30,
                    second: 5,
                    hundredths: 0,
                },
            },
            BACnetTimeStamp::SequenceNumber(42),
            BACnetTimeStamp::Time(Time {
                hour: 7,
                minute: 0,
                second: 0,
                hundredths: 0,
            }),
        ],
        notify_type: NotifyType::ALARM,
        event_enable: EventTransitionBits::all(),
        event_priorities: [3, 5, 200],
    }
}

#[test]
fn an_event_summary_is_named_in_json_and_cells() {
    let data = EventSummaryData::new(&summary(ai(1)));
    assert_eq!(
        serde_json::to_value(&data).unwrap(),
        json!({
            "object": {"type": "analog-input", "type_number": 0, "instance": 1},
            "event_state": {"value": 3, "name": "high-limit"},
            "acknowledged_transitions": {"bits": "011", "names": ["to-fault", "to-normal"]},
            "event_enable": {
                "bits": "111",
                "names": ["to-offnormal", "to-fault", "to-normal"]
            },
            "notify_type": {"value": 0, "name": "alarm"},
            "event_timestamps": {
                "to_offnormal": {"element": "time-stamp", "text": "2026-10-25 Sun 14:30:05"},
                "to_fault": {"element": "time-stamp", "text": "seq 42"},
                "to_normal": {"element": "time-stamp", "text": "07:00"}
            },
            "event_priorities": {"to_offnormal": 3, "to_fault": 5, "to_normal": 200}
        })
    );
    assert_eq!(
        data.cells(),
        [
            "analog-input 1",
            "high-limit",
            "to-fault, to-normal",
            "to-offnormal, to-fault, to-normal",
            "alarm",
            "2026-10-25 Sun 14:30:05",
            "seq 42",
            "07:00",
            "3/5/200",
        ]
    );
    assert_eq!(data.cells().len(), EventSummaryData::HEADERS.len());
}

#[test]
fn no_bits_set_and_unnamed_numbers_stay_readable() {
    let mut raw = summary(ai(2));
    raw.acknowledged_transitions = EventTransitionBits::empty();
    // A proprietary event state has no name; it shows as its number.
    raw.event_state = EventState::from_raw(600);
    let data = EventSummaryData::new(&raw);
    assert_eq!(
        serde_json::to_value(&data.acknowledged_transitions).unwrap(),
        json!({"bits": "000", "names": []})
    );
    assert_eq!(
        serde_json::to_value(&data.event_state).unwrap(),
        json!({"value": 600})
    );
    let cells = data.cells();
    assert_eq!(cells[1], "600");
    assert_eq!(cells[2], "none");
}

#[test]
fn an_answer_keeps_more_events() {
    let ack = GetEventInformationAck {
        list_of_event_summaries: vec![summary(ai(1)), summary(ai(2))],
        more_events: true,
    };
    let data = EventInformationData::new(&ack);
    assert!(data.more_events);
    assert_eq!(data.summaries.len(), 2);
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["more_events"], true);
    assert_eq!(json["summaries"][1]["object"]["instance"], 2);
}

#[test]
fn alarm_and_enrollment_entries_are_named() {
    let alarm = AlarmSummaryData::new(&AlarmSummaryEntry {
        object_identifier: ai(3),
        alarm_state: EventState::OFFNORMAL,
        acknowledged_transitions: EventTransitionBits::TO_NORMAL,
    });
    assert_eq!(
        serde_json::to_value(&alarm).unwrap(),
        json!({
            "object": {"type": "analog-input", "type_number": 0, "instance": 3},
            "alarm_state": {"value": 2, "name": "offnormal"},
            "acknowledged_transitions": {"bits": "001", "names": ["to-normal"]}
        })
    );
    assert_eq!(alarm.cells(), ["analog-input 3", "offnormal", "to-normal"]);

    let mut entry = EnrollmentSummaryEntry {
        object_identifier: ai(4),
        event_type: EventType::OUT_OF_RANGE,
        event_state: EventState::FAULT,
        priority: 7,
        notification_class: Some(2),
    };
    let enrollment = EnrollmentSummaryData::new(&entry);
    assert_eq!(
        serde_json::to_value(&enrollment).unwrap(),
        json!({
            "object": {"type": "analog-input", "type_number": 0, "instance": 4},
            "event_type": {"value": 5, "name": "out-of-range"},
            "event_state": {"value": 1, "name": "fault"},
            "priority": 7,
            "notification_class": 2
        })
    );
    assert_eq!(
        enrollment.cells(),
        ["analog-input 4", "out-of-range", "fault", "7", "2"]
    );
    entry.notification_class = None;
    let enrollment = EnrollmentSummaryData::new(&entry);
    assert_eq!(enrollment.notification_class, Datum::Null);
    assert_eq!(enrollment.cells()[4], "-");
}

/// A device answering from `pages`, each page the objects after the
/// cursor it was asked for; records the cursors asked for.
struct Pages {
    pages: Vec<(Vec<u32>, bool)>,
    asked: RefCell<Vec<Option<u32>>>,
}

impl Pages {
    fn new(pages: Vec<(Vec<u32>, bool)>) -> Self {
        Self {
            pages,
            asked: RefCell::new(Vec::new()),
        }
    }

    fn answer(
        &self,
        after: Option<ObjectIdentifier>,
    ) -> std::future::Ready<Result<GetEventInformationAck, Error>> {
        let mut asked = self.asked.borrow_mut();
        let (objects, more) = self.pages[asked.len()].clone();
        asked.push(after.map(|object| object.instance_number()));
        std::future::ready(Ok(GetEventInformationAck {
            list_of_event_summaries: objects.into_iter().map(|n| summary(ai(n))).collect(),
            more_events: more,
        }))
    }
}

fn instances(summaries: &[EventSummary]) -> Vec<u32> {
    summaries
        .iter()
        .map(|summary| summary.object_identifier.instance_number())
        .collect()
}

#[tokio::test]
async fn paging_follows_more_events_from_the_last_object() {
    let device = Pages::new(vec![
        (vec![1, 2], true),
        (vec![3, 4], true),
        (vec![5], false),
    ]);
    let (summaries, more) = read_event_information(100, |after| device.answer(after))
        .await
        .unwrap();
    assert_eq!(instances(&summaries), [1, 2, 3, 4, 5]);
    assert!(!more);
    assert_eq!(*device.asked.borrow(), [None, Some(2), Some(4)]);
}

/// A stop asked before each later answer ends the reading with what was
/// read and "more"; the first answer is always fetched.
#[tokio::test]
async fn paging_stops_when_asked_and_says_more_remain() {
    let device = Pages::new(vec![
        (vec![1, 2], true),
        (vec![3, 4], true),
        (vec![5], false),
    ]);
    let asked = std::cell::Cell::new(0);
    let stop = || {
        asked.set(asked.get() + 1);
        asked.get() > 1
    };
    let (summaries, more) = read_event_information_until(100, stop, |after| device.answer(after))
        .await
        .unwrap();
    assert_eq!(instances(&summaries), [1, 2, 3, 4]);
    assert!(more);
    assert_eq!(*device.asked.borrow(), [None, Some(2)]);
    let device = Pages::new(vec![(vec![1], true)]);
    let (summaries, more) =
        read_event_information_until(100, || true, |after| device.answer(after))
            .await
            .unwrap();
    assert_eq!(instances(&summaries), [1]);
    assert!(more);
}

#[tokio::test]
async fn paging_stops_at_the_cap_and_says_more_remain() {
    let device = Pages::new(vec![(vec![1, 2], true), (vec![3, 4], true)]);
    let (summaries, more) = read_event_information(3, |after| device.answer(after))
        .await
        .unwrap();
    assert_eq!(instances(&summaries), [1, 2, 3]);
    assert!(more);
    assert_eq!(device.asked.borrow().len(), 2);

    // A cap met exactly by a last page is not "more".
    let device = Pages::new(vec![(vec![1, 2], true), (vec![3], false)]);
    let (summaries, more) = read_event_information(3, |after| device.answer(after))
        .await
        .unwrap();
    assert_eq!(instances(&summaries), [1, 2, 3]);
    assert!(!more);
}

#[tokio::test]
async fn a_device_that_never_advances_is_an_error() {
    for pages in [vec![(vec![], true)], vec![(vec![1], true), (vec![], true)]] {
        let device = Pages::new(pages.clone());
        let error = read_event_information(100, |after| device.answer(after))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no new object"),
            "{pages:?}: {error}"
        );
    }
}

#[tokio::test]
async fn a_device_that_repeats_an_object_is_an_error() {
    for (pages, repeated) in [
        // The last object again.
        (vec![(vec![1], true), (vec![1], true)], 1),
        // A loop back to the start: A,B then C,D then A,B.
        (
            vec![(vec![1, 2], true), (vec![3, 4], true), (vec![1, 2], true)],
            1,
        ),
        // A repeat inside one answer.
        (vec![(vec![5, 5], false)], 5),
    ] {
        let device = Pages::new(pages.clone());
        let error = read_event_information(100, |after| device.answer(after))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains(&format!(
                "GetEventInformation named analog-input {repeated} again"
            )),
            "{pages:?}: {error}"
        );
    }
}
