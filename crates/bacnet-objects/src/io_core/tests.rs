//! What [`IoCore::write`] reports, the one reset every detector shares, and
//! the Reliability route of the objects without a fault algorithm.

use super::*;
use crate::analog::AnalogOutputObject;
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{OutOfRangeDetector, PendingTransition};
use crate::traits::{BACnetObject, ReliabilityEvaluation};
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{EngineeringUnits, NotifyType};

fn core() -> IoCore {
    IoCore::new(ObjectType::BINARY_VALUE, 1, "BV-1".into()).unwrap()
}

#[test]
fn core_write_says_what_it_did() {
    let mut core = core();
    let mut detector = ChangeOfStateDetector::default();
    let mut write = |core: &mut IoCore, property, value| {
        core.write(&mut detector, property, None, &value)
            .map(|result| result.map_err(drop))
    };
    use PropertyValue::{Boolean, CharacterString, Enumerated, Null, Unsigned};

    for value in [Boolean(true), Boolean(true), Boolean(false)] {
        assert_eq!(
            write(&mut core, P::OUT_OF_SERVICE, value),
            Some(Ok(CoreWrite::OutOfService))
        );
    }
    assert_eq!(
        write(&mut core, P::OUT_OF_SERVICE, Null),
        Some(Ok(CoreWrite::Relinquished))
    );
    assert_eq!(
        write(&mut core, P::OUT_OF_SERVICE, Unsigned(1)),
        Some(Err(()))
    );
    for value in [Boolean(true), Boolean(false)] {
        assert_eq!(
            write(&mut core, P::RELIABILITY_EVALUATION_INHIBIT, value),
            Some(Ok(CoreWrite::Inhibit))
        );
    }
    assert_eq!(
        write(&mut core, P::RELIABILITY_EVALUATION_INHIBIT, Null),
        Some(Err(()))
    );
    for (property, value) in [
        (P::OBJECT_NAME, CharacterString("BV-2".into())),
        (P::DESCRIPTION, CharacterString("pump".into())),
        (P::EVENT_DETECTION_ENABLE, Boolean(false)),
        (P::NOTIFICATION_CLASS, Unsigned(4)),
        (P::NOTIFY_TYPE, Enumerated(1)),
    ] {
        assert_eq!(
            write(&mut core, property, value),
            Some(Ok(CoreWrite::Other)),
            "{property:?}"
        );
    }
    // A client's Reliability, taken while Out_Of_Service is TRUE.
    assert_eq!(
        write(&mut core, P::RELIABILITY, Enumerated(1)),
        Some(Err(()))
    );
    write(&mut core, P::OUT_OF_SERVICE, Boolean(true));
    assert_eq!(
        write(&mut core, P::RELIABILITY, Enumerated(1)),
        Some(Ok(CoreWrite::Other))
    );
    // The object's own rows are not the core's.
    for property in [P::PRESENT_VALUE, P::ALARM_VALUE, P::PROPERTY_LIST] {
        assert_eq!(
            write(&mut core, property, Enumerated(1)),
            None,
            "{property:?}"
        );
    }
}

/// Detection off puts a `$detector`, its transition state set away from its
/// start, back to that start and keeps its parameters; detection on resets
/// nothing. The detector as detection on left it.
macro_rules! assert_reset {
    ($detector:ident $(, $field:ident: $value:expr)*) => {{
        let moved = || $detector {
            event_state: EventState::OFFNORMAL,
            acked_transitions: EventTransitionBits::TO_FAULT,
            pending: Some(PendingTransition {
                state: EventState::NORMAL,
                due: crate::event::test_clock::in_seconds(2),
            }),
            fault_reliability: Some(Reliability::NO_SENSOR),
            notification_class: 9,
            notify_type: NotifyType::EVENT,
            time_delay: 30,
            $($field: $value,)*
            ..Default::default()
        };
        let mut core = core();
        let mut detector = moved();
        let off = PropertyValue::Boolean(false);
        core.write(&mut detector, P::EVENT_DETECTION_ENABLE, None, &off)
            .unwrap()
            .unwrap();
        assert_eq!(detector.event_state, EventState::NORMAL);
        assert_eq!(detector.acked_transitions, EventTransitionBits::all());
        assert!(detector.pending.is_none());
        assert!(detector.fault_reliability.is_none());
        assert_eq!(
            (
                detector.notification_class,
                detector.notify_type,
                detector.time_delay
            ),
            (9, NotifyType::EVENT, 30)
        );
        let mut detector = moved();
        let on = PropertyValue::Boolean(true);
        core.write(&mut detector, P::EVENT_DETECTION_ENABLE, None, &on)
            .unwrap()
            .unwrap();
        assert_eq!(detector.event_state, EventState::OFFNORMAL);
        assert!(detector.pending.is_some());
        detector
    }};
}

#[test]
fn detection_off_resets_every_detector_the_same_way() {
    let alarm = assert_reset!(ChangeOfStateDetector, alarm_values: vec![1]);
    assert_eq!(alarm.alarm_values, [1]);
    assert_reset!(CommandFailureDetector);
    let range = assert_reset!(OutOfRangeDetector, high_limit: 80.0);
    assert_eq!(range.high_limit, 80.0);
}

/// The objects [`impl_reliability_eval!`] serves: a Reliability the
/// application sets stays, since nothing of theirs evaluates it, and
/// Reliability_Evaluation_Inhibit is what the capability reports.
#[test]
fn reliability_set_by_the_application_stays_on_the_objects_without_a_fault_algorithm() {
    let objects: [Box<dyn BACnetObject>; 4] = [
        Box::new(AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
    ];
    let no_sensor = PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw());
    for mut object in objects {
        let kind = object.object_identifier().object_type();
        let eval = object.reliability_eval().unwrap();
        eval.set_reliability(Reliability::NO_SENSOR).unwrap();
        assert_eq!(
            eval.evaluate_reliability().unwrap(),
            ReliabilityEvaluation::Unchanged,
            "{kind:?}"
        );
        assert!(!eval.reliability_evaluation_inhibited(), "{kind:?}");
        assert_eq!(
            object.read_property(P::RELIABILITY, None).unwrap(),
            no_sensor,
            "{kind:?}"
        );
        object
            .write_property(
                P::RELIABILITY_EVALUATION_INHIBIT,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        let eval = object.reliability_eval().unwrap();
        assert!(eval.reliability_evaluation_inhibited(), "{kind:?}");
        // Inhibited, the object no longer takes the application's either.
        assert!(
            eval.set_reliability(Reliability::NO_SENSOR).is_err(),
            "{kind:?}"
        );
    }
}
