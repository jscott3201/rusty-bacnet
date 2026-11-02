use super::*;
use bacnet_objects::schedule::{
    CalendarObject, ScheduleObject, ScheduleTargetOutcome, ScheduleWrite,
};
use bacnet_objects::traits::ScheduleRunner;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetObjectPropertyReference};
use bacnet_types::enums::Reliability;
use bacnet_types::primitives::Time;

/// The wrapped Schedule's runner, through the wrapper's accessor.
fn runner(object: &mut Box<dyn BACnetObject>) -> &mut dyn ScheduleRunner {
    object
        .schedule_runner()
        .expect("the wrapper lends the Schedule's runner")
}

#[test]
fn source_reporter_forwards_complete_schedule_targets() {
    let refs = vec![
        BACnetObjectPropertyReference::new(oid(ObjectType::ANALOG_OUTPUT, 2), 85),
        BACnetObjectPropertyReference::new_indexed(oid(ObjectType::MULTI_STATE_OUTPUT, 7), 110, 2),
    ];
    let mut schedule =
        ScheduleObject::new(3, "wrapped schedule", PropertyValue::Unsigned(1)).unwrap();
    for reference in &refs {
        schedule
            .add_object_property_reference(reference.clone())
            .unwrap();
    }
    schedule
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    let mut object: Box<dyn BACnetObject> = Box::new(schedule);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    // Monday 14 September 2026, noon.
    let today = SpecificDate::new(2026, 9, 14).unwrap();
    let noon = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let no_calendars = |_: ObjectIdentifier| false;
    assert_eq!(
        runner(&mut object).tick_schedule(today, noon, &no_calendars),
        Some(ScheduleWrite::new(
            PropertyValue::Unsigned(2),
            16,
            refs.clone()
        ))
    );
    assert!(
        runner(&mut object)
            .tick_schedule(today, noon, &no_calendars)
            .is_none()
    );

    // A Present_Value written out of service is owed through the wrapper.
    for (property, value) in [
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Unsigned(5),
        ),
    ] {
        object.write_property(property, None, value, None).unwrap();
    }
    let simulated = ScheduleWrite::new(PropertyValue::Unsigned(5), 16, refs.clone());
    assert_eq!(runner(&mut object).take_owed_schedule_writes(), [simulated]);
    assert!(runner(&mut object).take_owed_schedule_writes().is_empty());

    // A dropped reference is relinquished through the wrapper, and a target
    // refusing the schedule's datatype faults it (#1088, #1086).
    // AO-2 Present_Value: object [0], property [1].
    let kept = vec![0x0C, 0x00, 0x40, 0x00, 0x02, 0x19, 85];
    object
        .write_property(
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            PropertyValue::ApplicationData(kept),
            None,
        )
        .unwrap();
    assert_eq!(
        runner(&mut object).take_owed_schedule_writes(),
        [
            ScheduleWrite::new(PropertyValue::Null, 16, vec![refs[1].clone()])
                .with_relinquish(true),
            ScheduleWrite::new(PropertyValue::Unsigned(5), 16, vec![refs[0].clone()]),
        ]
    );
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert!(runner(&mut object).complete_schedule_write(
        &ScheduleWrite::new(PropertyValue::Unsigned(2), 16, vec![refs[0].clone()]),
        &[ScheduleTargetOutcome::DatatypeRefused],
    ));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
}

#[test]
fn source_reporter_forwards_calendar_state() {
    let mut calendar = CalendarObject::new(4, "wrapped calendar").unwrap();
    calendar
        .add_date_entry(BACnetCalendarEntry::Date(
            SpecificDate::new(2026, 12, 25).unwrap().to_date(),
        ))
        .unwrap();
    let mut object: Box<dyn BACnetObject> = Box::new(calendar);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    for (day, expected) in [(25, true), (24, false)] {
        let day = SpecificDate::new(2026, 12, day).unwrap();
        let state = object.calendar_state().map(|state| state.state_on(day));
        assert_eq!(state, Some(expected));
    }
}
