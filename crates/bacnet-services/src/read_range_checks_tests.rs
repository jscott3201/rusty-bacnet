use super::*;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use ReadRangeViolation as V;

fn request(range: Option<RangeSpec>) -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: Some(1),
        range,
    }
}

fn ack(request: &ReadRangeRequest, item_count: u32, first: Option<u64>) -> ReadRangeAck {
    ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: (true, true, false),
        item_count,
        item_data: Vec::new(),
        first_sequence_number: first,
    }
}

fn by_position(count: i32) -> ReadRangeRequest {
    request(Some(RangeSpec::ByPosition {
        reference_index: 1,
        count,
    }))
}

fn by_sequence(count: i32) -> ReadRangeRequest {
    request(Some(RangeSpec::BySequenceNumber {
        reference_seq: 1,
        count,
    }))
}

fn by_time(count: i32) -> ReadRangeRequest {
    request(Some(RangeSpec::ByTime {
        reference_time: (
            Date {
                year: 126,
                month: 3,
                day: 1,
                day_of_week: 7,
            },
            Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        ),
        count,
    }))
}

#[test]
fn an_answer_must_echo_the_request() {
    let request = by_position(1);
    let valid = ack(&request, 1, None);
    assert!(valid.violations(&request).is_empty());

    let mut wrong_object = valid.clone();
    wrong_object.object_identifier = ObjectIdentifier::new(ObjectType::TREND_LOG, 2).unwrap();
    assert_eq!(wrong_object.violations(&request), [V::ObjectMismatch]);

    let mut wrong_property = valid.clone();
    wrong_property.property_identifier = PropertyIdentifier::PRESENT_VALUE;
    assert_eq!(wrong_property.violations(&request), [V::PropertyMismatch]);

    let mut wrong_index = valid.clone();
    wrong_index.property_array_index = Some(2);
    assert_eq!(wrong_index.violations(&request), [V::ArrayIndexMismatch]);
    wrong_index.property_array_index = None;
    assert_eq!(wrong_index.violations(&request), [V::ArrayIndexMismatch]);
}

#[test]
fn first_sequence_number_must_fit_the_requested_range() {
    for sequenced in [by_sequence(1), by_time(1)] {
        assert!(ack(&sequenced, 1, Some(1))
            .violations(&sequenced)
            .is_empty());
        assert!(ack(&sequenced, 0, None).violations(&sequenced).is_empty());
        assert_eq!(
            ack(&sequenced, 1, None).violations(&sequenced),
            [V::MissingFirstSequenceNumber]
        );
        assert_eq!(
            ack(&sequenced, 1, Some(0)).violations(&sequenced),
            [V::ZeroFirstSequenceNumber]
        );
        assert_eq!(
            ack(&sequenced, 0, Some(1)).violations(&sequenced),
            [V::UnexpectedFirstSequenceNumber]
        );
    }
    for unsequenced in [by_position(1), request(None)] {
        assert!(ack(&unsequenced, 1, None)
            .violations(&unsequenced)
            .is_empty());
        assert_eq!(
            ack(&unsequenced, 1, Some(1)).violations(&unsequenced),
            [V::UnexpectedFirstSequenceNumber]
        );
    }
}

#[test]
fn more_items_contradicts_the_flag_for_the_end_the_read_moves_toward() {
    let forward = by_position(10);
    let backward = by_position(-10);
    let flagged = |request: &ReadRangeRequest, flags| ReadRangeAck {
        result_flags: flags,
        ..ack(request, 1, None)
    };
    // The spec's own examples: a forward page cut short has neither end; a
    // backward one keeps the items nearest its reference.
    assert!(flagged(&forward, (false, false, true))
        .violations(&forward)
        .is_empty());
    assert!(flagged(&backward, (false, true, true))
        .violations(&backward)
        .is_empty());
    assert!(flagged(&forward, (true, false, true))
        .violations(&forward)
        .is_empty());
    assert_eq!(
        flagged(&forward, (false, true, true)).violations(&forward),
        [V::MoreItemsPastEnd]
    );
    assert_eq!(
        flagged(&backward, (true, false, true)).violations(&backward),
        [V::MoreItemsPastEnd]
    );
    // A read with no range may give up either end when the page is cut;
    // only a page holding both ends and still claiming more contradicts.
    let all = request(None);
    for flags in [
        (false, true, true),
        (true, false, true),
        (false, false, true),
    ] {
        assert!(
            flagged(&all, flags).violations(&all).is_empty(),
            "{flags:?}"
        );
    }
    assert_eq!(
        flagged(&all, (true, true, true)).violations(&all),
        [V::MoreItemsPastEnd]
    );
}

#[test]
fn an_answer_holds_no_more_items_than_the_count_asked_for() {
    for request in [by_position(-2), by_sequence(2)] {
        let first = matches!(request.range, Some(RangeSpec::BySequenceNumber { .. })).then_some(4);
        assert!(ack(&request, 2, first).violations(&request).is_empty());
        assert_eq!(
            ack(&request, 3, first).violations(&request),
            [V::ItemCountExceedsRequest]
        );
    }
    // A read with no range asks for every item.
    let all = request(None);
    assert!(ack(&all, u32::MAX, None).violations(&all).is_empty());
}

#[test]
fn every_rule_broken_is_listed_in_check_order() {
    let request = by_sequence(1);
    let mut broken = ack(&request, 2, Some(0));
    broken.object_identifier = ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap();
    broken.property_array_index = None;
    broken.result_flags = (false, true, true);
    assert_eq!(
        broken.violations(&request),
        [
            V::ObjectMismatch,
            V::ArrayIndexMismatch,
            V::ZeroFirstSequenceNumber,
            V::MoreItemsPastEnd,
            V::ItemCountExceedsRequest,
        ]
    );
}

#[test]
fn strict_refuses_with_the_first_rule_and_lenient_keeps_the_answer() {
    let request = by_sequence(10);
    let wrapped = ack(&request, 10, Some(0));

    let refused =
        ReadRangeReply::check(&request, wrapped.clone(), ReadRangeValidation::Strict).unwrap_err();
    assert!(matches!(
        refused,
        Error::ReadRangeViolation(V::ZeroFirstSequenceNumber)
    ));
    assert!(refused.to_string().contains("first sequence number 0"));

    let kept = ReadRangeReply::check(&request, wrapped, ReadRangeValidation::Lenient).unwrap();
    assert_eq!(kept.violations, [V::ZeroFirstSequenceNumber]);
    assert_eq!(kept.ack.first_sequence_number, Some(0));
    assert_eq!(kept.ack.item_count, 10);

    let clean = ack(&request, 10, Some(7));
    for validation in [ReadRangeValidation::Strict, ReadRangeValidation::Lenient] {
        let reply = ReadRangeReply::check(&request, clean.clone(), validation).unwrap();
        assert!(reply.violations.is_empty());
    }
    assert_eq!(ReadRangeValidation::default(), ReadRangeValidation::Strict);
}

#[test]
fn every_rule_has_a_distinct_name() {
    let all = [
        V::ObjectMismatch,
        V::PropertyMismatch,
        V::ArrayIndexMismatch,
        V::MissingFirstSequenceNumber,
        V::ZeroFirstSequenceNumber,
        V::UnexpectedFirstSequenceNumber,
        V::MoreItemsPastEnd,
        V::ItemCountExceedsRequest,
    ];
    let names: std::collections::HashSet<_> = all.iter().map(|rule| rule.name()).collect();
    assert_eq!(names.len(), all.len());
    assert_eq!(
        V::ZeroFirstSequenceNumber.name(),
        "zero_first_sequence_number"
    );
}
