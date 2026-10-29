//! Present_Value writes while Out_Of_Service is TRUE (#1055): refused in
//! service, accepted out of service with the time-value datatype check, owed
//! once to the references, left alone by the tick, and handed back to the
//! calculation on the return to service.

use super::*;
use bacnet_types::error::RemoteError;

type P = PropertyIdentifier;

/// Monday 14 September 2026.
fn monday() -> SpecificDate {
    SpecificDate::new(2026, 9, 14).unwrap()
}

fn at(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn target() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

/// 21.0 from 08:00 on Mondays, default 10.0, commanding AO-2 at priority 9.
fn schedule() -> ScheduleObject {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap();
    sched
        .set_weekly_schedule(
            0,
            vec![BACnetTimeValue {
                time: at(8),
                value: PropertyValue::Real(21.0),
            }],
        )
        .unwrap();
    sched.set_priority_for_writing(9).unwrap();
    sched.add_object_property_reference(target()).unwrap();
    sched
}

fn tick(sched: &mut ScheduleObject) -> Option<ScheduleWrite> {
    sched.tick_schedule(monday(), at(9), &|_| false)
}

/// The one write owed apart from the calculation, if any.
fn take(sched: &mut ScheduleObject) -> Option<ScheduleWrite> {
    let mut owed = sched.take_owed_schedule_writes();
    assert!(owed.len() <= 1, "{owed:?}");
    owed.pop()
}

fn set_out_of_service(sched: &mut ScheduleObject, out_of_service: bool) {
    sched
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn write_pv(sched: &mut ScheduleObject, value: PropertyValue) -> Result<(), Error> {
    sched.write_property(P::PRESENT_VALUE, None, value, None)
}

fn owed(value: PropertyValue) -> Option<ScheduleWrite> {
    Some(ScheduleWrite {
        value,
        priority: 9,
        references: vec![target()],
        retry: false,
    })
}

fn assert_code(result: Result<(), Error>, code: ErrorCode, what: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code: c })) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{what}");
            assert_eq!(c, code, "{what}: expected {code:?}");
        }
        other => panic!("{what}: expected {code:?}, got {other:?}"),
    }
}

#[test]
fn present_value_write_is_refused_in_service() {
    let mut sched = schedule();
    assert_eq!(tick(&mut sched), owed(PropertyValue::Real(21.0)));
    for value in [PropertyValue::Real(30.0), PropertyValue::Null] {
        assert_code(
            write_pv(&mut sched, value),
            ErrorCode::WRITE_ACCESS_DENIED,
            "in service",
        );
    }
    assert_eq!(*sched.present_value(), PropertyValue::Real(21.0));
    assert_eq!(take(&mut sched), None);
}

#[test]
fn present_value_written_out_of_service_is_owed_to_the_references_once() {
    let mut sched = schedule();
    tick(&mut sched);
    set_out_of_service(&mut sched, true);
    // Going out of service owes nothing by itself.
    assert_eq!(take(&mut sched), None);

    write_pv(&mut sched, PropertyValue::Real(30.0)).unwrap();
    assert_eq!(
        sched.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Real(30.0)
    );
    assert_eq!(take(&mut sched), owed(PropertyValue::Real(30.0)));
    assert_eq!(take(&mut sched), None);

    // The value already held is owed again; NULL relinquishes.
    write_pv(&mut sched, PropertyValue::Real(30.0)).unwrap();
    assert_eq!(take(&mut sched), owed(PropertyValue::Real(30.0)));
    write_pv(&mut sched, PropertyValue::Null).unwrap();
    assert_eq!(*sched.present_value(), PropertyValue::Null);
    assert_eq!(take(&mut sched), owed(PropertyValue::Null));

    // Two writes before the pass owe the last one.
    write_pv(&mut sched, PropertyValue::Real(1.0)).unwrap();
    write_pv(&mut sched, PropertyValue::Real(2.0)).unwrap();
    assert_eq!(take(&mut sched), owed(PropertyValue::Real(2.0)));

    // Without references Present_Value still changes, but nothing is owed.
    let mut bare = ScheduleObject::new(2, "SCHED-2", PropertyValue::Real(10.0)).unwrap();
    set_out_of_service(&mut bare, true);
    write_pv(&mut bare, PropertyValue::Real(5.0)).unwrap();
    assert_eq!(*bare.present_value(), PropertyValue::Real(5.0));
    assert_eq!(take(&mut bare), None);
}

#[test]
fn present_value_write_out_of_service_takes_only_a_primitive_value() {
    let mut sched = schedule();
    tick(&mut sched);
    set_out_of_service(&mut sched, true);
    let cases = [
        (
            None,
            PropertyValue::List(vec![PropertyValue::Real(1.0)]),
            ErrorCode::INVALID_DATA_TYPE,
            "a list",
        ),
        (
            None,
            PropertyValue::ApplicationData(vec![0x44, 0x41, 0xA8, 0, 0]),
            ErrorCode::INVALID_DATA_TYPE,
            "encoded bytes",
        ),
        (
            Some(1),
            PropertyValue::Real(1.0),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            "an array index",
        ),
    ];
    for (index, value, code, what) in cases {
        assert_code(
            sched.write_property(P::PRESENT_VALUE, index, value, None),
            code,
            what,
        );
    }
    assert_eq!(*sched.present_value(), PropertyValue::Real(21.0));
    assert_eq!(take(&mut sched), None);
    // Any primitive datatype, not only the schedule's own.
    for value in [
        PropertyValue::Boolean(true),
        PropertyValue::Unsigned(3),
        PropertyValue::CharacterString("on".into()),
    ] {
        write_pv(&mut sched, value.clone()).unwrap();
        assert_eq!(take(&mut sched), owed(value));
    }
}

#[test]
fn the_tick_leaves_a_simulated_value_until_the_return_to_service() {
    let mut sched = schedule();
    tick(&mut sched);
    set_out_of_service(&mut sched, true);
    write_pv(&mut sched, PropertyValue::Real(30.0)).unwrap();
    take(&mut sched);
    // Contents changed out of service don't reach Present_Value either.
    sched
        .write_property(P::SCHEDULE_DEFAULT, None, PropertyValue::Real(12.0), None)
        .unwrap();
    assert_eq!(tick(&mut sched), None);
    assert_eq!(*sched.present_value(), PropertyValue::Real(30.0));

    set_out_of_service(&mut sched, false);
    assert_eq!(tick(&mut sched), owed(PropertyValue::Real(21.0)));
    assert_eq!(*sched.present_value(), PropertyValue::Real(21.0));
}

#[test]
fn a_value_written_before_the_return_to_service_is_still_owed() {
    let mut sched = schedule();
    tick(&mut sched);
    set_out_of_service(&mut sched, true);
    write_pv(&mut sched, PropertyValue::Real(21.0)).unwrap();
    set_out_of_service(&mut sched, false);
    // The calculation agrees with the written value, so the tick owes
    // nothing: the written value alone brings the targets back to 21.0.
    assert_eq!(take(&mut sched), owed(PropertyValue::Real(21.0)));
    assert_eq!(tick(&mut sched), None);
}

#[test]
fn present_value_simulation_leaves_reliability_to_its_owners() {
    let mut sched = schedule();
    tick(&mut sched);
    set_out_of_service(&mut sched, true);
    sched
        .write_property(
            P::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw()),
            None,
        )
        .unwrap();
    // A simulated fault doesn't hold the write back, and a value of another
    // datatype than the schedule's doesn't touch the simulated Reliability.
    write_pv(&mut sched, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(take(&mut sched), owed(PropertyValue::Boolean(true)));
    assert_eq!(
        sched.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    // Back in service: the evaluated Reliability, which never counted
    // Present_Value, and the calculated value.
    set_out_of_service(&mut sched, false);
    assert_eq!(
        sched.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(tick(&mut sched), owed(PropertyValue::Real(21.0)));
}
