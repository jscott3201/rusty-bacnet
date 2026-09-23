use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use crate::loop_obj::LoopObject;
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::network_port::NetworkPortObject;
use crate::schedule::ScheduleObject;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorCode, PropertyIdentifier as P, Reliability};
use bacnet_types::primitives::PropertyValue as V;

fn read(object: &dyn BACnetObject, property: P) -> V {
    object.read_property(property, None).unwrap()
}
fn write(object: &mut dyn BACnetObject, property: P, value: V) {
    object.write_property(property, None, value, None).unwrap();
}
fn snapshot(object: &dyn BACnetObject) -> Vec<V> {
    [
        P::DESCRIPTION,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::STATUS_FLAGS,
    ]
    .map(|p| read(object, p))
    .to_vec()
}

fn intrinsic_objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(AnalogInputObject::new(1, "AI", 62).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO", 62).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", 62).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
    ]
}

#[test]
fn scalar_null_relinquishment_preserves_common_description_and_oos_state() {
    let mut objects = intrinsic_objects();
    objects.extend([
        Box::new(NetworkPortObject::new(1, "NP", 5).unwrap()) as Box<dyn BACnetObject>,
        Box::new(ElevatorGroupObject::new(1, "Group").unwrap()),
        Box::new(EscalatorObject::new(1, "Escalator").unwrap()),
        Box::new(LiftObject::new(1, "Lift", 3).unwrap()),
        Box::new(ScheduleObject::new(1, "Schedule", V::Unsigned(2)).unwrap()),
    ]);
    for mut object in objects {
        for oos in [false, true] {
            write(
                &mut *object,
                P::DESCRIPTION,
                V::CharacterString("retained".into()),
            );
            write(&mut *object, P::OUT_OF_SERVICE, V::Boolean(oos));
            let before = snapshot(&*object);
            for property in [P::DESCRIPTION, P::OUT_OF_SERVICE] {
                write(&mut *object, property, V::Null);
                assert_eq!(
                    snapshot(&*object),
                    before,
                    "{} {property:?}",
                    object.object_name()
                );
            }
            for (property, value) in [
                (P::DESCRIPTION, V::Unsigned(1)),
                (P::OUT_OF_SERVICE, V::CharacterString("invalid".into())),
            ] {
                assert!(
                    matches!(object.write_property(property,None,value,None),Err(bacnet_types::error::Error::Protocol{code,..}) if code == ErrorCode::INVALID_DATA_TYPE.to_raw() as u32)
                );
                assert_eq!(snapshot(&*object), before);
            }
        }
    }
}

#[test]
fn scalar_null_relinquishment_preserves_saved_reliability_and_client_override() {
    let mut objects = intrinsic_objects();
    objects.extend([
        Box::new(LoopObject::new(1, "Loop", 62).unwrap()) as Box<dyn BACnetObject>,
        Box::new(ScheduleObject::new(1, "Schedule", V::Real(1.0)).unwrap()),
    ]);
    for mut object in objects {
        object
            .set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        write(&mut *object, P::OUT_OF_SERVICE, V::Boolean(true));
        write(
            &mut *object,
            P::RELIABILITY,
            V::Enumerated(Reliability::NO_SENSOR.to_raw()),
        );
        let before = snapshot(&*object);
        write(&mut *object, P::OUT_OF_SERVICE, V::Null);
        assert_eq!(snapshot(&*object), before, "{}", object.object_name());
        write(&mut *object, P::OUT_OF_SERVICE, V::Boolean(false));
        assert_eq!(
            read(&*object, P::RELIABILITY),
            V::Enumerated(Reliability::OVER_RANGE.to_raw()),
            "{} saved state lost",
            object.object_name()
        );
    }
    for mut object in intrinsic_objects() {
        write(&mut *object, P::OUT_OF_SERVICE, V::Boolean(true));
        write(
            &mut *object,
            P::RELIABILITY,
            V::Enumerated(Reliability::NO_SENSOR.to_raw()),
        );
        write(&mut *object, P::OUT_OF_SERVICE, V::Null);
        // The client ownership exception must survive NULL: enabling inhibit
        // retains the client's reliability rather than normalizing it.
        write(
            &mut *object,
            P::RELIABILITY_EVALUATION_INHIBIT,
            V::Boolean(true),
        );
        assert_eq!(
            read(&*object, P::RELIABILITY),
            V::Enumerated(Reliability::NO_SENSOR.to_raw())
        );
        let before = snapshot(&*object);
        write(&mut *object, P::OUT_OF_SERVICE, V::Null);
        assert_eq!(snapshot(&*object), before);
        write(&mut *object, P::OUT_OF_SERVICE, V::Boolean(false));
        assert_eq!(
            read(&*object, P::RELIABILITY),
            V::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
    }
}

#[test]
fn scalar_null_relinquishment_skips_multistate_reevaluation_but_boolean_write_keeps_it() {
    macro_rules! exercise {
        ($kind:ty) => {{
            let mut object = <$kind>::new(1, "multi-state", 3).unwrap();
            write(&mut object, P::OUT_OF_SERVICE, V::Boolean(true));
            write(&mut object, P::PRESENT_VALUE, V::Unsigned(2));
            write(&mut object, P::OUT_OF_SERVICE, V::Boolean(false));
            object.set_number_of_states(1).unwrap();
            let evaluated = read(&object, P::RELIABILITY);
            assert_ne!(
                evaluated,
                V::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
            );
            object
                .set_reliability_internal(Reliability::NO_FAULT_DETECTED.to_raw())
                .unwrap();
            let before = snapshot(&object);
            write(&mut object, P::OUT_OF_SERVICE, V::Null);
            assert_eq!(snapshot(&object), before);
            // Same-value FALSE is still a Boolean write and runs the existing
            // object-owned evaluation, unlike a NULL relinquishment.
            write(&mut object, P::OUT_OF_SERVICE, V::Boolean(false));
            assert_eq!(read(&object, P::RELIABILITY), evaluated);
        }};
    }
    exercise!(MultiStateInputObject);
    exercise!(MultiStateOutputObject);
    exercise!(MultiStateValueObject);
}

#[test]
fn scalar_null_does_not_replace_commandable_or_nullable_property_semantics() {
    let mut object = AnalogOutputObject::new(1, "commandable", 62).unwrap();
    object
        .write_property(P::PRESENT_VALUE, None, V::Real(10.0), Some(8))
        .unwrap();
    object
        .write_property(P::PRESENT_VALUE, None, V::Real(20.0), Some(4))
        .unwrap();
    object
        .write_property(P::PRESENT_VALUE, None, V::Null, Some(4))
        .unwrap();
    assert_eq!(read(&object, P::PRESENT_VALUE), V::Real(10.0));
    let mut schedule = ScheduleObject::new(1, "nullable", V::Unsigned(2)).unwrap();
    write(&mut schedule, P::SCHEDULE_DEFAULT, V::Null);
    assert_eq!(read(&schedule, P::SCHEDULE_DEFAULT), V::Null);
}
