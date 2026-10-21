//! Event_Algorithm_Inhibit written on an enrollment, with no reference
//! (#1507): the evaluator's gate, pass by pass (Clauses 13.2.2.1 and
//! 13.2.2.1.5).

use super::super::*;
use super::*;
use bacnet_objects::event_enrollment::EventEnrollmentEvalState;
use bacnet_types::constructed::ChangeOfValueCriteria;
use std::borrow::Cow;

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    db.get_mut(&oid)
        .unwrap()
        .write_property(property, None, value, None)
        .unwrap();
}

fn inhibit(db: &mut ObjectDatabase, ee: ObjectIdentifier, on: bool) {
    write(
        db,
        ee,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        PropertyValue::Boolean(on),
    );
}

fn set_value(db: &mut ObjectDatabase, ai: ObjectIdentifier, value: f32) {
    write(
        db,
        ai,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    write(
        db,
        ai,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(value),
    );
}

fn state(db: &ObjectDatabase, ee: ObjectIdentifier) -> EventEnrollmentEvalState {
    db.get(&ee)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
}

fn transitions(db: &mut ObjectDatabase) -> Vec<(EventState, EventState)> {
    evaluate_event_enrollments(db, 1)
        .into_iter()
        .map(|t| (t.change.from, t.change.to))
        .collect()
}

/// A written inhibit returns an offnormal enrollment to NORMAL at once,
/// inside pTimeDelayNormal, and the countdown toward NORMAL it was running
/// is gone with it. Inhibited at NORMAL, the high value reports nothing.
#[test]
fn written_inhibit_forces_normal_at_once_and_clears_the_countdown() {
    let (mut db, ee, ai) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    write(
        &mut db,
        ee,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(30),
    );
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
    set_value(&mut db, ai, 50.0);
    assert!(transitions(&mut db).is_empty());
    assert!(state(&db, ee).pending.is_some(), "TO_NORMAL countdown");

    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::HIGH_LIMIT, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).pending, None);

    set_value(&mut db, ai, 85.0);
    for _ in 0..3 {
        assert!(transitions(&mut db).is_empty());
    }
    inhibit(&mut db, ee, false);
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
}

/// A setpoint gap used to end the pass before the algorithm ran; under the
/// inhibit, an offnormal enrollment still returns to NORMAL.
#[test]
fn inhibit_forces_normal_through_a_setpoint_gap() {
    let (mut db, ee, _ai) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
    let setpoint = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    db.remove(&setpoint).unwrap();
    // Control: without the inhibit the gap changes nothing.
    assert!(transitions(&mut db).is_empty());

    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::HIGH_LIMIT, EventState::NORMAL)]
    );
}

/// A monitored object whose Present_Value can take any datatype.
struct Variable {
    oid: ObjectIdentifier,
    value: PropertyValue,
}

impl BACnetObject for Variable {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "variable"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        match property {
            PropertyIdentifier::PRESENT_VALUE => Ok(self.value.clone()),
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            _ => Err(bacnet_types::error::Error::Protocol {
                class: bacnet_types::enums::ErrorClass::PROPERTY,
                code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
            }),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        self.value = value;
        Ok(())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::PRESENT_VALUE,
        ])
    }
}

/// A CHANGE_OF_VALUE enrollment (REAL increment 1.0) on a [`Variable`] at
/// 10.0, past its first pass, so 10.0 is the baseline.
fn change_of_value(time_delay: u32) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let variable = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    db.add(Box::new(Variable {
        oid: variable,
        value: PropertyValue::Real(10.0),
    }))
    .unwrap();
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-COV", EventType::CHANGE_OF_VALUE).unwrap();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            variable,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(1.0),
    });
    enrollment.set_event_enable(EventTransitionBits::all());
    let ee = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    assert!(transitions(&mut db).is_empty());
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(10.0)));
    (db, ee, variable)
}

/// Put the enrollment in a state CHANGE_OF_VALUE doesn't name, as a rewrite
/// of Event_Parameters can leave behind.
fn leave_offnormal(db: &mut ObjectDatabase, ee: ObjectIdentifier) {
    db.get_mut(&ee)
        .unwrap()
        .set_event_state_internal(EventState::OFFNORMAL)
        .unwrap();
}

fn set_variable(db: &mut ObjectDatabase, variable: ObjectIdentifier, value: PropertyValue) {
    write(db, variable, PropertyIdentifier::PRESENT_VALUE, value);
}

/// The forced NORMAL is a NORMAL indication, so under CHANGE_OF_VALUE it
/// takes the current sample as the baseline (Clause 13.3.3): once the
/// inhibit clears, the unchanged value reports nothing.
#[test]
fn forced_normal_rebaselines_change_of_value() {
    let (mut db, ee, variable) = change_of_value(0);
    leave_offnormal(&mut db, ee);
    set_variable(&mut db, variable, PropertyValue::Real(20.0));
    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::OFFNORMAL, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(20.0)));

    inhibit(&mut db, ee, false);
    assert!(
        transitions(&mut db).is_empty(),
        "measured from the baseline the forced NORMAL took (20.0)"
    );
}

/// A monitored value the criteria can't read used to end the pass before
/// the algorithm ran; under the inhibit, the offnormal enrollment returns to
/// NORMAL anyway and takes that sample as its baseline.
#[test]
fn forced_normal_through_a_wrong_datatype_value() {
    let (mut db, ee, variable) = change_of_value(0);
    leave_offnormal(&mut db, ee);
    let bits = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0x80],
    };
    set_variable(&mut db, variable, bits.clone());
    // Control: without the inhibit the pass changes nothing.
    assert!(transitions(&mut db).is_empty());

    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::OFFNORMAL, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).cov_baseline, Some(bits));
}

/// Inhibited at NORMAL, the enrollment keeps its baseline: a change made
/// meanwhile is reported once the inhibit clears and its Time_Delay runs.
#[test]
fn inhibited_normal_keeps_its_baseline_until_the_clear() {
    let (mut db, ee, variable) = change_of_value(2);
    inhibit(&mut db, ee, true);
    set_variable(&mut db, variable, PropertyValue::Real(20.0));
    for _ in 0..3 {
        assert!(transitions(&mut db).is_empty());
    }
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(10.0)));

    inhibit(&mut db, ee, false);
    // Two passes of one second each: the countdown seeds, then runs.
    for _ in 0..2 {
        assert!(transitions(&mut db).is_empty());
    }
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(20.0)));
}

/// A transition that fires at once drops a countdown toward another target:
/// the TO_NORMAL countdown from before LOW_LIMIT doesn't carry on after it.
#[test]
fn an_immediate_transition_drops_the_countdown_under_way() {
    let (mut db, ee, ai) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    write(
        &mut db,
        ee,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(30),
    );
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
    set_value(&mut db, ai, 50.0);
    for _ in 0..10 {
        assert!(transitions(&mut db).is_empty());
    }
    assert!(state(&db, ee).pending.is_some(), "TO_NORMAL countdown");

    set_value(&mut db, ai, 10.0);
    assert_eq!(
        transitions(&mut db),
        [(EventState::HIGH_LIMIT, EventState::LOW_LIMIT)]
    );
    assert_eq!(state(&db, ee).pending, None);

    // Back in range: the full 30 s from here (one pass seeds, thirty run).
    set_value(&mut db, ai, 50.0);
    for pass in 0..30 {
        assert!(transitions(&mut db).is_empty(), "pass {pass}");
    }
    assert_eq!(
        transitions(&mut db),
        [(EventState::LOW_LIMIT, EventState::NORMAL)]
    );
}

/// An enrollment whose Event_Parameters, once `extended` is set, read as an
/// Extended set the encoder refuses, so its countdown fingerprint fails.
struct UnencodableParameters {
    inner: EventEnrollmentObject,
    extended: bool,
}

impl BACnetObject for UnencodableParameters {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.inner.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        if property == PropertyIdentifier::EVENT_PARAMETERS && self.extended {
            // Vendor 1, type 2, parameters that are no TLV sequence.
            return Ok(PropertyValue::List(vec![
                PropertyValue::Unsigned(9),
                PropertyValue::Unsigned(1),
                PropertyValue::Unsigned(2),
                PropertyValue::OctetString(vec![0xFF]),
            ]));
        }
        self.inner.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        if property == PropertyIdentifier::EVENT_PARAMETERS {
            self.extended = true;
            return Ok(());
        }
        self.inner
            .write_property(property, array_index, value, priority)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }

    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        self.inner.enrollment_eval_state_internal()
    }

    fn set_enrollment_eval_state_internal(
        &mut self,
        state: EventEnrollmentEvalState,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner.set_enrollment_eval_state_internal(state)
    }

    fn enrollment_eval_source_internal(
        &self,
    ) -> Option<Option<bacnet_objects::event_enrollment::EventEnrollmentMonitoredSource>> {
        self.inner.enrollment_eval_source_internal()
    }

    fn set_enrollment_eval_source_internal(
        &mut self,
        source: Option<bacnet_objects::event_enrollment::EventEnrollmentMonitoredSource>,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner.set_enrollment_eval_source_internal(source)
    }

    fn set_event_state_internal(
        &mut self,
        state: EventState,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner.set_event_state_internal(state)
    }

    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: EventTransitionBits,
        acknowledged: bool,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner
            .set_acked_transitions_internal(transition_bit, acknowledged)
    }

    fn commit_event_transition_internal(
        &mut self,
        commit: bacnet_objects::event::EventTransitionCommit,
    ) -> Result<(), bacnet_objects::event::EventTransitionCommitError> {
        self.inner.commit_event_transition_internal(commit)
    }

    fn commit_event_enrollment_reliability_internal(
        &mut self,
        commit: bacnet_objects::event_enrollment::EventEnrollmentReliabilityCommit,
    ) -> Result<(), bacnet_objects::event::EventTransitionCommitError> {
        self.inner
            .commit_event_enrollment_reliability_internal(commit)
    }
}

/// Parameters the countdown can't fingerprint end an uninhibited pass, but
/// an inhibited one needs no countdown, so the unmodeled (Extended) set still
/// gets its forced NORMAL.
#[test]
fn inhibit_forces_normal_when_the_parameters_cannot_be_fingerprinted() {
    let (mut db, _, ai) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let mut inner = EventEnrollmentObject::new(7, "EE-EXT", EventType::OUT_OF_RANGE).unwrap();
    inner
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
    inner.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    inner.set_event_enable(EventTransitionBits::all());
    let ee = inner.object_identifier();
    db.add(Box::new(UnencodableParameters {
        inner,
        extended: false,
    }))
    .unwrap();
    let mine = |db: &mut ObjectDatabase| {
        evaluate_event_enrollments(db, 1)
            .into_iter()
            .filter(|t| t.enrollment_oid == ee)
            .map(|t| (t.change.from, t.change.to))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        mine(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );

    write(
        &mut db,
        ee,
        PropertyIdentifier::EVENT_PARAMETERS,
        PropertyValue::Null,
    );
    // Control: without the inhibit the pass changes nothing.
    assert!(mine(&mut db).is_empty());

    inhibit(&mut db, ee, true);
    assert_eq!(
        mine(&mut db),
        [(EventState::HIGH_LIMIT, EventState::NORMAL)]
    );
}
