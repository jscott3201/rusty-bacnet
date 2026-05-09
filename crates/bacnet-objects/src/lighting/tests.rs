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
        PropertyValue::Enumerated(4), // fade-on
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

// --- ChannelObject ---

#[test]
fn channel_create() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    assert_eq!(obj.object_name(), "CH-1");
    assert_eq!(obj.object_identifier().object_type(), ObjectType::CHANNEL);
    assert_eq!(obj.object_identifier().instance_number(), 1);
}

#[test]
fn channel_read_present_value() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
    assert_eq!(pv.unwrap(), PropertyValue::Unsigned(0));
}

#[test]
fn channel_read_object_type() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(ot, PropertyValue::Enumerated(ObjectType::CHANNEL.to_raw()));
}

#[test]
fn channel_write_present_value() {
    let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(42),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Unsigned(42));

    // Verify last_priority was updated
    let lp = obj
        .read_property(PropertyIdentifier::LAST_PRIORITY, None)
        .unwrap();
    assert_eq!(lp, PropertyValue::Unsigned(8));
}

#[test]
fn channel_read_channel_number() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let cn = obj
        .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
        .unwrap();
    assert_eq!(cn, PropertyValue::Unsigned(5));
}

#[test]
fn channel_write_channel_number() {
    let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    obj.write_property(
        PropertyIdentifier::CHANNEL_NUMBER,
        None,
        PropertyValue::Unsigned(10),
        None,
    )
    .unwrap();
    let cn = obj
        .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
        .unwrap();
    assert_eq!(cn, PropertyValue::Unsigned(10));
}

#[test]
fn channel_read_write_status() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let ws = obj
        .read_property(PropertyIdentifier::WRITE_STATUS, None)
        .unwrap();
    assert_eq!(ws, PropertyValue::Enumerated(0)); // idle
}

#[test]
fn channel_read_last_priority_default() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let lp = obj
        .read_property(PropertyIdentifier::LAST_PRIORITY, None)
        .unwrap();
    assert_eq!(lp, PropertyValue::Unsigned(16)); // default priority
}

#[test]
fn channel_property_list() {
    let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::LAST_PRIORITY));
    assert!(props.contains(&PropertyIdentifier::WRITE_STATUS));
    assert!(props.contains(&PropertyIdentifier::CHANNEL_NUMBER));
    assert!(props.contains(&PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
}

#[test]
fn channel_write_pv_default_priority() {
    let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
    // Write without explicit priority — defaults to 16
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(99),
        None,
    )
    .unwrap();
    let lp = obj
        .read_property(PropertyIdentifier::LAST_PRIORITY, None)
        .unwrap();
    assert_eq!(lp, PropertyValue::Unsigned(16));
}
