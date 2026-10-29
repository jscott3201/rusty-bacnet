use super::*;

#[test]
fn filters_parse_by_name_in_either_spelling() {
    assert_eq!(
        parse_ack_filter("not-acked"),
        Ok(AcknowledgmentFilter::NOT_ACKED)
    );
    assert_eq!(parse_ack_filter("ALL"), Ok(AcknowledgmentFilter::ALL));
    assert_eq!(
        parse_state_filter("active"),
        Ok(EnrollmentSummaryEventStateFilter::ACTIVE)
    );
    assert_eq!(
        parse_event_type("change_of_value"),
        Ok(EventType::CHANGE_OF_VALUE)
    );
    let error = parse_ack_filter("some").unwrap_err();
    assert_eq!(
        error,
        "unknown acknowledgment filter 'some': expected one of all, acked, not-acked"
    );
}

#[test]
fn priorities_take_a_range_or_one_value() {
    let range = |min, max| {
        Ok(PriorityFilter {
            min_priority: min,
            max_priority: max,
        })
    };
    assert_eq!(parse_priority_range("3-200"), range(3, 200));
    assert_eq!(parse_priority_range("7"), range(7, 7));
    assert_eq!(parse_priority_range("0-255"), range(0, 255));
    assert!(parse_priority_range("256").is_err());
    assert!(
        parse_priority_range("9-3")
            .unwrap_err()
            .contains("backward")
    );
    assert!(parse_priority_range("-3").is_err());
}

#[test]
fn empty_answers_say_so() {
    let none = EventInformationData::from_summaries(&[], false);
    assert_eq!(
        event_information_text(&none, 10),
        "No active events or unacknowledged transitions."
    );
    let capped = EventInformationData::from_summaries(&[], true);
    assert_eq!(
        event_information_text(&capped, 10),
        "No active events or unacknowledged transitions.\n\
         The device holds more; stopped at --max 10."
    );
    assert_eq!(alarm_summary_text(&[]), "No objects in alarm.");
    assert_eq!(enrollment_summary_text(&[]), "No event enrollments match.");
}

#[test]
fn filters_become_the_request() {
    let filters = EnrollmentFilters {
        acknowledgment: AcknowledgmentFilter::ACKED,
        event_state: Some(EnrollmentSummaryEventStateFilter::FAULT),
        event_type: Some(EventType::OUT_OF_RANGE),
        priority: Some(PriorityFilter {
            min_priority: 1,
            max_priority: 9,
        }),
        notification_class: Some(4),
    };
    assert_eq!(
        filters.request(),
        GetEnrollmentSummaryRequest {
            acknowledgment_filter: AcknowledgmentFilter::ACKED,
            enrollment_filter: None,
            event_state_filter: Some(EnrollmentSummaryEventStateFilter::FAULT),
            event_type_filter: Some(EventType::OUT_OF_RANGE),
            priority_filter: Some(PriorityFilter {
                min_priority: 1,
                max_priority: 9,
            }),
            notification_class_filter: Some(4),
        }
    );
}
