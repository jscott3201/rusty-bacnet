use super::*;

#[test]
fn rpm_loop_program_metadata_selectors_preserve_bytes_and_budgets() {
    use super::super::property_metadata::assert_rpm_selector_bytes;
    use bacnet_objects::{loop_obj::LoopObject, program::ProgramObject, traits::BACnetObject};
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    // Independent legacy-order fixtures: classification must not be inferred
    // from the metadata being exercised. Property_List is read explicitly.
    let loop_all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::SETPOINT,
        P::PROPORTIONAL_CONSTANT,
        P::INTEGRAL_CONSTANT,
        P::DERIVATIVE_CONSTANT,
        P::OUTPUT_UNITS,
        P::UPDATE_INTERVAL,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::CONTROLLED_VARIABLE_REFERENCE,
        P::MANIPULATED_VARIABLE_REFERENCE,
        P::SETPOINT_REFERENCE,
    ];
    let loop_optional = [
        P::DESCRIPTION,
        P::PROPORTIONAL_CONSTANT,
        P::INTEGRAL_CONSTANT,
        P::DERIVATIVE_CONSTANT,
        P::UPDATE_INTERVAL,
        P::RELIABILITY,
    ];
    let program_all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PROGRAM_STATE,
        P::PROGRAM_CHANGE,
        P::REASON_FOR_HALT,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let program_optional = [P::DESCRIPTION, P::REASON_FOR_HALT, P::RELIABILITY];
    for configured in [false, true] {
        let mut lo = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let mut program = ProgramObject::new(1, "PRG-1").unwrap();
        if configured {
            let reference = bacnet_types::constructed::BACnetObjectPropertyReference::new_indexed(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
                P::PRESENT_VALUE.to_raw(),
                3,
            );
            lo.set_controlled_variable_reference(reference.clone());
            lo.set_manipulated_variable_reference(reference.clone());
            lo.set_setpoint_reference(reference);
            lo.set_present_value(42.0);
            program.set_program_state(5);
            program.set_reason_for_halt(3);
        }
        let objects: [Box<dyn BACnetObject>; 2] = [Box::new(lo), Box::new(program)];
        for mut object in objects {
            let oid = object.object_identifier();
            let oos = PropertyValue::Boolean(configured);
            object
                .write_property(P::OUT_OF_SERVICE, None, oos, None)
                .unwrap();
            if configured {
                object
                    .write_property(
                        P::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("long label".repeat(100)),
                        None,
                    )
                    .unwrap();
            }
            let (all, optional) = if oid.object_type() == ObjectType::LOOP {
                (loop_all.as_slice(), loop_optional.as_slice())
            } else {
                (program_all.as_slice(), program_optional.as_slice())
            };
            let required: Vec<_> = all
                .iter()
                .copied()
                .filter(|p| !optional.contains(p))
                .collect();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            for (selector, expected) in [
                (P::ALL, all),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}
