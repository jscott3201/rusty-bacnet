use super::*;

// --- LightingOutputObject ---

#[test]
fn lighting_output_create() {
    let obj = LightingOutputObject::new(1, "LO-1").unwrap();
    assert_eq!(obj.object_name(), "LO-1");
    assert_eq!(
        obj.object_identifier().object_type(),
        ObjectType::LIGHTING_OUTPUT
    );
    assert_eq!(obj.object_identifier().instance_number(), 1);
}

#[test]
fn lighting_output_read_present_value() {
    let obj = LightingOutputObject::new(1, "LO-1").unwrap();
    let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
    assert_eq!(pv.unwrap(), PropertyValue::Real(0.0));
}

#[test]
fn lighting_output_read_object_type() {
    let obj = LightingOutputObject::new(1, "LO-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::LIGHTING_OUTPUT.to_raw())
    );
}

#[test]
fn lighting_output_write_pv_commandable() {
    let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
    // Write at priority 8
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(75.0),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Real(75.0));

    // Write at priority 1 (higher) overrides
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Real(50.0));

    // Relinquish priority 1 — falls back to priority 8 value
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Real(75.0));
}

#[test]
fn lighting_output_pv_out_of_range() {
    let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
    let result = obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(101.0),
        Some(16),
    );
    assert!(result.is_err());

    let result = obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(-1.0),
        Some(16),
    );
    assert!(result.is_err());
}

#[test]
fn lighting_output_priority_array_read() {
    let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(8),
    )
    .unwrap();

    // Read array size (index 0)
    let size = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(16));

    // Read slot 8
    let slot = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Real(50.0));

    // Read empty slot 1
    let slot = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .unwrap();
    assert_eq!(slot, PropertyValue::Null);
}

#[test]
fn lighting_output_priority_array_direct_write() {
    let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Real(33.0),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Real(33.0));
}

#[test]
fn lighting_output_relinquish_default() {
    let obj = LightingOutputObject::new(1, "LO-1").unwrap();
    let rd = obj
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();
    assert_eq!(rd, PropertyValue::Real(0.0));
}

#[test]
fn lighting_output_lighting_properties() {
    let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();

    // TRACKING_VALUE
    let tv = obj
        .read_property(PropertyIdentifier::TRACKING_VALUE, None)
        .unwrap();
    assert_eq!(tv, PropertyValue::Real(0.0));

    // LIGHTING_COMMAND
    let lc = obj
        .read_property(PropertyIdentifier::LIGHTING_COMMAND, None)
        .unwrap();
    assert_eq!(lc, PropertyValue::OctetString(vec![]));

    // Write LIGHTING_COMMAND
    obj.write_property(
        PropertyIdentifier::LIGHTING_COMMAND,
        None,
        PropertyValue::OctetString(vec![0x01, 0x02]),
        None,
    )
    .unwrap();
    let lc = obj
        .read_property(PropertyIdentifier::LIGHTING_COMMAND, None)
        .unwrap();
    assert_eq!(lc, PropertyValue::OctetString(vec![0x01, 0x02]));

    // LIGHTING_COMMAND_DEFAULT_PRIORITY
    let lcdp = obj
        .read_property(PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY, None)
        .unwrap();
    assert_eq!(lcdp, PropertyValue::Unsigned(16));

    // IN_PROGRESS
    let ip = obj
        .read_property(PropertyIdentifier::IN_PROGRESS, None)
        .unwrap();
    assert_eq!(ip, PropertyValue::Enumerated(0));

    // BLINK_WARN_ENABLE
    let bwe = obj
        .read_property(PropertyIdentifier::BLINK_WARN_ENABLE, None)
        .unwrap();
    assert_eq!(bwe, PropertyValue::Boolean(false));

    // EGRESS_TIME
    let et = obj
        .read_property(PropertyIdentifier::EGRESS_TIME, None)
        .unwrap();
    assert_eq!(et, PropertyValue::Unsigned(0));

    // EGRESS_ACTIVE
    let ea = obj
        .read_property(PropertyIdentifier::EGRESS_ACTIVE, None)
        .unwrap();
    assert_eq!(ea, PropertyValue::Boolean(false));
}

#[test]
fn lighting_output_property_list() {
    let obj = LightingOutputObject::new(1, "LO-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::TRACKING_VALUE));
    assert!(props.contains(&PropertyIdentifier::LIGHTING_COMMAND));
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// --- BinaryLightingOutputObject ---

#[test]
fn binary_lighting_output_create() {
    let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    assert_eq!(obj.object_name(), "BLO-1");
    assert_eq!(
        obj.object_identifier().object_type(),
        ObjectType::BINARY_LIGHTING_OUTPUT
    );
    assert_eq!(obj.object_identifier().instance_number(), 1);
}

#[test]
fn binary_lighting_output_read_present_value() {
    let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
    assert_eq!(pv.unwrap(), PropertyValue::Enumerated(0)); // off
}

#[test]
fn binary_lighting_output_read_object_type() {
    let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::BINARY_LIGHTING_OUTPUT.to_raw())
    );
}

#[test]
fn binary_lighting_output_write_pv_commandable() {
    let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    // Write on (1) at priority 8
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(1));

    // Write warn (2) at priority 1 overrides
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2),
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(2));

    // Relinquish priority 1 — falls back to priority 8 (on)
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(1));
}

#[test]
fn binary_lighting_output_pv_out_of_range() {
    let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let result = obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(5), // > MAX_PV
        Some(16),
    );
    assert!(result.is_err());
}

#[test]
fn binary_lighting_output_all_valid_pv_values() {
    let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    for val in 0..=4 {
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(val),
            Some(16),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Enumerated(val));
    }
}

#[test]
fn binary_lighting_output_priority_array() {
    let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(5),
    )
    .unwrap();

    // Read array size
    let size = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(16));

    // Read slot 5
    let slot = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
        .unwrap();
    assert_eq!(slot, PropertyValue::Enumerated(1));

    // Read empty slot 1
    let slot = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .unwrap();
    assert_eq!(slot, PropertyValue::Null);
}

#[test]
fn binary_lighting_output_priority_array_direct_write() {
    let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(3),
        PropertyValue::Enumerated(4), // warn-relinquish
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(4));
}

#[test]
fn binary_lighting_output_property_list() {
    let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::BLINK_WARN_ENABLE));
    assert!(props.contains(&PropertyIdentifier::EGRESS_TIME));
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// ---------------------------------------------------------------------------
// #270 — writable Relinquish_Default (Lighting Output + Binary Lighting Output)
// ---------------------------------------------------------------------------

/// Lighting Output: Relinquish_Default is validated like a commanded
/// Present_Value (finite Real within the 0..=100 light level) and — with an
/// all-NULL priority array — Present_Value immediately resolves to it.
#[test]
fn lighting_output_relinquish_default_write_recaptures_present_value() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    assert!(lo.is_writable_property(PropertyIdentifier::RELINQUISH_DEFAULT));

    lo.write_property(
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        PropertyValue::Real(75.0),
        None,
    )
    .unwrap();
    assert_eq!(
        lo.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Real(75.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(75.0),
        "with an empty priority array, PV must resolve to the written default"
    );

    for value in [
        PropertyValue::Real(-1.0),     // below the light level range
        PropertyValue::Real(100.5),    // above it
        PropertyValue::Real(f32::NAN), // non-finite
        PropertyValue::Enumerated(1),  // wrong type
    ] {
        assert!(lo
            .write_property(PropertyIdentifier::RELINQUISH_DEFAULT, None, value, None)
            .is_err());
        assert_eq!(
            lo.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
                .unwrap(),
            PropertyValue::Real(75.0),
            "refused writes must leave Relinquish_Default untouched"
        );
    }
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(75.0)
    );
}

fn assert_property_error(error: Error, expected_code: bacnet_types::enums::ErrorCode) {
    match error {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
            );
            assert_eq!(code, expected_code.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / {expected_code:?}, got {other:?}"),
    }
}

fn assert_binary_lighting_output_command_state(
    blo: &BinaryLightingOutputObject,
    relinquish_default: u32,
    present_value: u32,
    priority_8: PropertyValue,
) {
    assert_eq!(
        blo.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Enumerated(relinquish_default)
    );
    assert_eq!(
        blo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(present_value)
    );
    assert_eq!(
        blo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        priority_8
    );
}

/// Binary Lighting Output Relinquish_Default admits only OFF/ON and recaptures
/// Present_Value immediately while the priority array is empty.
#[test]
fn binary_lighting_output_relinquish_default_accepts_off_on_and_recaptures_present_value() {
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    assert!(blo.is_writable_property(PropertyIdentifier::RELINQUISH_DEFAULT));

    for value in [1, 0] {
        blo.set_relinquish_default(value).unwrap();
        assert_binary_lighting_output_command_state(&blo, value, value, PropertyValue::Null);
    }
}

#[test]
fn binary_lighting_output_relinquish_default_rejects_non_binary_values_atomically() {
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.set_relinquish_default(1).unwrap();
    blo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        Some(8),
    )
    .unwrap();

    for value in [2, 3, 4, 5, 255, u32::MAX] {
        assert_property_error(
            blo.set_relinquish_default(value)
                .expect_err("non-binary local default must be refused"),
            bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_binary_lighting_output_command_state(&blo, 1, 0, PropertyValue::Enumerated(0));

        assert_property_error(
            blo.write_property(
                PropertyIdentifier::RELINQUISH_DEFAULT,
                None,
                PropertyValue::Enumerated(value),
                None,
            )
            .expect_err("non-binary object write must be refused"),
            bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_binary_lighting_output_command_state(&blo, 1, 0, PropertyValue::Enumerated(0));
    }
}

#[test]
fn binary_lighting_output_relinquish_default_wrong_type_is_atomic() {
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.set_relinquish_default(1).unwrap();
    blo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        Some(8),
    )
    .unwrap();

    assert_property_error(
        blo.write_property(
            PropertyIdentifier::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .expect_err("wrong Relinquish_Default datatype must be refused"),
        bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE,
    );
    assert_binary_lighting_output_command_state(&blo, 1, 0, PropertyValue::Enumerated(0));
}

#[test]
fn binary_lighting_output_active_command_outranks_relinquish_default() {
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        Some(8),
    )
    .unwrap();

    blo.set_relinquish_default(1).unwrap();
    assert_binary_lighting_output_command_state(&blo, 1, 0, PropertyValue::Enumerated(0));

    blo.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_binary_lighting_output_command_state(&blo, 1, 1, PropertyValue::Null);
}
