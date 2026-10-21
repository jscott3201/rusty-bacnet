use super::*;
use crate::clock::ClockFrame;

#[test]
fn pulse_converter_create_and_read_defaults() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(pc.object_name(), "PC-1");
    assert_eq!(
        pc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        pc.read_property(PropertyIdentifier::UNITS, None).unwrap(),
        PropertyValue::Enumerated(62)
    );
}

#[test]
fn pulse_converter_read_write_present_value_while_out_of_service() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    pc.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(123.45),
        None,
    )
    .unwrap();
    assert_eq!(
        pc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(123.45)
    );
}

#[test]
fn pulse_converter_read_scale_factor() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let val = pc
        .read_property(PropertyIdentifier::SCALE_FACTOR, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(1.0));
}

#[test]
fn pulse_converter_write_scale_factor() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.write_property(
        PropertyIdentifier::SCALE_FACTOR,
        None,
        PropertyValue::Real(2.5),
        None,
    )
    .unwrap();
    assert_eq!(
        pc.read_property(PropertyIdentifier::SCALE_FACTOR, None)
            .unwrap(),
        PropertyValue::Real(2.5)
    );
}

#[test]
fn pulse_converter_cov_increment() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(pc.cov_increment(), Some(0.0));
    pc.write_property(
        PropertyIdentifier::COV_INCREMENT,
        None,
        PropertyValue::Real(1.5),
        None,
    )
    .unwrap();
    assert_eq!(
        pc.read_property(PropertyIdentifier::COV_INCREMENT, None)
            .unwrap(),
        PropertyValue::Real(1.5)
    );
    assert_eq!(pc.cov_increment(), Some(1.5));
}

#[test]
fn pulse_converter_object_type() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let val = pc
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::PULSE_CONVERTER.to_raw())
    );
}

#[test]
fn pulse_converter_write_wrong_type_rejected() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let result = pc.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(42),
        None,
    );
    assert!(result.is_err());
}

/// The unset Input_Reference (#1417): [0] accumulator 4194303, [1]
/// present-value.
fn unset() -> PropertyValue {
    PropertyValue::ApplicationData(vec![0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x55])
}

#[test]
fn pulse_converter_input_reference_defaults_unset() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap(),
        unset()
    );
}

#[test]
fn pulse_converter_set_input_reference() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
    let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    pc.set_input_reference(BACnetObjectPropertyReference::new(oid, prop_raw));
    let val = pc
        .read_property(PropertyIdentifier::INPUT_REFERENCE, None)
        .unwrap();
    // The Clause 21 members: [0] accumulator 1, [1] present-value (#1312).
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![0x0C, 0x05, 0xC0, 0x00, 0x01, 0x19, 0x55])
    );
}

#[test]
fn pulse_converter_property_list() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let list = pc.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::SCALE_FACTOR));
    assert!(list.contains(&PropertyIdentifier::ADJUST_VALUE));
    assert!(list.contains(&PropertyIdentifier::COV_INCREMENT));
    assert!(list.contains(&PropertyIdentifier::INPUT_REFERENCE));
}

// --- #182: framed (context-tagged) Input_Reference writes ---

#[test]
fn pulse_converter_write_framed_indexed_input_reference_lands() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
    let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let r = BACnetObjectPropertyReference::new_indexed(oid, prop_raw, 4);
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(&mut buf, &r);
    pc.write_property(
        PropertyIdentifier::INPUT_REFERENCE,
        None,
        PropertyValue::ApplicationData(buf.to_vec()),
        None,
    )
    .unwrap();
    // It reads back as the octets written, the index member [2] included.
    assert_eq!(
        pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap(),
        PropertyValue::ApplicationData(buf.to_vec())
    );
    // Null is no reference, so it is refused as another datatype and the
    // reference stays (#1417).
    let refusal = pc
        .write_property(
            PropertyIdentifier::INPUT_REFERENCE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap_err();
    assert!(
        matches!(refusal, Error::Protocol { class, code }
            if class == bacnet_types::enums::ErrorClass::PROPERTY
                && code == bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE),
        "{refusal:?}"
    );
    assert_eq!(
        pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap(),
        PropertyValue::ApplicationData(buf.to_vec())
    );
    // The unset form it reads as without one clears it.
    pc.write_property(PropertyIdentifier::INPUT_REFERENCE, None, unset(), None)
        .unwrap();
    assert_eq!(
        pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap(),
        unset()
    );
}

#[test]
fn pulse_converter_write_malformed_framed_input_reference_rejected() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    // [0] object-identifier alone — property-identifier missing.
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut buf,
        0,
        &ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap(),
    );
    match pc
        .write_property(
            PropertyIdentifier::INPUT_REFERENCE,
            None,
            PropertyValue::ApplicationData(buf.to_vec()),
            None,
        )
        .unwrap_err()
    {
        Error::Protocol { class, code } => {
            assert_eq!(class, bacnet_types::enums::ErrorClass::PROPERTY);
            assert_eq!(code, bacnet_types::enums::ErrorCode::INVALID_DATA_ENCODING);
        }
        other => panic!("expected PROPERTY/INVALID_DATA_ENCODING, got {other:?}"),
    }
    assert_eq!(
        pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
            .unwrap(),
        unset()
    );
}

#[test]
fn pulse_converter_flat_input_reference_write_is_refused_and_changes_nothing() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
    let prop_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();
    pc.set_input_reference(BACnetObjectPropertyReference::new(oid, prop_raw));
    let before = pc
        .read_property(PropertyIdentifier::INPUT_REFERENCE, None)
        .unwrap();
    // The flat list reads used to serve is another datatype (#1312).
    for flat in [
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Enumerated(prop_raw),
        ]),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Enumerated(prop_raw),
            PropertyValue::Unsigned(4),
        ]),
    ] {
        match pc
            .write_property(PropertyIdentifier::INPUT_REFERENCE, None, flat, None)
            .unwrap_err()
        {
            Error::Protocol { class, code } => {
                assert_eq!(class, bacnet_types::enums::ErrorClass::PROPERTY);
                assert_eq!(code, bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE);
            }
            other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
        }
        assert_eq!(
            pc.read_property(PropertyIdentifier::INPUT_REFERENCE, None)
                .unwrap(),
            before
        );
    }
}

// --- #1092: Count, the count timestamps and Count_Before_Change ---

/// A clock that always reads the same valid frame.
struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
    }
}

fn frame(hour: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        local_time: Time {
            hour,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

fn datetime(frame: ClockFrame) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Date(frame.local_date),
        PropertyValue::Time(frame.local_time),
    ])
}

fn unspecified() -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Date(UNSPECIFIED_DATETIME.0),
        PropertyValue::Time(UNSPECIFIED_DATETIME.1),
    ])
}

fn read(pc: &PulseConverterObject, property: PropertyIdentifier) -> PropertyValue {
    pc.read_property(property, None).unwrap()
}

fn write(
    pc: &mut PulseConverterObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    pc.write_property(property, None, value, None)
}

fn assert_value_out_of_range(result: Result<(), Error>) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, bacnet_types::enums::ErrorClass::PROPERTY);
            assert_eq!(code, bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE);
        }
        other => panic!("expected PROPERTY/VALUE_OUT_OF_RANGE, got {other:?}"),
    }
}

/// Count, Count_Before_Change, Adjust_Value and the two timestamps, in that order.
fn count_state(pc: &PulseConverterObject) -> [PropertyValue; 5] {
    [
        read(pc, PropertyIdentifier::COUNT),
        read(pc, PropertyIdentifier::COUNT_BEFORE_CHANGE),
        read(pc, PropertyIdentifier::ADJUST_VALUE),
        read(pc, PropertyIdentifier::UPDATE_TIME),
        read(pc, PropertyIdentifier::COUNT_CHANGE_TIME),
    ]
}

#[test]
fn pulse_converter_count_rows_start_at_zero_and_unspecified() {
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT_BEFORE_CHANGE),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(read(&pc, PropertyIdentifier::UPDATE_TIME), unspecified());
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT_CHANGE_TIME),
        unspecified()
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::COV_PERIOD),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn pulse_converter_present_value_is_count_times_scale_factor() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(9)))));
    pc.add_pulses(55).unwrap();
    assert_eq!(pc.count(), 55);
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT),
        PropertyValue::Unsigned(55)
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::UPDATE_TIME),
        datetime(frame(9))
    );
    // Accumulating input is not an adjustment.
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT_CHANGE_TIME),
        unspecified()
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(55.0)
    );
    // Scale_Factor rescales Present_Value at once.
    write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(12.5),
    )
    .unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(687.5)
    );
    // Zero pulses change nothing, Update_Time included.
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(10)))));
    pc.add_pulses(0).unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::UPDATE_TIME),
        datetime(frame(9))
    );
    pc.add_pulses(1).unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::UPDATE_TIME),
        datetime(frame(10))
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(700.0)
    );
}

#[test]
fn pulse_converter_adjust_value_takes_the_truncated_quotient_off_count() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.add_pulses(55).unwrap();
    write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(12.5),
    )
    .unwrap();
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(11)))));
    // 30 / 12.5 = 2.4: the remainder is dropped and Count loses 2.
    write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(30.0),
    )
    .unwrap();
    assert_eq!(
        count_state(&pc),
        [
            PropertyValue::Unsigned(53),
            PropertyValue::Unsigned(55),
            PropertyValue::Real(30.0),
            unspecified(),
            datetime(frame(11)),
        ]
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(662.5)
    );
    // A negative adjustment raises Count; -30 / 12.5 truncates to -2.
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(12)))));
    write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(-30.0),
    )
    .unwrap();
    assert_eq!(pc.count(), 55);
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT_BEFORE_CHANGE),
        PropertyValue::Unsigned(53)
    );
    assert_eq!(
        read(&pc, PropertyIdentifier::COUNT_CHANGE_TIME),
        datetime(frame(12))
    );
    // Adjusting by the whole Present_Value brings Count to zero.
    write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(687.5),
    )
    .unwrap();
    assert_eq!(pc.count(), 0);
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn pulse_converter_adjust_value_out_of_range_changes_nothing() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(8)))));
    pc.add_pulses(3).unwrap();
    write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(1.0),
    )
    .unwrap();
    pc.bind_clock_internal(Some(Arc::new(FixedClock(frame(13)))));
    let before = count_state(&pc);
    // Below zero: 2 - 3.
    assert_value_out_of_range(write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(3.0),
    ));
    assert_eq!(count_state(&pc), before);
    // Past the largest Unsigned, and a quotient too large for any Count.
    pc.add_pulses(u64::MAX - 2).unwrap();
    let before = count_state(&pc);
    for adjust in [-1.0, f32::MAX, -f32::MAX] {
        assert_value_out_of_range(write(
            &mut pc,
            PropertyIdentifier::ADJUST_VALUE,
            PropertyValue::Real(adjust),
        ));
        assert_eq!(count_state(&pc), before, "{adjust}");
    }
    // A zero Scale_Factor leaves the quotient undefined.
    write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(0.0),
    )
    .unwrap();
    for adjust in [0.0, 1.0] {
        assert_value_out_of_range(write(
            &mut pc,
            PropertyIdentifier::ADJUST_VALUE,
            PropertyValue::Real(adjust),
        ));
        assert_eq!(count_state(&pc), before, "{adjust}");
    }
}

#[test]
fn pulse_converter_refuses_a_count_or_scale_past_the_largest_real() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.add_pulses(u64::MAX).unwrap();
    let update_time = read(&pc, PropertyIdentifier::UPDATE_TIME);
    assert_value_out_of_range(pc.add_pulses(1));
    assert_eq!(pc.count(), u64::MAX);
    assert_eq!(read(&pc, PropertyIdentifier::UPDATE_TIME), update_time);
    // About 1.8e19 pulses times 1e20 overflows a REAL.
    assert_value_out_of_range(write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(1e20),
    ));
    assert_eq!(
        read(&pc, PropertyIdentifier::SCALE_FACTOR),
        PropertyValue::Real(1.0)
    );
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(f32::MAX),
    )
    .unwrap();
    pc.add_pulses(1).unwrap();
    assert_value_out_of_range(pc.add_pulses(1));
    assert_eq!(pc.count(), 1);
}

#[test]
fn pulse_converter_out_of_service_decouples_present_value_from_count() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.add_pulses(10).unwrap();
    write(
        &mut pc,
        PropertyIdentifier::SCALE_FACTOR,
        PropertyValue::Real(2.0),
    )
    .unwrap();
    write(
        &mut pc,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    // Present_Value holds where it was while Count keeps moving.
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(20.0)
    );
    pc.add_pulses(5).unwrap();
    write(
        &mut pc,
        PropertyIdentifier::ADJUST_VALUE,
        PropertyValue::Real(4.0),
    )
    .unwrap();
    assert_eq!(pc.count(), 13);
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(20.0)
    );
    // A second TRUE write does not refreeze the simulated value.
    write(
        &mut pc,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(99.0),
    )
    .unwrap();
    write(
        &mut pc,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(99.0)
    );
    // Back in service, Present_Value follows Count again.
    write(
        &mut pc,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(
        read(&pc, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(26.0)
    );
}

#[test]
fn pulse_converter_count_rows_are_read_only_over_the_network() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.add_pulses(4).unwrap();
    for (property, value) in [
        (PropertyIdentifier::COUNT, PropertyValue::Unsigned(9)),
        (
            PropertyIdentifier::COUNT_BEFORE_CHANGE,
            PropertyValue::Unsigned(9),
        ),
        (PropertyIdentifier::UPDATE_TIME, unspecified()),
        (PropertyIdentifier::COUNT_CHANGE_TIME, unspecified()),
        (PropertyIdentifier::COV_PERIOD, PropertyValue::Unsigned(60)),
    ] {
        assert!(!pc.is_writable_property(property), "{property:?}");
        match write(&mut pc, property, value) {
            Err(Error::Protocol { class, code }) => {
                assert_eq!(class, bacnet_types::enums::ErrorClass::PROPERTY);
                assert_eq!(code, bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED);
            }
            other => panic!("{property:?}: expected WRITE_ACCESS_DENIED, got {other:?}"),
        }
    }
    assert_eq!(pc.count(), 4);
    assert_eq!(
        read(&pc, PropertyIdentifier::COV_PERIOD),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn pulse_converter_cov_reports_update_time() {
    use crate::traits::CovReportedProperty::Value;
    let pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    // Table 13-1 lists Update_Time after Present_Value and Status_Flags; a
    // change of it alone sends no notification.
    assert_eq!(
        pc.cov_reported_properties(),
        [Value(PropertyIdentifier::UPDATE_TIME)]
    );
    assert!(pc
        .property_list()
        .contains(&PropertyIdentifier::UPDATE_TIME));
}
