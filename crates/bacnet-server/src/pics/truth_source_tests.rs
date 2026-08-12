//! Truth-source cross-check tests for the PICS writability fix (issue #115).
//!
//! These tests live in a sibling module to `tests.rs` to keep `tests.rs` under
//! the workspace's 700-LOC file cap. They verify the central invariant of the
//! shared-truth-source design: for every core I/O/V object type, the
//! `BACnetObject::is_writable_property` override and the runtime
//! `write_property` match arms must agree — if the override reports a property
//! writable, `write_property` must accept it, and if it reports it read-only,
//! `write_property` must reject it. This is the regression guard that prevents
//! an override and its `write_property` arms from drifting apart (the exact
//! false-negative class issue #115 eliminates).

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;

/// Cross-check the central truth-source invariant: for every core I/O/V type,
/// `is_writable_property` and `write_property` must agree. If the override
/// reports a property writable, `write_property` must accept it; if it reports
/// it read-only, `write_property` must reject it. This guards against the
/// override and the `write_property` match arms drifting apart — the exact
/// false-negative class issue #115 is meant to eliminate.
///
/// Sample values are chosen to satisfy `write_property`'s validation so a true
/// agreement is tested (not a rejection on bad input). Input types (AI, BI,
/// MSI) require `out_of_service` before PRESENT_VALUE is accepted, so those
/// objects are pre-flipped where needed.
#[test]
fn is_writable_property_matches_write_property_on_all_core_types() {
    use bacnet_objects::event::LimitEnable;

    // A read-only property every type rejects — used as a universal negative.
    const READ_ONLY: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;

    // Helper: assert the override says writable AND write_property accepts it.
    fn assert_accepts(
        obj: &mut dyn BACnetObject,
        pid: PropertyIdentifier,
        value: PropertyValue,
        label: &str,
    ) {
        assert!(
            obj.is_writable_property(pid),
            "{label}: is_writable_property must report {pid:?} writable"
        );
        let result = obj.write_property(pid, None, value, None);
        assert!(
            result.is_ok(),
            "{label}: write_property must accept {pid:?}, got: {result:?}"
        );
    }

    // Helper: assert the override says read-only AND write_property rejects it.
    fn assert_rejects(obj: &mut dyn BACnetObject, pid: PropertyIdentifier, label: &str) {
        assert!(
            !obj.is_writable_property(pid),
            "{label}: is_writable_property must report {pid:?} NOT writable"
        );
        let result = obj.write_property(pid, None, PropertyValue::Null, None);
        assert!(
            result.is_err(),
            "{label}: write_property must reject {pid:?}, got: {result:?}"
        );
    }

    // Helper: like assert_accepts but supplies an array_index (PRIORITY_ARRAY
    // requires Some(1..=16); STATE_TEXT similarly takes an element index).
    fn assert_accepts_indexed(
        obj: &mut dyn BACnetObject,
        pid: PropertyIdentifier,
        index: u32,
        value: PropertyValue,
        label: &str,
    ) {
        assert!(
            obj.is_writable_property(pid),
            "{label}: is_writable_property must report {pid:?} writable"
        );
        let result = obj.write_property(pid, Some(index), value, None);
        assert!(
            result.is_ok(),
            "{label}: write_property must accept {pid:?}[{index}], got: {result:?}"
        );
    }

    // AnalogInput — PRESENT_VALUE needs out-of-service.
    let mut ai = AnalogInputObject::new(1, "ai", 95).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_accepts(
        &mut ai,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(50.0),
        "AI",
    );
    assert_accepts(
        &mut ai,
        PropertyIdentifier::COV_INCREMENT,
        PropertyValue::Real(1.0),
        "AI",
    );
    assert_accepts(
        &mut ai,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(0),
        "AI",
    );
    assert_accepts(
        &mut ai,
        PropertyIdentifier::LIMIT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        "AI",
    );
    assert_rejects(&mut ai, READ_ONLY, "AI");

    // AnalogOutput — commandable: PRIORITY_ARRAY + PRESENT_VALUE + event set.
    let mut ao = AnalogOutputObject::new(1, "ao", 95).unwrap();
    ao.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_accepts(
        &mut ao,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(50.0),
        "AO",
    );
    assert_accepts_indexed(
        &mut ao,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "AO",
    );
    assert_accepts(
        &mut ao,
        PropertyIdentifier::COV_INCREMENT,
        PropertyValue::Real(1.0),
        "AO",
    );
    assert_accepts(
        &mut ao,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(0),
        "AO",
    );
    assert_accepts(
        &mut ao,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyValue::Enumerated(0),
        "AO",
    );
    assert_rejects(&mut ao, READ_ONLY, "AO");

    // AnalogValue — same writable set as AnalogOutput.
    let mut av = AnalogValueObject::new(1, "av", 95).unwrap();
    av.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_accepts(
        &mut av,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(50.0),
        "AV",
    );
    assert_accepts_indexed(
        &mut av,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "AV",
    );
    assert_accepts(
        &mut av,
        PropertyIdentifier::COV_INCREMENT,
        PropertyValue::Real(1.0),
        "AV",
    );
    assert_accepts(
        &mut av,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(0),
        "AV",
    );
    assert_accepts(
        &mut av,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0], // all three transitions, MSB-first
        },
        "AV",
    );
    assert_rejects(&mut av, READ_ONLY, "AV");

    // BinaryInput — PRESENT_VALUE needs out-of-service; ACTIVE/INACTIVE_TEXT.
    let mut bi = BinaryInputObject::new(1, "bi").unwrap();
    bi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_accepts(
        &mut bi,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
        "BI",
    );
    assert_accepts(
        &mut bi,
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyValue::CharacterString("on".into()),
        "BI",
    );
    assert_accepts(
        &mut bi,
        PropertyIdentifier::INACTIVE_TEXT,
        PropertyValue::CharacterString("off".into()),
        "BI",
    );
    assert_accepts(
        &mut bi,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0], // all three transitions, MSB-first
        },
        "BI",
    );
    assert_rejects(&mut bi, READ_ONLY, "BI");

    // BinaryOutput — commandable + ACTIVE/INACTIVE_TEXT.
    let mut bo = BinaryOutputObject::new(1, "bo").unwrap();
    assert_accepts(
        &mut bo,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
        "BO",
    );
    assert_accepts_indexed(
        &mut bo,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "BO",
    );
    assert_accepts(
        &mut bo,
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyValue::CharacterString("on".into()),
        "BO",
    );
    assert_accepts(
        &mut bo,
        PropertyIdentifier::INACTIVE_TEXT,
        PropertyValue::CharacterString("off".into()),
        "BO",
    );
    assert_rejects(&mut bo, READ_ONLY, "BO");

    // BinaryValue — same writable set as BinaryOutput (no dedicated mirror
    // test exists, so this is the BV override's regression guard).
    let mut bv = BinaryValueObject::new(1, "bv").unwrap();
    assert_accepts(
        &mut bv,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
        "BV",
    );
    assert_accepts_indexed(
        &mut bv,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "BV",
    );
    assert_accepts(
        &mut bv,
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyValue::CharacterString("on".into()),
        "BV",
    );
    assert_accepts(
        &mut bv,
        PropertyIdentifier::INACTIVE_TEXT,
        PropertyValue::CharacterString("off".into()),
        "BV",
    );
    assert_accepts(
        &mut bv,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyValue::Enumerated(0),
        "BV",
    );
    assert_rejects(&mut bv, READ_ONLY, "BV");

    // MultiStateInput — PRESENT_VALUE needs out-of-service; STATE_TEXT (array).
    let mut msi = MultiStateInputObject::new(1, "msi", 2).unwrap();
    msi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_accepts(
        &mut msi,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Unsigned(1),
        "MSI",
    );
    assert_accepts_indexed(
        &mut msi,
        PropertyIdentifier::STATE_TEXT,
        1,
        PropertyValue::CharacterString("s1".into()),
        "MSI",
    );
    assert_accepts(
        &mut msi,
        PropertyIdentifier::TIME_DELAY,
        PropertyValue::Unsigned(5),
        "MSI",
    );
    assert_rejects(&mut msi, READ_ONLY, "MSI");

    // MultiStateOutput — commandable + STATE_TEXT (array).
    let mut mso = MultiStateOutputObject::new(1, "mso", 2).unwrap();
    assert_accepts(
        &mut mso,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Unsigned(1),
        "MSO",
    );
    assert_accepts_indexed(
        &mut mso,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "MSO",
    );
    assert_accepts_indexed(
        &mut mso,
        PropertyIdentifier::STATE_TEXT,
        1,
        PropertyValue::CharacterString("s1".into()),
        "MSO",
    );
    assert_rejects(&mut mso, READ_ONLY, "MSO");

    // MultiStateValue — commandable + STATE_TEXT (array).
    let mut msv = MultiStateValueObject::new(1, "msv", 2).unwrap();
    assert_accepts(
        &mut msv,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Unsigned(1),
        "MSV",
    );
    assert_accepts_indexed(
        &mut msv,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        PropertyValue::Null,
        "MSV",
    );
    assert_accepts_indexed(
        &mut msv,
        PropertyIdentifier::STATE_TEXT,
        1,
        PropertyValue::CharacterString("s1".into()),
        "MSV",
    );
    assert_accepts(
        &mut msv,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(3),
        "MSV",
    );
    assert_rejects(&mut msv, READ_ONLY, "MSV");
}

/// The per-object `write_property` arms and the `is_writable_property`
/// override must agree on the two objects hardened in the #182 review round:
/// Pulse Converter (whose historical default advertised INPUT_REFERENCE
/// read-only while the arm accepted it — and OBJECT_NAME writable while no
/// arm routes it) and Averaging (same OBJECT_NAME misadvertising; its
/// OBJECT_PROPERTY_REFERENCE arm stayed unadvertised). Each advertised
/// property is written with a canonical good value; each unadvertised one
/// must reject even a plausible value.
#[test]
fn is_writable_property_matches_write_property_on_pulse_converter_and_averaging() {
    use bacnet_objects::accumulator::PulseConverterObject;
    use bacnet_objects::averaging::AveragingObject;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    fn assert_exactly(
        obj: &mut dyn BACnetObject,
        label: &str,
        accepted: &[(PropertyIdentifier, PropertyValue)],
        rejected: &[(PropertyIdentifier, PropertyValue)],
    ) {
        for (pid, value) in accepted.iter().cloned() {
            assert!(
                obj.is_writable_property(pid),
                "{label}: {pid:?} must be advertised writable"
            );
            assert!(
                obj.write_property(pid, None, value, None).is_ok(),
                "{label}: {pid:?} advertised writable but the arm rejected a good value"
            );
        }
        for (pid, value) in rejected.iter().cloned() {
            assert!(
                !obj.is_writable_property(pid),
                "{label}: {pid:?} must NOT be advertised writable"
            );
            assert!(
                obj.write_property(pid, None, value, None).is_err(),
                "{label}: {pid:?} not advertised but the arm accepted a write"
            );
        }
    }

    let reference_value = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap(),
        ),
        PropertyValue::Enumerated(PropertyIdentifier::PRESENT_VALUE.to_raw()),
    ]);

    let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
    assert_exactly(
        &mut pc,
        "PC",
        &[
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(10.0)),
            (PropertyIdentifier::SCALE_FACTOR, PropertyValue::Real(1.5)),
            (PropertyIdentifier::ADJUST_VALUE, PropertyValue::Real(2.0)),
            (PropertyIdentifier::COV_INCREMENT, PropertyValue::Real(0.5)),
            (PropertyIdentifier::INPUT_REFERENCE, reference_value.clone()),
            (
                PropertyIdentifier::DESCRIPTION,
                PropertyValue::CharacterString("d".into()),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        ],
        &[
            (
                PropertyIdentifier::OBJECT_NAME,
                PropertyValue::CharacterString("renamed".into()),
            ),
            (PropertyIdentifier::UNITS, PropertyValue::Enumerated(95)),
            (
                PropertyIdentifier::RELIABILITY,
                PropertyValue::Enumerated(0),
            ),
            (
                PropertyIdentifier::EVENT_STATE,
                PropertyValue::Enumerated(0),
            ),
            (
                PropertyIdentifier::STATUS_FLAGS,
                PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![0],
                },
            ),
        ],
    );

    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_exactly(
        &mut avg,
        "AVG",
        &[
            (
                PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
                reference_value,
            ),
            (
                PropertyIdentifier::DESCRIPTION,
                PropertyValue::CharacterString("d".into()),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        ],
        &[
            (
                PropertyIdentifier::OBJECT_NAME,
                PropertyValue::CharacterString("renamed".into()),
            ),
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(1.0)),
            (PropertyIdentifier::MINIMUM_VALUE, PropertyValue::Real(1.0)),
            (
                PropertyIdentifier::WINDOW_INTERVAL,
                PropertyValue::Unsigned(60),
            ),
            (
                PropertyIdentifier::RELIABILITY,
                PropertyValue::Enumerated(0),
            ),
        ],
    );
}
