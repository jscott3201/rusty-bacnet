use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject};
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_types::enums::{EngineeringUnits, ErrorCode};

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn code(error: Error) -> ErrorCode {
    match error {
        Error::Remote(bacnet_types::error::RemoteError::Error { code, .. }) => code,
        Error::Structured { code, .. } => code,
        other => panic!("not a protocol error: {other:?}"),
    }
}

fn analog_input() -> AnalogInputObject {
    AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()
}

fn high_limit(limit: f32) -> AlgorithmKeywords {
    AlgorithmKeywords::analog(Some(limit), None, None, None, None, None)
}

/// Each analog keyword sets its property; an absent one leaves the default.
#[test]
fn analog_keywords_set_their_parameters_and_absent_ones_leave_the_defaults() {
    let mut ai = analog_input();
    let low_limit = read(&ai, PropertyIdentifier::LOW_LIMIT);
    AlgorithmKeywords::analog(
        Some(80.5),
        None,
        Some(1.5),
        Some(PyBitString::raw(6, &[0x40])),
        Some(7),
        None,
    )
    .apply(&mut ai)
    .unwrap();
    assert_eq!(
        read(&ai, PropertyIdentifier::HIGH_LIMIT),
        PropertyValue::Real(80.5)
    );
    assert_eq!(
        read(&ai, PropertyIdentifier::DEADBAND),
        PropertyValue::Real(1.5)
    );
    assert_eq!(
        read(&ai, PropertyIdentifier::LIMIT_ENABLE),
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0x40]
        }
    );
    assert_eq!(
        read(&ai, PropertyIdentifier::TIME_DELAY),
        PropertyValue::Unsigned(7)
    );
    assert_eq!(read(&ai, PropertyIdentifier::LOW_LIMIT), low_limit);
    // An unset Time_Delay_Normal reads as Time_Delay.
    assert_eq!(
        read(&ai, PropertyIdentifier::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(7)
    );
}

/// A value the object refuses on the wire is refused here, with the
/// object's own code.
#[test]
fn refused_values_fail_with_the_objects_own_code() {
    let out_of_range = ErrorCode::VALUE_OUT_OF_RANGE;
    let analog = |keywords: AlgorithmKeywords| keywords.apply(&mut analog_input());
    assert_eq!(
        code(analog(high_limit(f32::NAN)).unwrap_err()),
        out_of_range
    );
    let deadband = AlgorithmKeywords::analog(None, None, Some(-1.0), None, None, None);
    assert_eq!(code(analog(deadband).unwrap_err()), out_of_range);
    // Limit_Enable is two bits wide.
    let three_bits = Some(PyBitString::raw(5, &[0x40]));
    let width = AlgorithmKeywords::analog(None, None, None, three_bits, None, None);
    assert!(analog(width).is_err());

    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let alarm = |state| AlgorithmKeywords::binary(Some(state), None, None);
    assert_eq!(code(alarm(2).apply(&mut bi).unwrap_err()), out_of_range);
    alarm(0).apply(&mut bi).unwrap();
    assert_eq!(
        read(&bi, PropertyIdentifier::ALARM_VALUE),
        PropertyValue::Enumerated(0)
    );

    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let states = |states: Vec<u32>| AlgorithmKeywords::multistate(Some(states), None, None);
    for refused in [vec![0], vec![2, 4]] {
        assert_eq!(
            code(states(refused).apply(&mut msi).unwrap_err()),
            out_of_range
        );
    }
    states(vec![3, 2]).apply(&mut msi).unwrap();
    assert_eq!(
        read(&msi, PropertyIdentifier::ALARM_VALUES),
        PropertyValue::List(vec![PropertyValue::Unsigned(3), PropertyValue::Unsigned(2)])
    );

    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    AlgorithmKeywords::delays(None, Some(4))
        .apply(&mut bo)
        .unwrap();
    assert_eq!(
        read(&bo, PropertyIdentifier::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(4)
    );
}

/// Through the builders' `keep_reporting`, the keyword applies before
/// storage attaches: it is not saved, a saved write goes over it, and a
/// clearing start applies it again (#2410).
#[test]
fn keywords_apply_before_storage_attaches_and_again_after_a_clear() {
    pyo3::Python::initialize();
    let directory = std::env::temp_dir().join(format!(
        "rusty-bacnet-algorithm-keywords-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("ai-1");
    let keywords = high_limit(30.0);
    let build = |clear| {
        crate::object_options::keep_reporting(
            || {
                let mut ai = analog_input();
                keywords.apply(&mut ai)?;
                Ok(ai)
            },
            path.to_str(),
            clear,
            AnalogInputObject::with_reporting_persistence,
            AnalogInputObject::clear_written_reporting,
        )
        .unwrap()
    };
    let limit = PropertyIdentifier::HIGH_LIMIT;
    assert_eq!(read(&build(false), limit), PropertyValue::Real(30.0));
    assert!(!path.exists(), "the keyword value is never saved");

    build(false)
        .write_property(limit, None, PropertyValue::Real(80.5), None)
        .unwrap();
    assert_eq!(read(&build(false), limit), PropertyValue::Real(80.5));
    assert_eq!(read(&build(true), limit), PropertyValue::Real(30.0));
    assert_eq!(read(&build(false), limit), PropertyValue::Real(30.0));
    std::fs::remove_dir_all(&directory).unwrap();
}

/// OUT_OF_RANGE with no delay: above 40 or below 20, a deadband of 1.
const OUT_OF_RANGE: &str = "5e09001c41a000002c422000003c3f8000005f";

fn enrollment() -> bacnet_objects::event_enrollment::EventEnrollmentObject {
    bacnet_objects::event_enrollment::EventEnrollmentObject::new(
        1,
        "EE-1",
        bacnet_types::enums::EventType::CHANGE_OF_BITSTRING,
    )
    .unwrap()
}

fn encoded(hex: &str) -> PropertyValue {
    let octets = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect();
    PropertyValue::ApplicationData(octets)
}

/// The Event Enrollment keywords go the way a `write_property_local`
/// does: a malformed value is refused with the code the write gets (#2426).
#[test]
fn enrollment_keywords_are_checked_as_their_writes_are() {
    let parameters = encoded(OUT_OF_RANGE);
    let mut ee = enrollment();
    AlgorithmKeywords::enrollment(Some(&parameters), None, Some(9))
        .unwrap()
        .apply(&mut ee)
        .unwrap();
    assert_eq!(read(&ee, PropertyIdentifier::EVENT_PARAMETERS), parameters);
    assert_eq!(
        read(&ee, PropertyIdentifier::EVENT_TYPE),
        PropertyValue::Enumerated(5)
    );
    assert_eq!(
        read(&ee, PropertyIdentifier::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(9)
    );

    // A parameter value cut short, and a Fault_Parameters that is no
    // BACnetFaultParameter, are refused.
    let cut = encoded(&OUT_OF_RANGE[..OUT_OF_RANGE.len() - 2]);
    let not_fault = PropertyValue::Real(1.0);
    for keywords in [
        AlgorithmKeywords::enrollment(Some(&cut), None, None),
        AlgorithmKeywords::enrollment(None, Some(&not_fault), None),
    ] {
        assert!(keywords.unwrap().apply(&mut enrollment()).is_err());
    }
}

/// The enrollment pass evaluates with the keyword parameters: a value of
/// 50 raises HIGH_LIMIT under them, and nothing under the default
/// parameters (the `none` choice).
#[test]
fn the_enrollment_pass_uses_the_keyword_parameters() {
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
    use bacnet_types::enums::EventState;

    let pass = |keywords: AlgorithmKeywords| {
        let mut db = ObjectDatabase::new();
        let mut ai = analog_input();
        ai.set_present_value(50.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();
        let mut ee = enrollment();
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
        keywords.apply(&mut ee).unwrap();
        db.add(Box::new(ee)).unwrap();
        bacnet_server::event_enrollment::evaluate_event_enrollments(&mut db, 1)
    };

    assert!(pass(AlgorithmKeywords::default()).is_empty());
    let transitions =
        pass(AlgorithmKeywords::enrollment(Some(&encoded(OUT_OF_RANGE)), None, None).unwrap());
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}
