//! A pass with nothing else to send retries the references that refused the
//! Schedule's value (#1436), so a configuration fault can clear once its
//! cause is gone: the current value goes to those references alone, the slots
//! the Schedule holds stay as they were, a retry that fails otherwise ends the
//! refusal too, and a NULL value, Out_Of_Service and a day outside
//! Effective_Period retry nothing.

use super::*;
use bacnet_types::calendar::SpecificDate;

use ScheduleTargetOutcome::{Accepted, DatatypeRefused, Failed, ReferenceRefused};

/// Monday 14 September 2026.
fn monday() -> SpecificDate {
    SpecificDate::new(2026, 9, 14).unwrap()
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn reference(object_type: ObjectType, instance: u32) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(object_type, instance).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

fn av2() -> BACnetObjectPropertyReference {
    reference(ObjectType::ANALOG_VALUE, 2)
}

fn av9() -> BACnetObjectPropertyReference {
    reference(ObjectType::ANALOG_VALUE, 9)
}

fn ao4() -> BACnetObjectPropertyReference {
    reference(ObjectType::ANALOG_OUTPUT, 4)
}

fn tick(sched: &mut ScheduleObject, time: Time) -> Option<ScheduleWrite> {
    sched.tick_schedule(monday(), time, &|_| false)
}

fn faulted(sched: &ScheduleObject) -> bool {
    sched
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
        == PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
}

/// Default 10.0 at priority 9, commanding AV-2, AV-9 and AO-4, after the
/// first pass in the period: AV-9 refused it with `refusal`, the others took
/// it.
fn refused_by_av9(refusal: ScheduleTargetOutcome) -> ScheduleObject {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    sched.set_priority_for_writing(9).unwrap();
    sched
        .set_object_property_references(vec![av2(), av9(), ao4()])
        .unwrap();
    let write = tick(&mut sched, at(9, 0)).expect("the first pass in the period writes");
    assert!(!write.retry);
    assert_eq!(write.references, [av2(), av9(), ao4()]);
    assert!(sched.complete_schedule_write(&write, &[Accepted, refusal, Accepted]));
    assert!(faulted(&sched));
    sched
}

fn retry_to_av9() -> ScheduleWrite {
    ScheduleWrite {
        value: PropertyValue::Real(10.0),
        priority: 9,
        references: vec![av9()],
        retry: true,
        relinquish: false,
    }
}

#[test]
fn a_pass_with_nothing_else_to_send_retries_only_the_refused_reference() {
    let mut sched = refused_by_av9(ReferenceRefused);
    // Every pass offers the value to AV-9 again while it refuses.
    for minute in [1, 2] {
        let retry = tick(&mut sched, at(9, minute)).expect("a retry");
        assert_eq!(retry, retry_to_av9());
        assert!(!sched.complete_schedule_write(&retry, &[ReferenceRefused]));
        assert!(faulted(&sched), "AV-9 still refuses");
    }
    // AV-9 takes it, say once the object exists: the fault clears and the
    // retries stop.
    let retry = tick(&mut sched, at(9, 3)).expect("a retry");
    assert!(sched.complete_schedule_write(&retry, &[Accepted]));
    assert!(!faulted(&sched));
    assert_eq!(tick(&mut sched, at(9, 4)), None);
}

#[test]
fn a_datatype_refusal_is_retried_too() {
    let mut sched = refused_by_av9(DatatypeRefused);
    let retry = tick(&mut sched, at(9, 1)).expect("a retry");
    assert_eq!(retry, retry_to_av9());
    assert!(sched.complete_schedule_write(&retry, &[Accepted]));
    assert!(!faulted(&sched));
}

#[test]
fn a_retry_that_fails_otherwise_ends_the_refusal() {
    for refusal in [ReferenceRefused, DatatypeRefused] {
        let mut sched = refused_by_av9(refusal);
        // A write that isn't a retry and fails otherwise leaves the refusal.
        let write = ScheduleWrite {
            retry: false,
            relinquish: false,
            ..retry_to_av9()
        };
        assert!(!sched.complete_schedule_write(&write, &[Failed]));
        assert!(faulted(&sched), "{refusal:?}: not a retry");
        // A retry that fails otherwise, say out of range for the object
        // created since, ends it: AV-9 stands as if that were its first write.
        let retry = tick(&mut sched, at(9, 1)).expect("a retry");
        assert!(sched.complete_schedule_write(&retry, &[Failed]));
        assert!(!faulted(&sched), "{refusal:?}: the retry failed otherwise");
        assert_eq!(tick(&mut sched, at(9, 2)), None);
    }
}

#[test]
fn a_retry_leaves_the_held_slots_alone() {
    let mut sched = refused_by_av9(ReferenceRefused);
    let retry = tick(&mut sched, at(9, 1)).expect("a retry");
    sched.complete_schedule_write(&retry, &[Accepted]);
    // The Schedule still holds the slots its first write filled, so dropping
    // AV-2 and AO-4 relinquishes both, at the priority they were filled at.
    sched.set_object_property_references(vec![av9()]).unwrap();
    assert_eq!(
        sched.take_owed_schedule_writes(),
        [ScheduleWrite {
            value: PropertyValue::Null,
            priority: 9,
            references: vec![av2(), ao4()],
            retry: false,
            relinquish: true,
        }]
    );
}

#[test]
fn a_new_value_goes_to_the_whole_list_not_as_a_retry() {
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .set_weekly_schedule(
            0,
            vec![BACnetTimeValue {
                time: at(10, 0),
                value: PropertyValue::Real(20.0),
            }],
        )
        .unwrap();
    let write = tick(&mut sched, at(10, 0)).expect("a change writes");
    assert!(!write.retry);
    assert_eq!(write.value, PropertyValue::Real(20.0));
    assert_eq!(write.references, [av2(), av9(), ao4()]);
}

#[test]
fn nothing_is_retried_without_a_value_the_schedule_sends() {
    // A NULL can't clear a refusal, so a NULL Present_Value retries nothing.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();
    let relinquish = tick(&mut sched, at(9, 1)).expect("the change to NULL writes");
    assert!(!relinquish.retry);
    assert_eq!(relinquish.value, PropertyValue::Null);
    sched.complete_schedule_write(&relinquish, &[Accepted, ReferenceRefused, Accepted]);
    assert!(faulted(&sched), "a NULL counts for nothing");
    assert_eq!(tick(&mut sched, at(9, 2)), None);

    // Out of service the calculation sends nothing, retries included.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(tick(&mut sched, at(9, 1)), None);

    // Nor outside Effective_Period.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .set_effective_period(BACnetDateRange {
            start_date: SpecificDate::new(2026, 10, 1).unwrap().to_date(),
            end_date: unspecified_date(),
        })
        .unwrap();
    assert_eq!(tick(&mut sched, at(9, 1)), None);
    assert!(faulted(&sched));
}

#[test]
fn a_created_object_gets_a_retry_of_the_references_naming_it_alone() {
    // AV-9 and AO-4 both refused; creating AV-9 asks for AV-9's retry only,
    // with no pass run (#1440).
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    sched.set_priority_for_writing(9).unwrap();
    sched
        .set_object_property_references(vec![av2(), av9(), ao4()])
        .unwrap();
    let write = tick(&mut sched, at(9, 0)).expect("the first pass in the period writes");
    sched.complete_schedule_write(&write, &[Accepted, ReferenceRefused, DatatypeRefused]);
    let av9_id = av9().object_identifier;
    assert_eq!(sched.retry_refusals_naming(av9_id), Some(retry_to_av9()));
    let retry = ScheduleWrite {
        references: vec![ao4()],
        ..retry_to_av9()
    };
    assert_eq!(
        sched.retry_refusals_naming(ao4().object_identifier),
        Some(retry)
    );
    // AV-2 took the value, and an object no reference names has no refusal.
    assert_eq!(sched.retry_refusals_naming(av2().object_identifier), None);
    let elsewhere = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 10).unwrap();
    assert_eq!(sched.retry_refusals_naming(elsewhere), None);
    // Once AV-9 takes it, nothing is owed for AV-9.
    sched.complete_schedule_write(&retry_to_av9(), &[Accepted]);
    assert_eq!(sched.retry_refusals_naming(av9_id), None);
}

#[test]
fn a_created_object_gets_no_retry_where_a_pass_would_send_none() {
    let av9_id = av9().object_identifier;
    // Out of service.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(sched.retry_refusals_naming(av9_id), None);
    // Outside Effective_Period, as the last pass found it.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .set_effective_period(BACnetDateRange {
            start_date: SpecificDate::new(2026, 10, 1).unwrap().to_date(),
            end_date: unspecified_date(),
        })
        .unwrap();
    assert_eq!(tick(&mut sched, at(9, 1)), None);
    assert_eq!(sched.retry_refusals_naming(av9_id), None);
    // A NULL Present_Value can't clear a refusal.
    let mut sched = refused_by_av9(ReferenceRefused);
    sched
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();
    let relinquish = tick(&mut sched, at(9, 1)).expect("the change to NULL writes");
    sched.complete_schedule_write(&relinquish, &[Accepted, ReferenceRefused, Accepted]);
    assert_eq!(sched.retry_refusals_naming(av9_id), None);
}
