use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::{AuthenticationFactorType, PropertyIdentifier as P};
use bacnet_types::primitives::Time;

const FIELDS: [P; 5] = [
    P::ACCESS_EVENT,
    P::ACCESS_EVENT_TAG,
    P::ACCESS_EVENT_TIME,
    P::ACCESS_EVENT_CREDENTIAL,
    P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
];

fn fields(object: &dyn BACnetObject) -> Vec<PropertyValue> {
    FIELDS
        .map(|p| object.read_property(p, None).unwrap())
        .to_vec()
}

fn report(event: AccessEvent) -> AccessEventReport {
    AccessEventReport {
        credential: Some(
            ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1)
                .unwrap()
                .into(),
        ),
        ..AccessEventReport::new(event, 42)
    }
}

struct FixedClock(ClockFrame);
impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

fn frame(year: u16, month: u8, day: u8, hundredths: u8) -> ClockFrame {
    ClockFrame {
        local_date: SpecificDate::new(year, month, day).unwrap().to_date(),
        local_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

#[test]
fn native_event_batch_refuses_invalid_later_record_without_installing_prefix() {
    let mut point = AccessPointObject::new(1, "point").unwrap();
    point
        .set_access_event(report(AccessEvent::GRANTED))
        .unwrap();
    let original = fields(&point);
    let mut bad_reference = report(AccessEvent::DENIED_OTHER);
    bad_reference.credential = Some(
        ObjectIdentifier::new(ObjectType::ACCESS_USER, 1)
            .unwrap()
            .into(),
    );
    let mut bad_factor = report(AccessEvent::DENIED_OTHER);
    bad_factor.authentication_factor = Some(BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::from_raw(999),
        format_class: 0,
        value: vec![1],
    });
    let mut duplicate_stamp = report(AccessEvent::DENIED_OTHER);
    duplicate_stamp.time = Some(point.access_event_time.clone());
    for bad in [
        bad_reference,
        bad_factor,
        report(AccessEvent::from_raw(u32::MAX)),
        duplicate_stamp,
    ] {
        assert!(
            point
                .record_event_batch(&[report(AccessEvent::GRANTED), bad])
                .is_err()
        );
        assert_eq!(fields(&point), original);
    }
    let accepted = point
        .record_event_batch(&[report(AccessEvent::GRANTED), report(AccessEvent::GRANTED)])
        .unwrap();
    assert_eq!(accepted.len(), 2);
    assert_ne!(accepted[0].report.time, accepted[1].report.time);
    assert_eq!(fields(accepted[1].snapshot.as_ref()), fields(&point));
    let retained = fields(accepted[0].snapshot.as_ref());
    point
        .set_access_event(report(AccessEvent::DENIED_OTHER))
        .unwrap();
    assert_eq!(fields(accepted[0].snapshot.as_ref()), retained);
    assert_ne!(retained, fields(&point));
}

#[test]
fn native_event_batch_clockless_sequence_and_wrap_keep_same_code_triggers_distinct() {
    for initial in [0, 65534, 65535] {
        let mut point = AccessPointObject::new(1, "point").unwrap();
        point.access_event_time = BACnetTimeStamp::SequenceNumber(initial);
        let events = point
            .record_event_batch(&vec![report(AccessEvent::GRANTED); 4])
            .unwrap();
        let mut previous = initial;
        for event in events {
            let expected = if previous == 65535 { 1 } else { previous + 1 };
            assert_eq!(
                event.report.time,
                Some(BACnetTimeStamp::SequenceNumber(expected))
            );
            assert!(event.clock_frame.is_none());
            previous = expected;
        }
    }
}

#[test]
fn native_event_batch_equal_backward_clock_and_maximum_are_preflighted() {
    for clock_hundredths in [10, 20] {
        let mut point = AccessPointObject::new(1, "point").unwrap();
        let initial = frame(2026, 10, 23, 20);
        point.access_event_time = BACnetTimeStamp::DateTime {
            date: initial.local_date,
            time: initial.local_time,
        };
        point.clock = Some(Arc::new(FixedClock(frame(2026, 10, 23, clock_hundredths))));
        let events = point
            .record_event_batch(&vec![report(AccessEvent::GRANTED); 4])
            .unwrap();
        for (index, event) in events.iter().enumerate() {
            let mut expected = initial.local_time;
            expected.hundredths += index as u8 + 1;
            assert_eq!(
                event.report.time,
                Some(BACnetTimeStamp::DateTime {
                    date: initial.local_date,
                    time: expected
                })
            );
        }
    }
    let mut point = AccessPointObject::new(1, "point").unwrap();
    let end = frame(2154, 12, 31, 98);
    point.access_event_time = BACnetTimeStamp::DateTime {
        date: end.local_date,
        time: end.local_time,
    };
    point.clock = Some(Arc::new(FixedClock(end)));
    let original = fields(&point);
    assert!(
        point
            .record_event_batch(&[report(AccessEvent::GRANTED), report(AccessEvent::GRANTED)])
            .is_err()
    );
    assert_eq!(fields(&point), original);
    let final_event = point
        .record_event_batch(&[report(AccessEvent::GRANTED)])
        .unwrap();
    assert_eq!(final_event.len(), 1);
    let final_fields = fields(&point);
    assert!(
        point
            .record_event_batch(&[report(AccessEvent::GRANTED)])
            .is_err()
    );
    assert_eq!(fields(&point), final_fields);
}

#[test]
fn native_event_batch_refuses_unhealthy_or_oversized_admission_atomically() {
    for failure in 0..3 {
        let mut point = AccessPointObject::new(1, "point").unwrap();
        if failure == 0 {
            point.out_of_service = true;
        } else if failure == 1 {
            point.reliability = Reliability::CONFIGURATION_ERROR;
        }
        let before = fields(&point);
        let reports = vec![report(AccessEvent::GRANTED); if failure == 2 { 5 } else { 2 }];
        assert!(point.record_event_batch(&reports).is_err());
        assert_eq!(fields(&point), before);
        point.out_of_service = false;
        point.reliability = Reliability::NO_FAULT_DETECTED;
        assert_eq!(point.record_event_batch(&reports[..1]).unwrap().len(), 1);
    }
}
