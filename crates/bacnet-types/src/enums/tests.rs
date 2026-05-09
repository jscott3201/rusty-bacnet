use super::*;

#[test]
fn object_type_round_trip() {
    assert_eq!(ObjectType::DEVICE.to_raw(), 8);
    assert_eq!(ObjectType::from_raw(8), ObjectType::DEVICE);
}

#[test]
fn object_type_vendor_proprietary() {
    let vendor = ObjectType::from_raw(128);
    assert_eq!(vendor.to_raw(), 128);
    assert_eq!(format!("{}", vendor), "128");
    assert_eq!(format!("{:?}", vendor), "ObjectType(128)");
}

#[test]
fn object_type_display_known() {
    assert_eq!(format!("{}", ObjectType::ANALOG_INPUT), "ANALOG_INPUT");
    assert_eq!(format!("{:?}", ObjectType::DEVICE), "ObjectType::DEVICE");
}

#[test]
fn property_identifier_round_trip() {
    assert_eq!(PropertyIdentifier::PRESENT_VALUE.to_raw(), 85);
    assert_eq!(
        PropertyIdentifier::from_raw(85),
        PropertyIdentifier::PRESENT_VALUE
    );
}

#[test]
fn property_identifier_vendor() {
    let vendor = PropertyIdentifier::from_raw(512);
    assert_eq!(vendor.to_raw(), 512);
}

#[test]
fn pdu_type_values() {
    assert_eq!(PduType::CONFIRMED_REQUEST.to_raw(), 0);
    assert_eq!(PduType::ABORT.to_raw(), 7);
}

#[test]
fn confirmed_service_choice_values() {
    assert_eq!(ConfirmedServiceChoice::READ_PROPERTY.to_raw(), 12);
    assert_eq!(ConfirmedServiceChoice::WRITE_PROPERTY.to_raw(), 15);
}

#[test]
fn unconfirmed_service_choice_values() {
    assert_eq!(UnconfirmedServiceChoice::WHO_IS.to_raw(), 8);
    assert_eq!(UnconfirmedServiceChoice::I_AM.to_raw(), 0);
}

#[test]
fn bvlc_function_values() {
    assert_eq!(BvlcFunction::ORIGINAL_UNICAST_NPDU.to_raw(), 0x0A);
    assert_eq!(BvlcFunction::ORIGINAL_BROADCAST_NPDU.to_raw(), 0x0B);
}

#[test]
fn engineering_units_round_trip() {
    assert_eq!(EngineeringUnits::DEGREES_CELSIUS.to_raw(), 62);
    assert_eq!(
        EngineeringUnits::from_raw(62),
        EngineeringUnits::DEGREES_CELSIUS
    );
}

#[test]
fn engineering_units_ashrae_extended() {
    assert_eq!(
        EngineeringUnits::STANDARD_CUBIC_FEET_PER_DAY.to_raw(),
        47808
    );
}

#[test]
fn segmentation_values() {
    assert_eq!(Segmentation::BOTH.to_raw(), 0);
    assert_eq!(Segmentation::NONE.to_raw(), 3);
}

#[test]
fn network_message_type_values() {
    assert_eq!(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw(), 0x00);
    assert_eq!(NetworkMessageType::NETWORK_NUMBER_IS.to_raw(), 0x13);
}

#[test]
fn event_state_values() {
    assert_eq!(EventState::NORMAL.to_raw(), 0);
    assert_eq!(EventState::LIFE_SAFETY_ALARM.to_raw(), 5);
}

#[test]
fn reliability_gap_at_11() {
    // Value 11 is intentionally missing from the standard
    assert_eq!(Reliability::CONFIGURATION_ERROR.to_raw(), 10);
    assert_eq!(Reliability::COMMUNICATION_FAILURE.to_raw(), 12);
}
