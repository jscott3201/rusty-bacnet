//! A Pulse Converter's Input_Reference judged against the database (Clause
//! 12.23.9, #1341): CONFIGURATION_ERROR and the FAULT flag while it names a
//! property the converter can't count from, cleared once it names one it
//! can, re-judged as objects come and go.

use super::*;
use crate::accumulator::{AccumulatorObject, PulseConverterObject};
use crate::analog::AnalogInputObject;
use crate::value_types::IntegerValueObject;
use bacnet_types::enums::{EngineeringUnits, Reliability};
use bacnet_types::primitives::StatusFlags;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use PropertyIdentifier as P;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn pc1() -> ObjectIdentifier {
    oid(ObjectType::PULSE_CONVERTER, 1)
}

fn acc1() -> ObjectIdentifier {
    oid(ObjectType::ACCUMULATOR, 1)
}

/// The Clause 21 octets of a reference, as a client writes them.
fn octets(reference: &BACnetObjectPropertyReference) -> PropertyValue {
    let mut encoded = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(&mut encoded, reference);
    PropertyValue::ApplicationData(encoded.to_vec())
}

fn present_value(target: ObjectIdentifier) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(target, P::PRESENT_VALUE.to_raw())
}

fn reliability(db: &ObjectDatabase, oid: ObjectIdentifier) -> Reliability {
    match db.get(&oid).unwrap().read_property(P::RELIABILITY, None) {
        Ok(PropertyValue::Enumerated(raw)) => Reliability::from_raw(raw),
        other => panic!("Reliability read {other:?}"),
    }
}

fn fault(db: &ObjectDatabase, oid: ObjectIdentifier) -> bool {
    match db.get(&oid).unwrap().read_property(P::STATUS_FLAGS, None) {
        Ok(PropertyValue::BitString { data, .. }) => {
            StatusFlags::from_bits_truncate(data[0] >> 4).contains(StatusFlags::FAULT)
        }
        other => panic!("Status_Flags read {other:?}"),
    }
}

/// A Pulse Converter named PC-1 whose Input_Reference is `reference`.
fn converter(reference: Option<BACnetObjectPropertyReference>) -> Box<PulseConverterObject> {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::NO_UNITS).unwrap();
    if let Some(reference) = reference {
        pc.set_input_reference(reference);
    }
    Box::new(pc)
}

/// Write Input_Reference as a client does, then judge it as the server
/// does when the write commits; returns whether Reliability changed.
fn write_reference(db: &mut ObjectDatabase, reference: &BACnetObjectPropertyReference) -> bool {
    db.get_mut(&pc1())
        .unwrap()
        .write_property(P::INPUT_REFERENCE, None, octets(reference), None)
        .unwrap();
    db.check_input_reference(&pc1())
}

#[test]
fn a_missing_object_faults_the_converter_until_it_is_added() {
    let mut db = ObjectDatabase::new();
    db.add(converter(Some(present_value(acc1())))).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    assert!(fault(&db, pc1()));
    // Judged at its own add, so nothing is owed for COV yet.
    assert!(db.take_membership_work_internal().is_empty());

    // An Accumulator's Present_Value is an Unsigned: the fault clears when
    // it arrives, and the converter is owed a COV pass.
    db.add(Box::new(
        AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    assert!(!fault(&db, pc1()));
    assert_eq!(db.take_membership_work_internal().changed, [pc1()]);
    assert!(db.take_membership_work_internal().is_empty());

    // Deleting it brings the fault back.
    db.remove(&acc1()).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    assert_eq!(db.take_membership_work_internal().changed, [pc1()]);
}

#[test]
fn only_unsigned_and_integer_properties_are_inputs() {
    let mut db = ObjectDatabase::new();
    db.add(converter(None)).unwrap();
    db.add(Box::new(
        AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(IntegerValueObject::new(1, "IV-1").unwrap()))
        .unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    let indexed = |target, property: P, index| {
        BACnetObjectPropertyReference::new_indexed(target, property.to_raw(), index)
    };
    let analog_input = oid(ObjectType::ANALOG_INPUT, 1);
    let integer_value = oid(ObjectType::INTEGER_VALUE, 1);
    for (reference, usable, what) in [
        (present_value(acc1()), true, "an Unsigned"),
        (present_value(integer_value), true, "an INTEGER"),
        (present_value(analog_input), false, "a REAL"),
        (
            BACnetObjectPropertyReference::new(acc1(), P::OBJECT_NAME.to_raw()),
            false,
            "a CharacterString",
        ),
        (
            BACnetObjectPropertyReference::new(acc1(), P::HIGH_LIMIT.to_raw()),
            false,
            "a property the object doesn't have",
        ),
        (
            indexed(acc1(), P::PRESENT_VALUE, 1),
            false,
            "an index on a property that isn't an array",
        ),
        (
            indexed(integer_value, P::PRIORITY_ARRAY, 0),
            false,
            "an array's size, which counts no pulses",
        ),
        (
            indexed(integer_value, P::PRIORITY_ARRAY, 17),
            false,
            "an index past the end",
        ),
        (
            BACnetObjectPropertyReference::new(pc1(), P::COUNT.to_raw()),
            false,
            "the converter's own Count",
        ),
    ] {
        write_reference(&mut db, &reference);
        let expected = if usable {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::CONFIGURATION_ERROR
        };
        assert_eq!(reliability(&db, pc1()), expected, "{what}");
        assert_eq!(fault(&db, pc1()), !usable, "{what}");
    }
}

#[test]
fn an_unset_reference_is_no_fault_and_clears_one() {
    let mut db = ObjectDatabase::new();
    db.add(converter(Some(present_value(acc1())))).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    // The object clears it itself when the reference goes, so even a write
    // the database doesn't judge leaves no stale fault.
    db.get_mut(&pc1())
        .unwrap()
        .write_property(
            P::INPUT_REFERENCE,
            None,
            octets(&crate::reference::unset_reference(ObjectType::ACCUMULATOR)),
            None,
        )
        .unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    assert!(!db.check_input_reference(&pc1()));
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn the_verdict_waits_out_of_service_and_applies_on_the_return() {
    let mut db = ObjectDatabase::new();
    db.add(converter(None)).unwrap();
    let out_of_service = |db: &mut ObjectDatabase, value| {
        db.get_mut(&pc1())
            .unwrap()
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(value), None)
            .unwrap();
    };
    out_of_service(&mut db, true);
    // Judged while out of service: Reliability stays decoupled (Clause
    // 12.23.10), so nothing changed.
    assert!(!write_reference(&mut db, &present_value(acc1())));
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    out_of_service(&mut db, false);
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    assert!(fault(&db, pc1()));
}

fn count(db: &ObjectDatabase) -> PropertyValue {
    db.get(&pc1())
        .unwrap()
        .read_property(P::COUNT, None)
        .unwrap()
}

/// Accumulator `instance` with `value` as its Present_Value and 99 as its
/// Max_Pres_Value.
fn accumulator(instance: u32, value: u64) -> Box<AccumulatorObject> {
    let mut object = AccumulatorObject::new(
        instance,
        format!("ACC-{instance}"),
        EngineeringUnits::NO_UNITS,
    )
    .unwrap();
    object
        .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(99), None)
        .unwrap();
    object.set_present_value(value);
    Box::new(object)
}

#[test]
fn each_increase_of_an_accumulator_is_counted_across_its_wrap() {
    let mut db = ObjectDatabase::new();
    db.add(accumulator(1, 80)).unwrap();
    db.add(converter(Some(present_value(acc1())))).unwrap();
    // The first reading only sets the baseline.
    assert!(db.count_pulse_inputs().is_empty());
    assert_eq!(count(&db), PropertyValue::Unsigned(0));
    db.add(accumulator(1, 87)).unwrap();
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(count(&db), PropertyValue::Unsigned(7));
    // No increase, nothing counted.
    assert!(db.count_pulse_inputs().is_empty());
    // The Accumulator counts modulo Max_Pres_Value + 1 (Clause 12.61.4): from
    // 87 to 99 is 12 pulses, one more to 0, and 5 more to 5.
    db.add(accumulator(1, 5)).unwrap();
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(count(&db), PropertyValue::Unsigned(25));
    // Nothing to read drops the baseline, so the object's return counts
    // from its first reading again.
    db.remove(&acc1()).unwrap();
    assert!(db.count_pulse_inputs().is_empty());
    db.add(accumulator(1, 50)).unwrap();
    assert!(db.count_pulse_inputs().is_empty());
    db.add(accumulator(1, 51)).unwrap();
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(count(&db), PropertyValue::Unsigned(26));
    // A new reference starts its own baseline, even at a larger value.
    db.add(accumulator(2, 90)).unwrap();
    write_reference(&mut db, &present_value(oid(ObjectType::ACCUMULATOR, 2)));
    assert!(db.count_pulse_inputs().is_empty());
    assert_eq!(count(&db), PropertyValue::Unsigned(26));
}

#[test]
fn a_decrease_of_any_other_source_only_sets_the_baseline_again() {
    // ACC-1's Max_Pres_Value is an Unsigned with no wrap of its own.
    let mut db = ObjectDatabase::new();
    db.add(accumulator(1, 0)).unwrap();
    let max = BACnetObjectPropertyReference::new(acc1(), P::MAX_PRES_VALUE.to_raw());
    db.add(converter(Some(max))).unwrap();
    let set_max = |db: &mut ObjectDatabase, value| {
        db.get_mut(&acc1())
            .unwrap()
            .write_property(
                P::MAX_PRES_VALUE,
                None,
                PropertyValue::Unsigned(value),
                None,
            )
            .unwrap();
    };
    assert!(db.count_pulse_inputs().is_empty());
    set_max(&mut db, 120);
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(count(&db), PropertyValue::Unsigned(21));
    set_max(&mut db, 10);
    assert!(db.count_pulse_inputs().is_empty());
    set_max(&mut db, 14);
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(count(&db), PropertyValue::Unsigned(25));
}

#[test]
fn a_priority_array_slot_is_judged_by_the_commanded_datatype() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(IntegerValueObject::new(1, "IV-1").unwrap()))
        .unwrap();
    let integer_value = oid(ObjectType::INTEGER_VALUE, 1);
    let slot =
        BACnetObjectPropertyReference::new_indexed(integer_value, P::PRIORITY_ARRAY.to_raw(), 8);
    db.add(converter(Some(slot))).unwrap();
    // Relinquished for now, but an INTEGER Value commands INTEGERs: no fault,
    // and nothing to count.
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    assert!(db.count_pulse_inputs().is_empty());
    let origin = crate::command_source::CommandOrigin::Local {
        owner_device: oid(ObjectType::DEVICE, 1),
        initiating_object: None,
    };
    for value in [3, 10] {
        db.get_mut(&integer_value)
            .unwrap()
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Signed(value),
                Some(8),
                &origin,
            )
            .unwrap();
        db.count_pulse_inputs();
    }
    assert_eq!(count(&db), PropertyValue::Unsigned(7));
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
}

#[test]
fn the_counting_pass_judges_a_reference_changed_past_the_server() {
    let mut db = ObjectDatabase::new();
    db.add(accumulator(1, 0)).unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    db.add(converter(Some(present_value(acc1())))).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    // Written on the object directly, so nothing judged it as it committed.
    db.get_mut(&pc1())
        .unwrap()
        .write_property(
            P::INPUT_REFERENCE,
            None,
            octets(&present_value(oid(ObjectType::ANALOG_INPUT, 1))),
            None,
        )
        .unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::NO_FAULT_DETECTED);
    // The next pass does, and owes COV for the change.
    assert_eq!(db.count_pulse_inputs(), [pc1()]);
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    assert!(db.count_pulse_inputs().is_empty());
}

#[test]
fn reliability_takes_a_clients_value_out_of_service_until_the_return() {
    let mut db = ObjectDatabase::new();
    db.add(converter(Some(present_value(acc1())))).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
    let write = |db: &mut ObjectDatabase, property, value| {
        db.get_mut(&pc1())
            .unwrap()
            .write_property(property, None, value, None)
    };
    let simulated = PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw());
    // In service the verdict owns it (Clause 12.23.10).
    assert!(write(&mut db, P::RELIABILITY, simulated.clone()).is_err());
    write(&mut db, P::OUT_OF_SERVICE, PropertyValue::Boolean(true)).unwrap();
    write(&mut db, P::RELIABILITY, simulated).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::OVER_RANGE);
    // A pass while out of service leaves the client's value.
    db.count_pulse_inputs();
    assert_eq!(reliability(&db, pc1()), Reliability::OVER_RANGE);
    write(&mut db, P::OUT_OF_SERVICE, PropertyValue::Boolean(false)).unwrap();
    assert_eq!(reliability(&db, pc1()), Reliability::CONFIGURATION_ERROR);
}

#[test]
fn a_changed_converter_wakes_the_server_once_per_queued_change() {
    let mut db = ObjectDatabase::new();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    db.set_membership_waker_internal(Some(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })));
    db.add(converter(Some(present_value(acc1())))).unwrap();
    // Unrelated adds queue nothing and wake nobody.
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    db.add(Box::new(
        AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    // Replacing the target judges again; a verdict that stands changes
    // nothing, so it queues nothing.
    db.add(Box::new(
        AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(db.take_membership_work_internal().changed, [pc1()]);
}
